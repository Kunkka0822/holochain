use crate::core::ribosome::error::RibosomeResult;
use crate::core::ribosome::{CallContext, RibosomeT};
use crate::core::state::cascade::error::CascadeResult;
use crate::core::workflow::CallZomeWorkspace;
use futures::future::FutureExt;
use holochain_zome_types::Entry;
use holochain_zome_types::GetEntryInput;
use holochain_zome_types::GetEntryOutput;
use must_future::MustBoxFuture;
use std::sync::Arc;

#[allow(clippy::extra_unused_lifetimes)]
pub fn get_entry<'a>(
    _ribosome: Arc<impl RibosomeT>,
    call_context: Arc<CallContext>,
    input: GetEntryInput,
) -> RibosomeResult<GetEntryOutput> {
    let (hash, options) = input.into_inner();

    // Get the network from the context
    let network = call_context.host_access.network().clone();

    let call =
        |workspace: &'a mut CallZomeWorkspace| -> MustBoxFuture<'a, CascadeResult<Option<Entry>>> {
            async move {
                let mut cascade = workspace.cascade(network);
                Ok(cascade
                    .dht_get(hash.into(), options.into())
                    .await?
                    .and_then(|e| e.into_inner().1))
            }
            .boxed()
            .into()
        };
    // timeouts must be handled by the network
    let maybe_entry: Option<Entry> =
        tokio_safe_block_on::tokio_safe_block_forever_on(async move {
            unsafe { call_context.host_access.workspace().apply_mut(call).await }
        })??;
    Ok(GetEntryOutput::new(maybe_entry))
}

// we are relying on the commit entry tests to show the commit/get round trip
// @see commit_entry.rs
