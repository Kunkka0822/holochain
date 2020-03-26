//! internal websocket utility types and code

use crate::*;

use serde::{Deserialize, Serialize};

/// internal socket type
pub(crate) type RawSocket = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

/// internal sink type
pub(crate) type RawSink = futures::stream::SplitSink<RawSocket, tungstenite::Message>;

/// internal stream type
pub(crate) type RawStream = futures::stream::SplitStream<RawSocket>;

/// not sure if we should expose this or not
/// this is the actual wire message that is sent over the websocket.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum WireMessage {
    Signal { data: Vec<u8> },
    Request { id: String, data: Vec<u8> },
    Response { id: String, data: Vec<u8> },
}
try_from_serialized_bytes!(WireMessage);

/// internal helper to convert addrs to urls
pub(crate) fn addr_to_url(a: SocketAddr, scheme: &str) -> Url2 {
    url2!("{}://{}", scheme, a)
}

/// internal helper convert urls to socket addrs for binding / connection
pub(crate) async fn url_to_addr(url: &Url2, scheme: &str) -> Result<SocketAddr> {
    if url.scheme() != scheme || url.host_str().is_none() || url.port().is_none() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("got: '{}', expected: '{}://host:port'", scheme, url),
        ));
    }

    let rendered = format!("{}:{}", url.host_str().unwrap(), url.port().unwrap());

    if let Ok(mut iter) = tokio::net::lookup_host(rendered.clone()).await {
        let mut tmp = iter.next();
        let mut fallback = None;
        loop {
            if tmp.is_none() {
                break;
            }

            if tmp.as_ref().unwrap().is_ipv4() {
                return Ok(tmp.unwrap());
            }

            fallback = tmp;
            tmp = iter.next();
        }
        if let Some(addr) = fallback {
            return Ok(addr);
        }
    }

    Err(Error::new(
        ErrorKind::InvalidInput,
        format!("could not parse '{}', as 'host:port'", rendered),
    ))
}
