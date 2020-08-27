//! Genesis Workflow: Initialize the source chain with the initial entries:
//! - Dna
//! - AgentValidationPkg
//! - AgentId
//!

// FIXME: understand the details of actually getting the DNA
// FIXME: creating entries in the config db

use super::error::{WorkflowError, WorkflowResult};
use crate::conductor::api::CellConductorApiT;
use crate::core::{
    queue_consumer::OneshotWriter,
    state::{
        source_chain::SourceChainBuf,
        workspace::{Workspace, WorkspaceResult},
    },
};
use derive_more::Constructor;
use holochain_state::prelude::*;
use holochain_types::dna::DnaFile;
use holochain_types::prelude::*;
use tracing::*;

/// The struct which implements the genesis Workflow
#[derive(Constructor, Debug)]
pub struct GenesisWorkflowArgs {
    dna_file: DnaFile,
    agent_pubkey: AgentPubKey,
    membrane_proof: Option<SerializedBytes>,
}

#[instrument(skip(workspace, writer, api))]
pub async fn genesis_workflow<'env, Api: CellConductorApiT>(
    mut workspace: GenesisWorkspace<'env>,
    writer: OneshotWriter,
    api: Api,
    args: GenesisWorkflowArgs,
) -> WorkflowResult<()> {
    genesis_workflow_inner(&mut workspace, args, api).await?;

    // --- END OF WORKFLOW, BEGIN FINISHER BOILERPLATE ---

    // commit the workspace
    writer
        .with_writer(|writer| Ok(workspace.flush_to_txn(writer)?))
        .await?;

    Ok(())
}

async fn genesis_workflow_inner<Api: CellConductorApiT>(
    workspace: &mut GenesisWorkspace<'_>,
    args: GenesisWorkflowArgs,
    api: Api,
) -> WorkflowResult<()> {
    let GenesisWorkflowArgs {
        dna_file,
        agent_pubkey,
        membrane_proof,
    } = args;

    // TODO: this is a placeholder for a real DPKI request to show intent
    if api
        .dpki_request("is_agent_pubkey_valid".into(), agent_pubkey.to_string())
        .await
        .expect("TODO: actually implement this")
        == "INVALID"
    {
        return Err(WorkflowError::AgentInvalid(agent_pubkey.clone()));
    }

    workspace
        .source_chain
        .genesis(
            dna_file.dna_hash().clone(),
            agent_pubkey.clone(),
            membrane_proof,
        )
        .await
        .map_err(WorkflowError::from)?;

    Ok(())
}

/// The workspace for Genesis
pub struct GenesisWorkspace<'env> {
    source_chain: SourceChainBuf<'env>,
}

impl<'env> Workspace<'env> for GenesisWorkspace<'env> {
    /// Constructor
    #[allow(dead_code)]
    fn new(reader: &'env Reader<'env>, dbs: &impl GetDb) -> WorkspaceResult<Self> {
        Ok(Self {
            source_chain: SourceChainBuf::<'env>::new(reader, dbs)?,
        })
    }
    fn flush_to_txn(self, writer: &mut Writer) -> WorkspaceResult<()> {
        self.source_chain.flush_to_txn(writer)?;
        Ok(())
    }
}

#[cfg(test)]
pub mod tests {

    use super::*;
    use crate::core::state::workspace::Workspace;
    use crate::{
        conductor::api::MockCellConductorApi,
        core::{state::source_chain::SourceChain, SourceChainResult},
    };
    use fallible_iterator::FallibleIterator;
    use holochain_state::test_utils::test_cell_env;
    use holochain_types::{
        observability,
        test_utils::{fake_agent_pubkey_1, fake_dna_file},
    };
    use holochain_zome_types::Header;
    use matches::assert_matches;

    pub async fn fake_genesis(source_chain: &mut SourceChain<'_>) -> SourceChainResult<()> {
        let dna = fake_dna_file("cool dna");
        let dna_hash = dna.dna_hash().clone();
        let agent_pubkey = fake_agent_pubkey_1();

        source_chain.genesis(dna_hash, agent_pubkey, None).await
    }

    #[tokio::test(threaded_scheduler)]
    async fn genesis_initializes_source_chain() -> Result<(), anyhow::Error> {
        observability::test_run()?;
        let test_env = test_cell_env();
        let arc = test_env.env();
        let env = arc.guard().await;
        let dbs = arc.dbs().await;
        let dna = fake_dna_file("a");
        let agent_pubkey = fake_agent_pubkey_1();

        {
            let reader = env.reader()?;
            let workspace = GenesisWorkspace::new(&reader, &dbs)?;
            let mut api = MockCellConductorApi::new();
            api.expect_sync_dpki_request()
                .returning(|_, _| Ok("mocked dpki request response".to_string()));
            let args = GenesisWorkflowArgs {
                dna_file: dna.clone(),
                agent_pubkey: agent_pubkey.clone(),
                membrane_proof: None,
            };
            let _: () = genesis_workflow(workspace, arc.clone().into(), api, args).await?;
        }

        {
            let reader = env.reader()?;

            let source_chain = SourceChain::new(&reader, &dbs)?;
            assert_eq!(source_chain.agent_pubkey().await?, agent_pubkey);
            source_chain.chain_head().expect("chain head should be set");

            let mut iter = source_chain.iter_back();
            let mut headers = Vec::new();

            while let Some(h) = iter.next().unwrap() {
                let (h, _) = h.into_header_and_signature();
                let (h, _) = h.into_inner();
                headers.push(h);
            }

            assert_matches!(
                headers.as_slice(),
                [Header::EntryCreate(_), Header::AgentValidationPkg(_), Header::Dna(_)]
            );
        }

        Ok(())
    }
}

/* TODO: make doc-able

Called from:

 - Conductor upon first ACTIVATION of an installed DNA (trace: follow)



Parameters (expected types/structures):

- DNA hash to pull from path to file (or HCHC [FUTURE] )

- AgentID [SEEDLING] (already registered in DeepKey [LEAPFROG])

- Membrane Access Payload (optional invitation code / to validate agent join) [possible for LEAPFROG]



Data X (data & structure) from Store Y:

- Get DNA from HCHC by DNA hash

- or Get DNA from filesystem by filename



----

Functions / Workflows:

- check that agent key is valid [MOCKED dpki] (via real dpki [LEAPFROG])

- retrieve DNA from file path [in the future from HCHC]

- initialize lmdb environment and dbs, save to conductor runtime config.

- commit DNA entry (w/ special enum header with NULL  prev_header)

- commit CapGrant for author (agent key) (w/ normal header)



    fn commit_DNA

    fn produce_header



Examples / Tests / Acceptance Criteria:

- check hash of DNA =



----



Persisted X Changes to Store Y (data & structure):

- source chain HEAD 2 new headers

- CAS commit headers and genesis entries: DNA & Author Capabilities Grant (Agent Key)



- bootstrapped peers from attempt to publish key and join network



Spawned Tasks (don't wait for result -signals/log/tracing=follow):

- ZomeCall:init (for processing app initialization with bridges & networking)

- DHT transforms of genesis entries in CAS



Returned Results (type & structure):

- None
*/
