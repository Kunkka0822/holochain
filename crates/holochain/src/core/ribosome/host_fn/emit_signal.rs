use crate::core::ribosome::error::RibosomeResult;
use crate::core::ribosome::wasm_ribosome::WasmRibosome;
use crate::core::ribosome::CallContext;
use holochain_zome_types::EmitSignalInput;
use holochain_zome_types::EmitSignalOutput;
use std::sync::Arc;

pub fn emit_signal(
    _ribosome: Arc<WasmRibosome>,
    _host_context: Arc<CallContext>,
    _input: EmitSignalInput,
) -> RibosomeResult<EmitSignalOutput> {
    unimplemented!();
}
