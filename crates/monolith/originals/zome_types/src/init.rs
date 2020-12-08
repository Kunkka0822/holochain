use crate::holochain_zome_types::zome_io::ExternOutput;
use crate::holochain_zome_types::CallbackResult;
use holo_hash::EntryHash;
use holochain_serialized_bytes::prelude::*;

#[derive(Clone, PartialEq, Serialize, Deserialize, SerializedBytes)]
pub enum InitCallbackResult {
    Pass,
    Fail(String),
    UnresolvedDependencies(Vec<EntryHash>),
}

impl From<ExternOutput> for InitCallbackResult {
    fn from(callback_guest_output: ExternOutput) -> Self {
        match callback_guest_output.into_inner().try_into() {
            Ok(v) => v,
            Err(e) => Self::Fail(format!("{:?}", e)),
        }
    }
}

impl CallbackResult for InitCallbackResult {
    fn is_definitive(&self) -> bool {
        match self {
            InitCallbackResult::Fail(_) => true,
            _ => false,
        }
    }
}
