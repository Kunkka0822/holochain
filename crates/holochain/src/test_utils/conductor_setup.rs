use crate::{
    conductor::{
        api::RealAppInterfaceApi, dna_store::MockDnaStore, interface::SignalBroadcaster,
        ConductorHandle,
    },
    core::queue_consumer::InitialQueueTriggers,
    core::ribosome::wasm_ribosome::WasmRibosome,
    test_utils::setup_app,
};
use holochain_keystore::KeystoreSender;
use holochain_p2p::{actor::HolochainP2pRefToCell, HolochainP2pCell};
use holochain_serialized_bytes::SerializedBytes;
use holochain_state::env::EnvironmentWrite;
use holochain_types::{
    app::InstalledCell, cell::CellId, dna::DnaDef, dna::DnaFile, test_utils::fake_agent_pubkey_1,
    test_utils::fake_agent_pubkey_2,
};
use holochain_wasm_test_utils::TestWasm;
use holochain_zome_types::zome::ZomeName;
use std::{convert::TryFrom, sync::Arc};
use tempdir::TempDir;

use super::host_fn_api::CallData;

/// Everything you need to run a test that uses the conductor
pub struct ConductorTestData {
    pub __tmpdir: Arc<TempDir>,
    pub app_api: RealAppInterfaceApi,
    pub handle: ConductorHandle,
    pub alice_call_data: ConductorCallData,
    pub bob_call_data: Option<ConductorCallData>,
}

/// Everything you need to make a call with the host fn api
pub struct ConductorCallData {
    pub cell_id: CellId,
    pub env: EnvironmentWrite,
    pub ribosome: WasmRibosome,
    pub network: HolochainP2pCell,
    pub keystore: KeystoreSender,
    pub signal_tx: SignalBroadcaster,
    pub triggers: InitialQueueTriggers,
}

impl ConductorCallData {
    pub async fn new(cell_id: &CellId, handle: &ConductorHandle, dna_file: &DnaFile) -> Self {
        let env = handle.get_cell_env(cell_id).await.unwrap();
        let keystore = env.keystore().clone();
        let network = handle
            .holochain_p2p()
            .to_cell(cell_id.dna_hash().clone(), cell_id.agent_pubkey().clone());
        let triggers = handle.get_cell_triggers(cell_id).await.unwrap();

        let ribosome = WasmRibosome::new(dna_file.clone());
        let signal_tx = handle.signal_broadcaster().await;
        let call_data = ConductorCallData {
            cell_id: cell_id.clone(),
            env,
            ribosome,
            network,
            keystore,
            signal_tx,
            triggers,
        };
        call_data
    }

    /// Create a CallData for a specific zome and call
    pub fn call_data<I: Into<ZomeName>>(&self, zome_name: I) -> CallData {
        let zome_name: ZomeName = zome_name.into();
        let zome_path = (self.cell_id.clone(), zome_name).into();
        CallData {
            ribosome: self.ribosome.clone(),
            zome_path,
            network: self.network.clone(),
            keystore: self.keystore.clone(),
            signal_tx: self.signal_tx.clone(),
        }
    }
}

impl ConductorTestData {
    pub async fn new(zomes: Vec<TestWasm>, with_bob: bool) -> Self {
        let dna_file = DnaFile::new(
            DnaDef {
                name: "conductor_test".to_string(),
                uuid: "ba1d046d-ce29-4778-914b-47e6010d2faf".to_string(),
                properties: SerializedBytes::try_from(()).unwrap(),
                zomes: zomes.clone().into_iter().map(Into::into).collect(),
            },
            zomes.into_iter().map(Into::into),
        )
        .await
        .unwrap();

        let alice = || {
            let alice_agent_id = fake_agent_pubkey_1();
            let alice_cell_id = CellId::new(dna_file.dna_hash().to_owned(), alice_agent_id.clone());
            InstalledCell::new(alice_cell_id.clone(), "alice_handle".into())
        };
        let bob = || {
            let bob_agent_id = fake_agent_pubkey_2();
            let bob_cell_id = CellId::new(dna_file.dna_hash().to_owned(), bob_agent_id.clone());
            InstalledCell::new(bob_cell_id.clone(), "bob_handle".into())
        };

        let mut dna_store = MockDnaStore::new();

        dna_store.expect_get().return_const(Some(dna_file.clone()));
        dna_store.expect_add_dnas::<Vec<_>>().return_const(());
        dna_store.expect_add_entry_defs::<Vec<_>>().return_const(());
        dna_store.expect_get_entry_def().return_const(None);

        let mut cells = vec![];
        let alice_installed_cell = alice();
        let alice_cell_id = alice_installed_cell.as_id().clone();
        cells.push((alice_installed_cell, None));
        let bob_cell_id = if with_bob {
            let bob_installed_cell = bob();
            let bob_cell_id = Some(bob_installed_cell.as_id().clone());
            cells.push((bob_installed_cell, None));
            bob_cell_id
        } else {
            None
        };

        let (__tmpdir, app_api, handle) = setup_app(vec![("test_app", cells)], dna_store).await;

        let alice_call_data = ConductorCallData::new(&alice_cell_id, &handle, &dna_file).await;

        let bob_call_data = match bob_cell_id {
            Some(bob_cell_id) => {
                Some(ConductorCallData::new(&bob_cell_id, &handle, &dna_file).await)
            }
            None => None,
        };

        Self {
            __tmpdir,
            app_api,
            handle,
            alice_call_data,
            bob_call_data,
        }
    }
    /// Shutdown the conductor
    pub async fn shutdown_conductor(handle: ConductorHandle) {
        let shutdown = handle.take_shutdown_handle().await.unwrap();
        handle.shutdown().await;
        shutdown.await.unwrap();
    }
}
