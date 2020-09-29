use crate::*;
use futures::{sink::SinkExt, stream::StreamExt};
use ghost_actor::dependencies::tracing;

/// Tls ALPN identifier for kitsune proxy handshaking
const ALPN_KITSUNE_PROXY_0: &[u8] = b"kitsune-proxy/0";

/// Allow only these cipher suites for kitsune proxy Tls.
static CIPHER_SUITES: &[&rustls::SupportedCipherSuite] = &[
    &rustls::ciphersuite::TLS13_CHACHA20_POLY1305_SHA256,
    &rustls::ciphersuite::TLS13_AES_256_GCM_SHA384,
];

/// Wrap a transport listener sender/receiver in kitsune proxy logic.
pub async fn spawn_kitsune_proxy_listener(
    proxy_config: Arc<ProxyConfig>,
    sub_sender: ghost_actor::GhostSender<TransportListener>,
    mut sub_receiver: TransportIncomingChannelReceiver,
) -> TransportResult<(
    ghost_actor::GhostSender<TransportListener>,
    TransportIncomingChannelReceiver,
)> {
    let (tls, accept_proxy_cb, proxy_url): (TlsConfig, AcceptProxyCallback, Option<ProxyUrl>) =
        match proxy_config.as_ref() {
            ProxyConfig::RemoteProxyClient { tls, proxy_url } => (
                tls.clone(),
                AcceptProxyCallback::reject_all(),
                Some(proxy_url.clone()),
            ),
            ProxyConfig::LocalProxyServer {
                tls,
                accept_proxy_cb,
            } => (tls.clone(), accept_proxy_cb.clone(), None),
        };

    let this_url = sub_sender.bound_url().await?;
    let this_url = ProxyUrl::new(this_url.as_str(), tls.cert_digest.clone())?;

    let builder = ghost_actor::actor_builder::GhostActorBuilder::new();

    let channel_factory = builder.channel_factory().clone();

    let sender = channel_factory
        .create_channel::<TransportListener>()
        .await?;

    let i_s = channel_factory.create_channel::<Internal>().await?;

    let (evt_send, evt_recv) = futures::channel::mpsc::channel(10);

    tokio::task::spawn(
        builder.spawn(
            InnerListen::new(
                channel_factory,
                i_s.clone(),
                this_url,
                tls,
                accept_proxy_cb,
                sub_sender,
                evt_send,
            )
            .await?,
        ),
    );

    if let Some(proxy_url) = proxy_url {
        i_s.req_proxy(proxy_url).await?;
    }

    tokio::task::spawn(async move {
        while let Some((url, write, read)) = sub_receiver.next().await {
            // spawn so we can process incoming requests in parallel
            let i_s = i_s.clone();
            tokio::task::spawn(async move {
                let _ = i_s.incoming_channel(url, write, read).await;
            });
        }

        // Our incoming channels ended,
        // this also indicates we cannot establish outgoing connections.
        // I.e., we need to shut down.
        i_s.ghost_actor_shutdown().await?;

        TransportResult::Ok(())
    });

    Ok((sender, evt_recv))
}

pub(crate) fn gen_tls_configs(
    tls: &TlsConfig,
) -> TransportResult<(Arc<rustls::ServerConfig>, Arc<rustls::ClientConfig>)> {
    let cert = rustls::Certificate(tls.cert.0.to_vec());
    let cert_priv_key = rustls::PrivateKey(tls.cert_priv_key.0.to_vec());

    let mut tls_server_config =
        rustls::ServerConfig::with_ciphersuites(rustls::NoClientAuth::new(), CIPHER_SUITES);
    tls_server_config
        .set_single_cert(vec![cert], cert_priv_key)
        .map_err(TransportError::other)?;
    tls_server_config.set_protocols(&[ALPN_KITSUNE_PROXY_0.to_vec()]);
    let tls_server_config = Arc::new(tls_server_config);

    let mut tls_client_config = rustls::ClientConfig::with_ciphersuites(CIPHER_SUITES);
    tls_client_config
        .dangerous()
        .set_certificate_verifier(TlsServerVerifier::new());
    tls_client_config.set_protocols(&[ALPN_KITSUNE_PROXY_0.to_vec()]);
    let tls_client_config = Arc::new(tls_client_config);

    Ok((tls_server_config, tls_client_config))
}

#[allow(dead_code)]
struct InnerListen {
    channel_factory: ghost_actor::actor_builder::GhostActorChannelFactory<Self>,
    i_s: ghost_actor::GhostSender<Internal>,
    this_url: ProxyUrl,
    accept_proxy_cb: AcceptProxyCallback,
    sub_sender: ghost_actor::GhostSender<TransportListener>,
    evt_send: TransportIncomingChannelSender,
    tls: TlsConfig,
    tls_server_config: Arc<rustls::ServerConfig>,
    tls_client_config: Arc<rustls::ClientConfig>,
}

impl InnerListen {
    pub async fn new(
        channel_factory: ghost_actor::actor_builder::GhostActorChannelFactory<Self>,
        i_s: ghost_actor::GhostSender<Internal>,
        this_url: ProxyUrl,
        tls: TlsConfig,
        accept_proxy_cb: AcceptProxyCallback,
        sub_sender: ghost_actor::GhostSender<TransportListener>,
        evt_send: TransportIncomingChannelSender,
    ) -> TransportResult<Self> {
        tracing::info!(
            "{}: starting up with this_url: {}",
            this_url.short(),
            this_url
        );

        let (tls_server_config, tls_client_config) = gen_tls_configs(&tls)?;

        Ok(Self {
            channel_factory,
            i_s,
            this_url,
            accept_proxy_cb,
            sub_sender,
            evt_send,
            tls,
            tls_server_config,
            tls_client_config,
        })
    }
}

impl ghost_actor::GhostControlHandler for InnerListen {
    fn handle_ghost_actor_shutdown(mut self) -> MustBoxFuture<'static, ()> {
        async move {
            let _ = self.sub_sender.ghost_actor_shutdown().await;
            self.evt_send.close_channel();
        }
        .boxed()
        .into()
    }
}

ghost_actor::ghost_chan! {
    chan Internal<TransportError> {
        fn incoming_channel(
            base_url: url2::Url2,
            write: TransportChannelWrite,
            read: TransportChannelRead,
        ) -> ();

        fn incoming_req_proxy(
            base_url: url2::Url2,
            write: futures::channel::mpsc::Sender<ProxyWire>,
            read: futures::channel::mpsc::Receiver<ProxyWire>,
        ) -> ();

        fn incoming_chan_new(
            base_url: url2::Url2,
            dest_proxy_url: ProxyUrl,
            write: futures::channel::mpsc::Sender<ProxyWire>,
            read: futures::channel::mpsc::Receiver<ProxyWire>,
        ) -> ();

        fn create_low_level_channel(
            base_url: url2::Url2,
        ) -> (
            futures::channel::mpsc::Sender<ProxyWire>,
            futures::channel::mpsc::Receiver<ProxyWire>,
        );

        fn req_proxy(proxy_url: ProxyUrl) -> ();
        fn set_proxy_url(proxy_url: ProxyUrl) -> ();
    }
}

impl ghost_actor::GhostHandler<Internal> for InnerListen {}

fn cross_join_channel_forward(
    mut write: futures::channel::mpsc::Sender<ProxyWire>,
    mut read: futures::channel::mpsc::Receiver<ProxyWire>,
) {
    tokio::task::spawn(async move {
        while let Some(msg) = read.next().await {
            // do we need to inspect these??
            // for now just forwarding everything
            write.send(msg).await.map_err(TransportError::other)?;
        }
        TransportResult::Ok(())
    });
}

impl InternalHandler for InnerListen {
    fn handle_incoming_channel(
        &mut self,
        base_url: url2::Url2,
        write: TransportChannelWrite,
        read: TransportChannelRead,
    ) -> InternalHandlerResult<()> {
        let write = wire_write::wrap_wire_write(write);
        let mut read = wire_read::wrap_wire_read(read);
        let i_s = self.i_s.clone();
        Ok(async move {
            match read.next().await {
                Some(ProxyWire::ReqProxy(ReqProxy())) => {
                    i_s.incoming_req_proxy(base_url, write, read).await?;
                }
                Some(ProxyWire::ChanNew(ChanNew(proxy_url))) => {
                    i_s.incoming_chan_new(base_url, proxy_url.into(), write, read)
                        .await?;
                }
                _ => (),
            }
            Ok(())
        }
        .boxed()
        .into())
    }

    fn handle_incoming_req_proxy(
        &mut self,
        _base_url: url2::Url2,
        _write: futures::channel::mpsc::Sender<ProxyWire>,
        _read: futures::channel::mpsc::Receiver<ProxyWire>,
    ) -> InternalHandlerResult<()> {
        unimplemented!()
    }

    fn handle_incoming_chan_new(
        &mut self,
        base_url: url2::Url2,
        dest_proxy_url: ProxyUrl,
        mut write: futures::channel::mpsc::Sender<ProxyWire>,
        read: futures::channel::mpsc::Receiver<ProxyWire>,
    ) -> InternalHandlerResult<()> {
        if dest_proxy_url == self.this_url {
            // Hey! They're trying to talk to us!
            // Let's connect them to our owner.
            tls_srv::spawn_tls_server(
                base_url,
                self.tls_server_config.clone(),
                self.evt_send.clone(),
                write,
                read,
            );
            return Ok(async move { Ok(()) }.boxed().into());
        }
        // if we are proxying - forward to another channel
        let fut = self
            .i_s
            .create_low_level_channel(dest_proxy_url.as_base().clone());
        Ok(async move {
            let (fwd_write, fwd_read) = match fut.await {
                Err(e) => {
                    write
                        .send(ProxyWire::failure(format!("{:?}", e)))
                        .await
                        .map_err(TransportError::other)?;
                    return Ok(());
                }
                Ok(t) => t,
            };
            cross_join_channel_forward(fwd_write, read);
            cross_join_channel_forward(write, fwd_read);
            Ok(())
        }
        .boxed()
        .into())
    }

    fn handle_create_low_level_channel(
        &mut self,
        base_url: url2::Url2,
    ) -> InternalHandlerResult<(
        futures::channel::mpsc::Sender<ProxyWire>,
        futures::channel::mpsc::Receiver<ProxyWire>,
    )> {
        let fut = self.sub_sender.create_channel(base_url);
        Ok(async move {
            let (_url, write, read) = fut.await?;
            let write = wire_write::wrap_wire_write(write);
            let read = wire_read::wrap_wire_read(read);
            Ok((write, read))
        }
        .boxed()
        .into())
    }

    fn handle_req_proxy(&mut self, proxy_url: ProxyUrl) -> InternalHandlerResult<()> {
        tracing::info!(
            "{}: wishes to proxy through {}:{}",
            self.this_url.short(),
            proxy_url.short(),
            proxy_url
        );
        let fut = self.i_s.create_low_level_channel(proxy_url.into_base());
        let i_s = self.i_s.clone();
        Ok(async move {
            let (mut write, mut read) = fut.await?;

            write
                .send(ProxyWire::req_proxy())
                .await
                .map_err(TransportError::other)?;
            let res = match read.next().await {
                None => return Err("no response to proxy request".into()),
                Some(r) => r,
            };
            let proxy_url = match res {
                ProxyWire::ReqProxyOk(ReqProxyOk(proxy_url)) => proxy_url,
                ProxyWire::Failure(Failure(reason)) => {
                    return Err(format!("err response to proxy request: {:?}", reason).into());
                }
                _ => return Err(format!("unexpected: {:?}", res).into()),
            };
            i_s.set_proxy_url(proxy_url.into()).await?;
            Ok(())
        }
        .boxed()
        .into())
    }

    fn handle_set_proxy_url(&mut self, proxy_url: ProxyUrl) -> InternalHandlerResult<()> {
        self.this_url = proxy_url;
        Ok(async move { Ok(()) }.boxed().into())
    }
}

impl ghost_actor::GhostHandler<TransportListener> for InnerListen {}

impl TransportListenerHandler for InnerListen {
    fn handle_bound_url(&mut self) -> TransportListenerHandlerResult<url2::Url2> {
        let this_url: url2::Url2 = (&self.this_url).into();
        Ok(async move { Ok(this_url) }.boxed().into())
    }

    fn handle_create_channel(
        &mut self,
        _url: url2::Url2,
    ) -> TransportListenerHandlerResult<(url2::Url2, TransportChannelWrite, TransportChannelRead)>
    {
        unimplemented!()
    }
}

struct TlsServerVerifier;

impl TlsServerVerifier {
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl rustls::ServerCertVerifier for TlsServerVerifier {
    fn verify_server_cert(
        &self,
        _roots: &rustls::RootCertStore,
        _presented_certs: &[rustls::Certificate],
        _dns_name: webpki::DNSNameRef,
        _ocsp_response: &[u8],
    ) -> Result<rustls::ServerCertVerified, rustls::TLSError> {
        // TODO - check acceptable cert digest

        Ok(rustls::ServerCertVerified::assertion())
    }
}

/*
#[cfg(test)]
mod tests {
    use super::*;

    fn init_tracing() {
        let _ = ghost_actor::dependencies::tracing::subscriber::set_global_default(
            tracing_subscriber::FmtSubscriber::builder()
                .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
                .finish(),
        );
    }

    #[tokio::test(threaded_scheduler)]
    async fn test_inner_listen() {
        if let Err(e) = test_inner_listen_inner().await {
            panic!("{:?}", e);
        }
    }

    async fn connect(
        proxy_config: Arc<ProxyConfig>,
    ) -> TransportResult<ghost_actor::GhostSender<TransportListener>> {
        let (bind, evt) = kitsune_p2p_types::transport_mem::spawn_bind_transport_mem().await?;
        let addr = bind.bound_url().await?;
        tracing::warn!("got bind: {}", addr);

        let (bind, _evt) = spawn_kitsune_proxy_listener(proxy_config, bind, evt).await?;
        let addr = bind.bound_url().await?;
        tracing::warn!("got proxy: {}", addr);

        Ok(bind)
    }

    async fn test_inner_listen_inner() -> TransportResult<()> {
        init_tracing();

        let proxy_config1 = ProxyConfig::local_proxy_server(
            TlsConfig::new_ephemeral().await?,
            AcceptProxyCallback::accept_all(),
        );
        let bind1 = connect(proxy_config1).await?;
        let addr1 = bind1.bound_url().await?;

        let proxy_config2 = ProxyConfig::local_proxy_server(
            TlsConfig::new_ephemeral().await?,
            AcceptProxyCallback::accept_all(),
        );
        let bind2 = connect(proxy_config2).await?;

        let proxy_config3 =
            ProxyConfig::remote_proxy_client(TlsConfig::new_ephemeral().await?, addr1.into());
        let bind3 = connect(proxy_config3).await?;
        let addr3 = bind3.bound_url().await?;

        let (_send, _recv) = bind2.connect(addr3).await?;

        Ok(())
    }
}
*/
