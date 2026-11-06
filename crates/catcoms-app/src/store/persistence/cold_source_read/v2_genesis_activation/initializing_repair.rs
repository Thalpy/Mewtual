//! Exact-candidate cold Initializing1 repair. The authenticated ORIGINAL seed supplies
//! immutable bytes, while the actual scope/task/installed-writer guard supplies permission.
//! This is deliberately a different receipt family from warm Saved and unowned Current2.
//! A short pin/stage may grow through its SAME selected handle; it is never truncated,
//! replaced, resealed, or mistaken for a complete current head.
//!
//! The concrete Scope/Factory/caller cohort retains these exact receipts through new-mount
//! recovery. The library path is separate from native/live Chat V2 activation.
use super::*;
use crate::store::chat_admission::v2_current::ColdV2GenesisScopeBasis;
use crate::store::critical_namespaces::current_layout::{
    ColdInitializingV2GenesisReadGroupGuard, ColdInitializingV2GenesisRepairGroupGuard,
    ColdRepairedV2GenesisCurrentGroupGuard, ColdRepairedV2GenesisCurrentProofGroupGuard,
    ColdV2InitializingGroupReadGuard,
};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::OnceLock;

mod inventory;

#[cfg(test)]
std::thread_local! {
    // Synchronous one-shot seams on THIS test thread. Take the callback out before invoking
    // it so an adversarial source/name mutation cannot hold a test RefCell through proof.
    static AFTER_COMPARERS: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_SOURCE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_RECEIPT: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(in crate::store) fn after_comparers_for_test(hook: Box<dyn FnOnce()>) {
    AFTER_COMPARERS.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(test)]
pub(in crate::store) fn after_source_for_test(hook: Box<dyn FnOnce()>) {
    AFTER_SOURCE.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(test)]
pub(in crate::store) fn after_receipt_for_test(hook: Box<dyn FnOnce()>) {
    AFTER_RECEIPT.with(|slot| *slot.borrow_mut() = Some(hook));
}

/// A target is cached BEFORE its first identity query. A failed query cannot cause a later
/// retry to reopen an equal replacement. Its length is intentionally NOT cached: the real
/// repair writer may extend an authorized prefix through this same handle.
struct SelectedTarget {
    file: File,
    identity: OnceLock<catcoms_storage::kept::SessionId>,
}
impl SelectedTarget {
    fn new(file: File) -> Self {
        Self {
            file,
            identity: OnceLock::new(),
        }
    }

    fn identity(&self) -> Result<catcoms_storage::kept::SessionId, AppError> {
        if let Some(id) = self.identity.get() {
            return Ok(*id);
        }
        let id = catcoms_storage::kept::session_id(&self.file).map_err(io)?;
        // Two readonly comparisons can race, but may only observe this SAME held File.
        // Neither the loser nor a failed query may select a different object by name.
        let _ = self.identity.set(id);
        if self.identity.get() != Some(&id) {
            return Err(critical_namespaces::refused());
        }
        Ok(id)
    }

    /// Held/named identity and the already-compared length only. No decoder, allocation or
    /// large content comparison may follow the final physical/source proof before effects.
    fn require_name(&self, path: &Path, expected_len: usize) -> Result<(), AppError> {
        let id = self.identity()?;
        let metadata = self.file.metadata().map_err(io)?;
        if !metadata.is_file()
            || metadata.len() != expected_len as u64
            || catcoms_storage::kept::session_id(&self.file).map_err(io)? != id
            || !matches!(
                catcoms_storage::kept::session_at(path, id),
                catcoms_storage::kept::SessionAt::Ours
            )
        {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }

    /// Bounded exact-prefix comparison on the ORIGINAL selected handle, including EOF.
    /// The private Root serializer excludes another app writer sharing its cursor. No
    /// allocation is proportional to an untrusted file length; oversize refuses first.
    fn require_prefix(&self, path: &Path, expected: &[u8]) -> Result<usize, AppError> {
        let length = usize::try_from(self.file.metadata().map_err(io)?.len())
            .map_err(|_| critical_namespaces::refused())?;
        if length > expected.len() {
            return Err(critical_namespaces::refused());
        }
        self.require_name(path, length)?;
        let mut original = &self.file;
        original.seek(SeekFrom::Start(0)).map_err(io)?;
        let mut scratch = Zeroizing::new([0u8; 4096]);
        let mut offset = 0;
        while offset < length {
            let count = (length - offset).min(scratch.len());
            original.read_exact(&mut scratch[..count]).map_err(io)?;
            if scratch[..count] != expected[offset..offset + count] {
                return Err(critical_namespaces::refused());
            }
            offset += count;
        }
        let mut eof = [0u8; 1];
        if original.read(&mut eof).map_err(io)? != 0 {
            return Err(critical_namespaces::refused());
        }
        self.require_name(path, length)?;
        Ok(length)
    }
}

struct RepairCandidates {
    pin: Zeroizing<Vec<u8>>,
    head: Zeroizing<Vec<u8>>,
}

/// Private current-name classification of the SAME selected object. It is negative DATA,
/// never a role flag supplied by a caller or a substitute for actual scope permission.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HeadName {
    Absent,
    Stage,
    Current,
}

/// Mutable target custody is inert: OnceLock may acquire only a strict-created File or an
/// existing File selected at capture. It owns neither a task/scope sender nor SavedHead.
/// In particular there is NO Seed -> SavedHead -> Seed Arc cycle.
struct RepairFiles {
    basis: ColdV2GenesisScopeBasis,
    group_path: PathBuf,
    seed_path: PathBuf,
    pin_path: PathBuf,
    head_path: PathBuf,
    stage_path: PathBuf,
    group_directory: File,
    group_id: catcoms_storage::kept::SessionId,
    target_directory: File,
    target_id: catcoms_storage::kept::SessionId,
    seed: ReadControl,
    pin: OnceLock<SelectedTarget>,
    head: OnceLock<SelectedTarget>,
    candidates: OnceLock<RepairCandidates>,
    // LAST: the original fixed loan pays targets, candidate copies and weak control tails.
    resources: ColdV2GenesisReadResources,
}

/// NEW actual cold seed/file/barrier receipt. Parsed bytes alone cannot construct it.
/// It remains valid while this same writer extends its authorized pin/stage prefixes.
pub(in crate::store) struct ColdV2GenesisRepairSeed {
    files: Arc<RepairFiles>,
}

/// Capture owns every original selection before a comparer/identity/barrier may fail.
/// Opening an unsupported/ambiguous initial selection poisons only THIS attempt; it never
/// catches malformed presence and retries as absence or Current2.
pub(in crate::store) struct PreparedColdV2GenesisRepairCapture {
    selected: Option<Arc<RepairFiles>>,
    saved: Option<Arc<ColdV2GenesisRepairSeed>>,
    selection_failed: bool,
    #[cfg(test)]
    after_seed_allocation: Option<Box<dyn FnOnce() + Send>>,
    resources: ColdV2GenesisReadResources,
}

/// Owning work retains exact target custody/ciphertext and the one real returned receipt
/// across every refusal. It cannot enter any warm or Current2 writer.
pub(in crate::store) struct PreparedColdV2GenesisRepair {
    saved: Option<Arc<SavedColdV2GenesisRepairedHead>>,
    seed: Arc<ColdV2GenesisRepairSeed>,
}

/// Original repaired pin/current, synchronized through their original parents. Scope must
/// cache THIS Arc before the SAME owner mutates row1 -> row2. No mutation happens here.
pub(in crate::store) struct SavedColdV2GenesisRepairedHead {
    seed: Arc<ColdV2GenesisRepairSeed>,
    task: OriginalV2GenesisGroupObservation,
    writer: V2WriterObservation,
}
sanitized_debug!(
    ColdV2GenesisRepairSeed,
    PreparedColdV2GenesisRepairCapture,
    PreparedColdV2GenesisRepair,
    SavedColdV2GenesisRepairedHead
);

impl PreparedColdV2GenesisRepairCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<Self, AppError> {
        // New platform/profile/invoice must refuse BEFORE its debit, paths or effects.
        require_supported_profile()?;
        validate_repair_invoice()?;
        let resources = ColdV2GenesisReadResources::prepare_paid(store, source)?;
        Ok(Self {
            selected: None,
            saved: None,
            selection_failed: false,
            #[cfg(test)]
            after_seed_allocation: None,
            resources,
        })
    }

    pub(in crate::store) fn read_resources(&self) -> ColdV2GenesisReadResources {
        self.resources.clone()
    }

    pub(in crate::store) fn capture_initializing(
        &mut self,
        guard: &ColdV2InitializingGroupReadGuard<'_, '_>,
    ) -> Result<Arc<ColdV2GenesisRepairSeed>, AppError> {
        self.resources.require_same_original(guard.resources())?;
        guard.validate_source_only()?;
        if self.selection_failed {
            return Err(critical_namespaces::refused());
        }
        if self.selected.is_none() {
            match select_initial_files(&self.resources, guard) {
                Ok(files) => self.selected = Some(Arc::new(files)),
                Err(error) => {
                    self.selection_failed = true;
                    return Err(error);
                }
            }
        }
        let files = self
            .selected
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        if &files.basis != guard.scope_basis_data() {
            return Err(critical_namespaces::refused());
        }
        files.authenticate_candidates(guard.store(), guard.source())?;
        files.validate_under_archive_root(
            guard.store(),
            guard.source(),
            guard.archive_root(),
            Goal::Repairable,
        )?;
        // Loaded row1 supplies no new file/parent durability. Establish the original seed
        // barriers here; the separate even-clean scope1 barrier still precedes repair.
        files.seed.object.file.sync_all().map_err(io)?;
        files.group_directory.sync_all().map_err(io)?;
        files.target_directory.sync_all().map_err(io)?;
        guard.archive_root().directory.sync_all().map_err(io)?;
        files.validate_under_archive_root(
            guard.store(),
            guard.source(),
            guard.archive_root(),
            Goal::Repairable,
        )?;
        if self.saved.is_none() {
            self.saved = Some(Arc::new(ColdV2GenesisRepairSeed {
                files: files.clone(),
            }));
        }
        let saved = self
            .saved
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        #[cfg(test)]
        if let Some(hook) = self.after_seed_allocation.take() {
            hook();
        }
        saved.validate_under_archive_root(guard.store(), guard.source(), guard.archive_root())?;
        guard.validate_source_only()?;
        Ok(saved.clone())
    }
}

/// Pure negative platform predicate, also used at the public owning retry BEFORE wrapper/
/// scope payment or maintenance. It issues no permission and performs no IO/allocation.
pub(in crate::store) fn require_supported_profile() -> Result<(), AppError> {
    if cfg!(windows) {
        Ok(())
    } else {
        Err(critical_namespaces::refused())
    }
}

fn select_initial_files(
    resources: &ColdV2GenesisReadResources,
    guard: &ColdV2InitializingGroupReadGuard<'_, '_>,
) -> Result<RepairFiles, AppError> {
    resources.validate_original(guard.store(), guard.source())?;
    guard.validate_source_only()?;
    let (base, directory, group_id) = guard.directory_data();
    let capture = &guard.source().source().capture;
    let files = RepairFiles {
        basis: guard.scope_basis_data().clone(),
        group_path: cold_capture::owned_path(base)?,
        seed_path: child(base, SEED)?,
        pin_path: child(base, PIN)?,
        head_path: child(&capture.target, HEAD)?,
        stage_path: child(&capture.target, STAGE)?,
        group_directory: directory.try_clone().map_err(io)?,
        group_id,
        target_directory: capture.target_directory.try_clone().map_err(io)?,
        target_id: capture.target_id,
        // A present malformed/partial seed never becomes absence. The capture is not
        // issued until actual authentication and original SourceSaved comparisons succeed.
        seed: cold_capture::read_cold_control(&child(base, SEED)?, codec::MAX_ACTIVATION_FRAME)?,
        pin: OnceLock::new(),
        head: OnceLock::new(),
        candidates: OnceLock::new(),
        resources: resources.clone(),
    };
    if let Some(file) = open_selected_target(&files.pin_path)? {
        let _ = files.pin.set(SelectedTarget::new(file));
    }
    // Do not select either File if BOTH names exist. Equal complete bytes are still
    // ambiguous work, and this first producer has no authority to remove either name.
    let current_present = path_present(&files.head_path)?;
    let stage_present = path_present(&files.stage_path)?;
    if current_present && stage_present {
        return Err(critical_namespaces::refused());
    }
    let selected_path = if current_present {
        &files.head_path
    } else {
        &files.stage_path
    };
    if let Some(file) = open_selected_target(selected_path)? {
        let _ = files.head.set(SelectedTarget::new(file));
    }
    // No target identity/content query occurs until caller has cached this whole selection.
    Ok(files)
}

fn path_present(path: &Path) -> Result<bool, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        _ => Err(critical_namespaces::refused()),
    }
}

fn open_selected_target(path: &Path) -> Result<Option<File>, AppError> {
    if !path_present(path)? {
        return Ok(None);
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io)?;
    // Original identity/prefix checks happen AFTER this handle is cached in its slot.
    Ok(Some(file))
}

#[derive(Clone, Copy)]
enum Goal {
    Repairable,
    ReadyToPromote,
    Complete,
}

struct ComparedTargets {
    pin_len: Option<usize>,
    head: HeadName,
    head_len: Option<usize>,
}

impl RepairFiles {
    fn candidates(&self) -> Result<&RepairCandidates, AppError> {
        self.candidates
            .get()
            .ok_or_else(critical_namespaces::refused)
    }

    fn authenticate_candidates(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.resources.validate_original(store, source)?;
        verify_control(&self.seed_path, &self.seed)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = codec::open_activation_seed_data(&key, &self.seed.frame)?;
        let decoded =
            codec::parse_activation_seed_data(&key, self.resources.expectation_data(), &plain)?;
        let data = source.data();
        decoded.head.require_original_public_data(
            &decoded.pin,
            &codec::GenesisPublicData {
                profile: data.profile,
                admission: data.admission,
                header: data.header,
                image: data.state,
            },
        )?;
        if let Some(candidates) = self.candidates.get() {
            if candidates.pin.as_slice() != decoded.pin_frame
                || candidates.head.as_slice() != decoded.head_frame
            {
                return Err(critical_namespaces::refused());
            }
        } else {
            let candidates = RepairCandidates {
                pin: Zeroizing::new(decoded.pin_frame.to_vec()),
                head: Zeroizing::new(decoded.head_frame.to_vec()),
            };
            self.candidates
                .set(candidates)
                .map_err(|_| critical_namespaces::refused())?;
        }
        self.resources.require_original_negative(store, source)
    }

    fn head_name(&self) -> Result<HeadName, AppError> {
        let current = path_present(&self.head_path)?;
        let stage = path_present(&self.stage_path)?;
        match (self.head.get(), current, stage) {
            (None, false, false) => Ok(HeadName::Absent),
            (Some(_), false, true) => Ok(HeadName::Stage),
            (Some(_), true, false) => Ok(HeadName::Current),
            _ => Err(critical_namespaces::refused()),
        }
    }

    fn compare_targets(&self, goal: Goal) -> Result<ComparedTargets, AppError> {
        let candidates = self.candidates()?;
        let pin_len = if let Some(pin) = self.pin.get() {
            Some(pin.require_prefix(&self.pin_path, &candidates.pin)?)
        } else {
            super::super::require_absent(&self.pin_path)?;
            None
        };
        let head = self.head_name()?;
        let head_len = match head {
            HeadName::Absent => None,
            HeadName::Stage | HeadName::Current => {
                let path = if head == HeadName::Stage {
                    &self.stage_path
                } else {
                    &self.head_path
                };
                let length = self
                    .head
                    .get()
                    .ok_or_else(critical_namespaces::refused)?
                    .require_prefix(path, &candidates.head)?;
                // A partial current has never been an approved producer state. Never
                // repair it as a stage, even though a complete seed is available.
                if head == HeadName::Current && length != candidates.head.len() {
                    return Err(critical_namespaces::refused());
                }
                Some(length)
            }
        };
        let required_head = match goal {
            Goal::Repairable => None,
            Goal::ReadyToPromote => Some(HeadName::Stage),
            Goal::Complete => Some(HeadName::Current),
        };
        if required_head.is_some_and(|required_head| {
            pin_len != Some(candidates.pin.len())
                || head != required_head
                || head_len != Some(candidates.head.len())
        }) {
            return Err(critical_namespaces::refused());
        }
        Ok(ComparedTargets {
            pin_len,
            head,
            head_len,
        })
    }

    fn require_compared_names(&self, compared: &ComparedTargets) -> Result<(), AppError> {
        match (self.pin.get(), compared.pin_len) {
            (Some(pin), Some(length)) => pin.require_name(&self.pin_path, length)?,
            (None, None) => super::super::require_absent(&self.pin_path)?,
            _ => return Err(critical_namespaces::refused()),
        }
        if self.head_name()? != compared.head {
            return Err(critical_namespaces::refused());
        }
        match (self.head.get(), compared.head, compared.head_len) {
            (None, HeadName::Absent, None) => {}
            (Some(head), HeadName::Stage, Some(length)) => {
                head.require_name(&self.stage_path, length)?
            }
            (Some(head), HeadName::Current, Some(length)) => {
                head.require_name(&self.head_path, length)?
            }
            _ => return Err(critical_namespaces::refused()),
        }
        Ok(())
    }

    fn require_inventory(
        &self,
        source: &GenesisSourceDataRef<'_>,
        compared: &ComparedTargets,
    ) -> Result<(), AppError> {
        use inventory::GenesisRepairHeadInventory as Head;
        inventory::require_fixed_initializing_inventory(
            (&self.group_path, &self.group_directory, self.group_id),
            (
                &source.source().capture.target,
                &self.target_directory,
                self.target_id,
            ),
            compared.pin_len.is_some(),
            match compared.head {
                HeadName::Absent => Head::Absent,
                HeadName::Stage => Head::Stage,
                HeadName::Current => Head::Current,
            },
        )
    }

    fn validate_under_archive_root(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
        goal: Goal,
    ) -> Result<(), AppError> {
        self.authenticate_candidates(store, source)?;
        let compared = self.compare_targets(goal)?;
        self.require_inventory(source, &compared)?;
        #[cfg(test)]
        if let Some(hook) = AFTER_COMPARERS.with(|slot| slot.borrow_mut().take()) {
            hook();
        }
        source.source().validate_under_archive_root(store, root)?;
        // Comparers/source/account waits precede the LAST ID3 and held/named proofs. Repeat
        // the bounded negative inventory after source reads to reject new private work.
        self.require_inventory(source, &compared)?;
        source.source().validate_under_archive_root(store, root)?;
        #[cfg(test)]
        if let Some(hook) = AFTER_SOURCE.with(|slot| slot.borrow_mut().take()) {
            hook();
        }
        store.validate_mounted_current_layout_under_archive_root(root)?;
        let capture = &source.source().capture;
        require_directory_name(&capture.root, &capture.root_directory, capture.root_id)?;
        require_directory_name(&capture.target, &self.target_directory, self.target_id)?;
        require_directory_name(&self.group_path, &self.group_directory, self.group_id)?;
        verify_read_name(&self.seed_path, &self.seed.object)?;
        self.require_compared_names(&compared)?;
        self.resources.require_original_negative(store, source)
    }
}

impl ColdV2GenesisRepairSeed {
    pub(in crate::store) fn scope_basis_data(&self) -> &ColdV2GenesisScopeBasis {
        &self.files.basis
    }
    pub(in crate::store) fn expectation_data(&self) -> &SavedSourceExpectationData {
        self.files.resources.expectation_data()
    }
    pub(in crate::store) fn read_resources(&self) -> ColdV2GenesisReadResources {
        self.files.resources.clone()
    }
    pub(in crate::store) fn directory_data(
        &self,
    ) -> (&Path, &File, catcoms_storage::kept::SessionId) {
        (
            &self.files.group_path,
            &self.files.group_directory,
            self.files.group_id,
        )
    }
    pub(in crate::store) fn retains_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.files
            .resources
            .require_original_negative(store, source)
    }
    pub(in crate::store) fn validate_under_archive_root(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        self.files
            .validate_under_archive_root(store, source, root, Goal::Repairable)
    }
}

impl PreparedColdV2GenesisRepair {
    pub(in crate::store) fn new(seed: Arc<ColdV2GenesisRepairSeed>) -> Self {
        Self { saved: None, seed }
    }

    pub(in crate::store) fn require_same_saved_head(
        &self,
        head: &Arc<SavedColdV2GenesisRepairedHead>,
    ) -> Result<(), AppError> {
        if self
            .saved
            .as_ref()
            .is_some_and(|saved| Arc::ptr_eq(saved, head))
            && Arc::ptr_eq(&self.seed, head.seed())
        {
            Ok(())
        } else {
            Err(critical_namespaces::refused())
        }
    }

    fn validate_effect_guard(
        &self,
        guard: &ColdInitializingV2GenesisRepairGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.seed
            .files
            .resources
            .require_same_original(guard.resources())?;
        if !Arc::ptr_eq(&self.seed, guard.seed()) {
            return Err(critical_namespaces::refused());
        }
        guard.validate_source_only()?;
        self.seed.validate_under_archive_root(
            guard.store(),
            guard.source(),
            guard.archive_root(),
        )?;
        guard.validate_source_only()
    }

    /// Strict-create only when THIS receipt still selects absence. Cache the actual File
    /// before its first identity query or write. Neither lookup nor failure adopts by path.
    fn create_target(
        &self,
        guard: &ColdInitializingV2GenesisRepairGroupGuard<'_, '_>,
        slot: &OnceLock<SelectedTarget>,
        path: &Path,
        fault: u8,
    ) -> Result<(), AppError> {
        if slot.get().is_none() {
            self.validate_effect_guard(guard)?;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(io)?;
            slot.set(SelectedTarget::new(file))
                .map_err(|_| critical_namespaces::refused())?;
            #[cfg(test)]
            guard.store().archive_declaration_fault(fault)?;
        }
        #[cfg(not(test))]
        let _ = fault;
        Ok(())
    }

    fn finish_target(
        &self,
        guard: &ColdInitializingV2GenesisRepairGroupGuard<'_, '_>,
        slot: &OnceLock<SelectedTarget>,
        path: &Path,
        expected: &[u8],
        short_fault: u8,
    ) -> Result<(), AppError> {
        let selected = slot.get().ok_or_else(critical_namespaces::refused)?;
        let length = selected.require_prefix(path, expected)?;
        self.validate_effect_guard(guard)?;
        if length != expected.len() {
            let mut original = &selected.file;
            original.seek(SeekFrom::Start(0)).map_err(io)?;
            #[cfg(test)]
            if guard
                .store()
                .critical_namespace_fault
                .load(Ordering::SeqCst)
                == short_fault
            {
                original
                    .write_all(&expected[..expected.len() / 2])
                    .map_err(io)?;
                return Err(critical_namespaces::refused());
            }
            #[cfg(not(test))]
            let _ = short_fault;
            // Existing length was <=expected and an exact prefix. Full overwrite extends
            // the SAME object without an unexamined suffix. Never set_len/truncate.
            original.write_all(expected).map_err(io)?;
        }
        // A complete original candidate needs only its barrier, never another write.
        selected.file.sync_all().map_err(io)?;
        Ok(())
    }

    pub(in crate::store) fn repair(
        &mut self,
        guard: &ColdInitializingV2GenesisRepairGroupGuard<'_, '_>,
    ) -> Result<Arc<SavedColdV2GenesisRepairedHead>, AppError> {
        self.validate_effect_guard(guard)?;
        if let Some(saved) = &self.saved {
            saved.validate_under_repair_guard(guard)?;
            return Ok(saved.clone());
        }
        let files = &self.seed.files;
        let candidates = files.candidates()?;
        self.create_target(guard, &files.pin, &files.pin_path, 220)?;
        self.finish_target(guard, &files.pin, &files.pin_path, &candidates.pin, 221)?;
        self.validate_effect_guard(guard)?;
        files.group_directory.sync_all().map_err(io)?;
        if files.head_name()? != HeadName::Current {
            self.create_target(guard, &files.head, &files.stage_path, 222)?;
            self.finish_target(guard, &files.head, &files.stage_path, &candidates.head, 223)?;
            files.validate_under_archive_root(
                guard.store(),
                guard.source(),
                guard.archive_root(),
                Goal::ReadyToPromote,
            )?;
            guard.validate_source_only()?;
            // Final proof already checked exact candidate prefix/name, absent current,
            // source, ID3 and cached negatives. No new comparer/fee follows before rename.
            fs::rename(&files.stage_path, &files.head_path).map_err(io)?;
            #[cfg(test)]
            guard.store().archive_declaration_fault(224)?;
        } else {
            // Existing current must be the complete exact candidate, never a repair prefix.
            files
                .head
                .get()
                .ok_or_else(critical_namespaces::refused)?
                .file
                .sync_all()
                .map_err(io)?;
        }
        self.validate_effect_guard(guard)?;
        files.target_directory.sync_all().map_err(io)?;
        #[cfg(test)]
        guard.store().archive_declaration_fault(225)?;
        files.validate_under_archive_root(
            guard.store(),
            guard.source(),
            guard.archive_root(),
            Goal::Complete,
        )?;
        guard.validate_source_only()?;
        // Cache BEFORE a last allocation/physical proof can refuse. The worker is the ONLY
        // owner of this back-reference; no Seed/custody -> SavedHead cycle is possible.
        let saved = Arc::new(SavedColdV2GenesisRepairedHead {
            seed: self.seed.clone(),
            task: guard.task().observe_original(),
            writer: guard.writer_registration().observe_original(),
        });
        self.saved = Some(saved.clone());
        #[cfg(test)]
        if let Some(hook) = AFTER_RECEIPT.with(|slot| slot.borrow_mut().take()) {
            hook();
        }
        saved.validate_under_repair_guard(guard)?;
        Ok(saved)
    }
}

impl SavedColdV2GenesisRepairedHead {
    pub(in crate::store) fn seed(&self) -> &Arc<ColdV2GenesisRepairSeed> {
        &self.seed
    }
    pub(in crate::store) fn writer_observation(&self) -> &V2WriterObservation {
        &self.writer
    }
    pub(in crate::store) fn retains_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        self.task.require_same_original(task)?;
        task.retains_original(store, source)?;
        self.seed.retains_original(store, source)?;
        if self.writer.is_cancelled() {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }

    fn validate_under_repair_guard(
        &self,
        guard: &ColdInitializingV2GenesisRepairGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.seed, guard.seed()) {
            return Err(critical_namespaces::refused());
        }
        self.retains_original(guard.store(), guard.source(), guard.task())?;
        guard.validate_source_only()?;
        guard
            .writer_registration()
            .require_same_observation(&self.writer)?;
        self.seed.files.validate_under_archive_root(
            guard.store(),
            guard.source(),
            guard.archive_root(),
            Goal::Complete,
        )?;
        guard.validate_source_only()?;
        guard
            .writer_registration()
            .require_same_observation(&self.writer)
    }
}

// Three actual read/proof guards preserve their distinct scope semantics. This private
// implementation macro shares only their final physical/installed proof, not constructors.
macro_rules! repaired_read_proof {
    ($method:ident, $guard:ident) => {
        impl SavedColdV2GenesisRepairedHead {
            pub(in crate::store) fn $method(&self, guard: &$guard<'_, '_>) -> Result<(), AppError> {
                if !Arc::ptr_eq(&self.seed, guard.seed()) {
                    return Err(critical_namespaces::refused());
                }
                self.retains_original(guard.store(), guard.source(), guard.task())?;
                guard.validate_source_only()?;
                guard.require_installed_initializing_observation(&self.writer)?;
                self.seed.files.validate_under_archive_root(
                    guard.store(),
                    guard.source(),
                    guard.archive_root(),
                    Goal::Complete,
                )?;
                guard.validate_source_only()?;
                guard.require_installed_initializing_observation(&self.writer)
            }
        }
    };
}
repaired_read_proof!(
    validate_under_initializing_guard,
    ColdInitializingV2GenesisReadGroupGuard
);
repaired_read_proof!(
    validate_under_proof_guard,
    ColdRepairedV2GenesisCurrentProofGroupGuard
);
repaired_read_proof!(
    validate_under_current_guard,
    ColdRepairedV2GenesisCurrentGroupGuard
);

fn validate_repair_invoice() -> Result<(), AppError> {
    // Actual simultaneous layouts, PLUS bounded path/AEAD/copy/native-walk scratch.
    // Source/read/provider/SDK graphs, the scope table/buffers, task and writer have their
    // own accounts. Reusing the cold loan does not reuse a Current2 receipt or invoice.
    let bytes = 8 * (MAX_PATH + 256)
        + 12 * (codec::MAX_ACTIVATION_FRAME + codec::MAX_PIN_FRAME + codec::MAX_HEAD_FRAME)
        + cold_capture::COLD_READ_RESOURCE_BODY_BYTES
        + 2 * std::mem::size_of::<RepairFiles>()
        + std::mem::size_of::<PreparedColdV2GenesisRepairCapture>()
        + std::mem::size_of::<PreparedColdV2GenesisRepair>()
        + 2 * std::mem::size_of::<ColdV2GenesisRepairSeed>()
        + 2 * std::mem::size_of::<SavedColdV2GenesisRepairedHead>()
        + 8 * std::mem::size_of::<SelectedTarget>()
        + crate::store::critical_namespaces::current_layout::COLD_V2_GROUP_READ_GUARD_BYTES
        + crate::store::critical_namespaces::current_layout::COLD_INITIALIZING_GROUP_GUARD_BYTES
        + crate::store::chat_admission::v2_current::COLD_INITIALIZING_SCOPE_METADATA_BYTES
        + super::genesis_inventory::GENESIS_FIXED_INVENTORY_BYTES
        + 8 * 64
        + 16 * 1024;
    if bytes > cold_capture::COLD_CAPTURE_METADATA_FEE {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
