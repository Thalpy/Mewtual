//! A bounded physical token-index descent from an actual selected O/H generation.
//!
//! Neither a decoded leaf nor an empty root authorizes a new Create. In particular an
//! unselected complete WAL/stage can coexist with O and must be classified separately.
//! This owner never signs, edits a selector, adopts a caller root or scans a directory.
mod completed_source;
use super::archive::MAX_ARCHIVE_PLAIN;
use super::capture::{original_context, SelectedV2TextHead};
use super::common::{refused, TextHeadIdData};
use super::context::TextContextData;
use super::head::{TextPendingData, TextRootData, MAX_TEXT_HEAD_FRAME};
use super::io::MAX_TEXT_PATH;
use super::private_index::{
    PrivateIndexContextData, PrivateIndexKindData, PrivateIndexPathData, PrivateIndexValueData,
    PRIVATE_INDEX_BOUNDARY_OPEN_BYTES, PRIVATE_INDEX_PATH_METADATA_BYTES,
};
use super::request::{TextTokenKeyData, TokenValueData};
use super::resources::V2TextReadResources;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextTokenQuery, V2TextCurrentGroupGuard, V2TextQueryLease,
    V2TextTokenQuerySourceSlot, V2_TEXT_CURRENT_GROUP_GUARD_BYTES,
    V2_TEXT_TOKEN_QUERY_ADMISSION_LAYOUT_BYTES,
};
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::ServerStore;
use crate::AppError;
use catcoms_replication::chat::cold_index::{MAX_INDEX_FRAME_BYTES, MAX_INDEX_NODE_BYTES};
use catcoms_storage::kept::{session_at, session_id, SessionAt, SessionId};
use catcoms_storage::private_chat_page::PrivateChatPageId;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Weak};
use zeroize::Zeroizing;

const QUERY_FEE_BYTES: usize = 1024 * 1024;
const MAX_QUERY_PAGES: usize = 10;

/// One independently charged query, including its retained path and actual active source,
/// native path conversions, selected-frame copies, final rechecks and parent guard. Original
/// read/SDK/Source payments remain ADDITIONAL. The full assertion precedes any allocation.
pub(in crate::store) const V2_TEXT_TOKEN_QUERY_LAYOUT_BYTES: usize = 2 * std::mem::size_of::<PreparedV2TextTokenQuery>()
        + 2 * std::mem::size_of::<CapturedV2TextTokenQuery>()
        + 2 * std::mem::size_of::<QueryPlan>()
        + 2 * std::mem::size_of::<SelectedQueryParent>()
        + 12 * std::mem::size_of::<OriginalV2TextTokenQueryObservation>()
        + std::mem::size_of::<QueryPayment>()
        + 2 * std::mem::size_of::<QueryControl>()
        + 2 * std::mem::size_of::<V2TextTokenQueryCaptureClaim>()
        + 2 * std::mem::size_of::<V2TextTokenQueryReaderClaim>()
        + V2_TEXT_CURRENT_GROUP_GUARD_BYTES
        + V2_TEXT_TOKEN_QUERY_ADMISSION_LAYOUT_BYTES
        + PRIVATE_INDEX_PATH_METADATA_BYTES
        + MAX_QUERY_PAGES * (MAX_INDEX_NODE_BYTES + 16)
        + 2 * PRIVATE_INDEX_BOUNDARY_OPEN_BYTES
        + 12 * (MAX_TEXT_PATH + 256)
        + 8 * (MAX_TEXT_HEAD_FRAME + MAX_ARCHIVE_PLAIN + 40)
        + 32 * std::mem::size_of::<File>()
        + 32 * 64
        // Root's ONE actual Lookup phase, including outside active/admitted slots and
        // simultaneous transition tags. Root asserts the concrete phase is <=8KiB.
        + 8 * 1024
        + 48 * 1024;
const _: () = assert!(V2_TEXT_TOKEN_QUERY_LAYOUT_BYTES <= QUERY_FEE_BYTES);

struct QueryPayment {
    pool: CriticalCatalogReadPool,
    // LAST: the original account remains retained until the final actual debit refund.
    fee: CatalogReadReservation,
}

/// Resource-only loan. Active source=None wrappers also retain this SAME payment; they
/// cannot outlive their credit merely because the original PreparedQuery was dropped.
#[derive(Clone)]
pub(in crate::store) struct V2TextTokenQueryResources {
    body: Arc<QueryPayment>,
}
impl V2TextTokenQueryResources {
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

/// Immutable exact selected physical facts, constructible ONLY from the actual capture.
/// It retains no obsolete O/H File, Store, SDK, key, source owner or physical session.
struct SelectedQueryParent {
    target: PathBuf,
    target_id: SessionId,
    pages: PathBuf,
    pages_id: SessionId,
    head_path: PathBuf,
    archive_path: PathBuf,
    selected_head: TextHeadIdData,
    head: Zeroizing<Vec<u8>>,
    archive: Zeroizing<Vec<u8>>,
    context: TextContextData,
    generation: u64,
    directory: TextRootData,
    tokens: TextRootData,
    publications: TextRootData,
    pending: TextPendingData,
    resources: V2TextReadResources,
}
impl SelectedQueryParent {
    fn from_selected(selected: &SelectedV2TextHead) -> Result<Self, AppError> {
        let source = selected.physical_query_data();
        let head = selected.head_data();
        Ok(Self {
            target: owned_path(source.target)?,
            target_id: source.target_id,
            pages: owned_path(source.pages)?,
            pages_id: source.pages_id,
            head_path: owned_path(source.head_path)?,
            archive_path: owned_path(source.archive_path)?,
            selected_head: TextHeadIdData::of_data(source.head_frame),
            head: Zeroizing::new(source.head_frame.to_vec()),
            archive: Zeroizing::new(source.archive_frame.to_vec()),
            context: head.context.clone(),
            generation: head.generation,
            directory: head.directory,
            tokens: head.tokens,
            publications: head.publications,
            pending: head.pending,
            resources: selected.resources().clone(),
        })
    }
    fn require_same_selected(&self, other: &Self) -> Result<(), AppError> {
        self.resources.require_same_original(&other.resources)?;
        if self.target != other.target
            || self.target_id != other.target_id
            || self.pages != other.pages
            || self.pages_id != other.pages_id
            || self.head_path != other.head_path
            || self.archive_path != other.archive_path
            || self.head != other.head
            || self.archive != other.archive
            || self.context != other.context
            || self.generation != other.generation
            || self.directory != other.directory
            || self.tokens != other.tokens
            || self.publications != other.publications
            || self.pending != other.pending
        {
            return Err(refused());
        }
        Ok(())
    }
}

const CAPTURE_PREPARATION: u8 = 1;
const READER_PREPARATION: u8 = 2;
struct QueryControl {
    preparations: AtomicU8,
}

/// One maximum active source precursor for this exact query. Factory declares this AFTER
/// actual Files/key/OS lease, so reuse becomes possible only after their physical cleanup.
/// No public/adopting constructor or Clone can duplicate its bounded invoice slot.
pub(in crate::store) struct V2TextTokenQueryCaptureClaim {
    control: Arc<QueryControl>,
    // LAST: even a failed preparation retains the actual independent query debit.
    _payment: V2TextTokenQueryResources,
}
/// One maximum reader precursor/result lifetime. It transfers unchanged from candidate to
/// actual registered reader and clears only AFTER that real reader slot's final cleanup.
pub(in crate::store) struct V2TextTokenQueryReaderClaim {
    control: Arc<QueryControl>,
    _payment: V2TextTokenQueryResources,
}
macro_rules! release_query_claim {
    ($ty:ident, $bit:ident) => {
        impl Drop for $ty {
            fn drop(&mut self) {
                let previous = self.control.preparations.fetch_and(!$bit, Ordering::AcqRel);
                debug_assert_ne!(previous & $bit, 0, "one affine query preparation claim");
            }
        }
    };
}
release_query_claim!(V2TextTokenQueryCaptureClaim, CAPTURE_PREPARATION);
release_query_claim!(V2TextTokenQueryReaderClaim, READER_PREPARATION);

struct QueryPlan {
    identity: Arc<QueryControl>,
    parent: Arc<SelectedQueryParent>,
    token: [u8; 32],
    key: [u8; 32],
    // LAST: every copied selected frame/path, context and negative identity precedes fee.
    payment: V2TextTokenQueryResources,
}

/// Opaque original query and complete selected-generation observation. No weak upgrade,
/// account/FS wait or liveness check occurs in its identity comparisons. An already admitted
/// finite read can finish after cancellation without converting this observation into Current.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextTokenQueryObservation {
    identity: Weak<QueryControl>,
    parent: Arc<SelectedQueryParent>,
    // LAST: the parent/data/control tail cannot refund before its final actual drop.
    payment: V2TextTokenQueryResources,
}
impl OriginalV2TextTokenQueryObservation {
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
        self.parent.require_same_selected(&other.parent)
    }
}

/// A unique paid physical query. Its completed path is immutable DATA; the actual Factory
/// reader remains necessary for present use and a separate clear-workspace proof is required
/// for new Create admission. Errors preserve the original path/next-page opportunity.
pub(in crate::store) struct PreparedV2TextTokenQuery {
    path: Option<PrivateIndexPathData>,
    completed: bool,
    // Actual affine root reader drains AFTER the path, before the final paid plan.
    completed_reader: Option<V2TextQueryLease>,
    #[cfg(all(test, windows))]
    after_capture: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    after_run_source: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    fail_after_completed: bool,
    // LAST: retained plaintext/path and all phase metadata drain before this final fee.
    plan: Arc<QueryPlan>,
}
impl PreparedV2TextTokenQuery {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        token: [u8; 32],
    ) -> Result<Self, AppError> {
        selected.resources().validate_original(store, source)?;
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool
            .clone();
        let fee = pool.try_reserve(QUERY_FEE_BYTES)?;
        fee.validate(&pool, QUERY_FEE_BYTES)?;
        // Repeat source/mount/SDK negatives after the account wait and BEFORE copying.
        selected.resources().validate_original(store, source)?;
        let payment = V2TextTokenQueryResources {
            body: Arc::new(QueryPayment { pool, fee }),
        };
        let parent = Arc::new(SelectedQueryParent::from_selected(selected)?);
        let key = *TextTokenKeyData::from_request_data(parent.context.id_data()?, token).as_bytes();
        Ok(Self {
            path: None,
            completed: false,
            completed_reader: None,
            #[cfg(all(test, windows))]
            after_capture: None,
            #[cfg(all(test, windows))]
            after_run_source: None,
            #[cfg(all(test, windows))]
            fail_after_completed: false,
            plan: Arc::new(QueryPlan {
                identity: Arc::new(QueryControl {
                    preparations: AtomicU8::new(0),
                }),
                parent,
                token,
                key,
                payment,
            }),
        })
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.plan.parent.resources
    }
    pub(in crate::store) fn query_resources(&self) -> &V2TextTokenQueryResources {
        &self.plan.payment
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextTokenQueryObservation {
        OriginalV2TextTokenQueryObservation {
            identity: Arc::downgrade(&self.plan.identity),
            parent: self.plan.parent.clone(),
            payment: self.plan.payment.clone(),
        }
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
    /// Claim BEFORE Factory allocates a source shell or retains its OS lease. Merely
    /// borrowing `&PreparedQuery` cannot authorize two simultaneous copies under one fee.
    pub(in crate::store) fn claim_capture_preparation(
        &self,
    ) -> Result<V2TextTokenQueryCaptureClaim, AppError> {
        self.claim_preparation(CAPTURE_PREPARATION)?;
        Ok(V2TextTokenQueryCaptureClaim {
            control: self.plan.identity.clone(),
            _payment: self.plan.payment.clone(),
        })
    }
    /// Separate maximum-one slot for candidate-generation backing/actual reader custody.
    /// It is not a reader permit; the real Current/generation registry still issues that.
    pub(in crate::store) fn claim_reader_preparation(
        &self,
    ) -> Result<V2TextTokenQueryReaderClaim, AppError> {
        self.claim_preparation(READER_PREPARATION)?;
        Ok(V2TextTokenQueryReaderClaim {
            control: self.plan.identity.clone(),
            _payment: self.plan.payment.clone(),
        })
    }
    fn claim_preparation(&self, bit: u8) -> Result<(), AppError> {
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
    /// Pure request/parent DATA binding for the paired pre-Create workspace. An empty
    /// lookup for another token cannot be reused as this request's predecessor evidence.
    /// No source liveness, allocation, account, encode, FS or permission follows here.
    pub(in crate::store) fn require_token_parent_data(
        &self,
        selected: &SelectedV2TextHead,
        token: [u8; 32],
    ) -> Result<(), AppError> {
        let expected = &self.plan.parent;
        expected
            .resources
            .require_same_original(selected.resources())?;
        let actual = selected.physical_query_data();
        let head = selected.head_data();
        if self.plan.token != token
            || expected.target != actual.target
            || expected.target_id != actual.target_id
            || expected.pages != actual.pages
            || expected.pages_id != actual.pages_id
            || expected.head_path != actual.head_path
            || expected.archive_path != actual.archive_path
            || expected.head.as_slice() != actual.head_frame
            || expected.archive.as_slice() != actual.archive_frame
            || expected.context != head.context
            || expected.generation != head.generation
            || expected.directory != head.directory
            || expected.tokens != head.tokens
            || expected.publications != head.publications
            || expected.pending != head.pending
        {
            return Err(refused());
        }
        Ok(())
    }
    /// FULL exact selected-source proof BEFORE Factory installs even a negative capture
    /// pin. A stale genuine snapshot cannot recreate a retired root generation merely
    /// because its original source/namespace is still alive. These temporary comparison
    /// Files all drain before return; later retained Source capture independently repeats.
    pub(in crate::store) fn validate_parent_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.plan.validate_selected_under_current_guard(guard)
    }
    /// Called only while the outside coupled Source+OS-lease bundle is borrowed. Cache
    /// its inert Source first, then each actual File/key before any later failure.
    pub(in crate::store) fn capture(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        slot: &mut V2TextTokenQuerySourceSlot<'_>,
    ) -> Result<(), AppError> {
        self.plan.payment.validate_prepaid_account()?;
        self.resources().require_same_original(guard.resources())?;
        if slot.get().is_none() {
            slot.cache_initial(CapturedV2TextTokenQuery {
                target: HeldQueryDirectory::empty(),
                pages: HeldQueryDirectory::empty(),
                head: HeldQueryFrame::empty(),
                archive: HeldQueryFrame::empty(),
                key: None,
                plan: self.plan.clone(),
            });
        }
        let source = slot.get_mut().ok_or_else(refused)?;
        source.require_same_query(&self.observe_original())?;
        source.fill(guard)?;
        #[cfg(all(test, windows))]
        if let Some(hook) = self.after_capture.take() {
            hook();
        }
        source.validate_under_current_guard(guard)
    }
    /// The actual affine reader/physical marker owns all source Files/key through local
    /// page reads and cleanup. Path steps are cached before the next fallible read; retry
    /// does not manufacture an empty subtree after an error or skip to a newer root.
    pub(in crate::store) fn run(
        &mut self,
        admitted: AdmittedV2TextTokenQuery,
    ) -> Result<(), AppError> {
        admitted.require_same_query(&self.observe_original())?;
        if self.completed_reader.is_some() {
            return Err(refused());
        }
        self.plan.payment.validate_prepaid_account()?;
        let result = self.run_admitted(&admitted);
        // Every helper-local page File and scratch buffer has now drained. On success the
        // SAME affine reader transfers after source/key/OS-lease cleanup; no clone/count
        // can make the result appear live before the real finite IO has ended.
        if result.is_ok() {
            admitted.finish_into(&mut self.completed_reader);
            // Lost-return regression cut AFTER actual Files/session drain and SAME reader
            // caching. The result stays discoverable even though its caller sees an error.
            #[cfg(all(test, windows))]
            if std::mem::take(&mut self.fail_after_completed) {
                return Err(refused());
            }
        } else {
            drop(admitted);
        }
        result
    }
    fn run_admitted(&mut self, admitted: &AdmittedV2TextTokenQuery) -> Result<(), AppError> {
        let source = admitted.source();
        source.require_same_query(&self.observe_original())?;
        source.require_unchanged()?;
        #[cfg(all(test, windows))]
        if let Some(hook) = self.after_run_source.take() {
            hook();
        }
        if self.completed {
            return Ok(());
        }
        let parent = &self.plan.parent;
        let TextRootData::Present(root) = parent.tokens else {
            // This is a real selected empty ROOT, not absence inferred from a failed read.
            source.require_unchanged()?;
            self.completed = true;
            return Ok(());
        };
        if self.path.is_none() {
            self.path = Some(
                PrivateIndexPathData::new_data(
                    PrivateIndexContextData {
                        cid: parent.context.id_data()?,
                        kind: PrivateIndexKindData::Token,
                    },
                    root,
                    self.plan.key,
                )
                .map_err(|_| refused())?,
            );
        }
        let key = source.key.as_ref().ok_or_else(refused)?;
        let path = self.path.as_mut().ok_or_else(refused)?;
        for _ in 0..MAX_QUERY_PAGES {
            let Some(page) = path.next_page_data().map_err(|_| refused())? else {
                path.row_data().map_err(|_| refused())?;
                source.require_unchanged()?;
                self.completed = true;
                return Ok(());
            };
            let name = page_child(&parent.pages, page)?;
            // This IO is OUTSIDE all writers. Its helper-local File/scratch always drain
            // before the admitted marker, including malformed/hash/AEAD/name failures.
            let mut frame = HeldQueryFrame::empty();
            frame.capture(&name, MAX_INDEX_FRAME_BYTES)?;
            source.pages.require_name(&parent.pages, parent.pages_id)?;
            accept_query_page(
                path,
                key,
                &frame,
                &name,
                #[cfg(test)]
                None,
            )?;
        }
        // The tenth accepted page may be the leaf; observe completion without an eleventh
        // IO. More work or a still-branch tenth page refuses rather than extending a budget.
        if path.next_page_data().map_err(|_| refused())?.is_some() {
            return Err(refused());
        }
        path.row_data().map_err(|_| refused())?;
        source.require_unchanged()?;
        self.completed = true;
        Ok(())
    }
    /// DATA availability only; the actual final use still requires fresh Current and the
    /// same live registered reader. This flag issues no Create/miss or selected-root grant.
    pub(in crate::store) fn has_completed_data(&self) -> bool {
        self.completed && self.completed_reader.is_some()
    }
    pub(in crate::store) fn token_value_data(&self) -> Result<Option<TokenValueData>, AppError> {
        if !self.has_completed_data() {
            return Err(refused());
        }
        let Some(path) = &self.path else {
            if self.plan.parent.tokens != TextRootData::Empty {
                return Err(refused());
            }
            return Ok(None);
        };
        match path.row_data().map_err(|_| refused())? {
            None => Ok(None),
            Some(row) => match row.value {
                PrivateIndexValueData::Token(value) if value.token == self.plan.token => {
                    Ok(Some(value))
                }
                _ => Err(refused()),
            },
        }
    }
    /// Authenticated bounded COW-input DATA. Retained path alone is not a live source,
    /// original Action verification, workspace-clear proof or write admission.
    pub(in crate::store) fn path_data(&self) -> Result<Option<&PrivateIndexPathData>, AppError> {
        self.token_value_data()?;
        Ok(self.path.as_ref())
    }
    pub(in crate::store) fn validate_completed_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.token_value_data()?;
        let reader = self.completed_reader.as_ref().ok_or_else(refused)?;
        reader.require_same_query(&self.observe_original())?;
        reader.require_live_query(&self.observe_original())?;
        self.plan.validate_selected_under_current_guard(guard)?;
        // This is the actual same registered native generation/reader slot, sampled after
        // all account, codec and FS work. Closing is monotone; an old result cannot renew.
        reader.require_live_under_current_guard(guard, &self.observe_original())?;
        reader.require_live_query(&self.observe_original())?;
        guard.validate_source_only()
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn fail_after_completed_for_test(&mut self) {
        self.fail_after_completed = true;
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
}

/// Imminent active read source ONLY. It stays structurally coupled to the SAME mount's
/// physical lease even after capture error. No constructor/getter lets callers detach it.
pub(in crate::store) struct CapturedV2TextTokenQuery {
    target: HeldQueryDirectory,
    pages: HeldQueryDirectory,
    head: HeldQueryFrame,
    archive: HeldQueryFrame,
    key: Option<Zeroizing<[u8; 32]>>,
    // LAST: all actual source Files/raw buffers/key precede the query fee through this Arc.
    plan: Arc<QueryPlan>,
}
impl CapturedV2TextTokenQuery {
    fn require_same_query(
        &self,
        original: &OriginalV2TextTokenQueryObservation,
    ) -> Result<(), AppError> {
        self.plan.observe_original().require_same_original(original)
    }
    fn fill(&mut self, guard: &V2TextCurrentGroupGuard<'_, '_>) -> Result<(), AppError> {
        self.plan.validate_original(guard)?;
        if self.key.is_none() {
            self.key = Some(Zeroizing::new(guard.store().keys.db_key()?));
        }
        let parent = &self.plan.parent;
        self.target.capture(&parent.target)?;
        self.pages.capture(&parent.pages)?;
        self.head.capture(&parent.head_path, MAX_TEXT_HEAD_FRAME)?;
        self.archive
            .capture(&parent.archive_path, MAX_ARCHIVE_PLAIN + 40)?;
        Ok(())
    }
    pub(in crate::store) fn validate_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.plan.validate_original(guard)?;
        let key = self.key.as_ref().ok_or_else(refused)?;
        if original_context(guard, key)? != self.plan.parent.context {
            return Err(refused());
        }
        self.require_unchanged()?;
        guard.validate_original_namespace()?;
        self.require_unchanged()?;
        guard.validate_source_only()
    }
    fn require_unchanged(&self) -> Result<(), AppError> {
        let parent = &self.plan.parent;
        if self.head.raw()? != parent.head.as_slice()
            || self.archive.raw()? != parent.archive.as_slice()
        {
            return Err(refused());
        }
        self.head.require_unchanged(&parent.head_path)?;
        self.archive.require_unchanged(&parent.archive_path)?;
        self.target.require_name(&parent.target, parent.target_id)?;
        self.pages.require_name(&parent.pages, parent.pages_id)
    }
}

impl QueryPlan {
    fn observe_original(&self) -> OriginalV2TextTokenQueryObservation {
        OriginalV2TextTokenQueryObservation {
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
    fn validate_selected_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        // SAME canonical guard orders this negative gate before every new O/H File/key.
        // An admitted control writer keeps it closed until actual IO/handle cleanup.
        guard.require_text_control_write_idle_for_query()?;
        self.validate_original(guard)?;
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        if original_context(guard, &key)? != self.parent.context {
            return Err(refused());
        }
        // Ephemeral comparison controls drain before leaving this guarded read. This is
        // not a live root reader; the Factory registry is checked independently by caller.
        let mut target = HeldQueryDirectory::empty();
        let mut pages = HeldQueryDirectory::empty();
        let mut head = HeldQueryFrame::empty();
        let mut archive = HeldQueryFrame::empty();
        target.capture(&self.parent.target)?;
        pages.capture(&self.parent.pages)?;
        head.capture(&self.parent.head_path, MAX_TEXT_HEAD_FRAME)?;
        archive.capture(&self.parent.archive_path, MAX_ARCHIVE_PLAIN + 40)?;
        if head.raw()? != self.parent.head.as_slice()
            || archive.raw()? != self.parent.archive.as_slice()
            || TextHeadIdData::of_data(head.raw()?) != TextHeadIdData::of_data(&self.parent.head)
        {
            return Err(refused());
        }
        guard.validate_original_namespace()?;
        head.require_unchanged(&self.parent.head_path)?;
        archive.require_unchanged(&self.parent.archive_path)?;
        target.require_name(&self.parent.target, self.parent.target_id)?;
        pages.require_name(&self.parent.pages, self.parent.pages_id)?;
        guard.validate_source_only()
    }
}

/// The cached node is mathematical snapshot DATA after exact expected-id/AEAD/context
/// and a held whole-frame/name proof. A later name failure may retain that DATA: retry
/// still needs a NEW actual query admission and SAME selected O/root reader. This does
/// not promise each old page is presently named or make an IO failure into an empty result.
fn accept_query_page(
    path: &mut PrivateIndexPathData,
    key: &[u8; 32],
    frame: &HeldQueryFrame,
    name: &Path,
    #[cfg(test)] after_cache: Option<Box<dyn FnOnce()>>,
) -> Result<(), AppError> {
    frame.require_unchanged(name)?;
    path.accept_frame_data(key, frame.raw()?)
        .map_err(|_| refused())?;
    #[cfg(test)]
    if let Some(hook) = after_cache {
        hook();
    }
    frame.require_name(name)
}

fn owned_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().len() > MAX_TEXT_PATH {
        return Err(refused());
    }
    let mut copy = PathBuf::with_capacity(path.as_os_str().len());
    copy.push(path);
    if copy.capacity() > MAX_TEXT_PATH {
        return Err(refused());
    }
    Ok(copy)
}
fn page_child(base: &Path, page: PrivateChatPageId) -> Result<PathBuf, AppError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let capacity = base
        .as_os_str()
        .len()
        .checked_add(1 + 70)
        .filter(|n| *n <= MAX_TEXT_PATH)
        .ok_or_else(refused)?;
    let mut name = String::with_capacity(70);
    name.push_str("p-");
    for byte in page.as_bytes() {
        name.push(HEX[(byte >> 4) as usize] as char);
        name.push(HEX[(byte & 15) as usize] as char);
    }
    name.push_str(".bin");
    let mut path = PathBuf::with_capacity(capacity);
    path.push(base);
    path.push(name);
    if path.capacity() > MAX_TEXT_PATH {
        return Err(refused());
    }
    Ok(path)
}
fn io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}
fn regular(metadata: &fs::Metadata) -> bool {
    let plain = metadata.file_type().is_file() && !metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        plain && metadata.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    plain
}

/// Cached in the active Source before the first fallible metadata/name/content operation.
struct HeldQueryFrame {
    file: Option<File>,
    id: Option<SessionId>,
    raw: Option<Zeroizing<Vec<u8>>>,
    used: usize,
    complete: bool,
}
impl HeldQueryFrame {
    fn empty() -> Self {
        Self {
            file: None,
            id: None,
            raw: None,
            used: 0,
            complete: false,
        }
    }
    fn file(&self) -> Result<&File, AppError> {
        self.file.as_ref().ok_or_else(refused)
    }
    fn raw(&self) -> Result<&[u8], AppError> {
        if !self.complete {
            return Err(refused());
        }
        Ok(&self.raw.as_ref().ok_or_else(refused)?[..self.used])
    }
    fn require_name(&self, path: &Path) -> Result<(), AppError> {
        let id = self.id.ok_or_else(refused)?;
        if session_id(self.file()?).map_err(io)? != id
            || !matches!(session_at(path, id), SessionAt::Ours)
        {
            return Err(refused());
        }
        Ok(())
    }
    fn capture(&mut self, path: &Path, maximum: usize) -> Result<(), AppError> {
        if maximum == 0 || maximum > MAX_INDEX_FRAME_BYTES {
            return Err(refused());
        }
        if self.complete {
            return self.require_unchanged(path);
        }
        if self.file.is_none() {
            let metadata = fs::symlink_metadata(path).map_err(io)?;
            if !regular(&metadata) || metadata.len() == 0 || metadata.len() > maximum as u64 {
                return Err(refused());
            }
            self.file = Some(File::open(path).map_err(io)?);
        }
        let metadata = self.file()?.metadata().map_err(io)?;
        if !regular(&metadata) || metadata.len() == 0 || metadata.len() > maximum as u64 {
            return Err(refused());
        }
        if self.id.is_none() {
            self.id = Some(session_id(self.file()?).map_err(io)?);
        }
        self.require_name(path)?;
        if self.raw.is_none() {
            self.raw = Some(Zeroizing::new(vec![0; maximum + 1]));
        }
        let file = self.file.as_mut().ok_or_else(refused)?;
        file.seek(SeekFrom::Start(0)).map_err(io)?;
        let raw = self.raw.as_mut().ok_or_else(refused)?;
        let mut used = 0;
        while used < raw.len() {
            match file.read(&mut raw[used..]) {
                Ok(0) => break,
                Ok(n) => used += n,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        if used != metadata.len() as usize {
            return Err(refused());
        }
        self.used = used;
        self.complete = true;
        self.require_name(path)
    }
    fn require_unchanged(&self, path: &Path) -> Result<(), AppError> {
        self.require_name(path)?;
        let expected = self.raw()?;
        let file = self.file()?;
        let metadata = file.metadata().map_err(io)?;
        if !regular(&metadata) || metadata.len() != expected.len() as u64 {
            return Err(refused());
        }
        let mut offset = 0;
        let mut buffer = [0u8; 4096];
        while offset < expected.len() {
            let count = buffer.len().min(expected.len() - offset);
            match positioned_read(file, &mut buffer[..count], offset as u64) {
                Ok(0) => return Err(refused()),
                Ok(n) => {
                    if buffer[..n] != expected[offset..offset + n] {
                        return Err(refused());
                    }
                    offset += n;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        loop {
            match positioned_read(file, &mut buffer[..1], offset as u64) {
                Ok(0) => break,
                Ok(_) => return Err(refused()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        self.require_name(path)
    }
}

struct HeldQueryDirectory {
    file: Option<File>,
    id: Option<SessionId>,
}
impl HeldQueryDirectory {
    fn empty() -> Self {
        Self {
            file: None,
            id: None,
        }
    }
    fn capture(&mut self, path: &Path) -> Result<(), AppError> {
        if self.file.is_none() {
            #[cfg(windows)]
            let file = {
                use std::os::windows::fs::OpenOptionsExt;
                use windows_sys::Win32::Storage::FileSystem::{
                    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
                    FILE_SHARE_READ, FILE_SHARE_WRITE,
                };
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(path)
                    .map_err(io)?
            };
            #[cfg(not(windows))]
            let file = File::open(path).map_err(io)?;
            self.file = Some(file);
        }
        let file = self.file.as_ref().ok_or_else(refused)?;
        let metadata = file.metadata().map_err(io)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(refused());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(refused());
            }
        }
        if self.id.is_none() {
            self.id = Some(session_id(file).map_err(io)?);
        }
        self.require_name(path, self.id.ok_or_else(refused)?)
    }
    fn require_name(&self, path: &Path, expected: SessionId) -> Result<(), AppError> {
        let file = self.file.as_ref().ok_or_else(refused)?;
        if self.id != Some(expected) {
            return Err(refused());
        }
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            path, file, expected,
        )
    }
}
fn positioned_read(file: &File, bytes: &mut [u8], offset: u64) -> std::io::Result<usize> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        file.seek_read(bytes, offset)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_at(bytes, offset)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (file, bytes, offset);
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}

macro_rules! redacted_debug {
    ($($name:ident),+$(,)?) => {$(impl std::fmt::Debug for $name {
        fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
            f.debug_struct(stringify!($name)).finish_non_exhaustive()
        }
    })+};
}
redacted_debug!(
    V2TextTokenQueryResources,
    V2TextTokenQueryCaptureClaim,
    V2TextTokenQueryReaderClaim,
    OriginalV2TextTokenQueryObservation,
    PreparedV2TextTokenQuery,
    CapturedV2TextTokenQuery
);

#[cfg(test)]
mod tests;

mod drain;
