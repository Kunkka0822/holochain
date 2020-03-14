use super::{WorkflowEffects, WorkflowError, WorkflowResult};
use crate::{conductor::api::CellConductorApiT, core::state::workspace::GenesisWorkspace};
use sx_types::{agent::AgentId, dna::Dna, entry::Entry};

/// Initialize the source chain with the initial entries:
/// - Dna
/// - AgentId
/// - CapTokenGrant
///
/// FIXME: understand the details of actually getting the DNA
/// FIXME: creating entries in the config db
pub async fn genesis<'env, Api: CellConductorApiT>(
    mut workspace: GenesisWorkspace<'env>,
    api: Api,
    dna: Dna,
    agent_id: AgentId,
) -> WorkflowResult<GenesisWorkspace<'env>> {
    if api
        .dpki_request("is_agent_id_valid".into(), agent_id.pub_sign_key().into())
        .await?
        == "INVALID".to_string()
    {
        return Err(WorkflowError::AgentIdInvalid(agent_id.clone()));
    }

    workspace
        .source_chain
        .put_entry(Entry::Dna(Box::new(dna)), &agent_id);
    workspace
        .source_chain
        .put_entry(Entry::AgentId(agent_id.clone()), &agent_id);

    Ok(WorkflowEffects {
        workspace,
        triggers: Default::default(),
        signals: Default::default(),
        callbacks: Default::default(),
    })
}

#[cfg(test)]
mod tests {

    use super::genesis;
    use crate::{
        conductor::api::MockCellConductorApi,
        core::{
            state::{source_chain::SourceChain, workspace::{GenesisWorkspace, Workspace}},
            test_utils::{fake_agent_id, fake_dna},
            workflow::WorkflowError,
        },
    };
    use sx_types::prelude::*;
    use sx_state::{env::*, test_utils::test_env};

    #[tokio::test]
    async fn genesis_initializes_source_chain() -> Result<(), WorkflowError> {
        let arc = test_env();
        let env = arc.guard().await;
        let dna = fake_dna("a");
        let agent_id = fake_agent_id("a");
        let dbs = arc.dbs().await?;

        {
            let reader = env.reader()?;
            let workspace = GenesisWorkspace::new(&reader, &dbs)?;
            let mut api = MockCellConductorApi::new();
            api
                .expect_sync_dpki_request()
                .returning(|_, _| Ok("mocked dpki request response".to_string()));
            let fx = genesis(workspace, api, dna.clone(), agent_id.clone()).await?;
            let writer = env.writer()?;
            fx.workspace.commit_txn(writer)?;
        }

        env.with_reader(|reader| {
            let source_chain = SourceChain::new(&reader, &dbs)?;
            assert_eq!(source_chain.agent_id()?, agent_id);
            // TODO: implement actual source chain iterator
            let mut it = source_chain.iter_back()?;
            let (_, agent_header) = it.next().unwrap();
            let (_, dna_header) = it.next().unwrap();
            assert!(it.next().is_none());
            assert_eq!(*agent_header.entry_address(), agent_id.address());
            assert_eq!(*dna_header.entry_address(), dna.address());
            Result::<_, WorkflowError>::Ok(())
        })?;
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

- commit CapToken Grant for author (agent key) (w/ normal header)



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
