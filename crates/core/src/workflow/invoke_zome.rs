use crate::cursor::CasCursorX;
use crate::types::ZomeInvocationResult;
use crate::{agent::SourceChain, ribosome::Ribosome, types::ZomeInvocation};
use sx_types::error::SkunkResult;
#[allow(unused_imports)]
use sx_types::shims::*;

pub async fn invoke_zome(
    invocation: ZomeInvocation,
    source_chain: SourceChain,
    cursor: CasCursorX,
) -> SkunkResult<ZomeInvocationResult> {
    let dna = source_chain.get_dna()?;
    let ribosome = Ribosome::new(dna);
    let (result, cursor) = ribosome.call_zome_function(cursor, invocation, source_chain.clone())?;
    source_chain.try_commit(cursor)?;
    Ok(result)
}
