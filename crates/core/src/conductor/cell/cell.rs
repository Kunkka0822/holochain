use crate::conductor::{api::CellConductorApi, cell::error::CellResult};
use std::hash::{Hash, Hasher};
use sx_conductor_api::ConductorApiResult;
use crate::{ribosome::Ribosome, runner::RunnerCellT};
use sx_state::env::Environment;
use sx_types::{
    agent::AgentId,
    autonomic::AutonomicProcess,
    cell::CellId,
    dna::DnaAddress,
    error::SkunkResult,
    nucleus::{ZomeInvocation, ZomeInvocationResponse},
    shims::*,
};

impl Hash for Cell {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        (self.dna_address(), self.agent_id()).hash(state);
    }
}

impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// A Cell is a grouping of the resources necessary to run workflows
/// on behalf of an agent. It does not have a lifetime of its own aside
/// from the lifetimes of the resources which it holds references to.
/// Any work it does is through running a workflow, passing references to
/// the resources needed to complete that workflow.
///
/// XXX: The important methods of the Cell are actually not implemented here!
/// See `impl ApiCellT for Cell` in conductor_lib for the actual implementations,
/// as well as an explanation for why this is the case right now.
pub struct Cell {
    id: CellId,
    state_env: Environment,
}

impl Cell {
    fn dna_address(&self) -> &DnaAddress {
        &self.id.dna_address()
    }

    fn agent_id(&self) -> &AgentId {
        &self.id.agent_id()
    }

    pub async fn handle_network_message(
        &self,
        _msg: Lib3hToClient,
    ) -> CellResult<Option<Lib3hToClientResponse>> {
        unimplemented!()
    }

    pub async fn handle_autonomic_process(&self, process: AutonomicProcess) -> CellResult<()> {
        match process {
            AutonomicProcess::SlowHeal => unimplemented!(),
            AutonomicProcess::HealthCheck => unimplemented!(),
        }
    }

    pub async fn invoke_zome(
        &self,
        _conductor_api: CellConductorApi,
        _invocation: ZomeInvocation,
    ) -> ConductorApiResult<ZomeInvocationResponse> {
        unimplemented!()
    }
}

impl RunnerCellT for Cell {
    fn get_ribosome(&self) -> Ribosome {
        unimplemented!()
    }

    fn state_env(&self) -> Environment {
        self.state_env.clone()
    }
}

////////////////////////////////////////////////////////////////////////////////////
// The following is a sketch from the skunkworx phase, and can probably be removed

// These are possibly composable traits that describe how to get a resource,
// so instead of explicitly building resources, we can downcast a Cell to exactly
// the right set of resource getter traits
trait NetSend {
    fn network_send(&self, msg: Lib3hClientProtocol) -> SkunkResult<()>;
}

/// Simplification of holochain_net::connection::NetSend
/// Could use the trait instead, but we will want an impl of it
/// for just a basic crossbeam_channel::Sender, so I'm simplifying
/// to avoid making a change to holochain_net
///
/// This is just a "sketch", can be removed.
pub type NetSender = tokio::sync::mpsc::Sender<Lib3hClientProtocol>;
