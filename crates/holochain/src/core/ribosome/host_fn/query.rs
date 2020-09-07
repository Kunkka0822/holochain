use crate::core::ribosome::error::RibosomeResult;
use crate::core::ribosome::CallContext;
use crate::core::ribosome::RibosomeT;
use holochain_zome_types::QueryInput;
use holochain_zome_types::QueryOutput;
use std::sync::Arc;

pub fn query(
    _ribosome: Arc<impl RibosomeT>,
    _call_context: Arc<CallContext>,
    _input: QueryInput,
) -> RibosomeResult<QueryOutput> {
    unimplemented!();
}

#[cfg(test)]
#[cfg(feature = "slow_tests")]
pub mod slow_tests {

    use crate::{core::ribosome::ZomeCallHostAccess, fixt::ZomeCallHostAccessFixturator};
    use fixt::prelude::*;
    use hdk3::prelude::*;
    use holochain_state::test_utils::TestEnvironment;
    use query::ChainQuery;

    use holochain_wasm_test_utils::TestWasm;
    use test_wasm_common::*;

    async fn setup() -> (TestEnvironment, ZomeCallHostAccess) {
        let test_env = holochain_state::test_utils::test_cell_env();
        let env = test_env.env();

        let mut workspace =
            crate::core::workflow::CallZomeWorkspace::new(env.clone().into()).unwrap();

        // commits fail validation if we don't do genesis
        crate::core::workflow::fake_genesis(&mut workspace.source_chain)
            .await
            .unwrap();

        let workspace_lock = crate::core::workflow::CallZomeWorkspaceLock::new(workspace);
        let mut host_access = fixt!(ZomeCallHostAccess);
        host_access.workspace = workspace_lock;
        (test_env, host_access)
    }

    #[tokio::test(threaded_scheduler)]
    async fn query_chain() {
        let (_test_env, host_access) = setup().await;

        let _: () = crate::call_test_ribosome!(
            host_access,
            TestWasm::Query,
            "add_path",
            TestString::from("a".to_string())
        );
        let _: () = crate::call_test_ribosome!(
            host_access,
            TestWasm::Query,
            "add_path",
            TestString::from("b".to_string())
        );

        let elements: HeaderHashes = crate::call_test_ribosome!(
            host_access,
            TestWasm::Query,
            "query",
            ChainQuery::default()
        );

        assert_eq!(elements.0.len(), 2);
    }
}
