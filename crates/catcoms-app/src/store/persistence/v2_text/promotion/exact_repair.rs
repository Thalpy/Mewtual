//! Distinct authenticated-seed recovery of NONSELECTED empty-promotion children.
//!
//! This is not warm-created-file authority. A complete independently checked journal and
//! fresh Current/lane/quiescence admission authorize reconstructing its exact listed child,
//! even when the prior process closed its partial File. No selected head, MAIN seed, shared
//! historical page, wrong prefix or foreign context can be rewritten by this role.
use super::recovery::{
    self, require_absent, require_control_name, HeldRecoveryControl, HeldRecoveryDirectory,
    RecoveryPlan,
};
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextPromotionSeedRepair, V2TextPromotionSeedRepairSourceSlot,
};
use catcoms_storage::kept::{session_at, session_id, SessionAt};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// No Files or physical lease in dormant preparation. The NEW independent4MiB plan is
/// shared by every active capture/marker; no copied frame can outlive its actual payment.
pub(in crate::store) struct PreparedV2TextPromotionSeedRepair {
    ready: Option<ReadyV2TextPromotionSelection>,
    // DATA handoff bookkeeping only; never sampled by any admission or physical worker.
    installed: bool,
    #[cfg(all(test, windows))]
    after_capture: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    fail_after_ready: bool,
    plan: Arc<RecoveryPlan>,
}
impl PreparedV2TextPromotionSeedRepair {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        original: &PreparedV2TextPromotion,
    ) -> Result<Self, AppError> {
        let plan = RecoveryPlan::from_prepared(store, source, original)?;
        Ok(Self {
            ready: None,
            installed: false,
            #[cfg(all(test, windows))]
            after_capture: None,
            #[cfg(all(test, windows))]
            fail_after_ready: false,
            plan,
        })
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        self.plan.resources()
    }
    pub(in crate::store) fn recovery_resources(
        &self,
    ) -> recovery::V2TextPromotionRecoveryResources {
        self.plan.payment.clone()
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextPromotionObservation {
        self.plan.original.clone()
    }
    /// DATA bookkeeping only: a lost run return must not hide the already cached result.
    /// No Current, read/miss, stage, signing or selector permission follows from this bit.
    pub(in crate::store) fn has_completed_ready_data(&self) -> bool {
        self.ready.is_some() || self.installed
    }
    pub(in crate::store) fn validate_preparation(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        if self.installed {
            return Err(refused());
        }
        self.plan.validate_account()?;
        self.resources().validate_original(store, source)
    }
    /// The slot can only be borrowed from the SAME affine active-capture owner which
    /// already owns the physical lease. No owning source/take API detaches Files from it.
    pub(in crate::store) fn capture(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        slot: &mut V2TextPromotionSeedRepairSourceSlot<'_>,
    ) -> Result<(), AppError> {
        if self.installed {
            return Err(refused());
        }
        self.plan.require_guard(guard)?;
        if slot.get().is_none() {
            // Cache inert source FIRST, before any File/native identity/read can fail.
            slot.cache_initial(CapturedV2TextPromotionSeedRepair::empty(self.plan.clone()));
        }
        let source = slot.get_mut().ok_or_else(refused)?;
        source
            .plan
            .original
            .require_same_original(&self.observe_original())?;
        source.fill(guard)?;
        #[cfg(all(test, windows))]
        if let Some(hook) = self.after_capture.take() {
            hook();
        }
        source.validate_under_current_guard(guard)
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
    /// Caller runs outside all writers. Marker owns every captured readonly FD BEFORE its
    /// lease, and local writable FDs drain before that marker drops on every Err/return.
    pub(in crate::store) fn run(
        &mut self,
        admitted: AdmittedV2TextPromotionSeedRepair,
    ) -> Result<(), AppError> {
        admitted.require_same_promotion(&self.observe_original())?;
        self.plan.validate_account()?;
        let source = admitted.source();
        source
            .plan
            .original
            .require_same_original(&self.observe_original())?;
        let slots = repair_children(source)?;
        let facts = work::PromotionReadyFacts::from_recovery_data(&self.plan, slots);
        self.ready = Some(ReadyV2TextPromotionSelection {
            original: self.observe_original(),
            facts: Arc::new(facts),
            work: self.plan.original_work.clone(),
        });
        #[cfg(all(test, windows))]
        if std::mem::take(&mut self.fail_after_ready) {
            return Err(AppError::Io(
                "injected lost completed seed-repair return".into(),
            ));
        }
        drop(admitted);
        Ok(())
    }
    /// Move the actual completed Ready into the SAME original promotion. No IO, signer,
    /// key, new candidate or selection permission follows. Every refusal retains both.
    pub(in crate::store) fn install_ready_into(
        &mut self,
        original: &mut PreparedV2TextPromotion,
    ) -> Result<(), AppError> {
        if self.installed {
            return self
                .plan
                .original
                .require_same_original(&original.observe_original());
        }
        self.plan.require_prepared(original)?;
        let ready = self.ready.as_ref().ok_or_else(refused)?;
        ready
            .original
            .require_same_original(&original.observe_original())?;
        let slots = ready.facts.slots_data();
        let ready = self
            .ready
            .take()
            .expect("completed original recovery Ready checked");
        original.slots = slots;
        original.ready = Some(ready);
        self.installed = true;
        Ok(())
    }
}

enum OptionalDirectory {
    Unselected,
    Absent,
    Present(HeldRecoveryDirectory),
}
enum DependentSelection {
    Unselected,
    Absent,
    Present(HeldRecoveryControl),
}
impl DependentSelection {
    fn capture(&mut self, path: &Path, expected: &[u8]) -> Result<(), AppError> {
        if matches!(self, Self::Unselected) {
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => *self = Self::Absent,
                Ok(_) => *self = Self::Present(HeldRecoveryControl::empty()),
                Err(error) => return Err(recovery::io(error)),
            }
        }
        match self {
            Self::Absent => require_absent(path),
            Self::Present(held) => {
                held.capture(path, expected.len())?;
                if !expected.starts_with(held.raw()?) {
                    return Err(refused());
                }
                Ok(())
            }
            Self::Unselected => Err(refused()),
        }
    }
    fn require_current(&self, path: &Path, expected: &[u8]) -> Result<(), AppError> {
        match self {
            Self::Absent => require_absent(path),
            Self::Present(held) => {
                held.require_unchanged(path)?;
                if !expected.starts_with(held.raw()?) {
                    return Err(refused());
                }
                Ok(())
            }
            Self::Unselected => Err(refused()),
        }
    }
    fn require_name(&self, path: &Path) -> Result<(), AppError> {
        match self {
            Self::Absent => require_absent(path),
            Self::Present(held) => require_control_name(path, held),
            Self::Unselected => Err(refused()),
        }
    }
}

/// Actual read closure, not mutation permission. It lives ONLY in the Factory's coupled
/// active source+lease owner, then its admitted marker. Every field capable of retaining a
/// File precedes the independently paid plan. It has no owned-source extraction API.
pub(in crate::store) struct CapturedV2TextPromotionSeedRepair {
    target: HeldRecoveryDirectory,
    parent: HeldRecoveryControl,
    seed: HeldRecoveryControl,
    pages: OptionalDirectory,
    archive: DependentSelection,
    directory: DependentSelection,
    head_stage: DependentSelection,
    plan: Arc<RecoveryPlan>,
}
impl CapturedV2TextPromotionSeedRepair {
    fn empty(plan: Arc<RecoveryPlan>) -> Self {
        Self {
            target: HeldRecoveryDirectory::empty(),
            parent: HeldRecoveryControl::empty(),
            seed: HeldRecoveryControl::empty(),
            pages: OptionalDirectory::Unselected,
            archive: DependentSelection::Unselected,
            directory: DependentSelection::Unselected,
            head_stage: DependentSelection::Unselected,
            plan,
        }
    }
    fn fill(&mut self, guard: &V2TextCurrentGroupGuard<'_, '_>) -> Result<(), AppError> {
        self.plan.require_guard(guard)?;
        self.target.capture(&self.plan.target)?;
        if self.target.id()? != self.plan.target_id {
            return Err(refused());
        }
        self.parent.capture(&self.plan.paths.head, 4136)?;
        if self.parent.raw()? != self.plan.original_head.as_slice() {
            return Err(refused());
        }
        self.seed.capture(&self.plan.paths.seed, 32768 + 40)?;
        recovery::verify_complete_seed(&self.plan, guard, self.seed.raw()?)?;
        require_absent(&self.plan.paths.seed_stage)?;
        require_absent(&self.plan.paths.wal)?;
        require_absent(&self.plan.paths.wal_stage)?;
        if matches!(self.pages, OptionalDirectory::Unselected) {
            match fs::symlink_metadata(&self.plan.paths.pages) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.pages = OptionalDirectory::Absent
                }
                Ok(_) => self.pages = OptionalDirectory::Present(HeldRecoveryDirectory::empty()),
                Err(error) => return Err(recovery::io(error)),
            }
        }
        match &mut self.pages {
            OptionalDirectory::Present(pages) => {
                pages.capture(&self.plan.paths.pages)?;
                self.archive
                    .capture(&self.plan.paths.archive, &self.plan.archive)?;
                self.directory
                    .capture(&self.plan.paths.directory, &self.plan.directory)?;
            }
            OptionalDirectory::Absent => {
                require_absent(&self.plan.paths.pages)?;
                self.archive = DependentSelection::Absent;
                self.directory = DependentSelection::Absent;
            }
            OptionalDirectory::Unselected => return Err(refused()),
        }
        self.head_stage
            .capture(&self.plan.paths.head_stage, &self.plan.head)
    }
    pub(in crate::store) fn validate_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.plan.require_guard(guard)?;
        self.require_current_data()?;
        recovery::verify_complete_seed(&self.plan, guard, self.seed.raw()?)?;
        let mut names = [""; 10];
        names[..7].copy_from_slice(&[
            "origin",
            "source.seed",
            "source.bin",
            "records.bin",
            "manifest.bin",
            "head.bin",
            "text.seed",
        ]);
        let mut count = 7;
        if matches!(self.pages, OptionalDirectory::Present(_)) {
            names[count] = "text.pages";
            count += 1;
        }
        if matches!(self.head_stage, DependentSelection::Present(_)) {
            names[count] = "head.stage";
            count += 1;
        }
        work::require_exact_inventory(guard.directory_data().0, &["pin.bin"], false)?;
        work::require_exact_inventory(&self.plan.target, &names[..count], true)?;
        guard.validate_original_namespace()?;
        self.require_names()?;
        guard.validate_source_only()
    }
    fn require_current_data(&self) -> Result<(), AppError> {
        self.target.require_name(&self.plan.target)?;
        self.parent.require_unchanged(&self.plan.paths.head)?;
        if self.parent.raw()? != self.plan.original_head.as_slice() {
            return Err(refused());
        }
        self.seed.require_unchanged(&self.plan.paths.seed)?;
        if self.seed.raw()? != self.plan.seed.as_slice() {
            return Err(refused());
        }
        require_absent(&self.plan.paths.seed_stage)?;
        require_absent(&self.plan.paths.wal)?;
        require_absent(&self.plan.paths.wal_stage)?;
        match &self.pages {
            OptionalDirectory::Absent => require_absent(&self.plan.paths.pages)?,
            OptionalDirectory::Present(pages) => pages.require_name(&self.plan.paths.pages)?,
            OptionalDirectory::Unselected => return Err(refused()),
        }
        self.archive
            .require_current(&self.plan.paths.archive, &self.plan.archive)?;
        self.directory
            .require_current(&self.plan.paths.directory, &self.plan.directory)?;
        self.head_stage
            .require_current(&self.plan.paths.head_stage, &self.plan.head)
    }
    fn require_names(&self) -> Result<(), AppError> {
        self.target.require_name(&self.plan.target)?;
        require_control_name(&self.plan.paths.head, &self.parent)?;
        require_control_name(&self.plan.paths.seed, &self.seed)?;
        match &self.pages {
            OptionalDirectory::Absent => require_absent(&self.plan.paths.pages)?,
            OptionalDirectory::Present(pages) => pages.require_name(&self.plan.paths.pages)?,
            OptionalDirectory::Unselected => return Err(refused()),
        }
        self.archive.require_name(&self.plan.paths.archive)?;
        self.directory.require_name(&self.plan.paths.directory)?;
        self.head_stage.require_name(&self.plan.paths.head_stage)?;
        require_absent(&self.plan.paths.seed_stage)?;
        require_absent(&self.plan.paths.wal)?;
        require_absent(&self.plan.paths.wal_stage)
    }
}

fn repair_children(
    source: &CapturedV2TextPromotionSeedRepair,
) -> Result<work::PromotionSlots, AppError> {
    source.plan.validate_account()?;
    source.require_current_data()?;
    // The consumed marker already authenticated the complete MAIN and old selected parent.
    // No key/SDK/Store call follows. Rereads protect its exact still-held objects during IO.
    let target = work::require_directory(&source.plan.target, source.plan.target_id)?;
    let pages_id = match &source.pages {
        OptionalDirectory::Present(pages) => pages.id()?,
        OptionalDirectory::Absent => {
            fs::create_dir(&source.plan.paths.pages).map_err(recovery::io)?;
            let pages =
                crate::store::persistence::open_archive_directory(&source.plan.paths.pages)?;
            let id = session_id(&pages).map_err(recovery::io)?;
            work::sync_parent(&source.plan.target, source.plan.target_id)?;
            id
        }
        OptionalDirectory::Unselected => return Err(refused()),
    };
    let _pages = work::require_directory(&source.plan.paths.pages, pages_id)?;
    let archive_id = repair_child(
        &source.plan.paths.archive,
        &source.plan.archive,
        &source.archive,
    )?;
    let directory_id = repair_child(
        &source.plan.paths.directory,
        &source.plan.directory,
        &source.directory,
    )?;
    work::sync_parent(&source.plan.paths.pages, pages_id)?;
    let head_id = repair_child(
        &source.plan.paths.head_stage,
        &source.plan.head,
        &source.head_stage,
    )?;
    work::sync_parent(&source.plan.target, source.plan.target_id)?;
    crate::store::persistence::v2_genesis_activation::require_directory_name(
        &source.plan.target,
        &target,
        source.plan.target_id,
    )?;
    require_control_name(&source.plan.paths.seed, &source.seed)?;
    require_control_name(&source.plan.paths.head, &source.parent)?;
    Ok(work::PromotionSlots::from_recovery_data(
        source.seed.id()?,
        pages_id,
        archive_id,
        directory_id,
        head_id,
    ))
}

/// SeedExactRepair, intentionally distinct from warm creation. The authenticated journal
/// authorizes THIS closed nonselected path/frame. A held readonly source FD prevents native
/// ID reuse during a present-prefix repair; a fresh writable same-object FD is local to this
/// admitted run. Only the missing exact suffix is appended, with no truncate/reseal/sign.
fn repair_child(
    path: &Path,
    expected: &[u8],
    selection: &DependentSelection,
) -> Result<SessionId, AppError> {
    let mut file = match selection {
        DependentSelection::Absent => OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(recovery::io)?,
        DependentSelection::Present(held) => {
            held.require_unchanged(path)?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(recovery::io)?;
            if session_id(&file).map_err(recovery::io)? != held.id()? {
                return Err(refused());
            }
            file
        }
        DependentSelection::Unselected => return Err(refused()),
    };
    let id = session_id(&file).map_err(recovery::io)?;
    if !matches!(session_at(path, id), SessionAt::Ours) {
        return Err(refused());
    }
    let length =
        usize::try_from(file.metadata().map_err(recovery::io)?.len()).map_err(|_| refused())?;
    if length > expected.len() {
        return Err(refused());
    }
    file.seek(SeekFrom::Start(0)).map_err(recovery::io)?;
    let mut offset = 0;
    let mut buffer = [0u8; 4096];
    while offset < length {
        let wanted = buffer.len().min(length - offset);
        match file.read(&mut buffer[..wanted]) {
            Ok(0) => return Err(refused()),
            Ok(n) => {
                if buffer[..n] != expected[offset..offset + n] {
                    return Err(refused());
                }
                offset += n;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(recovery::io(error)),
        }
    }
    if !matches!(session_at(path, id), SessionAt::Ours) {
        return Err(refused());
    }
    file.write_all(&expected[offset..]).map_err(recovery::io)?;
    file.sync_all().map_err(recovery::io)?;
    if !matches!(session_at(path, id), SessionAt::Ours) {
        return Err(refused());
    }
    // Verify the whole exact result through this SAME held writable File, without opening
    // another authority path or allocating an unbounded response buffer.
    let mut offset = 0;
    while offset < expected.len() {
        let wanted = buffer.len().min(expected.len() - offset);
        match recovery::positioned_read(&file, &mut buffer[..wanted], offset as u64) {
            Ok(0) => return Err(refused()),
            Ok(n) => {
                if buffer[..n] != expected[offset..offset + n] {
                    return Err(refused());
                }
                offset += n;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(recovery::io(error)),
        }
    }
    if recovery::positioned_read(&file, &mut buffer[..1], offset as u64).map_err(recovery::io)? != 0
    {
        return Err(refused());
    }
    Ok(id)
}

impl std::fmt::Debug for PreparedV2TextPromotionSeedRepair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2TextPromotionSeedRepair")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for CapturedV2TextPromotionSeedRepair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturedV2TextPromotionSeedRepair")
            .finish_non_exhaustive()
    }
}
