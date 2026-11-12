//! DISTINCT pre-Create completed-WC source. WAL absence is NEVER completed; the
//! initial Clear role remains unchanged. This family accepts only the SAME actual
//! live resident's exact complete WC, selected public/private source and completed
//! token miss. It performs no action creation, signing, write, CAS or quota refund.
use super::archive::{TextArchiveData, MAX_ARCHIVE_PLAIN};
use super::capture::{original_context, SelectedV2TextHead};
use super::common::*;
use super::context::TextContextData;
use super::control::{open_control_data, TextControlKindData};
use super::head::{TextHeadData, TextPendingData, TextRootData, MAX_TEXT_HEAD_FRAME};
use super::ordinary_data::{
    OriginalV2RetainObservation, PreparedV2RetainData, V2TextRetainWorkspaceClaim,
    V2TextTransactionMetadataResources, RETAIN_DATA_LAYOUT_BYTES,
};
use super::query::{OriginalV2TextTokenQueryObservation, PreparedV2TextTokenQuery};
use super::resources::{V2TextReadResources, V2TextWorkResources, TEXT_WORK_BUFFER_FEE};
use super::retirement::{
    RetiredTotalsData, RetirementKindData, RetirementPlanData, MAX_RETIREMENT_ENTRIES,
};
use super::wal::{TextWalFrameData, MAX_COMPLETION_WAL_PLAIN};
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextCompletedWorkspaceRead, V2TextCompletedWorkspaceSourceSlot,
    V2TextCurrentGroupGuard, V2_TEXT_COMPLETED_WORKSPACE_ADMISSION_LAYOUT_BYTES,
    V2_TEXT_CURRENT_GROUP_GUARD_BYTES,
};
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::persistence::{
    GenesisSourceDataRef, OriginalV2TextResidentObservation, PreparedV2TextResidentProjection,
};
use crate::store::ServerStore;
use crate::AppError;
use catcoms_storage::kept::SessionId;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use zeroize::Zeroizing;
mod io;
mod parent;
mod read;
mod seed;
use io::{HeldWorkspaceControl, HeldWorkspaceDirectory};

const MAX_WORKSPACE_PATH: usize = 5120;
const MAX_WORKSPACE_WAL_FRAME: usize = 2 * 1024 * 1024 + 40;
const MAX_WORKSPACE_SEED_FRAME: usize = 32768 + 40;
const COMPLETED_PARENT_FEE: usize = 1024 * 1024;
// Complete simultaneous persistent facts,43closed absent names, Current held-name
// proof/backing, outer move slots and metadata. New A1.25MiB/work16M/read/query/SDK/model
// payments remain ADDITIONAL; no max(persistent) or hypothetical earlier refund.
const COMPLETED_PARENT_LAYOUT_BYTES: usize = 4 * std::mem::size_of::<CompletedBase>()
    + 4 * std::mem::size_of::<CompletedPlan>()
    + 4 * std::mem::size_of::<CompletedV2TextParentData>()
    + 4 * std::mem::size_of::<PreparedV2TextCompletedWorkspaceCapture>()
    + 4 * std::mem::size_of::<CapturedV2TextCompletedWorkspace>()
    + 4 * std::mem::size_of::<ReadyV2TextCompletedWorkspace>()
    + 16 * std::mem::size_of::<OriginalV2TextCompletedWorkspaceObservation>()
    + 8 * std::mem::size_of::<V2TextCompletedWorkspaceCaptureClaim>()
    + V2_TEXT_CURRENT_GROUP_GUARD_BYTES
    + V2_TEXT_COMPLETED_WORKSPACE_ADMISSION_LAYOUT_BYTES
    + (2 * MAX_RETIREMENT_ENTRIES + 28) * (MAX_WORKSPACE_PATH + 256)
    + 4 * MAX_WORKSPACE_SEED_FRAME
    + 8 * (1725 + 1519 + 2318 + 2022)
    + 32 * std::mem::size_of::<std::fs::File>()
    + 64 * 1024;
const _: () = assert!(COMPLETED_PARENT_LAYOUT_BYTES <= COMPLETED_PARENT_FEE);
const COMPLETED_OPEN_WORK_BYTES: usize = RETAIN_DATA_LAYOUT_BYTES
    + 4 * MAX_WORKSPACE_SEED_FRAME
    + 4 * MAX_WORKSPACE_SEED_FRAME
    + 8 * (1725 + 1519 + 2318 + 2022)
    + 128 * std::mem::size_of::<std::fs::File>()
    + 128 * 1024;
const _: () = assert!(COMPLETED_OPEN_WORK_BYTES <= TEXT_WORK_BUFFER_FEE);
struct CompletedPayment {
    pool: CriticalCatalogReadPool,
    fee: CatalogReadReservation,
}
#[derive(Clone)]
struct CompletedMetadata {
    body: Arc<CompletedPayment>,
}
impl CompletedMetadata {
    fn validate(&self) -> Result<(), AppError> {
        self.body
            .fee
            .validate(&self.body.pool, COMPLETED_PARENT_FEE)
    }
    fn require_same(&self, other: &Self) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.body, &other.body) {
            return Err(refused());
        }
        Ok(())
    }
}
struct CompletedControl {
    active: AtomicBool,
    _whole: V2TextRetainWorkspaceClaim,
}
pub(in crate::store) struct V2TextCompletedWorkspaceCaptureClaim {
    control: Arc<CompletedControl>,
    _metadata: CompletedMetadata,
}
impl Drop for V2TextCompletedWorkspaceCaptureClaim {
    fn drop(&mut self) {
        let previous = self.control.active.swap(false, Ordering::AcqRel);
        debug_assert!(previous);
    }
}

/// Tiny exact plan/resident/request association. No16M, model, SDK, read body, File
/// or physical Source owner is retained. It cannot renew Current or a WC proof.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextCompletedWorkspaceObservation {
    identity: Weak<()>,
    original: OriginalV2RetainObservation,
    resident: OriginalV2TextResidentObservation,
    selected: TextHeadIdData,
    completion: [u8; 32],
    plan: TextRetirementPlanIdData,
    metadata: CompletedMetadata,
}
impl OriginalV2TextCompletedWorkspaceObservation {
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        self.original.require_same_original(&other.original)?;
        self.resident.require_same_original(&other.resident)?;
        self.metadata.require_same(&other.metadata)?;
        if !self.identity.ptr_eq(&other.identity)
            || self.selected != other.selected
            || self.completion != other.completion
            || self.plan != other.plan
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn require_same_resident(
        &self,
        other: &OriginalV2TextResidentObservation,
    ) -> Result<(), AppError> {
        self.resident.require_same_original(other)
    }
    pub(in crate::store) fn original_resident(&self) -> &OriginalV2TextResidentObservation {
        &self.resident
    }
}
struct CompletedPaths {
    head: PathBuf,
    archive: PathBuf,
    seed: PathBuf,
    wal: PathBuf,
    seed_stage: PathBuf,
    wal_stage: PathBuf,
    head_stage: PathBuf,
}
struct CompletedBase {
    identity: Arc<()>,
    original: OriginalV2RetainObservation,
    resident: OriginalV2TextResidentObservation,
    selected: TextHeadIdData,
    context: TextContextData,
    target: PathBuf,
    target_id: SessionId,
    pages: PathBuf,
    pages_id: SessionId,
    paths: CompletedPaths,
    head: Zeroizing<Vec<u8>>,
    archive: Zeroizing<Vec<u8>>,
    parsed: TextHeadData,
    parsed_archive: TextArchiveData,
    expected_completion: [u8; 32],
    expected_plan: TextRetirementPlanIdData,
    expected_transaction: TextTransactionIdData,
    reclaimed: RetiredTotalsData,
    // The original profile/genesis trust comes from the real native owner, never WC.
    original_header: catcoms_replication::chat::owner_header::SignedOwnerHeader,
    original_image: Zeroizing<Vec<u8>>,
    original_manifest: catcoms_storage::private_chat_page::PrivateChatPageId,
    original_admission: catcoms_replication::chat::owner_facts::OwnerFactId,
    resources: V2TextReadResources,
    metadata: CompletedMetadata,
}
struct CompletedPlan {
    base: Arc<CompletedBase>,
    query: OriginalV2TextTokenQueryObservation,
    completed: OnceLock<Arc<CompletedV2TextParentData>>,
    work: Arc<V2TextWorkResources>,
    metadata: V2TextTransactionMetadataResources,
    control: Arc<CompletedControl>,
}
/// Independently funded immutable completed predecessor, created ONLY by the
/// actual finite authenticated read. NextA may retain it after workspace/work
/// preparation drains; no previous B receipt/model/SDK/16M graph follows it.
pub(in crate::store) struct CompletedV2TextParentData {
    base: Arc<CompletedBase>,
    completion: Zeroizing<Vec<u8>>,
    completion_native: SessionId,
    plan: RetirementPlanData,
    retired: Vec<PathBuf>,
    seed: [u8; 32],
    // LAST: base contains the SAME actual completed-parent metadata debit.
    metadata: CompletedMetadata,
}
pub(in crate::store) struct PreparedV2TextCompletedWorkspaceCapture {
    ready: Option<ReadyV2TextCompletedWorkspace>,
    #[cfg(all(test, windows))]
    after_capture: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    fail_after_ready: bool,
    plan: Arc<CompletedPlan>,
}
pub(in crate::store) struct ReadyV2TextCompletedWorkspace {
    plan: Arc<CompletedPlan>,
}
pub(in crate::store) struct CapturedV2TextCompletedWorkspace {
    target: HeldWorkspaceDirectory,
    pages: HeldWorkspaceDirectory,
    head: HeldWorkspaceControl,
    archive: HeldWorkspaceControl,
    seed: HeldWorkspaceControl,
    wal: HeldWorkspaceControl,
    seed_stage: HeldWorkspaceControl,
    wal_stage: HeldWorkspaceControl,
    head_stage: HeldWorkspaceControl,
    key: Option<Zeroizing<[u8; 32]>>,
    plan: Arc<CompletedPlan>,
}
impl PreparedV2TextCompletedWorkspaceCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        producer: &PreparedV2RetainData,
        selected: &SelectedV2TextHead,
        query: &PreparedV2TextTokenQuery,
        resident: &PreparedV2TextResidentProjection,
    ) -> Result<Self, AppError> {
        producer.metadata_resources().validate_prepaid_account()?;
        producer.work_resources().validate_original(store, source)?;
        producer.require_unmaterialized_data()?;
        producer.require_selected_data(selected)?;
        query.require_token_parent_data(selected, producer.token_data())?;
        if query.token_value_data()?.is_some() {
            return Err(refused());
        }
        resident.validate_completed_parent_data(store)?;
        let whole = producer.claim_workspace_preparation()?;
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool
            .clone();
        let fee = pool.try_reserve(COMPLETED_PARENT_FEE)?;
        fee.validate(&pool, COMPLETED_PARENT_FEE)?;
        producer.work_resources().validate_original(store, source)?;
        producer.require_unmaterialized_data()?;
        resident.validate_completed_parent_data(store)?;
        let p = resident.parent_data();
        let h = selected.head_data();
        let a = selected.archive_data().spec_data();
        require_resident_selected(resident, selected)?;
        if h.pending != TextPendingData::None {
            return Err(refused());
        }
        let raw = selected.physical_query_data();
        let public = source.data();
        let metadata = CompletedMetadata {
            body: Arc::new(CompletedPayment { pool, fee }),
        };
        let base = Arc::new(CompletedBase {
            identity: Arc::new(()),
            original: producer.observe_original(),
            resident: resident.observe_original(),
            selected: selected.head_id_data(),
            context: h.context.clone(),
            target: owned_path(raw.target)?,
            target_id: raw.target_id,
            pages: owned_path(raw.pages)?,
            pages_id: raw.pages_id,
            paths: CompletedPaths::prepare(raw.target, raw.archive_path)?,
            head: Zeroizing::new(raw.head_frame.to_vec()),
            archive: Zeroizing::new(raw.archive_frame.to_vec()),
            parsed: TextHeadData::decode_data(&h.encode_data()?)?,
            parsed_archive: TextArchiveData::decode_data(&selected.archive_data().encode_data()?)?,
            expected_completion: p.completion_frame_id_data(),
            expected_plan: p.plan_id_data(),
            expected_transaction: p.transaction_data(),
            reclaimed: RetiredTotalsData {
                bytes: p.reclaimed_data().0,
                objects: p.reclaimed_data().1,
            },
            original_header: public.header.clone(),
            original_image: Zeroizing::new(public.state.to_vec()),
            original_manifest: public.context.spec_data().manifest,
            original_admission: public.admission.admission_id(),
            resources: producer.resources().clone(),
            metadata: metadata.clone(),
        });
        let _ = a;
        let plan = Arc::new(CompletedPlan {
            base,
            query: query.observe_original(),
            completed: OnceLock::new(),
            work: producer.work_resources().clone(),
            metadata: producer.metadata_resources().clone(),
            control: Arc::new(CompletedControl {
                active: AtomicBool::new(false),
                _whole: whole,
            }),
        });
        producer
            .resources()
            .require_bound_original_negative(store, source)?;
        Ok(Self {
            ready: None,
            #[cfg(all(test, windows))]
            after_capture: None,
            #[cfg(all(test, windows))]
            fail_after_ready: false,
            plan,
        })
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.plan.base.resources
    }
    pub(in crate::store) fn work_resources(&self) -> &Arc<V2TextWorkResources> {
        &self.plan.work
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2RetainObservation {
        self.plan.base.original.clone()
    }
    pub(in crate::store) fn original_resident(&self) -> OriginalV2TextResidentObservation {
        self.plan.base.resident.clone()
    }
    pub(in crate::store) fn validate_preparation(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.plan.base.metadata.validate()?;
        self.plan.metadata.validate_prepaid_account()?;
        self.plan.work.validate_original(store, source)
    }
    pub(in crate::store) fn claim_capture_preparation(
        &self,
    ) -> Result<V2TextCompletedWorkspaceCaptureClaim, AppError> {
        self.plan.base.metadata.validate()?;
        self.plan.work.validate_prepaid_account()?;
        self.plan
            .control
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        Ok(V2TextCompletedWorkspaceCaptureClaim {
            control: self.plan.control.clone(),
            _metadata: self.plan.base.metadata.clone(),
        })
    }
    pub(in crate::store) fn validate_parent_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
        query: &PreparedV2TextTokenQuery,
        resident: &PreparedV2TextResidentProjection,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.plan.validate_original(guard)?;
        self.plan.base.require_resident(resident, guard)?;
        self.plan
            .query
            .require_same_original(&query.observe_original())?;
        if query.token_value_data()?.is_some() {
            return Err(refused());
        }
        query.validate_completed_under_current_guard(guard)
    }
    pub(in crate::store) fn capture(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        slot: &mut V2TextCompletedWorkspaceSourceSlot<'_>,
    ) -> Result<(), AppError> {
        self.plan.validate_original(guard)?;
        if slot.get().is_none() {
            slot.cache_initial(CapturedV2TextCompletedWorkspace::empty(self.plan.clone()));
        }
        let source = slot.get_mut().ok_or_else(refused)?;
        source.require_same_plan(&self.plan)?;
        source.capture_metadata(guard)?;
        #[cfg(all(test, windows))]
        if let Some(hook) = self.after_capture.take() {
            hook();
        }
        source.validate_under_current_guard(guard)
    }
    pub(in crate::store) fn run_read(
        &mut self,
        mut admitted: AdmittedV2TextCompletedWorkspaceRead,
    ) -> Result<(), AppError> {
        admitted.require_same_work(&self.observe_original())?;
        self.plan.work.validate_prepaid_account()?;
        self.plan.base.metadata.validate()?;
        let id = admitted.prove_completed_data(&self.observe_original())?;
        if id != self.plan.base.expected_completion || self.plan.completed.get().is_none() {
            return Err(refused());
        }
        if self.ready.is_none() {
            self.ready = Some(ReadyV2TextCompletedWorkspace {
                plan: self.plan.clone(),
            });
        }
        drop(admitted);
        #[cfg(all(test, windows))]
        if std::mem::take(&mut self.fail_after_ready) {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn ready_completed(
        &self,
    ) -> Result<&ReadyV2TextCompletedWorkspace, AppError> {
        self.ready.as_ref().ok_or_else(refused)
    }
    pub(in crate::store) fn has_ready_data(&self) -> bool {
        self.ready.is_some()
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_after_capture_files_for_test(
        &mut self,
        hook: Box<dyn FnOnce() + Send>,
    ) {
        self.after_capture = Some(hook);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn fail_after_ready_for_test(&mut self) {
        self.fail_after_ready = true;
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) const fn completed_parent_metadata_fee_bytes_for_test() -> usize {
        COMPLETED_PARENT_FEE
    }
}
impl ReadyV2TextCompletedWorkspace {
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.plan.base.resources
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2RetainObservation {
        self.plan.base.original.clone()
    }
    pub(in crate::store) fn original_resident(&self) -> OriginalV2TextResidentObservation {
        self.plan.base.resident.clone()
    }
    pub(in crate::store) fn observe_completed_original(
        &self,
    ) -> OriginalV2TextCompletedWorkspaceObservation {
        self.plan.base.observe_completed()
    }
    pub(in crate::store) fn completed_parent_data(
        &self,
    ) -> Result<&Arc<CompletedV2TextParentData>, AppError> {
        self.plan.completed.get().ok_or_else(refused)
    }
    pub(in crate::store) fn validate_completed_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
        query: &PreparedV2TextTokenQuery,
        resident: &PreparedV2TextResidentProjection,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.plan.validate_original(guard)?;
        self.plan.base.require_resident(resident, guard)?;
        self.plan
            .query
            .require_same_original(&query.observe_original())?;
        if query.token_value_data()?.is_some() {
            return Err(refused());
        }
        let mut source = CapturedV2TextCompletedWorkspace::empty(self.plan.clone());
        source.capture_metadata(guard)?;
        source.wal.read_complete(&self.plan.base.paths.wal)?;
        source.seed.read_complete(&self.plan.base.paths.seed)?;
        let parent = self.completed_parent_data()?;
        parent.require_source(&source)?;
        source.require_all_unchanged()?;
        require_inventory(&self.plan.base.target)?;
        query.validate_completed_under_current_guard(guard)?;
        source.require_all_unchanged()?;
        // SAME held controls, exact bytes AFTER final source/ID3. Metadata/native
        // length alone cannot exclude in-place equal-length corruption at that seam.
        source.seed.require_unchanged(&self.plan.base.paths.seed)?;
        source.wal.require_unchanged(&self.plan.base.paths.wal)?;
        parent.require_absences()?;
        resident.require_current_source_under_guard(guard)?;
        guard.validate_source_only()
    }
}
impl CompletedBase {
    fn observe_completed(&self) -> OriginalV2TextCompletedWorkspaceObservation {
        OriginalV2TextCompletedWorkspaceObservation {
            identity: Arc::downgrade(&self.identity),
            original: self.original.clone(),
            resident: self.resident.clone(),
            selected: self.selected,
            completion: self.expected_completion,
            plan: self.expected_plan,
            metadata: self.metadata.clone(),
        }
    }
    fn require_resident(
        &self,
        resident: &PreparedV2TextResidentProjection,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        resident.validate_completed_parent_data(guard.store())?;
        self.resident
            .require_same_original(&resident.observe_original())?;
        resident.require_current_source_under_guard(guard)
    }
}
impl CompletedPlan {
    fn validate_original(&self, guard: &V2TextCurrentGroupGuard<'_, '_>) -> Result<(), AppError> {
        self.base
            .resources
            .require_same_original(guard.resources())?;
        self.base.metadata.validate()?;
        self.metadata.validate_prepaid_account()?;
        self.work.validate_prepaid_account()?;
        self.base
            .resources
            .validate_original(guard.store(), guard.source())?;
        let (target, _, id) = guard.target_directory_data();
        if target != self.base.target || id != self.base.target_id {
            return Err(refused());
        }
        self.base.resident.validate_negative()
    }
}
impl CompletedPaths {
    fn prepare(target: &Path, archive: &Path) -> Result<Self, AppError> {
        Ok(Self {
            head: fixed_child(target, "head.bin")?,
            archive: owned_path(archive)?,
            seed: fixed_child(target, "text.seed")?,
            wal: fixed_child(target, "text.wal")?,
            seed_stage: fixed_child(target, "text.seed.stage.v1")?,
            wal_stage: fixed_child(target, "text.wal.stage.v1")?,
            head_stage: fixed_child(target, "head.stage")?,
        })
    }
}
fn owned_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().len() > MAX_WORKSPACE_PATH {
        return Err(refused());
    }
    let mut out = PathBuf::with_capacity(path.as_os_str().len());
    out.push(path);
    if out.capacity() > MAX_WORKSPACE_PATH {
        return Err(refused());
    }
    Ok(out)
}
fn fixed_child(base: &Path, name: &'static str) -> Result<PathBuf, AppError> {
    if !matches!(
        name,
        "head.bin"
            | "head.stage"
            | "text.seed"
            | "text.seed.stage.v1"
            | "text.wal"
            | "text.wal.stage.v1"
    ) {
        return Err(refused());
    }
    owned_child(base, name)
}
/// Precompute complete capacity BEFORE push. Growing a base-sized PathBuf can
/// otherwise double its backing while the final path length still fits the cap.
fn owned_child(base: &Path, name: &str) -> Result<PathBuf, AppError> {
    let required = closed_child_capacity_data(base.as_os_str().len(), name.len())?;
    let mut out = PathBuf::with_capacity(required);
    out.push(base);
    out.push(name);
    if out.as_os_str().len() > MAX_WORKSPACE_PATH || out.capacity() > MAX_WORKSPACE_PATH {
        return Err(refused());
    }
    Ok(out)
}

fn closed_child_capacity_data(base: usize, leaf: usize) -> Result<usize, AppError> {
    base.checked_add(1)
        .and_then(|n| n.checked_add(leaf))
        .filter(|n| *n <= MAX_WORKSPACE_PATH)
        .ok_or_else(refused)
}
fn require_inventory(target: &Path) -> Result<(), AppError> {
    const NAMES: &[&str] = &[
        "origin",
        "source.seed",
        "source.bin",
        "records.bin",
        "manifest.bin",
        "head.bin",
        "text.seed",
        "text.pages",
        "text.wal",
    ];
    let mut seen = 0u16;
    for (index, entry) in std::fs::read_dir(target).map_err(io::io)?.enumerate() {
        if index >= NAMES.len() {
            return Err(refused());
        }
        let name = entry.map_err(io::io)?.file_name();
        let index = NAMES.iter().position(|n| name == *n).ok_or_else(refused)?;
        let bit = 1u16 << index;
        if seen & bit != 0 {
            return Err(refused());
        }
        seen |= bit;
    }
    if seen != (1u16 << NAMES.len()) - 1 {
        return Err(refused());
    }
    Ok(())
}
fn root_optional(root: TextRootData) -> Option<catcoms_replication::chat::cold_index::NodeRefData> {
    match root {
        TextRootData::Empty => None,
        TextRootData::Present(r) => Some(r),
    }
}
fn require_resident_selected(
    resident: &PreparedV2TextResidentProjection,
    selected: &SelectedV2TextHead,
) -> Result<(), AppError> {
    let p = resident.parent_data();
    let h = selected.head_data();
    let a = selected.archive_data().spec_data();
    if p.context_data() != &h.context
        || p.selected_data() != selected.head_id_data()
        || p.archive_data() != h.archive
        || p.public_header_data() != a.header.id()
        || p.public_state_data() != a.header.spec().state
        || Some(p.directory_root_data()) != root_optional(h.directory)
        || p.token_root_data() != root_optional(h.tokens)
        || p.publication_root_data() != root_optional(h.publications)
        || p.generation_data() != h.generation
        || p.selection_attempt_data() != h.selection_attempt
        || p.queue_front_data() != h.queue_front
        || p.queue_next_data() != h.queue_next
        || p.baseline_data() != (h.live_bytes, h.live_objects)
    {
        return Err(refused());
    }
    Ok(())
}

#[cfg(test)]
mod path_tests;
