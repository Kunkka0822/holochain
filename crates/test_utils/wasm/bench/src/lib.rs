//! externs to help bench the wasm ribosome

use hdk3::prelude::*;

/// round trip bytes back to the host
/// useful to see what the basic throughput of our wasm implementation is
#[hdk(extern)]
fn echo_bytes(sb: SerializedBytes) -> ExternResult<SerializedBytes> {
    Ok(sb)
}
