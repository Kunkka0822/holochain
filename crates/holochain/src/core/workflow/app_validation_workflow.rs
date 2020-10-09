//! The workflow and queue consumer for sys validation

use std::{collections::BinaryHeap, convert::TryInto, sync::Arc};

use super::{
    error::WorkflowError, error::WorkflowResult,
    produce_dht_ops_workflow::dht_op_light::light_to_op, CallZomeWorkspace, CallZomeWorkspaceLock,
};
use crate::{
    conductor::api::CellConductorApiT,
    conductor::entry_def_store::get_entry_def,
    core::ribosome::guest_callback::validate_link::ValidateCreateLinkInvocation,
    core::ribosome::guest_callback::validate_link::ValidateDeleteLinkInvocation,
    core::ribosome::guest_callback::validate_link::ValidateLinkHostAccess,
    core::ribosome::guest_callback::validate_link::ValidateLinkInvocation,
    core::ribosome::guest_callback::validate_link::ValidateLinkResult,
    core::ribosome::wasm_ribosome::WasmRibosome,
    core::ribosome::Invocation,
    core::ribosome::ZomesToInvoke,
    core::state::cascade::Cascade,
    core::{
        queue_consumer::{OneshotWriter, TriggerSender, WorkComplete},
        ribosome::guest_callback::validate::ValidateHostAccess,
        ribosome::guest_callback::validate::ValidateInvocation,
        ribosome::guest_callback::validate::ValidateResult,
        ribosome::RibosomeT,
        state::{
            cascade::DbPair,
            cascade::DbPairMut,
            dht_op_integration::{
                IntegratedDhtOpsStore, IntegrationLimboStore, IntegrationLimboValue,
            },
            element_buf::ElementBuf,
            metadata::MetadataBuf,
            validation_db::{ValidationLimboStatus, ValidationLimboStore, ValidationLimboValue},
            workspace::{Workspace, WorkspaceResult},
        },
        validation::DhtOpOrder,
        validation::OrderedOp,
    },
};
use error::AppValidationResult;
pub use error::*;
use fallible_iterator::FallibleIterator;
use holo_hash::DhtOpHash;
use holochain_p2p::{HolochainP2pCell, HolochainP2pCellT};
use holochain_state::{
    buffer::{BufferedStore, KvBufFresh},
    db::{INTEGRATED_DHT_OPS, INTEGRATION_LIMBO},
    fresh_reader,
    prelude::*,
};
use holochain_types::{
    dht_op::DhtOp, dna::zome::Zome, dna::DnaFile, test_utils::which_agent,
    validate::ValidationStatus, Entry, HeaderHashed, Timestamp,
};
use holochain_zome_types::{
    element::Element,
    element::SignedHeaderHashed,
    entry_def::EntryDef,
    entry_def::EntryDefId,
    header::AppEntryType,
    header::EntryType,
    header::{CreateLink, DeleteLink, ZomeId},
    validate::RequiredValidationType,
    validate::ValidationPackage,
    zome::ZomeName,
    Header,
};
use tracing::*;
pub use types::Outcome;

#[cfg(test)]
mod network_call_tests;
#[cfg(test)]
mod tests;

mod error;
mod types;

#[instrument(skip(workspace, writer, trigger_integration, conductor_api, network))]
pub async fn app_validation_workflow(
    mut workspace: AppValidationWorkspace,
    writer: OneshotWriter,
    trigger_integration: &mut TriggerSender,
    conductor_api: impl CellConductorApiT,
    network: HolochainP2pCell,
) -> WorkflowResult<WorkComplete> {
    let complete = app_validation_workflow_inner(&mut workspace, conductor_api, &network).await?;
    // --- END OF WORKFLOW, BEGIN FINISHER BOILERPLATE ---

    // commit the workspace
    writer.with_writer(|writer| Ok(workspace.flush_to_txn(writer)?))?;

    // trigger other workflows
    trigger_integration.trigger();

    Ok(complete)
}
async fn app_validation_workflow_inner(
    workspace: &mut AppValidationWorkspace,
    conductor_api: impl CellConductorApiT,
    network: &HolochainP2pCell,
) -> WorkflowResult<WorkComplete> {
    let env = workspace.validation_limbo.env().clone();

    // Drain the ops into a sorted binary heap
    let sorted_ops: BinaryHeap<OrderedOp<ValidationLimboValue>> = fresh_reader!(env, |r| {
        let validation_limbo = &mut workspace.validation_limbo;
        let element_pending = &workspace.element_pending;

        let sorted_ops: Result<BinaryHeap<OrderedOp<ValidationLimboValue>>, WorkflowError> =
            validation_limbo
                .drain_iter_filter(&r, |(_, vlv)| {
                    match vlv.status {
                        // We only want sys validated or awaiting app dependency ops
                        ValidationLimboStatus::SysValidated
                        | ValidationLimboStatus::AwaitingAppDeps(_) => Ok(true),
                        ValidationLimboStatus::Pending
                        | ValidationLimboStatus::AwaitingSysDeps(_) => Ok(false),
                    }
                })?
                .map_err(WorkflowError::from)
                .map(|vlv| {
                    // Sort the ops into a min-heap
                    let op = light_to_op(vlv.op.clone(), element_pending)?;

                    let hash = DhtOpHash::with_data_sync(&op);
                    let order = DhtOpOrder::from(&op);
                    let v = OrderedOp {
                        order,
                        hash,
                        op,
                        value: vlv,
                    };
                    // We want a min-heap
                    Ok(v)
                })
                .iterator()
                .collect();
        sorted_ops
    })?;

    // Validate all the ops
    for so in sorted_ops.into_sorted_vec() {
        let OrderedOp {
            hash,
            op,
            value: mut vlv,
            ..
        } = so;

        match &vlv.status {
            ValidationLimboStatus::AwaitingAppDeps(_) | ValidationLimboStatus::SysValidated => {
                // Validate this op
                let outcome = validate_op(op.clone(), &conductor_api, workspace, &network)
                    .await
                    // Get the outcome or return the error
                    .or_else(|outcome_or_err| outcome_or_err.try_into())?;

                match outcome {
                    Outcome::Accepted => {
                        let iv = IntegrationLimboValue {
                            validation_status: ValidationStatus::Valid,
                            op: vlv.op,
                        };
                        workspace.put_int_limbo(hash, iv, op)?;
                    }
                    Outcome::AwaitingDeps(deps) => {
                        vlv.status = ValidationLimboStatus::AwaitingAppDeps(deps);
                        workspace.put_val_limbo(hash, vlv)?;
                    }
                    Outcome::Rejected(_) => {
                        let iv = IntegrationLimboValue {
                            op: vlv.op,
                            validation_status: ValidationStatus::Rejected,
                        };
                        workspace.put_int_limbo(hash, iv, op)?;
                    }
                }
            }
            _ => unreachable!("Should not contain any other status"),
        }
    }
    Ok(WorkComplete::Complete)
}

fn to_zome_name(zomes_to_invoke: ZomesToInvoke) -> AppValidationResult<ZomeName> {
    match zomes_to_invoke {
        ZomesToInvoke::All => Err(AppValidationError::LinkMultipleZomes),
        ZomesToInvoke::One(zn) => Ok(zn),
    }
}

async fn validate_op(
    op: DhtOp,
    conductor_api: &impl CellConductorApiT,
    workspace: &mut AppValidationWorkspace,
    network: &HolochainP2pCell,
) -> AppValidationOutcome<Outcome> {
    // Get the workspace for the validation calls
    let workspace_lock = workspace.validation_workspace();

    // Create the element
    let element = get_element(op)?;

    // Check for caps
    check_for_caps(&element)?;

    // Get the dna file
    let dna_file = { conductor_api.get_this_dna().await };
    let dna_file =
        dna_file.ok_or_else(|| AppValidationError::DnaMissing(conductor_api.cell_id().clone()))?;

    // Get the EntryDefId associated with this Element if there is one
    let entry_def = {
        let cascade = workspace.full_cascade(network.clone());
        get_associated_entry_def(&element, &dna_file, conductor_api, cascade).await?
    };

    // Get the validation package
    let validation_package = get_validation_package(&element, &entry_def, network.clone()).await?;

    // Get the EntryDefId associated with this Element if there is one
    let entry_def_id = entry_def.map(|ed| ed.id);

    // Get the zome names
    let zomes_to_invoke = get_zomes_to_invoke(&element, &dna_file, workspace, network).await?;

    // Create the ribosome
    let ribosome = WasmRibosome::new(dna_file);

    let outcome = match element.header() {
        Header::DeleteLink(delete_link) => {
            let zome_name = to_zome_name(zomes_to_invoke)?;
            // Run the link validation
            run_delete_link_validation_callback(
                zome_name,
                delete_link.clone(),
                &ribosome,
                workspace_lock.clone(),
                network.clone(),
            )?
        }
        Header::CreateLink(link_add) => {
            // Get the base and target for this link
            let mut cascade = workspace.full_cascade(network.clone());
            let base = cascade
                .retrieve_entry(link_add.base_address.clone(), Default::default())
                .await?
                .map(|e| e.into_content())
                .ok_or_else(|| Outcome::awaiting(&link_add.base_address))?;
            let target = cascade
                .retrieve_entry(link_add.target_address.clone(), Default::default())
                .await?
                .map(|e| e.into_content())
                .ok_or_else(|| Outcome::awaiting(&link_add.target_address))?;

            let link_add = Arc::new(link_add.clone());
            let base = Arc::new(base);
            let target = Arc::new(target);

            let zome_name = to_zome_name(zomes_to_invoke)?;

            // Run the link validation
            run_create_link_validation_callback(
                zome_name,
                link_add,
                base,
                target,
                &ribosome,
                workspace_lock.clone(),
                network.clone(),
            )?
        }
        _ => {
            // Element

            // Call the callback
            let element = Arc::new(element);
            let validation_package = validation_package.map(Arc::new);
            // Call the element validation
            run_validation_callback_inner(
                zomes_to_invoke,
                element,
                validation_package,
                entry_def_id,
                &ribosome,
                workspace_lock.clone(),
                network.clone(),
            )?
        }
    };
    if let Outcome::AwaitingDeps(_) | Outcome::Rejected(_) = &outcome {
        warn!(
            agent = %which_agent(conductor_api.cell_id().agent_pubkey()),
            msg = "DhtOp has failed app validation",
            outcome = ?outcome,
        );
    }

    Ok(outcome)
}

/// Get the [EntryDef] associated with this
/// element if there is one.
///
/// Create and Update will get the def from
/// the AppEntryType on their header.
///
/// Delete will get the def from the
/// header on the `deletes_address` field.
///
/// Other header types will None.
async fn get_associated_entry_def(
    element: &Element,
    dna_file: &DnaFile,
    conductor_api: &impl CellConductorApiT,
    cascade: Cascade<'_>,
) -> AppValidationOutcome<Option<EntryDef>> {
    match get_app_entry_type(element, cascade).await? {
        Some(aet) => {
            let zome = get_zome_info(&aet, dna_file)?.1.clone();
            Ok(get_entry_def(aet.id(), zome, dna_file, conductor_api).await?)
        }
        None => Ok(None),
    }
}

/// Get the element from the op or
/// return accepted because we don't app
/// validate this op.
fn get_element(op: DhtOp) -> AppValidationOutcome<Element> {
    match op {
        DhtOp::RegisterAgentActivity(_, _) => Outcome::accepted(),
        DhtOp::StoreElement(s, h, e) => match h {
            Header::Delete(_) | Header::CreateLink(_) | Header::DeleteLink(_) => Ok(Element::new(
                SignedHeaderHashed::with_presigned(HeaderHashed::from_content_sync(h), s),
                None,
            )),
            Header::Update(_) | Header::Create(_) => Ok(Element::new(
                SignedHeaderHashed::with_presigned(HeaderHashed::from_content_sync(h), s),
                e.map(|e| *e),
            )),
            _ => Outcome::accepted(),
        },
        DhtOp::StoreEntry(s, h, e) => Ok(Element::new(
            SignedHeaderHashed::with_presigned(HeaderHashed::from_content_sync(h.into()), s),
            Some(*e),
        )),
        DhtOp::RegisterUpdatedBy(s, h, e) => Ok(Element::new(
            SignedHeaderHashed::with_presigned(HeaderHashed::from_content_sync(h.into()), s),
            e.map(|e| *e),
        )),
        DhtOp::RegisterDeletedEntryHeader(s, h) => Ok(Element::new(
            SignedHeaderHashed::with_presigned(HeaderHashed::from_content_sync(h.into()), s),
            None,
        )),
        DhtOp::RegisterDeletedBy(s, h) => Ok(Element::new(
            SignedHeaderHashed::with_presigned(HeaderHashed::from_content_sync(h.into()), s),
            None,
        )),
        DhtOp::RegisterAddLink(s, h) => Ok(Element::new(
            SignedHeaderHashed::with_presigned(HeaderHashed::from_content_sync(h.into()), s),
            None,
        )),
        DhtOp::RegisterRemoveLink(s, h) => Ok(Element::new(
            SignedHeaderHashed::with_presigned(HeaderHashed::from_content_sync(h.into()), s),
            None,
        )),
    }
}

/// Check for capability headers
/// and exit as we don't want to validate them
fn check_for_caps(element: &Element) -> AppValidationOutcome<()> {
    match element.header().entry_type() {
        Some(EntryType::CapClaim) | Some(EntryType::CapGrant) => Outcome::accepted(),
        _ => Ok(()),
    }
}

/// Get the zome name from the app entry type
/// or get all zome names.
async fn get_zomes_to_invoke(
    element: &Element,
    dna_file: &DnaFile,
    workspace: &mut AppValidationWorkspace,
    network: &HolochainP2pCell,
) -> AppValidationOutcome<ZomesToInvoke> {
    let aet = {
        let cascade = workspace.full_cascade(network.clone());
        get_app_entry_type(element, cascade).await?
    };
    match aet {
        Some(aet) => Ok(ZomesToInvoke::One(get_zome_name(&aet, &dna_file)?)),
        None => match element.header() {
            Header::CreateLink(_) | Header::DeleteLink(_) => {
                get_link_zome(element, dna_file, workspace, network).await
            }
            _ => Ok(ZomesToInvoke::All),
        },
    }
}

fn get_zome_info<'a>(
    entry_type: &AppEntryType,
    dna_file: &'a DnaFile,
) -> AppValidationResult<&'a (ZomeName, Zome)> {
    let zome_index = u8::from(entry_type.zome_id()) as usize;
    Ok(dna_file
        .dna()
        .zomes
        .get(zome_index)
        .ok_or_else(|| AppValidationError::ZomeId(entry_type.zome_id()))?)
}

fn get_zome_name(entry_type: &AppEntryType, dna_file: &DnaFile) -> AppValidationResult<ZomeName> {
    zome_id_to_zome_name(entry_type.zome_id(), dna_file)
}

fn zome_id_to_zome_name(zome_id: ZomeId, dna_file: &DnaFile) -> AppValidationResult<ZomeName> {
    let zome_index = u8::from(zome_id) as usize;
    Ok(dna_file
        .dna()
        .zomes
        .get(zome_index)
        .ok_or_else(|| AppValidationError::ZomeId(zome_id))?
        .0
        .clone())
}

/// Either get the app entry type
/// from this entry or from the dependency.
async fn get_app_entry_type(
    element: &Element,
    cascade: Cascade<'_>,
) -> AppValidationOutcome<Option<AppEntryType>> {
    match element.header().entry_data() {
        Some((_, et)) => match et.clone() {
            EntryType::App(aet) => Ok(Some(aet)),
            EntryType::AgentPubKey | EntryType::CapClaim | EntryType::CapGrant => Ok(None),
        },
        None => get_app_entry_type_from_dep(element, cascade).await,
    }
}

async fn get_link_zome(
    element: &Element,
    dna_file: &DnaFile,
    workspace: &mut AppValidationWorkspace,
    network: &HolochainP2pCell,
) -> AppValidationOutcome<ZomesToInvoke> {
    match element.header() {
        Header::CreateLink(cl) => {
            let zome_name = zome_id_to_zome_name(cl.zome_id, dna_file)?;
            Ok(ZomesToInvoke::One(zome_name))
        }
        Header::DeleteLink(dl) => {
            let mut cascade = workspace.full_cascade(network.clone());
            let shh = cascade
                .retrieve_header(dl.link_add_address.clone(), Default::default())
                .await?
                .ok_or_else(|| Outcome::awaiting(&dl.link_add_address))?;

            match shh.header() {
                Header::CreateLink(cl) => {
                    let zome_name = zome_id_to_zome_name(cl.zome_id, dna_file)?;
                    Ok(ZomesToInvoke::One(zome_name))
                }
                // The header that was found was the wrong type
                // so lets try again.
                _ => Err(Outcome::awaiting(&dl.link_add_address)),
            }
        }
        _ => unreachable!(),
    }
}

/// Retrieve the dependency and extract
/// the app entry type so we know which zome to call
async fn get_app_entry_type_from_dep(
    element: &Element,
    mut cascade: Cascade<'_>,
) -> AppValidationOutcome<Option<AppEntryType>> {
    match element.header() {
        Header::Delete(ed) => {
            let el = cascade
                .retrieve(ed.deletes_address.clone().into(), Default::default())
                .await?
                .ok_or_else(|| Outcome::awaiting(&ed.deletes_address))?;
            Ok(extract_app_type(&el))
        }
        _ => Ok(None),
    }
}

fn extract_app_type(element: &Element) -> Option<AppEntryType> {
    element
        .header()
        .entry_data()
        .and_then(|(_, entry_type)| match entry_type {
            EntryType::App(aet) => Some(aet.clone()),
            _ => None,
        })
}

/// Get the validation package based on
/// the requirements set by the AppEntryType
async fn get_validation_package(
    element: &Element,
    entry_def: &Option<EntryDef>,
    mut network: HolochainP2pCell,
) -> AppValidationResult<Option<ValidationPackage>> {
    match entry_def {
        Some(entry_def) => {
            Ok(match entry_def.required_validation_type {
                // Only needs the element
                RequiredValidationType::Element => None,
                RequiredValidationType::SubChain | RequiredValidationType::Full => {
                    // TODO: What if this is the same author that is validating?
                    // We probably don't want to do a network call although
                    // it will just short circuit

                    // Get from author
                    let agent_id = element.header().author().clone();
                    let header_hash = element.header_address().clone();
                    network
                        .get_validation_package(agent_id, header_hash)
                        .await?
                        .into()
                    // TODO: Fallback to gossiper if author is unavailable
                    // TODO: Fallback to RegisterAgentActivity if gossiper is unavailable
                }
            })
        }
        None => {
            // Not an entry header type so no package
            Ok(None)
        }
    }
}

pub async fn run_validation_callback_direct(
    zome_name: ZomeName,
    element: Element,
    ribosome: &impl RibosomeT,
    workspace_lock: CallZomeWorkspaceLock,
    network: HolochainP2pCell,
    conductor_api: &impl CellConductorApiT,
) -> AppValidationResult<Outcome> {
    let outcome = {
        let mut workspace = workspace_lock.write().await;
        let cascade = workspace.cascade(network.clone());
        get_associated_entry_def(&element, ribosome.dna_file(), conductor_api, cascade).await
    };

    // The outcome could be awaiting a dependency to get the entry def
    // so we need to check that here and exit early if that is the case
    let entry_def = match outcome {
        Ok(ed) => ed,
        Err(outcome) => return outcome.try_into(),
    };

    let validation_package = get_validation_package(&element, &entry_def, network.clone())
        .await?
        .map(Arc::new);
    let entry_def_id = entry_def.map(|ed| ed.id);

    let element = Arc::new(element);

    run_validation_callback_inner(
        ZomesToInvoke::One(zome_name),
        element,
        validation_package,
        entry_def_id,
        ribosome,
        workspace_lock,
        network,
    )
}

fn run_validation_callback_inner(
    zomes_to_invoke: ZomesToInvoke,
    element: Arc<Element>,
    validation_package: Option<Arc<ValidationPackage>>,
    entry_def_id: Option<EntryDefId>,
    ribosome: &impl RibosomeT,
    workspace_lock: CallZomeWorkspaceLock,
    network: HolochainP2pCell,
) -> AppValidationResult<Outcome> {
    let validate: ValidateResult = ribosome.run_validate(
        ValidateHostAccess::new(workspace_lock, network),
        ValidateInvocation {
            zomes_to_invoke,
            element,
            validation_package,
            entry_def_id,
        },
    )?;
    match validate {
        ValidateResult::Valid => Ok(Outcome::Accepted),
        ValidateResult::Invalid(reason) => Ok(Outcome::Rejected(reason)),
        ValidateResult::UnresolvedDependencies(hashes) => Ok(Outcome::AwaitingDeps(hashes)),
    }
}

pub fn run_create_link_validation_callback(
    zome_name: ZomeName,
    link_add: Arc<CreateLink>,
    base: Arc<Entry>,
    target: Arc<Entry>,
    ribosome: &impl RibosomeT,
    workspace_lock: CallZomeWorkspaceLock,
    network: HolochainP2pCell,
) -> AppValidationResult<Outcome> {
    let invocation = ValidateCreateLinkInvocation {
        zome_name,
        link_add,
        base,
        target,
    };
    let invocation = ValidateLinkInvocation::<ValidateCreateLinkInvocation>::new(invocation);
    run_link_validation_callback(invocation, ribosome, workspace_lock, network)
}

pub fn run_delete_link_validation_callback(
    zome_name: ZomeName,
    delete_link: DeleteLink,
    ribosome: &impl RibosomeT,
    workspace_lock: CallZomeWorkspaceLock,
    network: HolochainP2pCell,
) -> AppValidationResult<Outcome> {
    let invocation = ValidateDeleteLinkInvocation {
        zome_name,
        delete_link,
    };
    let invocation = ValidateLinkInvocation::<ValidateDeleteLinkInvocation>::new(invocation);
    run_link_validation_callback(invocation, ribosome, workspace_lock, network)
}

pub fn run_link_validation_callback<I: Invocation + 'static>(
    invocation: ValidateLinkInvocation<I>,
    ribosome: &impl RibosomeT,
    workspace_lock: CallZomeWorkspaceLock,
    network: HolochainP2pCell,
) -> AppValidationResult<Outcome> {
    let access = ValidateLinkHostAccess::new(workspace_lock, network);
    let validate = ribosome.run_validate_link(access, invocation)?;
    match validate {
        ValidateLinkResult::Valid => Ok(Outcome::Accepted),
        ValidateLinkResult::Invalid(reason) => Ok(Outcome::Rejected(reason)),
        ValidateLinkResult::UnresolvedDependencies(hashes) => Ok(Outcome::AwaitingDeps(hashes)),
    }
}

pub struct AppValidationWorkspace {
    pub integrated_dht_ops: IntegratedDhtOpsStore,
    pub integration_limbo: IntegrationLimboStore,
    pub validation_limbo: ValidationLimboStore,
    // Integrated data
    pub element_vault: ElementBuf,
    pub meta_vault: MetadataBuf,
    // Data pending validation
    pub element_pending: ElementBuf<PendingPrefix>,
    pub meta_pending: MetadataBuf<PendingPrefix>,
    // Read only rejected store for finding dependency data
    pub element_rejected: ElementBuf<RejectedPrefix>,
    pub meta_rejected: MetadataBuf<RejectedPrefix>,
    // Read only authored store for finding dependency data
    pub element_authored: ElementBuf<AuthoredPrefix>,
    pub meta_authored: MetadataBuf<AuthoredPrefix>,
    // Cached data
    pub element_cache: ElementBuf,
    pub meta_cache: MetadataBuf,
    pub call_zome_workspace_lock: Option<CallZomeWorkspaceLock>,
}

impl AppValidationWorkspace {
    pub fn new(env: EnvironmentRead) -> WorkspaceResult<Self> {
        let db = env.get_db(&*INTEGRATED_DHT_OPS)?;
        let integrated_dht_ops = KvBufFresh::new(env.clone(), db);
        let db = env.get_db(&*INTEGRATION_LIMBO)?;
        let integration_limbo = KvBufFresh::new(env.clone(), db);

        let validation_limbo = ValidationLimboStore::new(env.clone())?;

        let element_vault = ElementBuf::vault(env.clone(), false)?;
        let meta_vault = MetadataBuf::vault(env.clone())?;
        let element_cache = ElementBuf::cache(env.clone())?;
        let meta_cache = MetadataBuf::cache(env.clone())?;

        let element_pending = ElementBuf::pending(env.clone())?;
        let meta_pending = MetadataBuf::pending(env.clone())?;

        // TODO: We probably want to use the app validation workspace instead of the call zome workspace
        // but we don't have a lock for that.
        // If we decide to allow app validation callbacks to be able to get dependencies from the
        // pending / judged stores then this will be needed as well.
        let call_zome_workspace = CallZomeWorkspace::new(env.clone())?;
        let call_zome_workspace_lock = Some(CallZomeWorkspaceLock::new(call_zome_workspace));

        // READ ONLY
        let element_authored = ElementBuf::authored(env.clone(), false)?;
        let meta_authored = MetadataBuf::authored(env.clone())?;
        let element_rejected = ElementBuf::rejected(env.clone())?;
        let meta_rejected = MetadataBuf::rejected(env)?;

        Ok(Self {
            integrated_dht_ops,
            integration_limbo,
            validation_limbo,
            element_vault,
            meta_vault,
            element_authored,
            meta_authored,
            element_pending,
            meta_pending,
            element_rejected,
            meta_rejected,
            element_cache,
            meta_cache,
            call_zome_workspace_lock,
        })
    }

    fn validation_workspace(&self) -> CallZomeWorkspaceLock {
        self.call_zome_workspace_lock
            .clone()
            .expect("Tried to use the validation workspace after it was flushed")
    }

    fn put_val_limbo(
        &mut self,
        hash: DhtOpHash,
        mut vlv: ValidationLimboValue,
    ) -> WorkflowResult<()> {
        vlv.last_try = Some(Timestamp::now());
        vlv.num_tries += 1;
        self.validation_limbo.put(hash, vlv)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, hash))]
    fn put_int_limbo(
        &mut self,
        hash: DhtOpHash,
        iv: IntegrationLimboValue,
        op: DhtOp,
    ) -> WorkflowResult<()> {
        self.integration_limbo.put(hash, iv)?;
        Ok(())
    }

    /// Get a cascade over all local databases and the network
    fn full_cascade<Network: HolochainP2pCellT>(
        &mut self,
        network: Network,
    ) -> Cascade<'_, Network> {
        let integrated_data = DbPair {
            element: &self.element_vault,
            meta: &self.meta_vault,
        };
        let authored_data = DbPair {
            element: &self.element_authored,
            meta: &self.meta_authored,
        };
        let pending_data = DbPair {
            element: &self.element_pending,
            meta: &self.meta_pending,
        };
        let rejected_data = DbPair {
            element: &self.element_rejected,
            meta: &self.meta_rejected,
        };
        let cache_data = DbPairMut {
            element: &mut self.element_cache,
            meta: &mut self.meta_cache,
        };
        Cascade::empty()
            .with_integrated(integrated_data)
            .with_authored(authored_data)
            .with_pending(pending_data)
            .with_cache(cache_data)
            .with_rejected(rejected_data)
            .with_network(network)
    }
}

impl Workspace for AppValidationWorkspace {
    fn flush_to_txn_ref(&mut self, writer: &mut Writer) -> WorkspaceResult<()> {
        self.validation_limbo.0.flush_to_txn_ref(writer)?;
        self.integration_limbo.flush_to_txn_ref(writer)?;
        self.element_pending.flush_to_txn_ref(writer)?;
        self.meta_pending.flush_to_txn_ref(writer)?;

        // Flush for cascade
        self.element_cache.flush_to_txn_ref(writer)?;
        self.meta_cache.flush_to_txn_ref(writer)?;

        // Need to flush the call zome workspace because of the cache.
        // TODO: If cache becomes a separate env then remove this
        if let Some(czws) = self
            .call_zome_workspace_lock
            .take()
            .and_then(|o| Arc::try_unwrap(o.into_inner()).ok())
        {
            let mut czws: CallZomeWorkspace = czws.into_inner();
            czws.flush_to_txn_ref(writer)?;
        }
        Ok(())
    }
}
