//! Actual bounded accepted Directory/Actions lookup from the SAME saved-A issuer.
//!
//! A token hit is retained custody, not acceptance. This distinct query pays for and
//! authenticates its own paths. A structural hit is only a locator until the complete
//! canonical record/action/header source has been verified. Neither DATA result mints
//! a signer, projection, accepted receipt, selector or private-root retirement grant.
use super::archive::{TextArchiveData, TextArchiveOperationData, MAX_ARCHIVE_PLAIN};
use super::capture::original_context;
use super::common::{refused, TextHeadIdData};
use super::control::{open_control_data, TextControlKindData};
use super::head::{TextHeadData, TextPendingData, TextRootData, MAX_TEXT_HEAD_FRAME};
use super::index::{
    AcceptedIndexPathData, ACCEPTED_INDEX_PATH_METADATA_BYTES, BOUNDARY_OPEN_BYTES,
};
use super::resources::V2TextReadResources;
use super::transaction::{
    OriginalV2TextRetainTransactionObservation, PreparedV2TextRetainTransaction,
    SavedV2RetainedAction,
};
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextAcceptedQuery, V2TextAcceptedQueryLease, V2TextAcceptedQuerySourceSlot,
    V2TextCurrentGroupGuard, V2_TEXT_ACCEPTED_QUERY_ADMISSION_LAYOUT_BYTES,
    V2_TEXT_CURRENT_GROUP_GUARD_BYTES,
};
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::ServerStore;
use crate::AppError;
use catcoms_replication::chat::cold_actions::OriginalActionDigest;
use catcoms_replication::chat::cold_index::{
    ActionClaimData, DirectoryData, DirectoryTreeData, IndexContextData, IndexScopeData, ValueData,
    MAX_INDEX_FRAME_BYTES, MAX_INDEX_NODE_BYTES,
};
use catcoms_replication::chat::cold_manifest::ArchiveHeadId;
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::owner_state::StateFrame;
use catcoms_replication::chat::ActionId;
use catcoms_storage::kept::SessionId;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Weak};
use zeroize::Zeroizing;

mod completed;
mod io;
mod next;
use io::{HeldAcceptedDirectory, HeldAcceptedFrame};
// Directory/Actions plus the completed-parent publication path have a full
// compiled Win invoice of 1,082,516 bytes. Reserve the SAME independent class
// before copies; other source/read/SDK/model/transaction debits stay additional.
// The real named assertion below must continue to fit on every supported target.
const QUERY_FEE_BYTES: usize = 1152 * 1024;
const MAX_PATH: usize = 5120;
const MAX_PATH_PAGES: usize = 10;
const CAPTURE_PREPARATION: u8 = 1;
const READER_PREPARATION: u8 = 2;

#[cfg(all(test, windows))]
thread_local! {
    // Scoped and consumed at the actual debit, before any paid source copy is made.
    static NEXT_PREPARE_OBSERVATION: std::cell::RefCell<Option<PrepareObservation>> = const { std::cell::RefCell::new(None) };
}
#[cfg(all(test, windows))]
struct PrepareObservation {
    fee: Arc<std::sync::atomic::AtomicBool>,
    parent: Arc<std::sync::atomic::AtomicBool>,
    after_copy: Box<dyn FnOnce() + Send>,
}
#[cfg(all(test, windows))]
struct ParentDropNotice(
    Option<(
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicBool>,
    )>,
);
#[cfg(all(test, windows))]
impl Drop for ParentDropNotice {
    fn drop(&mut self) {
        if let Some((parent, fee)) = &self.0 {
            assert!(
                !fee.load(Ordering::SeqCst),
                "accepted-query debit refunded before copied parent drained"
            );
            parent.store(true, Ordering::SeqCst);
        }
    }
}

/// Twenty retained path plains are paid, not just ten per independent scope. Original
/// Saved/read/SDK/model/transaction payments remain ADDITIONAL. The asserted bound includes
/// every simultaneous parent copy, native path/helper, decoder, claim and Root phase.
pub(in crate::store) const V2_TEXT_ACCEPTED_QUERY_LAYOUT_BYTES: usize =
    2 * std::mem::size_of::<PreparedV2TextAcceptedQuery>()
        + 2 * std::mem::size_of::<CapturedV2TextAcceptedQuery>()
        + 2 * std::mem::size_of::<AcceptedQueryPlan>()
        + 2 * std::mem::size_of::<AcceptedParent>()
        + 12 * std::mem::size_of::<OriginalV2TextAcceptedQueryObservation>()
        + next::COMPLETED_PUBLICATION_QUERY_LAYOUT_BYTES
        + 2 * ACCEPTED_INDEX_PATH_METADATA_BYTES
        + 2 * MAX_PATH_PAGES * (MAX_INDEX_NODE_BYTES + 16)
        + 2 * BOUNDARY_OPEN_BYTES
        + 12 * (MAX_PATH + 256)
        + 12 * (MAX_TEXT_HEAD_FRAME + MAX_ARCHIVE_PLAIN + 40)
        + 2 * catcoms_replication::chat::MAX_ACTION_BYTES
        + V2_TEXT_CURRENT_GROUP_GUARD_BYTES
        + V2_TEXT_ACCEPTED_QUERY_ADMISSION_LAYOUT_BYTES
        + 64 * std::mem::size_of::<File>()
        + 16 * 1024
        + 48 * 1024;
const _: () = assert!(V2_TEXT_ACCEPTED_QUERY_LAYOUT_BYTES <= QUERY_FEE_BYTES);

struct QueryPayment {
    pool: CriticalCatalogReadPool,
    // LAST: all objects/copies covered by this new independent debit drain first.
    fee: CatalogReadReservation,
}
#[derive(Clone)]
pub(in crate::store) struct V2TextAcceptedQueryResources {
    body: Arc<QueryPayment>,
}
impl V2TextAcceptedQueryResources {
    pub(in crate::store) fn validate_prepaid_account(&self) -> Result<(), AppError> {
        self.body.fee.validate(&self.body.pool, QUERY_FEE_BYTES)
    }
    fn require_same_account(&self, other: &Self) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.body, &other.body) {
            return Err(refused());
        }
        Ok(())
    }
}

struct AcceptedParent {
    target: PathBuf,
    target_id: SessionId,
    pages: PathBuf,
    pages_id: SessionId,
    head_path: PathBuf,
    archive_path: PathBuf,
    selected_head: TextHeadIdData,
    head_frame: Zeroizing<Vec<u8>>,
    archive_frame: Zeroizing<Vec<u8>>,
    head: TextHeadData,
    archive: TextArchiveData,
    accepted_archive: TextArchiveData,
    accepted_archive_id: ArchiveHeadId,
    // Only fixed original action labels are copied, not the SDK packet or plaintext.
    action: ActionId,
    original_action: OriginalActionDigest,
    original: OriginalV2TextRetainTransactionObservation,
    // LAST: genuine small Saved/read tails are additional to this query's own invoice.
    saved: Arc<SavedV2RetainedAction>,
    resources: V2TextReadResources,
    // LAST: observes destruction of all query-owned copied source fields above.
    #[cfg(all(test, windows))]
    _drop_notice: ParentDropNotice,
}
impl AcceptedParent {
    fn require_same_parent(&self, other: &Self) -> Result<(), AppError> {
        self.resources.require_same_original(&other.resources)?;
        self.original.require_same_original(&other.original)?;
        if self.target != other.target
            || self.target_id != other.target_id
            || self.pages != other.pages
            || self.pages_id != other.pages_id
            || self.head_path != other.head_path
            || self.archive_path != other.archive_path
            || self.head_frame != other.head_frame
            || self.archive_frame != other.archive_frame
            || self.head.context != other.head.context
            || self.head.directory != other.head.directory
            || self.head.tokens != other.head.tokens
            || self.head.publications != other.head.publications
            || self.head.generation != other.head.generation
            || self.accepted_archive_id != other.accepted_archive_id
        {
            return Err(refused());
        }
        Ok(())
    }
}
struct QueryControl {
    preparations: AtomicU8,
}
pub(in crate::store) struct V2TextAcceptedQueryCaptureClaim {
    control: Arc<QueryControl>,
    _payment: V2TextAcceptedQueryResources,
}
pub(in crate::store) struct V2TextAcceptedQueryReaderClaim {
    control: Arc<QueryControl>,
    _payment: V2TextAcceptedQueryResources,
}
macro_rules! claim_end {
    ($ty:ident,$bit:ident) => {
        impl Drop for $ty {
            fn drop(&mut self) {
                let previous = self.control.preparations.fetch_and(!$bit, Ordering::AcqRel);
                debug_assert_ne!(previous & $bit, 0, "one affine accepted-query claim");
            }
        }
    };
}
claim_end!(V2TextAcceptedQueryCaptureClaim, CAPTURE_PREPARATION);
claim_end!(V2TextAcceptedQueryReaderClaim, READER_PREPARATION);

struct AcceptedQueryPlan {
    identity: Arc<QueryControl>,
    parent: Arc<AcceptedParent>,
    // LAST: frame/path/observer copies cannot outlive the debit funding those copies.
    payment: V2TextAcceptedQueryResources,
}
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextAcceptedQueryObservation {
    identity: Weak<QueryControl>,
    parent: Arc<AcceptedParent>,
    payment: V2TextAcceptedQueryResources,
}
impl OriginalV2TextAcceptedQueryObservation {
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        self.payment.require_same_account(&other.payment)?;
        if !self.identity.ptr_eq(&other.identity) {
            return Err(refused());
        }
        self.require_same_selected_parent(other)
    }
    pub(in crate::store) fn require_same_selected_parent(
        &self,
        other: &Self,
    ) -> Result<(), AppError> {
        self.parent.require_same_parent(&other.parent)
    }
}

/// Physical complete-path DATA only. Present is an UNVERIFIED locator until canonical
/// record/action/header evidence is checked; it must never become an implicit miss.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum V2TextAcceptedQueryResultData {
    Absent,
    Present(ActionClaimData),
}

pub(in crate::store) struct PreparedV2TextAcceptedQuery {
    directory: Option<AcceptedIndexPathData>,
    actions: Option<AcceptedIndexPathData>,
    publication_path:
        Option<crate::store::persistence::v2_text::private_index::PrivateIndexPathData>,
    result: Option<V2TextAcceptedQueryResultData>,
    completed_reader: Option<V2TextAcceptedQueryLease>,
    #[cfg(all(test, windows))]
    after_capture: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    after_run_source: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    fail_after_completed: bool,
    // LAST after paths, real reader and every cached result/control tail.
    plan: Arc<AcceptedQueryPlan>,
}
impl PreparedV2TextAcceptedQuery {
    /// The actual independently reserved class, for exact-payment/pressure tests.
    #[cfg(all(test, windows))]
    pub(in crate::store) const fn metadata_fee_bytes_for_test() -> usize {
        QUERY_FEE_BYTES
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn with_prepare_observation_for_test<T>(
        fee: Arc<std::sync::atomic::AtomicBool>,
        parent: Arc<std::sync::atomic::AtomicBool>,
        after_copy: Box<dyn FnOnce() + Send>,
        prepare: impl FnOnce() -> T,
    ) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                NEXT_PREPARE_OBSERVATION.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        NEXT_PREPARE_OBSERVATION.with(|slot| {
            assert!(slot.borrow().is_none(), "nested accepted-query observation");
            *slot.borrow_mut() = Some(PrepareObservation {
                fee,
                parent,
                after_copy,
            });
        });
        let _reset = Reset;
        let output = prepare();
        NEXT_PREPARE_OBSERVATION.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "fixture did not acquire its actual query debit"
            );
        });
        output
    }
    /// Only the actual original Cell can lend this exact SavedA source. No caller root,
    /// generic parsed snapshot, token-miss Boolean or imported SDK can select it.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        cell: &PreparedV2TextRetainTransaction,
    ) -> Result<Self, AppError> {
        let input = cell.borrow_saved_a_query_source_data()?;
        input.resources().validate_original(store, source)?;
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool
            .clone();
        let fee = pool.try_reserve(QUERY_FEE_BYTES)?;
        #[cfg(all(test, windows))]
        let observation = NEXT_PREPARE_OBSERVATION.with(|slot| slot.borrow_mut().take());
        #[cfg(all(test, windows))]
        let fee = {
            let mut fee = fee;
            if let Some(observation) = &observation {
                fee.observe_drop_for_test(observation.fee.clone());
            }
            fee
        };
        fee.validate(&pool, QUERY_FEE_BYTES)?;
        // Declare the actual payment BEFORE keys, decoded controls and copied parent.
        // On every early return, reverse local destruction order must drain those
        // allocations before refunding this independent query allowance.
        let payment = V2TextAcceptedQueryResources {
            body: Arc::new(QueryPayment { pool, fee }),
        };
        input.resources().validate_original(store, source)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let head_plain = open_control_data(
            &key,
            TextControlKindData::Head.id_data(input.current_head_frame()),
            input.current_head_frame(),
        )?;
        let head = TextHeadData::decode_data(&head_plain)?;
        let archive_plain = open_control_data(
            &key,
            TextControlKindData::Archive.id_data(input.current_archive_frame()),
            input.current_archive_frame(),
        )?;
        let archive = TextArchiveData::decode_data(&archive_plain)?;
        let accepted = input.accepted_archive_data();
        let a = archive.spec_data();
        let g = accepted.spec_data();
        let c = head.context.spec_data();
        let accepted_id = ArchiveHeadId::from_bytes(
            *TextControlKindData::Archive
                .id_data(input.accepted_archive_frame())
                .as_bytes(),
        );
        if input.head_id_data() != TextHeadIdData::of_data(input.current_head_frame())
            || head.archive.as_bytes()
                != TextControlKindData::Archive
                    .id_data(input.current_archive_frame())
                    .as_bytes()
            || !matches!(head.pending, TextPendingData::Retained(_))
            || a.context != head.context.id_data()?
            || g.context != a.context
            || a.operation != TextArchiveOperationData::RetainAction
            || a.previous != Some(accepted_id)
            || a.header != g.header
            || a.image != g.image
            || c.profile != source.data().profile.id()
            || c.admission != source.data().admission.id()
            || c.signer != *input.action_data().signer()
        {
            return Err(refused());
        }
        source
            .data()
            .admission
            .matches_action_delegation(input.action_data())
            .map_err(|_| refused())?;
        let (target, target_id) = input.target_data();
        let (pages, pages_id) = input.pages_data();
        let (head_path, archive_path) = input.paths_data();
        let parent = Arc::new(AcceptedParent {
            target: owned_path(target)?,
            target_id,
            pages: owned_path(pages)?,
            pages_id,
            head_path: owned_path(head_path)?,
            archive_path: owned_path(archive_path)?,
            selected_head: TextHeadIdData::of_data(input.current_head_frame()),
            head_frame: Zeroizing::new(input.current_head_frame().to_vec()),
            archive_frame: Zeroizing::new(input.current_archive_frame().to_vec()),
            accepted_archive: TextArchiveData::decode_data(&accepted.encode_data()?)?,
            accepted_archive_id: accepted_id,
            action: input.action_data().id(),
            original_action: input.original_action_digest_data(),
            original: input.original_data().clone(),
            saved: input.saved_data().clone(),
            resources: input.resources().clone(),
            head,
            archive,
            #[cfg(all(test, windows))]
            _drop_notice: ParentDropNotice(
                observation
                    .as_ref()
                    .map(|notice| (notice.parent.clone(), notice.fee.clone())),
            ),
        });
        #[cfg(all(test, windows))]
        if let Some(observation) = observation {
            (observation.after_copy)();
        }
        parent
            .resources
            .require_bound_original_negative(store, source)?;
        Ok(Self {
            directory: None,
            actions: None,
            publication_path: None,
            result: None,
            completed_reader: None,
            #[cfg(all(test, windows))]
            after_capture: None,
            #[cfg(all(test, windows))]
            after_run_source: None,
            #[cfg(all(test, windows))]
            fail_after_completed: false,
            plan: Arc::new(AcceptedQueryPlan {
                identity: Arc::new(QueryControl {
                    preparations: AtomicU8::new(0),
                }),
                parent,
                payment,
            }),
        })
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.plan.parent.resources
    }
    /// Immutable association only, intended for an owning I constructor BEFORE takes.
    /// It cannot renew Current, certify a miss or adopt another Cell's completed query.
    pub(in crate::store) fn require_same_retained_data(
        &self,
        cell: &PreparedV2TextRetainTransaction,
    ) -> Result<(), AppError> {
        let source = cell.borrow_saved_a_query_source_data()?;
        self.plan
            .parent
            .original
            .require_same_original(source.original_data())?;
        self.resources().require_same_original(source.resources())?;
        if !Arc::ptr_eq(&self.plan.parent.saved, source.saved_data())
            || self.plan.parent.action != cell.producer_data().original_action_id_data()?
            || self.plan.parent.original_action != source.original_action_digest_data()
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn query_resources(&self) -> &V2TextAcceptedQueryResources {
        &self.plan.payment
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextAcceptedQueryObservation {
        self.plan.observe_original()
    }
    pub(in crate::store) fn validate_preparation(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        if self.completed_reader.is_some() {
            return Err(refused());
        }
        self.plan.payment.validate_prepaid_account()?;
        self.resources().validate_original(store, source)
    }
    fn claim(&self, bit: u8) -> Result<(), AppError> {
        self.plan.payment.validate_prepaid_account()?;
        if self.completed_reader.is_some() {
            return Err(refused());
        }
        self.plan
            .identity
            .preparations
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                if old & bit == 0 {
                    Some(old | bit)
                } else {
                    None
                }
            })
            .map_err(|_| refused())?;
        Ok(())
    }
    pub(in crate::store) fn claim_capture_preparation(
        &self,
    ) -> Result<V2TextAcceptedQueryCaptureClaim, AppError> {
        self.claim(CAPTURE_PREPARATION)?;
        Ok(V2TextAcceptedQueryCaptureClaim {
            control: self.plan.identity.clone(),
            _payment: self.plan.payment.clone(),
        })
    }
    pub(in crate::store) fn claim_reader_preparation(
        &self,
    ) -> Result<V2TextAcceptedQueryReaderClaim, AppError> {
        self.claim(READER_PREPARATION)?;
        Ok(V2TextAcceptedQueryReaderClaim {
            control: self.plan.identity.clone(),
            _payment: self.plan.payment.clone(),
        })
    }
    pub(in crate::store) fn validate_parent_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.plan.validate_selected(guard)
    }
    pub(in crate::store) fn capture(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        slot: &mut V2TextAcceptedQuerySourceSlot<'_>,
    ) -> Result<(), AppError> {
        self.plan.validate_original(guard)?;
        if slot.get().is_none() {
            slot.cache_initial(CapturedV2TextAcceptedQuery::empty(self.plan.clone()));
        }
        let captured = slot.get_mut().ok_or_else(refused)?;
        captured.require_same_query(&self.observe_original())?;
        captured.fill(guard)?;
        #[cfg(all(test, windows))]
        if let Some(hook) = self.after_capture.take() {
            hook();
        }
        captured.validate_under_current_guard(guard)
    }
    pub(in crate::store) fn has_completed_data(&self) -> bool {
        self.result.is_some() && self.completed_reader.is_some()
    }
    /// Consume the actual accepted-query admission. Completed path/result DATA caches
    /// before Source/key/OS/finiteIO cleanup and SAME logical-reader transfer. Failed IO
    /// never produces Absent; it preserves already authenticated path nodes for retry.
    pub(in crate::store) fn run(
        &mut self,
        admitted: AdmittedV2TextAcceptedQuery,
    ) -> Result<(), AppError> {
        admitted.require_same_query(&self.observe_original())?;
        if self.completed_reader.is_some() {
            return Err(refused());
        }
        self.plan.payment.validate_prepaid_account()?;
        let result = self.run_admitted(&admitted);
        if result.is_ok() {
            admitted.finish_into(&mut self.completed_reader);
            #[cfg(all(test, windows))]
            if std::mem::take(&mut self.fail_after_completed) {
                return Err(refused());
            }
        } else {
            drop(admitted);
        }
        result
    }
    fn run_admitted(&mut self, admitted: &AdmittedV2TextAcceptedQuery) -> Result<(), AppError> {
        let source = admitted.source();
        source.require_same_query(&self.observe_original())?;
        source.require_unchanged()?;
        #[cfg(all(test, windows))]
        if let Some(hook) = self.after_run_source.take() {
            hook();
        }
        if self.result.is_some() {
            return Ok(());
        }
        let p = &self.plan.parent;
        let c = p.head.context.spec_data();
        let TextRootData::Present(directory_root) = p.head.directory else {
            return Err(refused());
        };
        if self.directory.is_none() {
            self.directory = Some(AcceptedIndexPathData::new_data(
                IndexContextData {
                    vault: c.layout.vault,
                    group: c.group,
                    profile: c.profile,
                    scope: IndexScopeData::Directory,
                },
                directory_root,
                *c.admission.as_bytes(),
                None,
            )?);
        }
        let directory_path = self.directory.as_mut().ok_or_else(refused)?;
        descend(source, directory_path)?;
        // Missing OWN Directory is incomplete history, not accepted absence. Unknown
        // tree evidence likewise remains unavailable instead of choosing a new child.
        let row = directory_path.row_data()?.ok_or_else(refused)?;
        let ValueData::Directory(directory) = row.value else {
            return Err(refused());
        };
        if row.key != *c.admission.as_bytes() {
            return Err(refused());
        }
        validate_directory(&self.plan.parent, directory)?;
        let result = match directory.tree {
            DirectoryTreeData::Unknown => return Err(refused()),
            DirectoryTreeData::Empty => V2TextAcceptedQueryResultData::Absent,
            DirectoryTreeData::Present(actions_root) => {
                if self.actions.is_none() {
                    self.actions = Some(AcceptedIndexPathData::new_data(
                        IndexContextData {
                            vault: c.layout.vault,
                            group: c.group,
                            profile: c.profile,
                            scope: IndexScopeData::Actions(OwnerFactId::from_bytes(
                                *c.admission.as_bytes(),
                            )),
                        },
                        actions_root,
                        *p.action.as_bytes(),
                        Some(directory.accepted_leaf_count),
                    )?);
                }
                let actions = self.actions.as_mut().ok_or_else(refused)?;
                descend(source, actions)?;
                match actions.row_data()? {
                    None => V2TextAcceptedQueryResultData::Absent,
                    Some(row) => {
                        if row.key != *p.action.as_bytes() {
                            return Err(refused());
                        }
                        let ValueData::Action(claim) = row.value else {
                            return Err(refused());
                        };
                        V2TextAcceptedQueryResultData::Present(claim)
                    }
                }
            }
        };
        self.read_publication_insertion_data(source)?;
        source.require_unchanged()?;
        self.result = Some(result);
        Ok(())
    }
    pub(in crate::store) fn result_data(&self) -> Result<&V2TextAcceptedQueryResultData, AppError> {
        if !self.has_completed_data() {
            return Err(refused());
        }
        self.result.as_ref().ok_or_else(refused)
    }
    pub(in crate::store) fn validate_completed_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.result_data()?;
        let original = self.observe_original();
        let reader = self.completed_reader.as_ref().ok_or_else(refused)?;
        reader.require_same_query(&original)?;
        reader.require_live_query(&original)?;
        self.plan.validate_selected(guard)?;
        reader.require_live_under_current_guard(guard, &original)?;
        reader.require_live_query(&original)?;
        guard.validate_source_only()
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_after_capture_files_for_test(
        &mut self,
        hook: Box<dyn FnOnce() + Send>,
    ) {
        self.after_capture = Some(hook);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_after_run_source_for_test(
        &mut self,
        hook: Box<dyn FnOnce() + Send>,
    ) {
        self.after_run_source = Some(hook);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn fail_after_completed_for_test(&mut self) {
        self.fail_after_completed = true;
    }
}

/// Public slot/image consistency is necessary but does not establish selected governing
/// replay by itself. The later I consumer also retains its original verified math/prefix.
/// H_G remains the accepted prefix while H_A's new manifest only retains unaccepted A.
fn validate_directory(parent: &AcceptedParent, directory: DirectoryData) -> Result<(), AppError> {
    let c = parent.head.context.spec_data();
    let accepted = parent.accepted_archive.spec_data();
    let current = parent.archive.spec_data();
    let state = StateFrame::decode(current.image).map_err(|_| refused())?;
    state
        .verify_header_binding(current.header)
        .map_err(|_| refused())?;
    let slot = state
        .slots()
        .find(|s| s.admission == c.admission)
        .ok_or_else(refused)?;
    if slot.subject != c.signer
        || directory.accepted_leaf_count != slot.accepted_count
        || directory.accepted_root != slot.accepted_root
        || directory.tip_body != slot.tip_body
        || directory.own_tip_header != accepted.header.id()
        || directory.prefix_head != parent.accepted_archive_id
        || directory.prefix_manifest != accepted.manifest
        || directory.prefix_sequence != accepted.manifest_sequence
    {
        return Err(refused());
    }
    Ok(())
}

fn descend(
    source: &CapturedV2TextAcceptedQuery,
    path: &mut AcceptedIndexPathData,
) -> Result<(), AppError> {
    let key = source.key.as_ref().ok_or_else(refused)?;
    let parent = &source.plan.parent;
    for _ in 0..MAX_PATH_PAGES {
        let Some(page) = path.next_page_data()? else {
            path.row_data()?;
            return Ok(());
        };
        let name = page_child(&parent.pages, page.as_bytes())?;
        let mut frame = HeldAcceptedFrame::empty();
        frame.capture(&name, MAX_INDEX_FRAME_BYTES)?;
        source.pages.require_name(&parent.pages, parent.pages_id)?;
        frame.require_unchanged(&name)?;
        path.accept_frame_data(key, frame.raw()?)?;
        // Retain authenticated mathematical progress before this last fallible name
        // proof. A retry still needs a NEW actual read admission on the SAME source.
        frame.require_name(&name)?;
    }
    if path.next_page_data()?.is_some() {
        return Err(refused());
    }
    path.row_data()?;
    Ok(())
}

impl AcceptedQueryPlan {
    fn observe_original(&self) -> OriginalV2TextAcceptedQueryObservation {
        OriginalV2TextAcceptedQueryObservation {
            identity: Arc::downgrade(&self.identity),
            parent: self.parent.clone(),
            payment: self.payment.clone(),
        }
    }
    fn validate_original(&self, guard: &V2TextCurrentGroupGuard<'_, '_>) -> Result<(), AppError> {
        self.payment.validate_prepaid_account()?;
        self.parent
            .resources
            .require_same_original(guard.resources())?;
        self.parent
            .resources
            .validate_original(guard.store(), guard.source())?;
        let (target, _, id) = guard.target_directory_data();
        if target != self.parent.target || id != self.parent.target_id {
            return Err(refused());
        }
        Ok(())
    }
    fn validate_selected(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.validate_original(guard)?;
        self.parent
            .saved
            .validate_selected_under_current_guard(guard)
    }
}

/// All retained native ownership remains in the concrete Factory marker. Its public loan
/// is immutable; no mutable Source/File/owning Option can escape or swap vault custody.
pub(in crate::store) struct CapturedV2TextAcceptedQuery {
    target: HeldAcceptedDirectory,
    pages: HeldAcceptedDirectory,
    head: HeldAcceptedFrame,
    archive: HeldAcceptedFrame,
    key: Option<Zeroizing<[u8; 32]>>,
    plan: Arc<AcceptedQueryPlan>,
}
impl CapturedV2TextAcceptedQuery {
    fn empty(plan: Arc<AcceptedQueryPlan>) -> Self {
        Self {
            target: HeldAcceptedDirectory::empty(),
            pages: HeldAcceptedDirectory::empty(),
            head: HeldAcceptedFrame::empty(),
            archive: HeldAcceptedFrame::empty(),
            key: None,
            plan,
        }
    }
    fn require_same_query(
        &self,
        original: &OriginalV2TextAcceptedQueryObservation,
    ) -> Result<(), AppError> {
        self.plan.observe_original().require_same_original(original)
    }
    fn fill(&mut self, guard: &V2TextCurrentGroupGuard<'_, '_>) -> Result<(), AppError> {
        self.plan.validate_original(guard)?;
        if self.key.is_none() {
            self.key = Some(Zeroizing::new(guard.store().keys.db_key()?));
        }
        let p = &self.plan.parent;
        self.target.capture(&p.target)?;
        self.pages.capture(&p.pages)?;
        self.head.capture(&p.head_path, MAX_TEXT_HEAD_FRAME)?;
        self.archive
            .capture(&p.archive_path, MAX_ARCHIVE_PLAIN + 40)
    }
    pub(in crate::store) fn validate_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.plan.validate_original(guard)?;
        if original_context(guard, self.key.as_ref().ok_or_else(refused)?)?
            != self.plan.parent.head.context
        {
            return Err(refused());
        }
        self.require_unchanged()?;
        // Capture callbacks can fail or observe change after the pre-pin proof. Repeat
        // the genuine complete small SavedA closure, including T_A, before admission.
        self.plan
            .parent
            .saved
            .validate_selected_under_current_guard(guard)?;
        guard.validate_original_namespace()?;
        self.require_unchanged()?;
        guard.validate_source_only()
    }
    fn require_unchanged(&self) -> Result<(), AppError> {
        let p = &self.plan.parent;
        if self.head.raw()? != p.head_frame.as_slice()
            || self.archive.raw()? != p.archive_frame.as_slice()
        {
            return Err(refused());
        }
        self.head.require_unchanged(&p.head_path)?;
        self.archive.require_unchanged(&p.archive_path)?;
        self.target.require_name(&p.target, p.target_id)?;
        self.pages.require_name(&p.pages, p.pages_id)
    }
}

fn owned_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().len() > MAX_PATH {
        return Err(refused());
    }
    let mut out = PathBuf::with_capacity(path.as_os_str().len());
    out.push(path);
    if out.capacity() > MAX_PATH {
        return Err(refused());
    }
    Ok(out)
}
fn page_child(base: &Path, id: &[u8; 32]) -> Result<PathBuf, AppError> {
    let cap = base
        .as_os_str()
        .len()
        .checked_add(71)
        .filter(|n| *n <= MAX_PATH)
        .ok_or_else(refused)?;
    let mut name = String::with_capacity(70);
    name.push_str("p-");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in id {
        name.push(HEX[(byte >> 4) as usize] as char);
        name.push(HEX[(byte & 15) as usize] as char);
    }
    name.push_str(".bin");
    let mut out = PathBuf::with_capacity(cap);
    out.push(base);
    out.push(name);
    if out.capacity() > MAX_PATH {
        return Err(refused());
    }
    Ok(out)
}

macro_rules! redacted {($($ty:ident),+$(,)?)=>{$(impl std::fmt::Debug for $ty{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.debug_struct(stringify!($ty)).finish_non_exhaustive()}})+};}
redacted!(
    V2TextAcceptedQueryResources,
    V2TextAcceptedQueryCaptureClaim,
    V2TextAcceptedQueryReaderClaim,
    OriginalV2TextAcceptedQueryObservation,
    PreparedV2TextAcceptedQuery,
    CapturedV2TextAcceptedQuery
);

mod drain;
