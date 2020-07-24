use super::error::{WorkflowError, WorkflowResult};
use crate::core::ribosome::guest_callback::validate::ValidateInvocation;
use crate::core::ribosome::guest_callback::validate::{ValidateHostAccess, ValidateResult};
use crate::core::ribosome::ZomeCallInvocation;
use crate::core::ribosome::ZomeCallInvocationResponse;
use crate::core::ribosome::{error::RibosomeResult, RibosomeT, ZomeCallHostAccess};
use crate::core::state::source_chain::SourceChainError;
use crate::core::state::workspace::Workspace;
use crate::core::{
    queue_consumer::{OneshotWriter, TriggerSender},
    state::{
        cascade::Cascade, chain_cas::ChainCasBuf, metadata::MetadataBuf, source_chain::SourceChain,
        workspace::WorkspaceResult,
    },
    sys_validate_element,
};
use fallible_iterator::FallibleIterator;
use holochain_keystore::KeystoreSender;
use holochain_p2p::HolochainP2pCell;
use holochain_state::prelude::*;
use holochain_types::element::ChainElement;
use std::sync::Arc;
use unsafe_invoke_zome_workspace::UnsafeInvokeZomeWorkspace;

pub mod unsafe_invoke_zome_workspace;

/// Placeholder for the return value of a zome invocation
/// TODO: do we want this to be the same as ZomeCallInvocationRESPONSE?
pub type ZomeCallInvocationResult = RibosomeResult<ZomeCallInvocationResponse>;

pub struct InvokeZomeWorkflowArgs<Ribosome: RibosomeT> {
    pub ribosome: Ribosome,
    pub invocation: ZomeCallInvocation,
}

// TODO: #[instrument]
pub async fn invoke_zome_workflow<'env, Ribosome: RibosomeT>(
    mut workspace: InvokeZomeWorkspace<'env>,
    network: HolochainP2pCell,
    keystore: KeystoreSender,
    writer: OneshotWriter,
    args: InvokeZomeWorkflowArgs<Ribosome>,
    mut trigger_produce_dht_ops: TriggerSender,
) -> WorkflowResult<ZomeCallInvocationResult> {
    let result = invoke_zome_workflow_inner(&mut workspace, network, keystore, args).await?;

    // --- END OF WORKFLOW, BEGIN FINISHER BOILERPLATE ---

    // commit the workspace
    writer
        .with_writer(|writer| workspace.flush_to_txn(writer).expect("TODO"))
        .await?;

    trigger_produce_dht_ops.trigger();

    Ok(result)
}

async fn invoke_zome_workflow_inner<'env, Ribosome: RibosomeT>(
    workspace: &mut InvokeZomeWorkspace<'env>,
    network: HolochainP2pCell,
    keystore: KeystoreSender,
    args: InvokeZomeWorkflowArgs<Ribosome>,
) -> WorkflowResult<ZomeCallInvocationResult> {
    let InvokeZomeWorkflowArgs {
        ribosome,
        invocation,
    } = args;

    let zome_name = invocation.zome_name.clone();

    // Get the current head
    let chain_head_start = workspace.source_chain.chain_head()?.clone();

    let agent_key = invocation.provenance.clone();

    tracing::trace!(line = line!());
    // Create the unsafe sourcechain for use with wasm closure
    let result = {
        let (_g, raw_workspace) = UnsafeInvokeZomeWorkspace::from_mut(workspace);
        let host_access = ZomeCallHostAccess::new(raw_workspace, keystore, network);
        ribosome.call_zome_function(host_access, invocation)
    };
    tracing::trace!(line = line!());

    // Get the new head
    let chain_head_end = workspace.source_chain.chain_head()?;

    // collect all the elements we need to validate in wasm
    let mut to_app_validate: Vec<ChainElement> = vec![];

    // Has there been changes?
    if chain_head_start != *chain_head_end {
        // get the changes
        let mut new_headers = workspace
            .source_chain
            .iter_back()
            .scan(None, |current_header, element| {
                let my_header = current_header.clone();
                *current_header = element.header().prev_header().cloned();
                let r = match my_header {
                    Some(current_header) if current_header == chain_head_start => None,
                    _ => Some(element),
                };
                Ok(r)
            })
            .map_err(WorkflowError::from);

        while let Some(header) = new_headers.next()? {
            let chain_element = workspace
                .source_chain
                .get_element(header.header_address())
                .await?;
            let prev_chain_element = match chain_element {
                Some(ref c) => match c.header().prev_header() {
                    Some(h) => workspace.source_chain.get_element(&h).await?,
                    None => None,
                },
                None => None,
            };
            if let Some(ref chain_element) = chain_element {
                sys_validate_element(&agent_key, chain_element, prev_chain_element.as_ref())
                    .await?;
                to_app_validate.push(chain_element.to_owned());
            }
        }
    }

    for chain_element in to_app_validate {
        match chain_element.entry() {
            holochain_types::element::ChainElementEntry::Present(entry) => {
                let validate: ValidateResult = ribosome.run_validate(
                    ValidateHostAccess,
                    ValidateInvocation {
                        zome_name: zome_name.clone(),
                        entry: Arc::new(entry.clone()),
                    },
                )?;
                match validate {
                    ValidateResult::Valid => {}
                    // when the wasm is being called directly in a zome invocation any
                    // state other than valid is not allowed for new entries
                    // e.g. we require that all dependencies are met when committing an
                    // entry to a local source chain
                    // this is different to the case where we are validating data coming in
                    // from the network where unmet dependencies would need to be
                    // rescheduled to attempt later due to partitions etc.
                    ValidateResult::Invalid(reason) => {
                        Err(SourceChainError::InvalidCommit(reason))?
                    }
                    ValidateResult::UnresolvedDependencies(hashes) => {
                        Err(SourceChainError::InvalidCommit(format!("{:?}", hashes)))?
                    }
                }
            }
            // if there is no entry this is a noop
            _ => {}
        }
    }

    Ok(result)
}

pub struct InvokeZomeWorkspace<'env> {
    pub source_chain: SourceChain<'env>,
    pub meta: MetadataBuf<'env>,
    pub cache_cas: ChainCasBuf<'env>,
    pub cache_meta: MetadataBuf<'env>,
}

impl<'env: 'a, 'a> InvokeZomeWorkspace<'env> {
    pub fn cascade(&'a mut self, network: HolochainP2pCell) -> Cascade<'env, 'a> {
        Cascade::new(
            &self.source_chain.cas(),
            &self.meta,
            &mut self.cache_cas,
            &mut self.cache_meta,
            network,
        )
    }
}

impl<'env> Workspace<'env> for InvokeZomeWorkspace<'env> {
    fn new(reader: &'env Reader<'env>, dbs: &impl GetDb) -> WorkspaceResult<Self> {
        let source_chain = SourceChain::new(reader, dbs)?;

        let cache_cas = ChainCasBuf::cache(reader, dbs)?;
        let meta = MetadataBuf::primary(reader, dbs)?;
        let cache_meta = MetadataBuf::cache(reader, dbs)?;

        Ok(InvokeZomeWorkspace {
            source_chain,
            meta,
            cache_cas,
            cache_meta,
        })
    }

    fn flush_to_txn(self, writer: &mut Writer) -> WorkspaceResult<()> {
        self.source_chain.into_inner().flush_to_txn(writer)?;
        self.meta.flush_to_txn(writer)?;
        self.cache_cas.flush_to_txn(writer)?;
        self.cache_meta.flush_to_txn(writer)?;
        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::core::{
        ribosome::MockRibosomeT,
        workflow::{error::WorkflowError, genesis_workflow::tests::fake_genesis},
    };
    use crate::fixt::KeystoreSenderFixturator;
    use fixt::prelude::*;
    use holochain_p2p::HolochainP2pCellFixturator;
    use holochain_serialized_bytes::prelude::*;
    use holochain_state::{env::ReadManager, test_utils::test_cell_env};
    use holochain_types::{observability, test_utils::fake_agent_pubkey_1};
    use holochain_wasm_test_utils::TestWasm;
    use holochain_zome_types::entry::Entry;
    use holochain_zome_types::GuestOutput;
    use holochain_zome_types::HostInput;
    use matches::assert_matches;

    #[derive(Debug, serde::Serialize, serde::Deserialize, SerializedBytes)]
    struct Payload {
        a: u32,
    }

    async fn run_call_zome<'env, Ribosome: RibosomeT + Send + Sync + 'env>(
        workspace: &mut InvokeZomeWorkspace<'env>,
        ribosome: Ribosome,
        invocation: ZomeCallInvocation,
    ) -> WorkflowResult<ZomeCallInvocationResult> {
        let keystore = fixt!(KeystoreSender);
        let network = fixt!(HolochainP2pCell);
        let args = InvokeZomeWorkflowArgs {
            invocation,
            ribosome,
        };
        invoke_zome_workflow_inner(workspace, network, keystore, args).await
    }

    // 1.  Check if there is a Capability token secret in the parameters.
    // If there isn't and the function to be called isn't public,
    // we stop the process and return an error. MVT
    // TODO: B-01553: Finish this test when capabilities land
    #[ignore]
    #[allow(unused_variables, unreachable_code)]
    #[tokio::test]
    async fn private_zome_call() {
        let env = test_cell_env();
        let dbs = env.dbs().await;
        let env_ref = env.guard().await;
        let reader = env_ref.reader().unwrap();
        let workspace = InvokeZomeWorkspace::new(&reader, &dbs).unwrap();
        let ribosome = MockRibosomeT::new();
        // FIXME: CAP: Set this function to private
        let invocation = crate::core::ribosome::ZomeCallInvocationFixturator::new(
            crate::core::ribosome::NamedInvocation(
                holochain_types::cell::CellIdFixturator::new(fixt::Unpredictable)
                    .next()
                    .unwrap(),
                TestWasm::Foo.into(),
                "fun_times".into(),
                HostInput::new(Payload { a: 1 }.try_into().unwrap()),
            ),
        )
        .next()
        .unwrap();
        invocation.cap = todo!("Make secret cap token");
        let error = run_call_zome(&mut workspace, ribosome, invocation)
            .await
            .unwrap_err();
        assert_matches!(error, WorkflowError::CapabilityMissing);
    }

    // TODO: B-01553: Finish these tests when capabilities land
    // 1.1 If there is a secret, we look up our private CAS and see if it matches any secret for a
    // Capability Grant entry that we have stored. If it does, check that this Capability Grant is
    //not revoked and actually grants permissions to call the ZomeFn that is being called. (MVI)

    // 1.2 Check if the Capability Grant has assignees=None (means this Capability is transferable).
    // If it has assignees=Vec<Address> (means this Capability is on Assigned mode, check that the
    // provenance's agent key is in that assignees. (MVI)

    // 1.3 If the CapabiltyGrant has pre-filled parameters, check that the ui is passing exactly the
    // parameters needed and no more to complete the call. (MVI)

    // 2. Set Context (Cascading Cursor w/ Pre-flight chain extension) MVT

    // 3. Invoke WASM (w/ Cursor) MVM
    // WASM receives external call handles:
    // (gets & commits via cascading cursor, crypto functions & bridge calls via conductor,
    // send via network function call for send direct message)

    // There is no test for `3.` only that it compiles

    // 4. When the WASM code execution finishes, If workspace has new chain entries:
    // 4.1. Call system validation of list of entries and headers: (MVI)
    // - Check entry hash
    // - Check header hash
    // - Check header signature
    // - Check header timestamp is later than previous timestamp
    // - Check entry content matches entry schema
    //   Depending on the type of the commit, validate all possible validations for the
    //   DHT Op that would be produced by it

    // TODO: B-01100 Make sure this test is in the right place when SysValidation complete
    // so we aren't duplicating the unit test inside sys val.
    #[ignore]
    #[tokio::test]
    async fn calls_system_validation<'a>() {
        observability::test_run().ok();
        let env = test_cell_env();
        let dbs = env.dbs().await;
        let env_ref = env.guard().await;
        let reader = env_ref.reader().unwrap();
        let mut workspace = InvokeZomeWorkspace::new(&reader, &dbs).unwrap();

        // Genesis
        fake_genesis(&mut workspace.source_chain).await.unwrap();

        let agent_pubkey = fake_agent_pubkey_1();
        let _agent_entry = Entry::Agent(agent_pubkey.clone().into());
        let mut ribosome = MockRibosomeT::new();
        // Call zome mock that it writes to source chain
        ribosome
            .expect_call_zome_function()
            .returning(move |_workspace, _invocation| {
                let x = SerializedBytes::try_from(Payload { a: 3 }).unwrap();
                Ok(ZomeCallInvocationResponse::ZomeApiFn(GuestOutput::new(x)))
            });

        let invocation = crate::core::ribosome::ZomeCallInvocationFixturator::new(
            crate::core::ribosome::NamedInvocation(
                holochain_types::cell::CellIdFixturator::new(fixt::Unpredictable)
                    .next()
                    .unwrap(),
                TestWasm::Foo.into(),
                "fun_times".into(),
                HostInput::new(Payload { a: 1 }.try_into().unwrap()),
            ),
        )
        .next()
        .unwrap();
        // IDEA: Mock the system validation and check it's called
        /* This is one way to test the correctness of the calls to sys val
        let mut sys_val = MockSystemValidation::new();
        sys_val
            .expect_check_entry_hash()
            .times(1)
            .returning(|_entry_hash| Ok(()));
        */

        let _result = run_call_zome(&mut workspace, ribosome, invocation)
            .await
            .unwrap();
    }

    // 4.2. Call app validation of list of entries and headers: (MVI)
    // - Call validate_set_of_entries_and_headers (any necessary get
    //   results where we receive None / Timeout on retrieving validation dependencies, should produce error/fail)
    // TODO: B-01093: Finish when app val lands
    #[ignore]
    #[tokio::test]
    async fn calls_app_validation() {
        let env = test_cell_env();
        let dbs = env.dbs().await;
        let env_ref = env.guard().await;
        let reader = env_ref.reader().unwrap();
        let mut workspace = InvokeZomeWorkspace::new(&reader, &dbs).unwrap();
        let ribosome = MockRibosomeT::new();
        let invocation = crate::core::ribosome::ZomeCallInvocationFixturator::new(
            crate::core::ribosome::NamedInvocation(
                holochain_types::cell::CellIdFixturator::new(fixt::Unpredictable)
                    .next()
                    .unwrap(),
                TestWasm::Foo.into(),
                "fun_times".into(),
                HostInput::new(Payload { a: 1 }.try_into().unwrap()),
            ),
        )
        .next()
        .unwrap();
        // TODO: B-01093: Mock the app validation and check it's called
        // TODO: B-01093: How can I pass a app validation into this?
        // These are just static calls
        let _result = run_call_zome(&mut workspace, ribosome, invocation)
            .await
            .unwrap();
    }

    // 4.3. Write output results via SC gatekeeper (wrap in transaction): (MVI)
    // This is handled by the workflow runner however I should test that
    // we can create outputs
    #[ignore]
    #[tokio::test]
    async fn creates_outputs() {
        let env = test_cell_env();
        let dbs = env.dbs().await;
        let env_ref = env.guard().await;
        let reader = env_ref.reader().unwrap();
        let mut workspace = InvokeZomeWorkspace::new(&reader, &dbs).unwrap();
        let ribosome = MockRibosomeT::new();
        // TODO: Make this mock return an output
        let invocation = crate::core::ribosome::ZomeCallInvocationFixturator::new(
            crate::core::ribosome::NamedInvocation(
                holochain_types::cell::CellIdFixturator::new(fixt::Unpredictable)
                    .next()
                    .unwrap(),
                TestWasm::Foo.into(),
                "fun_times".into(),
                HostInput::new(Payload { a: 1 }.try_into().unwrap()),
            ),
        )
        .next()
        .unwrap();
        let _result = run_call_zome(&mut workspace, ribosome, invocation)
            .await
            .unwrap();
        // TODO: Check the workspace has changes
    }
}
