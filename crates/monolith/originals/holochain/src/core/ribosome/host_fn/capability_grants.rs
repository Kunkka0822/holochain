use crate::holochain::core::ribosome::error::RibosomeResult;
use crate::holochain::core::ribosome::CallContext;
use crate::holochain::core::ribosome::RibosomeT;
use holochain_zome_types::CapabilityGrantsInput;
use holochain_zome_types::CapabilityGrantsOutput;
use std::sync::Arc;

/// list all the grants stored locally in the chain filtered by tag
/// this is only the current grants as per local CRUD
pub fn capability_grants(
    _ribosome: Arc<impl RibosomeT>,
    _call_context: Arc<CallContext>,
    _input: CapabilityGrantsInput,
) -> RibosomeResult<CapabilityGrantsOutput> {
    unimplemented!();
}

#[cfg(test)]
#[cfg(feature = "slow_tests")]
pub mod wasm_test {
    use crate::destructure_test_cells;
    use crate::holochain::conductor::dna_store::MockDnaStore;
    use crate::holochain::conductor::interface::websocket::test_utils::setup_app;
    use crate::holochain::conductor::ConductorBuilder;
    use crate::holochain::core::ribosome::ZomeCall;
    use crate::holochain::core::workflow::call_zome_workflow::CallZomeWorkspace;
    use crate::holochain::fixt::ZomeCallHostAccessFixturator;
    use crate::holochain::test_utils::test_conductor::MaybeElement;
    use crate::holochain::test_utils::test_conductor::TestConductorHandle;
    use holochain_wasm_test_utils::TestWasm;
    use ::fixt::prelude::*;
    use hdk3::prelude::*;
    use holochain_lmdb::test_utils::test_environments;
    use holochain_types::app::InstalledCell;
    use holochain_types::dna::DnaDef;
    use holochain_types::dna::DnaFile;
    use holochain_types::fixt::CapSecretFixturator;
    use holochain_types::test_utils::fake_agent_pubkey_1;
    use holochain_types::test_utils::fake_agent_pubkey_2;
    use holochain_zome_types::cell::CellId;
    use matches::assert_matches;

    #[tokio::test(threaded_scheduler)]
    async fn ribosome_capability_secret_test<'a>() {
        observability::test_run().ok();
        // test workspace boilerplate
        let test_env = holochain_lmdb::test_utils::test_cell_env();
        let env = test_env.env();
        let mut workspace = CallZomeWorkspace::new(env.clone().into()).unwrap();

        crate::holochain::core::workflow::fake_genesis(&mut workspace.source_chain)
            .await
            .unwrap();
        let workspace_lock =
            crate::holochain::core::workflow::CallZomeWorkspaceLock::new(workspace);
        let mut host_access = fixt!(ZomeCallHostAccess);
        host_access.workspace = workspace_lock.clone();

        let _output: CapSecret =
            crate::call_test_ribosome!(host_access, TestWasm::Capability, "cap_secret", ());
    }

    #[tokio::test(threaded_scheduler)]
    async fn ribosome_transferable_cap_grant<'a>() {
        observability::test_run().ok();
        // test workspace boilerplate
        let test_env = holochain_lmdb::test_utils::test_cell_env();
        let env = test_env.env();
        let mut workspace = CallZomeWorkspace::new(env.clone().into()).unwrap();

        crate::holochain::core::workflow::fake_genesis(&mut workspace.source_chain)
            .await
            .unwrap();
        let workspace_lock =
            crate::holochain::core::workflow::CallZomeWorkspaceLock::new(workspace);
        let mut host_access = fixt!(ZomeCallHostAccess);
        host_access.workspace = workspace_lock.clone();

        let secret: CapSecret =
            crate::call_test_ribosome!(host_access, TestWasm::Capability, "cap_secret", ());
        let header: HeaderHash = crate::call_test_ribosome!(
            host_access,
            TestWasm::Capability,
            "transferable_cap_grant",
            secret
        );
        let entry: GetOutput =
            crate::call_test_ribosome!(host_access, TestWasm::Capability, "get_entry", header);

        let entry_secret: CapSecret = match entry.into_inner() {
            Some(element) => {
                let cap_grant_entry: CapGrantEntry = element.entry().to_grant_option().unwrap();
                match cap_grant_entry.access {
                    CapAccess::Transferable { secret, .. } => secret,
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        };
        assert_eq!(entry_secret, secret,);
    }

    // TODO: [ B-03669 ] can move this to an integration test (may need to switch to using a RealDnaStore)
    #[tokio::test(threaded_scheduler)]
    async fn ribosome_authorized_call() {
        let (dna_file, _) = DnaFile::unique_from_test_wasms(vec![TestWasm::Capability])
            .await
            .unwrap();

        let alice_agent_id = fake_agent_pubkey_1();
        let bob_agent_id = fake_agent_pubkey_2();

        let mut dna_store = MockDnaStore::new();
        dna_store.expect_get().return_const(Some(dna_file.clone()));
        dna_store.expect_add_dna().return_const(());
        dna_store.expect_add_dnas::<Vec<_>>().return_const(());
        dna_store
            .expect_add_entry_defs::<Vec<_>>()
            .times(2)
            .return_const(());

        let envs = test_environments();
        let handle: TestConductorHandle = ConductorBuilder::with_mock_dna_store(dna_store)
            .test(&envs)
            .await
            .unwrap()
            .into();

        let setup_data = handle
            .setup_app_for_agents_with_no_membrane_proof(
                "app-",
                &[alice_agent_id.clone(), bob_agent_id.clone()],
                &[dna_file],
            )
            .await;

        let ((alice,), (bobbo,)) = destructure_test_cells!(setup_data);
        // There's only one zome to call, so let's peel that off now.
        let alice = alice.zome(TestWasm::Capability);
        let bobbo = bobbo.zome(TestWasm::Capability);

        // ALICE FAILING AN UNAUTHED CALL

        #[derive(serde::Serialize, serde::Deserialize, SerializedBytes)]
        pub struct CapFor(CapSecret, AgentPubKey);

        let original_secret = CapSecretFixturator::new(Unpredictable).next().unwrap();

        let output: ZomeCallResponse = alice
            .call(
                "try_cap_claim",
                CapFor(original_secret, bob_agent_id.clone().try_into().unwrap()),
            )
            .await;

        assert_matches!(output, ZomeCallResponse::Unauthorized(_, _, _, _));

        // BOB COMMITS A TRANSFERABLE GRANT WITH THE SECRET SHARED WITH ALICE

        let original_grant_hash: HeaderHash =
            bobbo.call("transferable_cap_grant", original_secret).await;

        // ALICE CAN NOW CALL THE AUTHED REMOTE FN

        let response: ZomeCallResponse = alice
            .call(
                "try_cap_claim",
                CapFor(original_secret, bob_agent_id.clone()),
            )
            .await;

        assert_eq!(
            response,
            ZomeCallResponse::Ok(ExternOutput::new(().try_into().unwrap())),
        );

        // BOB ROLLS THE GRANT SO ONLY THE NEW ONE WILL WORK FOR ALICE

        let new_grant_header_hash: HeaderHash =
            bobbo.call("roll_cap_grant", original_grant_hash).await;

        let output: MaybeElement = bobbo.call("get_entry", new_grant_header_hash.clone()).await;

        let new_secret: CapSecret = match output.0 {
            Some(element) => match element.entry().to_grant_option() {
                Some(zome_call_cap_grant) => match zome_call_cap_grant.access {
                    CapAccess::Transferable { secret, .. } => secret,
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            },
            _ => unreachable!("Couldn't get {:?}", new_grant_header_hash),
        };

        let output: ZomeCallResponse = alice
            .call(
                "try_cap_claim",
                CapFor(original_secret, bob_agent_id.clone().try_into().unwrap()),
            )
            .await;

        assert_matches!(output, ZomeCallResponse::Unauthorized(_, _, _, _));

        let output: ZomeCallResponse = alice
            .call(
                "try_cap_claim",
                CapFor(new_secret, bob_agent_id.clone().try_into().unwrap()),
            )
            .await;
        assert_eq!(
            output,
            ZomeCallResponse::Ok(ExternOutput::new(().try_into().unwrap())),
        );

        // BOB DELETES THE GRANT SO NO SECRETS WORK

        let _: HeaderHash = bobbo.call("delete_cap_grant", new_grant_header_hash).await;

        let output: ZomeCallResponse = alice
            .call(
                "try_cap_claim",
                CapFor(original_secret, bob_agent_id.clone().try_into().unwrap()),
            )
            .await;

        assert_matches!(output, ZomeCallResponse::Unauthorized(_, _, _, _));

        let output: ZomeCallResponse = alice
            .call(
                "try_cap_claim",
                CapFor(new_secret, bob_agent_id.clone().try_into().unwrap()),
            )
            .await;

        // the inner response should be unauthorized
        assert_matches!(output, ZomeCallResponse::Unauthorized(_, _, _, _));

        let shutdown = handle.take_shutdown_handle().await.unwrap();
        handle.shutdown().await;
        shutdown.await.unwrap();
    }
}
