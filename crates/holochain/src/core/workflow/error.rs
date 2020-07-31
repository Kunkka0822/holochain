// Error types are self-explanatory
#![allow(missing_docs)]

use super::produce_dht_ops_workflow::dht_op_light::error::DhtOpConvertError;
use crate::{
    conductor::{api::error::ConductorApiError, CellError},
    core::{
        queue_consumer::QueueTriggerClosedError,
        ribosome::error::RibosomeError,
        state::{source_chain::SourceChainError, workspace::WorkspaceError},
    },
};
use holochain_p2p::HolochainP2pError;
use holochain_state::error::DatabaseError;
use holochain_types::{dht_op::error::DhtOpError, prelude::*};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkflowError {
    #[error("Agent is invalid: {0:?}")]
    AgentInvalid(AgentPubKey),

    #[error("Conductor API error: {0}")]
    ConductorApi(#[from] Box<ConductorApiError>),

    #[error("Workspace error: {0}")]
    WorkspaceError(#[from] WorkspaceError),

    #[error("Database error: {0}")]
    DatabaseError(#[from] DatabaseError),

    #[error(transparent)]
    RibosomeError(#[from] RibosomeError),

    #[error("Source chain error: {0}")]
    SourceChainError(#[from] SourceChainError),

    #[error("Capability token missing")]
    CapabilityMissing,

    #[error(transparent)]
    SerializedBytesError(#[from] SerializedBytesError),

    #[error(transparent)]
    DhtOpConvertError(#[from] DhtOpConvertError),

    #[error(transparent)]
    CellError(#[from] CellError),

    #[error(transparent)]
    QueueTriggerClosedError(#[from] QueueTriggerClosedError),

    #[error(transparent)]
    HolochainP2pError(#[from] HolochainP2pError),

    #[error(transparent)]
    DhtOpError(#[from] DhtOpError),
}

/// Internal type to handle running workflows
pub type WorkflowResult<T> = Result<T, WorkflowError>;
