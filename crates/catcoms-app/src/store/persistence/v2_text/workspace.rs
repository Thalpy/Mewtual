//! Actual pre-Create workspace inspection, distinct from token lookup and write admission.
//!
//! This first positive role proves a clean selected workspace only. ANY present WAL or
//! head stage is preserved/refused until its complete WORK/WD/WC continuation is paired;
//! it is never called Clear because a decoder/read failed. ReadyClear remains DATA. Only
//! a separate fresh Current/create-lane edge may bind this exact serialized predecessor.
use super::archive::{TextArchiveData, TextArchiveOperationData, MAX_ARCHIVE_PLAIN};
use super::capture::{original_context, SelectedV2TextHead};
use super::common::refused;
use super::context::TextContextData;
use super::control::{open_control_data, TextControlKindData};
use super::head::{
    TextHeadData, TextParentData, TextPendingData, TextRootData, MAX_TEXT_HEAD_FRAME,
};
use super::ordinary_data::{
    OriginalV2RetainObservation, PreparedV2RetainData, V2TextRetainWorkspaceClaim,
    V2TextTransactionMetadataResources, RETAIN_DATA_LAYOUT_BYTES, TEXT_TRANSACTION_METADATA_FEE,
    TRANSACTION_METADATA_BASE_LAYOUT_BYTES,
};
use super::query::{OriginalV2TextTokenQueryObservation, PreparedV2TextTokenQuery};
use super::resources::{V2TextReadResources, V2TextWorkResources, TEXT_WORK_BUFFER_FEE};
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextWorkspaceRead, V2TextCurrentGroupGuard, V2TextWorkspaceSourceSlot,
    V2_TEXT_CURRENT_GROUP_GUARD_BYTES, V2_TEXT_WORKSPACE_ADMISSION_LAYOUT_BYTES,
};
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::ServerStore;
use crate::AppError;
use catcoms_replication::chat::owner_header::SignedOwnerHeader;
use catcoms_storage::kept::SessionId;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use zeroize::Zeroizing;

mod io;
mod seed;
use io::{HeldWorkspaceControl, HeldWorkspaceDirectory};

const MAX_WORKSPACE_PATH: usize = 5120;
const MAX_WORKSPACE_WAL_FRAME: usize = 2 * 1024 * 1024 + 40;
const MAX_WORKSPACE_SEED_FRAME: usize = 32768 + 40;

/// Full simultaneous open/decrypt phase, including the original retained producer's own
/// concrete inventory. These are NEW16MiB work bytes, never query/read/SDK/metadata credit.
/// Opening two raw WALs plus one sequential ciphertext/plaintext pair is four full caps.
pub(in crate::store) const V2_TEXT_WORKSPACE_OPEN_LAYOUT_BYTES: usize = RETAIN_DATA_LAYOUT_BYTES
    + 4 * MAX_WORKSPACE_WAL_FRAME
    + 4 * MAX_WORKSPACE_SEED_FRAME
    + 8 * (MAX_TEXT_HEAD_FRAME + MAX_ARCHIVE_PLAIN + 40)
    + 128 * std::mem::size_of::<std::fs::File>()
    + 64 * 1024;
const _: () = assert!(V2_TEXT_WORKSPACE_OPEN_LAYOUT_BYTES <= TEXT_WORK_BUFFER_FEE);

/// NEW independent transaction1.25MiB. Original read1MiB/query1MiB/SDK payments are additional.
/// Eight owned final paths total: target plus seven closed paths. No per-artifact PathBuf.
pub(in crate::store) const V2_TEXT_WORKSPACE_METADATA_LAYOUT_BYTES: usize =
    TRANSACTION_METADATA_BASE_LAYOUT_BYTES
        + 2 * std::mem::size_of::<PreparedV2TextWorkspaceCapture>()
        + 2 * std::mem::size_of::<CapturedV2TextWorkspace>()
        + 4 * std::mem::size_of::<ReadyV2TextWorkspaceClear>()
        + 2 * std::mem::size_of::<WorkspacePlan>()
        + 2 * std::mem::size_of::<WorkspaceControl>()
        + 4 * std::mem::size_of::<V2TextWorkspaceCaptureClaim>()
        + V2_TEXT_WORKSPACE_ADMISSION_LAYOUT_BYTES
        + V2_TEXT_CURRENT_GROUP_GUARD_BYTES
        + 24 * (MAX_WORKSPACE_PATH + 256)
        + 128 * (std::mem::size_of::<SessionId>() + 64)
        + 8 * 1024
        + 32 * 1024;
const _: () = assert!(V2_TEXT_WORKSPACE_METADATA_LAYOUT_BYTES <= TEXT_TRANSACTION_METADATA_FEE);

struct WorkspaceControl {
    active_capture: AtomicBool,
    // LAST: central maximum-one WHOLE workspace claim retains BOTH actual payers, even
    // after the producer/Prepared drops but Ready or an active capture still holds DATA.
    _original_claim: V2TextRetainWorkspaceClaim,
}

/// Maximum ONE active source precursor within the SAME already claimed workspace. Root's
/// central producer claim also excludes a second inactive workspace plan/large WAL result.
/// Factory declares this AFTER Files/key/physical lease/end; it is neither Clone nor public.
pub(in crate::store) struct V2TextWorkspaceCaptureClaim {
    control: Arc<WorkspaceControl>,
}
impl Drop for V2TextWorkspaceCaptureClaim {
    fn drop(&mut self) {
        let previous = self.control.active_capture.swap(false, Ordering::AcqRel);
        debug_assert!(previous, "one affine workspace source precursor");
    }
}

/// Immutable original facts copied only after both actual fees and the central claim.
/// No Store, SDK, graph, File, key, node, runtime or physical lease is retained here.
struct WorkspacePlan {
    original: OriginalV2RetainObservation,
    query: OriginalV2TextTokenQueryObservation,
    context: TextContextData,
    target: PathBuf,
    target_id: SessionId,
    pages_id: SessionId,
    paths: WorkspacePaths,
    head: Zeroizing<Vec<u8>>,
    archive: Zeroizing<Vec<u8>>,
    pending: TextPendingData,
    public_header: SignedOwnerHeader,
    public_state: Zeroizing<Vec<u8>>,
    public_manifest: catcoms_storage::private_chat_page::PrivateChatPageId,
    public_admission: catcoms_replication::chat::owner_facts::OwnerFactId,
    resources: V2TextReadResources,
    work: Arc<V2TextWorkResources>,
    metadata: V2TextTransactionMetadataResources,
    // LAST: copied frames/paths and all separate observer tails precede final payer loans.
    control: Arc<WorkspaceControl>,
}

/// An owning original pre-packet inspection. Preparation cannot call Clock/RNG/sign/export.
/// The SAME central claim follows all Prepared/Ready/source tails, preventing silent copies
/// of large results under one sixteen-megabyte class after an earlier worker completes.
pub(in crate::store) struct PreparedV2TextWorkspaceCapture {
    ready: Option<ReadyV2TextWorkspaceClear>,
    #[cfg(all(test, windows))]
    after_capture: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    after_run_source: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    fail_after_ready: bool,
    plan: Arc<WorkspacePlan>,
}
impl PreparedV2TextWorkspaceCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        producer: &PreparedV2RetainData,
        selected: &SelectedV2TextHead,
        query: &PreparedV2TextTokenQuery,
    ) -> Result<Self, AppError> {
        producer.metadata_resources().validate_prepaid_account()?;
        producer.work_resources().validate_original(store, source)?;
        producer.require_unmaterialized_data()?;
        producer.require_selected_data(selected)?;
        query.require_token_parent_data(selected, producer.token_data())?;
        if query.token_value_data()?.is_some() {
            return Err(refused());
        }
        // Central claim BEFORE any new workspace Arc/path/frame/control allocation.
        let original_claim = producer.claim_workspace_preparation()?;
        producer.work_resources().validate_original(store, source)?;
        producer.require_unmaterialized_data()?;
        producer.require_selected_data(selected)?;
        let original = selected.physical_query_data();
        if original.archive_path.parent() != Some(original.pages) {
            return Err(refused());
        }
        let public = source.data();
        let plan = Arc::new(WorkspacePlan {
            original: producer.observe_original(),
            query: query.observe_original(),
            context: selected.head_data().context.clone(),
            target: owned_path(original.target)?,
            target_id: original.target_id,
            pages_id: original.pages_id,
            paths: WorkspacePaths::prepare(original.target, original.archive_path)?,
            head: Zeroizing::new(original.head_frame.to_vec()),
            archive: Zeroizing::new(original.archive_frame.to_vec()),
            pending: selected.head_data().pending,
            public_header: public.header.clone(),
            public_state: Zeroizing::new(public.state.to_vec()),
            public_manifest: public.context.spec_data().manifest,
            public_admission: public.admission.admission_id(),
            resources: producer.resources().clone(),
            work: producer.work_resources().clone(),
            metadata: producer.metadata_resources().clone(),
            control: Arc::new(WorkspaceControl {
                active_capture: AtomicBool::new(false),
                _original_claim: original_claim,
            }),
        });
        Ok(Self {
            ready: None,
            #[cfg(all(test, windows))]
            after_capture: None,
            #[cfg(all(test, windows))]
            after_run_source: None,
            #[cfg(all(test, windows))]
            fail_after_ready: false,
            plan,
        })
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.plan.resources
    }
    pub(in crate::store) fn work_resources(&self) -> &Arc<V2TextWorkResources> {
        &self.plan.work
    }
    pub(in crate::store) fn metadata_resources(&self) -> &V2TextTransactionMetadataResources {
        &self.plan.metadata
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2RetainObservation {
        self.plan.original.clone()
    }
    pub(in crate::store) fn validate_preparation(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.plan.metadata.validate_prepaid_account()?;
        self.plan.work.validate_original(store, source)
    }
    pub(in crate::store) fn claim_capture_preparation(
        &self,
    ) -> Result<V2TextWorkspaceCaptureClaim, AppError> {
        self.plan.metadata.validate_prepaid_account()?;
        self.plan.work.validate_prepaid_account()?;
        self.plan
            .control
            .active_capture
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        Ok(V2TextWorkspaceCaptureClaim {
            control: self.plan.control.clone(),
        })
    }
    /// FULL exact selected O/H and actual completed-reader proof BEFORE Factory installs
    /// even its negative finite-control pin. This creates no pin or workspace permission.
    pub(in crate::store) fn validate_parent_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
        query: &PreparedV2TextTokenQuery,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.plan.validate_original(guard)?;
        self.plan
            .query
            .require_same_original(&query.observe_original())?;
        if query.token_value_data()?.is_some() {
            return Err(refused());
        }
        query.validate_completed_under_current_guard(guard)
    }
    /// The outside bundle is already claimed/paid and coupled to its SAME actual session.
    /// No retained source File or key is opened before its empty Source is cached here.
    pub(in crate::store) fn capture(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        slot: &mut V2TextWorkspaceSourceSlot<'_>,
    ) -> Result<(), AppError> {
        self.plan.validate_original(guard)?;
        if slot.get().is_none() {
            slot.cache_initial(CapturedV2TextWorkspace::empty(self.plan.clone()));
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
    /// LARGE reads/decryption happen only here, outside all writers. The finite marker's
    /// source/key/lease/end all drain before return on every path. Caching Ready does not
    /// bind the lane or authorize an SDK draft; the fresh predecessor edge is separate.
    pub(in crate::store) fn run_read(
        &mut self,
        mut admitted: AdmittedV2TextWorkspaceRead,
    ) -> Result<(), AppError> {
        admitted.require_same_work(&self.observe_original())?;
        self.plan.work.validate_prepaid_account()?;
        self.plan.metadata.validate_prepaid_account()?;
        let result = (|| {
            #[cfg(all(test, windows))]
            if let Some(hook) = self.after_run_source.take() {
                hook();
            }
            let seed_id = admitted.prove_clear_data(&self.observe_original())?;
            if let Some(ready) = &self.ready {
                if ready.seed != seed_id || !Arc::ptr_eq(&ready.plan, &self.plan) {
                    return Err(refused());
                }
            } else {
                self.ready = Some(ReadyV2TextWorkspaceClear {
                    seed: seed_id,
                    plan: self.plan.clone(),
                });
            }
            Ok(())
        })();
        // Source FDs/raw/key, SAME OS lease, actual work end, then precursor claim.
        drop(admitted);
        #[cfg(all(test, windows))]
        if result.is_ok() && std::mem::take(&mut self.fail_after_ready) {
            return Err(refused());
        }
        result
    }
    /// DATA availability only. The real final permission uses fresh Current+lane+query.
    pub(in crate::store) fn has_ready_data(&self) -> bool {
        self.ready.is_some()
    }
    pub(in crate::store) fn ready_clear(&self) -> Result<&ReadyV2TextWorkspaceClear, AppError> {
        self.ready.as_ref().ok_or_else(refused)
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
    pub(in crate::store) fn fail_after_ready_for_test(&mut self) {
        self.fail_after_ready = true;
    }
}

/// Frozen read result only, with no File/key/Store/SDK/session. It is not a Clear boolean,
/// a registered lane, a materialized action, signing permission or an A write admission.
pub(in crate::store) struct ReadyV2TextWorkspaceClear {
    seed: [u8; 32],
    plan: Arc<WorkspacePlan>,
}
impl ReadyV2TextWorkspaceClear {
    pub(in crate::store) fn observe_original(&self) -> OriginalV2RetainObservation {
        self.plan.original.clone()
    }
    pub(in crate::store) fn validate_clear_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
        query: &PreparedV2TextTokenQuery,
    ) -> Result<(), AppError> {
        // SAME canonical guard prevents an admitted control writer from starting between
        // this negative gate and destruction of all local readonly comparison Files.
        guard.require_text_control_write_idle_for_query()?;
        self.plan.validate_original(guard)?;
        self.plan
            .query
            .require_same_original(&query.observe_original())?;
        if query.token_value_data()?.is_some() || self.plan.pending != TextPendingData::None {
            return Err(refused());
        }
        // Local readonly comparisons do not escape this callback; the real source/read
        // pin was drained before Ready. The prospective clear role performs no large WAL
        // read here: every WAL/stage must be GENUINELY absent after full original proof.
        let mut source = CapturedV2TextWorkspace::empty(self.plan.clone());
        source.capture_metadata(guard)?;
        source.seed.read_complete(&self.plan.paths.seed)?;
        if *blake3::hash(source.seed.raw()?).as_bytes() != self.seed {
            return Err(refused());
        }
        source.require_clear_presence()?;
        source.require_all_unchanged()?;
        require_inventory(&self.plan.target)?;
        query.validate_completed_under_current_guard(guard)?;
        source.require_all_unchanged()?;
        source.seed.require_unchanged(&self.plan.paths.seed)?;
        guard.validate_source_only()
    }
}

/// Imminent actual source only. Factory keeps this in the same outside lease bundle on
/// capture Err; no API detaches Files/key into idle Prepared/Ready metadata.
pub(in crate::store) struct CapturedV2TextWorkspace {
    target: HeldWorkspaceDirectory,
    pages: HeldWorkspaceDirectory,
    head: HeldWorkspaceControl,
    archive: HeldWorkspaceControl,
    seed: HeldWorkspaceControl,
    seed_stage: HeldWorkspaceControl,
    wal: HeldWorkspaceControl,
    wal_stage: HeldWorkspaceControl,
    head_stage: HeldWorkspaceControl,
    key: Option<Zeroizing<[u8; 32]>>,
    plan: Arc<WorkspacePlan>,
}
impl CapturedV2TextWorkspace {
    fn empty(plan: Arc<WorkspacePlan>) -> Self {
        Self {
            target: HeldWorkspaceDirectory::empty(),
            pages: HeldWorkspaceDirectory::empty(),
            head: HeldWorkspaceControl::empty(),
            archive: HeldWorkspaceControl::empty(),
            seed: HeldWorkspaceControl::empty(),
            seed_stage: HeldWorkspaceControl::empty(),
            wal: HeldWorkspaceControl::empty(),
            wal_stage: HeldWorkspaceControl::empty(),
            head_stage: HeldWorkspaceControl::empty(),
            key: None,
            plan,
        }
    }
    fn require_same_plan(&self, plan: &Arc<WorkspacePlan>) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.plan, plan) {
            return Err(refused());
        }
        Ok(())
    }
    fn capture_metadata(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.plan.validate_original(guard)?;
        if self.key.is_none() {
            self.key = Some(Zeroizing::new(guard.store().keys.db_key()?));
        }
        let p = &self.plan;
        self.target.capture(&p.target, p.target_id)?;
        self.pages.capture(p.pages()?, p.pages_id)?;
        self.head
            .capture_metadata(&p.paths.head, MAX_TEXT_HEAD_FRAME)?;
        self.archive
            .capture_metadata(&p.paths.archive, MAX_ARCHIVE_PLAIN + 40)?;
        // O/H are the small complete selected-source proof; WAL/seed payload IO is deferred.
        self.head.read_complete(&p.paths.head)?;
        self.archive.read_complete(&p.paths.archive)?;
        self.seed
            .capture_metadata(&p.paths.seed, MAX_WORKSPACE_SEED_FRAME)?;
        self.seed_stage
            .capture_metadata(&p.paths.seed_stage, MAX_WORKSPACE_SEED_FRAME)?;
        self.wal
            .capture_metadata(&p.paths.wal, MAX_WORKSPACE_WAL_FRAME)?;
        self.wal_stage
            .capture_metadata(&p.paths.wal_stage, MAX_WORKSPACE_WAL_FRAME)?;
        self.head_stage
            .capture_metadata(&p.paths.head_stage, MAX_TEXT_HEAD_FRAME)?;
        Ok(())
    }
    pub(in crate::store) fn validate_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.plan.validate_original(guard)?;
        if original_context(guard, self.key.as_ref().ok_or_else(refused)?)? != self.plan.context {
            return Err(refused());
        }
        self.require_all_unchanged()?;
        guard.validate_original_namespace()?;
        self.require_all_unchanged()?;
        guard.validate_source_only()
    }
    fn require_clear_presence(&self) -> Result<(), AppError> {
        if self.plan.pending != TextPendingData::None || !self.seed.is_present_data() {
            return Err(refused());
        }
        for (path, control) in [
            (&self.plan.paths.seed_stage, &self.seed_stage),
            (&self.plan.paths.wal, &self.wal),
            (&self.plan.paths.wal_stage, &self.wal_stage),
            (&self.plan.paths.head_stage, &self.head_stage),
        ] {
            control.require_absent(path)?;
        }
        Ok(())
    }
    fn require_all_unchanged(&self) -> Result<(), AppError> {
        let p = &self.plan;
        if self.head.raw()? != p.head.as_slice() || self.archive.raw()? != p.archive.as_slice() {
            return Err(refused());
        }
        self.head.require_unchanged(&p.paths.head)?;
        self.archive.require_unchanged(&p.paths.archive)?;
        for (path, control) in [
            (&p.paths.seed, &self.seed),
            (&p.paths.seed_stage, &self.seed_stage),
            (&p.paths.wal, &self.wal),
            (&p.paths.wal_stage, &self.wal_stage),
            (&p.paths.head_stage, &self.head_stage),
        ] {
            control.require_metadata(path)?;
        }
        self.target.require_name(&p.target, p.target_id)?;
        self.pages.require_name(p.pages()?, p.pages_id)
    }
    /// Nonextracting finite DATA read for this actual original producer only. The
    /// admitted native marker retains this entire Source through all File cleanup.
    pub(in crate::store) fn prove_clear_for_original_data(
        &mut self,
        original: &OriginalV2RetainObservation,
    ) -> Result<[u8; 32], AppError> {
        self.plan.original.require_same_original(original)?;
        self.plan.work.validate_prepaid_account()?;
        self.plan.metadata.validate_prepaid_account()?;
        self.require_clear_presence()?;
        self.require_all_unchanged()?;
        self.seed.read_complete(&self.plan.paths.seed)?;
        seed::verify_anchor(
            &self.plan,
            self.key.as_ref().ok_or_else(refused)?,
            self.seed.raw()?,
        )?;
        require_inventory(&self.plan.target)?;
        self.require_clear_presence()?;
        self.require_all_unchanged()?;
        self.seed.require_unchanged(&self.plan.paths.seed)?;
        Ok(*blake3::hash(self.seed.raw()?).as_bytes())
    }
}

impl WorkspacePlan {
    fn pages(&self) -> Result<&Path, AppError> {
        self.paths.archive.parent().ok_or_else(refused)
    }
    fn validate_original(&self, guard: &V2TextCurrentGroupGuard<'_, '_>) -> Result<(), AppError> {
        self.metadata.validate_prepaid_account()?;
        self.work.validate_prepaid_account()?;
        self.resources.require_same_original(guard.resources())?;
        self.resources
            .validate_original(guard.store(), guard.source())?;
        let (target, _, id) = guard.target_directory_data();
        if target != self.target || id != self.target_id {
            return Err(refused());
        }
        Ok(())
    }
}

struct WorkspacePaths {
    head: PathBuf,
    archive: PathBuf,
    seed: PathBuf,
    seed_stage: PathBuf,
    wal: PathBuf,
    wal_stage: PathBuf,
    head_stage: PathBuf,
}
impl WorkspacePaths {
    fn prepare(target: &Path, archive: &Path) -> Result<Self, AppError> {
        Ok(Self {
            head: fixed_child(target, "head.bin")?,
            archive: owned_path(archive)?,
            seed: fixed_child(target, "text.seed")?,
            seed_stage: fixed_child(target, "text.seed.stage.v1")?,
            wal: fixed_child(target, "text.wal")?,
            wal_stage: fixed_child(target, "text.wal.stage.v1")?,
            head_stage: fixed_child(target, "head.stage")?,
        })
    }
}
fn owned_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().len() > MAX_WORKSPACE_PATH {
        return Err(refused());
    }
    let mut copy = PathBuf::with_capacity(path.as_os_str().len());
    copy.push(path);
    if copy.capacity() > MAX_WORKSPACE_PATH {
        return Err(refused());
    }
    Ok(copy)
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
    let capacity = base
        .as_os_str()
        .len()
        .checked_add(1 + name.len())
        .filter(|n| *n <= MAX_WORKSPACE_PATH)
        .ok_or_else(refused)?;
    let mut path = PathBuf::with_capacity(capacity);
    path.push(base);
    path.push(name);
    if path.capacity() > MAX_WORKSPACE_PATH {
        return Err(refused());
    }
    Ok(path)
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
    ];
    let mut seen = 0u16;
    for (index, entry) in std::fs::read_dir(target).map_err(io::io)?.enumerate() {
        if index >= NAMES.len() {
            return Err(refused());
        }
        let name = entry.map_err(io::io)?.file_name();
        let index = NAMES
            .iter()
            .position(|expected| name == *expected)
            .ok_or_else(refused)?;
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

macro_rules! redacted_debug {
    ($($ty:ident),+$(,)?) => {$(impl std::fmt::Debug for $ty {
        fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+};
}
redacted_debug!(
    V2TextWorkspaceCaptureClaim,
    PreparedV2TextWorkspaceCapture,
    ReadyV2TextWorkspaceClear,
    CapturedV2TextWorkspace
);
