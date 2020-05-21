//! Placeholder for the ZomeApi interface. May be deleted.

use super::error::ZomeApiResult;
use crate::core::ribosome::{ZomeCallInvocation, ZomeCallInvocationResponse};

/// The ZomeApi defines the functions which are imported into the Wasm runtime,
/// i.e. the core API which is made accessible to hApps for interacting with Holochain.
pub trait ZomeApi {
    /// Invoke a zome function
    fn call(&self, invocation: ZomeCallInvocation) -> ZomeApiResult<ZomeCallInvocationResponse>;

    // fn commit_capability_claim();
    // fn commit_capability_grant();
    // fn commit_entry();
    // fn commit_entry_result();
    // fn debug();
    // fn decrypt();
    // fn emit_signal();
    // fn encrypt();
    // fn entry_address();
    // // fn entry_type_properties();
    // fn get_entry();
    // // fn get_entry_history();
    // // fn get_entry_initial();
    // fn get_entry_results();

    // fn get_links();
    // // et al...

    // fn link_entries();
    // fn property(); // --> get_property ?
    // fn query();
    // fn query_result();
    // fn remove_link();
    // fn send();
    // fn sign();
    // fn sign_one_time();
    // fn sleep();
    // fn verify_signature();
    // fn remove_entry();
    // // fn update_agent();
    // fn update_entry();
    // fn version();
    // fn version_hash();
}
