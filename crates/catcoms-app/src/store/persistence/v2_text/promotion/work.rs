//! Finite, leased empty-promotion IO. Every File is local to one helper invocation; none
//! survives a run return, including a failed write/rename/barrier. Persistent retry state
//! keeps the exact original frames and native IDs, not an idle physical-session lease.
use super::*;
use crate::store::persistence::v2_text::io::OpenedTextControl;
use catcoms_storage::kept::{session_at, session_id, SessionAt};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Deterministic actual-IO cuts; absent from production layouts and behavior. The pause
/// callback runs with no Store/Root/account lock and can park the real open File/OSlease.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum V2TextPromotionTestCut {
    SeedCreated,
    SeedPartial,
    SeedStaged,
    SeedRenamed,
    SeedSaved,
    ArchiveCreated,
    DirectoryCreated,
    DirectoryWritten,
    HeadCreated,
    HeadStaged,
    HeadRenamed,
    SavedCached,
}
#[cfg(test)]
#[derive(Default)]
pub(super) struct PromotionTestHooks {
    pub(super) fault: Option<V2TextPromotionTestCut>,
    pub(super) pause: Option<(V2TextPromotionTestCut, Box<dyn FnOnce() + Send>)>,
}
#[cfg(test)]
impl PromotionTestHooks {
    fn hit(&mut self, point: V2TextPromotionTestCut) -> Result<(), AppError> {
        if self.pause.as_ref().is_some_and(|(cut, _)| *cut == point) {
            let (_, callback) = self.pause.take().expect("matched exact IO pause");
            callback();
        }
        if self.take_fault(point) {
            return Err(io("injected operational text promotion IO failure"));
        }
        Ok(())
    }
    fn take_fault(&mut self, point: V2TextPromotionTestCut) -> bool {
        if self.fault == Some(point) {
            self.fault = None;
            true
        } else {
            false
        }
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct PromotionSlots {
    seed: Option<SessionId>,
    pages: Option<SessionId>,
    archive: Option<SessionId>,
    directory: Option<SessionId>,
    head: Option<SessionId>,
}

impl PromotionSlots {
    /// Native labels are DATA only. The real seed-repair worker supplies these after its
    /// actual complete child/file/parent barriers; this constructor grants no permission.
    pub(super) fn from_recovery_data(
        seed: SessionId,
        pages: SessionId,
        archive: SessionId,
        directory: SessionId,
        head: SessionId,
    ) -> Self {
        Self {
            seed: Some(seed),
            pages: Some(pages),
            archive: Some(archive),
            directory: Some(directory),
            head: Some(head),
        }
    }
    pub(super) fn permits_first_seed_scratch_data(&self) -> bool {
        self.pages.is_none()
            && self.archive.is_none()
            && self.directory.is_none()
            && self.head.is_none()
    }
    pub(super) fn is_unstarted_data(&self) -> bool {
        self.seed.is_none() && self.permits_first_seed_scratch_data()
    }
    pub(super) fn seed_native_data(&self) -> Option<SessionId> {
        self.seed
    }
    /// Same-plan actual scratch receipt is checked by the private caller BEFORE clearing
    /// this obsolete negative file fact. This refunds no bytes/objects/execution allowance.
    pub(super) fn clear_seed_scratch_data(&mut self) -> Result<(), AppError> {
        if !self.permits_first_seed_scratch_data() {
            return Err(refused());
        }
        self.seed = None;
        Ok(())
    }
}

/// Fixed successful stage facts; no File, Store/key/session/SDK/parent receipt chain.
pub(super) struct PromotionReadyFacts {
    context: TextContextData,
    target: PathBuf,
    target_id: SessionId,
    slots: PromotionSlots,
    original_head: Zeroizing<Vec<u8>>,
    head: Zeroizing<Vec<u8>>,
    seed: Zeroizing<Vec<u8>>,
    archive: Zeroizing<Vec<u8>>,
    archive_id: [u8; 32],
    directory: Zeroizing<Vec<u8>>,
    directory_id: [u8; 32],
    #[cfg(all(test, windows))]
    _drop_notice: PromotionFactsDropNotice,
}

impl PromotionReadyFacts {
    /// Frozen output DATA only. The separate consuming seed-repair marker and actual
    /// worker barriers are required before the caller may cache a Ready wrapper.
    pub(super) fn from_recovery_data(
        plan: &super::recovery::RecoveryPlan,
        slots: PromotionSlots,
    ) -> Self {
        let mut target = PathBuf::with_capacity(plan.target.as_os_str().len());
        target.push(&plan.target);
        Self {
            context: plan.context.clone(),
            target,
            target_id: plan.target_id,
            slots,
            original_head: Zeroizing::new(plan.original_head.to_vec()),
            head: Zeroizing::new(plan.head.to_vec()),
            seed: Zeroizing::new(plan.seed.to_vec()),
            archive: Zeroizing::new(plan.archive.to_vec()),
            archive_id: plan.archive_id,
            directory: Zeroizing::new(plan.directory.to_vec()),
            directory_id: *plan.directory_id.as_bytes(),
            #[cfg(all(test, windows))]
            _drop_notice: PromotionFactsDropNotice(None),
        }
    }
    pub(super) fn slots_data(&self) -> PromotionSlots {
        self.slots
    }
}

#[cfg(all(test, windows))]
struct PromotionFactsDropNotice(
    Option<(
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicBool>,
    )>,
);
#[cfg(all(test, windows))]
impl Drop for PromotionFactsDropNotice {
    fn drop(&mut self) {
        if let Some((facts_dropped, exact_fee_dropped)) = &self.0 {
            use std::sync::atomic::Ordering;
            assert!(
                !exact_fee_dropped.load(Ordering::SeqCst),
                "exact read debit refunded before the actual saved facts/frames drained"
            );
            facts_dropped.store(true, Ordering::SeqCst);
        }
    }
}

/// Shared Ready/Saved facts plus one complete final readonly proof, paid independently of
/// the active16MiB worker. The fixed directory traversal belongs only to active
/// Ready/Prepared (which retain that actual work fee), not the escaping Saved metadata.
const PROMOTION_READ_METADATA_BYTES: usize = 4 * std::mem::size_of::<PromotionReadyFacts>()
    + 2 * std::mem::size_of::<ReadyReadFiles>()
    + 4 * std::mem::size_of::<OriginalV2TextPromotionObservation>()
    + 2 * std::mem::size_of::<SavedV2TextPromotion>()
    + crate::store::critical_namespaces::current_layout::V2_TEXT_CURRENT_GROUP_GUARD_BYTES
    + 11 * (MAX_TEXT_PATH + 256)
    + 2 * (24773 + 1519 + 1725 + 16470 + 4136)
    + 32 * 1024;
const _: () = assert!(PROMOTION_READ_METADATA_BYTES <= TEXT_READ_METADATA_FEE);

fn io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

pub(super) fn require_original_genesis_inventory(
    guard: &V2TextCurrentGroupGuard<'_, '_>,
) -> Result<(), AppError> {
    // Both supported readers have an explicit fixed fee. Linux borrows a fixed
    // getdents buffer and independently opens the original held directory.
    require_exact_inventory(guard.directory_data(), &["pin.bin"], false)?;
    require_exact_inventory(
        guard.target_directory_data(),
        &[
            "origin",
            "source.seed",
            "source.bin",
            "records.bin",
            "manifest.bin",
            "head.bin",
        ],
        false,
    )
}

pub(super) fn require_exact_inventory(
    directory: (&Path, &File, SessionId),
    names: &[&str],
    permits_pages_directory: bool,
) -> Result<(), AppError> {
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (directory, names, permits_pages_directory);
        Err(refused())
    }
    #[cfg(target_os = "linux")]
    {
        if !directory.0.is_absolute() || directory.0.as_os_str().len() > MAX_TEXT_PATH {
            return Err(refused());
        }
        crate::store::persistence::exact_inventory_linux::require_exact_names(
            directory,
            names,
            permits_pages_directory,
        )
    }
    #[cfg(windows)]
    {
        let path = directory.0;
        if path.as_os_str().len() > MAX_TEXT_PATH || names.len() > 10 {
            return Err(refused());
        }
        let mut entries = fs::read_dir(path).map_err(io)?;
        let mut seen = 0u16;
        // At most known-count+1 observations; no collect, sort, entry.path or recursion.
        for _ in 0..=names.len() {
            let Some(entry) = entries.next() else {
                return if seen == (1u16 << names.len()) - 1 {
                    Ok(())
                } else {
                    Err(refused())
                };
            };
            let entry = entry.map_err(io)?;
            let name = entry.file_name();
            let index = names
                .iter()
                .position(|expected| name == std::ffi::OsStr::new(expected))
                .ok_or_else(refused)?;
            let file_type = entry.file_type().map_err(io)?;
            if if permits_pages_directory && names[index] == "text.pages" {
                !file_type.is_dir()
            } else {
                !file_type.is_file()
            } {
                return Err(refused());
            }
            let bit = 1u16 << index;
            if seen & bit != 0 {
                return Err(refused());
            }
            seen |= bit;
        }
        Err(refused())
    }
}

pub(super) fn page_child(base: &Path, id: &[u8; 32]) -> Result<PathBuf, AppError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut leaf = String::with_capacity(70);
    leaf.push_str("p-");
    for byte in id {
        leaf.push(HEX[(byte >> 4) as usize] as char);
        leaf.push(HEX[(byte & 15) as usize] as char);
    }
    leaf.push_str(".bin");
    let capacity = base
        .as_os_str()
        .len()
        .checked_add(1 + leaf.len())
        .filter(|n| *n <= MAX_TEXT_PATH)
        .ok_or_else(refused)?;
    let mut path = PathBuf::with_capacity(capacity);
    path.push(base);
    path.push(leaf);
    if path.capacity() > MAX_TEXT_PATH {
        return Err(refused());
    }
    Ok(path)
}

pub(super) fn require_directory(path: &Path, expected: SessionId) -> Result<File, AppError> {
    let directory = crate::store::persistence::open_archive_directory(path)?;
    crate::store::persistence::v2_genesis_activation::require_directory_name(
        path, &directory, expected,
    )?;
    Ok(directory)
}

fn require_absent(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(refused()),
    }
}

fn require_exact(
    path: &Path,
    expected_id: SessionId,
    expected: &[u8],
) -> Result<OpenedTextControl, AppError> {
    let control = read_control(path, expected.len())?;
    if control.id != expected_id || control.frame.as_slice() != expected {
        return Err(refused());
    }
    Ok(control)
}

/// Read capture deliberately opens readonly. Windows FlushFileBuffers requires a writable
/// handle, so only this already-admitted physical path obtains a second SAME-object handle
/// for syncing a COMPLETE candidate. It performs no content write or truncation. Both
/// handles stay inside the finite run and drain before its marker/OSlease can be released.
fn sync_exact_control(
    path: &Path,
    expected_id: SessionId,
    expected: &[u8],
) -> Result<OpenedTextControl, AppError> {
    let control = require_exact(path, expected_id, expected)?;
    let writable = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io)?;
    if session_id(&writable).map_err(io)? != control.id {
        return Err(refused());
    }
    require_named(path, &control)?;
    writable.sync_all().map_err(io)?;
    require_named(path, &control)?;
    Ok(control)
}

/// Fresh creation is the ONLY content-write path. A cached native ID is a negative filter,
/// not an incarnation proof after its original File closed: a reopened object must already
/// contain the COMPLETE exact encrypted frame and receives only an idempotent sync. Even a
/// matching-ID/prefix partial is preserved/refused. Continuing it requires a separate actual
/// authenticated-seed exact-repair role, not implicit renewal of this warm creation permit.
fn write_same(
    path: &Path,
    expected: &[u8],
    recorded: &mut Option<SessionId>,
    #[cfg(test)] point: V2TextPromotionTestCut,
    #[cfg(test)] hooks: &mut PromotionTestHooks,
) -> Result<(), AppError> {
    let fresh = recorded.is_none();
    let mut file = if let Some(id) = recorded {
        if !matches!(session_at(path, *id), SessionAt::Ours) {
            return Err(refused());
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(io)?;
        if session_id(&file).map_err(io)? != *id {
            return Err(refused());
        }
        file
    } else {
        let created_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(io)?;
        // Cache the actual created object's identity BEFORE its first content write.
        // If native identity itself fails, preserve the unknown file; there is no adopter.
        *recorded = Some(session_id(&created_file).map_err(io)?);
        created_file
    };
    let id = recorded.ok_or_else(refused)?;
    #[cfg(test)]
    if fresh {
        hooks.hit(point)?;
        if point == V2TextPromotionTestCut::SeedCreated
            && hooks.take_fault(V2TextPromotionTestCut::SeedPartial)
        {
            file.write_all(&expected[..expected.len() / 2])
                .map_err(io)?;
            return Err(io("injected partial first text promotion seed write"));
        }
    }
    let metadata = file.metadata().map_err(io)?;
    if !metadata.file_type().is_file()
        || metadata.len() > expected.len() as u64
        || (!fresh && metadata.len() != expected.len() as u64)
    {
        return Err(refused());
    }
    file.seek(SeekFrom::Start(0)).map_err(io)?;
    let length = metadata.len() as usize;
    let mut used = 0;
    let mut buffer = [0; 8192];
    while used < length {
        let wanted = (length - used).min(buffer.len());
        match file.read(&mut buffer[..wanted]) {
            Ok(0) => return Err(refused()),
            Ok(n) => {
                if buffer[..n] != expected[used..used + n] {
                    return Err(refused());
                }
                used += n;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(io(error)),
        }
    }
    if !matches!(session_at(path, id), SessionAt::Ours) {
        return Err(refused());
    }
    if fresh {
        file.write_all(&expected[used..]).map_err(io)?;
    }
    file.sync_all().map_err(io)?;
    if !matches!(session_at(path, id), SessionAt::Ours) {
        return Err(refused());
    }
    Ok(())
}

pub(super) fn sync_parent(path: &Path, id: SessionId) -> Result<(), AppError> {
    // Keep the actual original directory alive through the barrier/name check. This File
    // and all control/helper Files drop before the caller releases its physical lease.
    let directory = require_directory(path, id)?;
    crate::store::persistence::sync_directory(path).map_err(io)?;
    crate::store::persistence::v2_genesis_activation::require_directory_name(path, &directory, id)
}

fn ensure_seed_main(
    target: &Path,
    target_id: SessionId,
    expected: &[u8],
    slot: &mut Option<SessionId>,
    #[cfg(test)] hooks: &mut PromotionTestHooks,
) -> Result<(), AppError> {
    let main = fixed_child(target, "text.seed")?;
    let stage = fixed_child(target, "text.seed.stage.v1")?;
    let _target = require_directory(target, target_id)?;
    if slot.is_some_and(|id| matches!(session_at(&main, id), SessionAt::Ours)) {
        require_absent(&stage)?;
        let main_file = sync_exact_control(&main, slot.ok_or_else(refused)?, expected)?;
        sync_parent(target, target_id)?;
        require_named(&main, &main_file)?;
        return Ok(());
    }
    require_absent(&main)?;
    write_same(
        &stage,
        expected,
        slot,
        #[cfg(test)]
        V2TextPromotionTestCut::SeedCreated,
        #[cfg(test)]
        hooks,
    )?;
    #[cfg(test)]
    hooks.hit(V2TextPromotionTestCut::SeedStaged)?;
    // Both the complete file AND pre-rename parent barrier precede dependent objects.
    sync_parent(target, target_id)?;
    let staged = require_exact(&stage, slot.ok_or_else(refused)?, expected)?;
    require_absent(&main)?;
    require_named(&stage, &staged)?;
    fs::rename(&stage, &main).map_err(io)?;
    #[cfg(test)]
    hooks.hit(V2TextPromotionTestCut::SeedRenamed)?;
    // The SAME ID in slot records the possible rename before its uncertain post-barrier.
    sync_parent(target, target_id)?;
    require_named(&main, &staged)?;
    require_absent(&stage)
}

fn require_promotion_workspace(
    promotion: &PreparedV2TextPromotion,
    guard: &V2TextCurrentGroupGuard<'_, '_>,
) -> Result<(), AppError> {
    require_exact_inventory(guard.directory_data(), &["pin.bin"], false)?;
    let target = &promotion.captured.target;
    let mut names = [""; 10];
    names[..6].copy_from_slice(&[
        "origin",
        "source.seed",
        "source.bin",
        "records.bin",
        "manifest.bin",
        "head.bin",
    ]);
    let mut count = 6;
    let seed_main = fixed_child(target, "text.seed")?;
    let seed_stage = fixed_child(target, "text.seed.stage.v1")?;
    match promotion.slots.seed {
        None => {
            require_absent(&seed_main)?;
            require_absent(&seed_stage)?;
        }
        Some(id) if matches!(session_at(&seed_main, id), SessionAt::Ours) => {
            require_absent(&seed_stage)?;
            names[count] = "text.seed";
            count += 1;
        }
        Some(id) if matches!(session_at(&seed_stage, id), SessionAt::Ours) => {
            require_absent(&seed_main)?;
            names[count] = "text.seed.stage.v1";
            count += 1;
        }
        Some(_) => return Err(refused()),
    }
    for (name, slot) in [
        ("text.pages", promotion.slots.pages),
        ("head.stage", promotion.slots.head),
    ] {
        let path = fixed_child(target, name)?;
        if let Some(id) = slot {
            if name == "text.pages" {
                require_directory(&path, id)?;
            } else if !matches!(session_at(&path, id), SessionAt::Ours) {
                return Err(refused());
            }
            names[count] = name;
            count += 1;
        } else {
            require_absent(&path)?;
        }
    }
    // WAL and unknown scratch cannot hide behind a successful original-head comparison.
    require_exact_inventory(guard.target_directory_data(), &names[..count], true)
}

impl PreparedV2TextPromotion {
    pub(in crate::store) fn validate_stage_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.resources.require_same_original(guard.resources())?;
        self.work.validate_prepaid_account()?;
        if self.saved.is_some() || self.seed.is_none() {
            return Err(refused());
        }
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        if original_context(guard, &key)? != self.captured.context {
            return Err(refused());
        }
        let (target, _, id) = guard.target_directory_data();
        if target != self.captured.target || id != self.captured.target_id {
            return Err(refused());
        }
        let path = fixed_child(target, "head.bin")?;
        let head = read_control(&path, self.captured.original_head.len())?;
        if head.frame != self.captured.original_head {
            return Err(refused());
        }
        require_promotion_workspace(self, guard)?;
        guard.validate_original_namespace()?;
        require_named(&path, &head)?;
        guard.validate_source_only()
    }

    /// Runs outside all host writers. The actual affine marker stays alive until every
    /// helper-local File and cleanup has drained on BOTH success and error. No File is a
    /// Prepared field, so returning Err cannot leak active IO into quiescent debt.
    pub(in crate::store) fn run_stage(
        &mut self,
        admitted: AdmittedV2TextPromotionStage,
    ) -> Result<(), AppError> {
        admitted.require_same_promotion(&self.observe_original())?;
        self.work.validate_prepaid_account()?;
        if self.saved.is_some() {
            return Err(refused());
        }
        let facts = self.stage_effects()?;
        // The complete actual outcome is cached before the marker can drop or the caller
        // can perform a fallible final Current check. No newer read replaces this result.
        self.ready = Some(ReadyV2TextPromotionSelection {
            original: self.observe_original(),
            facts: Arc::new(facts),
            work: self.work.clone(),
        });
        drop(admitted);
        Ok(())
    }

    fn stage_effects(&mut self) -> Result<PromotionReadyFacts, AppError> {
        let seed = self.seed.as_ref().ok_or_else(refused)?.frame_data();
        let archive = self.archive.as_ref().ok_or_else(refused)?;
        let directory = self.directory.as_ref().ok_or_else(refused)?;
        let head = self.head.as_ref().ok_or_else(refused)?.frame_data();
        let target = &self.captured.target;
        let target_id = self.captured.target_id;
        let _target = require_directory(target, target_id)?;
        // First complete MAIN barrier is mandatory before pages-directory/child creation.
        ensure_seed_main(
            target,
            target_id,
            seed,
            &mut self.slots.seed,
            #[cfg(test)]
            &mut self.hooks,
        )?;
        #[cfg(test)]
        self.hooks.hit(V2TextPromotionTestCut::SeedSaved)?;
        let pages = fixed_child(target, "text.pages")?;
        if self.slots.pages.is_none() {
            fs::create_dir(&pages).map_err(io)?;
            let directory = crate::store::persistence::open_archive_directory(&pages)?;
            self.slots.pages = Some(session_id(&directory).map_err(io)?);
            sync_parent(target, target_id)?;
        }
        let pages_id = self.slots.pages.ok_or_else(refused)?;
        let _pages = require_directory(&pages, pages_id)?;
        let archive_path = archive_child(&pages, archive.id_data().as_bytes())?;
        write_same(
            &archive_path,
            archive.frame_data(),
            &mut self.slots.archive,
            #[cfg(test)]
            V2TextPromotionTestCut::ArchiveCreated,
            #[cfg(test)]
            &mut self.hooks,
        )?;
        let directory_path = page_child(&pages, directory.id().as_bytes())?;
        write_same(
            &directory_path,
            directory.as_bytes(),
            &mut self.slots.directory,
            #[cfg(test)]
            V2TextPromotionTestCut::DirectoryCreated,
            #[cfg(test)]
            &mut self.hooks,
        )?;
        #[cfg(test)]
        self.hooks.hit(V2TextPromotionTestCut::DirectoryWritten)?;
        sync_parent(&pages, pages_id)?;
        let stage = fixed_child(target, "head.stage")?;
        write_same(
            &stage,
            head,
            &mut self.slots.head,
            #[cfg(test)]
            V2TextPromotionTestCut::HeadCreated,
            #[cfg(test)]
            &mut self.hooks,
        )?;
        #[cfg(test)]
        self.hooks.hit(V2TextPromotionTestCut::HeadStaged)?;
        sync_parent(target, target_id)?;
        require_exact(
            &archive_path,
            self.slots.archive.ok_or_else(refused)?,
            archive.frame_data(),
        )?;
        require_exact(
            &directory_path,
            self.slots.directory.ok_or_else(refused)?,
            directory.as_bytes(),
        )?;
        require_exact(&stage, self.slots.head.ok_or_else(refused)?, head)?;
        let mut target_owned = PathBuf::with_capacity(target.as_os_str().len());
        target_owned.push(target);
        Ok(PromotionReadyFacts {
            context: self.captured.context.clone(),
            target: target_owned,
            target_id,
            slots: self.slots,
            original_head: Zeroizing::new(self.captured.original_head.to_vec()),
            head: Zeroizing::new(head.to_vec()),
            seed: Zeroizing::new(seed.to_vec()),
            archive: Zeroizing::new(archive.frame_data().to_vec()),
            archive_id: *archive.id_data().as_bytes(),
            directory: Zeroizing::new(directory.as_bytes().to_vec()),
            directory_id: *directory.id().as_bytes(),
            #[cfg(all(test, windows))]
            _drop_notice: PromotionFactsDropNotice(self.facts_drop_notice.take()),
        })
    }

    /// Distinct fresh commit admission, never permission inherited from successful staging.
    /// Ambiguous post-rename retry classifies only this exact staged native object at O_G;
    /// it does not rerun preparation, reseal, replace another head or call any signing key.
    pub(in crate::store) fn run_commit(
        &mut self,
        admitted: AdmittedV2TextPromotionCommit,
    ) -> Result<(), AppError> {
        admitted.require_same_promotion(&self.observe_original())?;
        self.work.validate_prepaid_account()?;
        let ready = self.ready.as_ref().ok_or_else(refused)?;
        commit_effects(
            &ready.facts,
            #[cfg(test)]
            &mut self.hooks,
        )?;
        if self.saved.is_none() {
            self.saved = Some(Arc::new(SavedV2TextPromotion {
                original: self.observe_original(),
                head: TextHeadIdData::of_data(&ready.facts.head),
                facts: ready.facts.clone(),
            }));
        }
        #[cfg(test)]
        self.hooks.hit(V2TextPromotionTestCut::SavedCached)?;
        drop(admitted);
        Ok(())
    }
}

fn commit_effects(
    facts: &PromotionReadyFacts,
    #[cfg(test)] hooks: &mut PromotionTestHooks,
) -> Result<(), AppError> {
    let _target = require_directory(&facts.target, facts.target_id)?;
    let stage_path = fixed_child(&facts.target, "head.stage")?;
    let head_path = fixed_child(&facts.target, "head.bin")?;
    let staged_id = facts.slots.head.ok_or_else(refused)?;
    if matches!(session_at(&head_path, staged_id), SessionAt::Ours) {
        require_absent(&stage_path)?;
        let selected = sync_exact_control(&head_path, staged_id, &facts.head)?;
        sync_parent(&facts.target, facts.target_id)?;
        require_named(&head_path, &selected)?;
        return Ok(());
    }
    let parent = read_control(&head_path, facts.original_head.len())?;
    if parent.frame != facts.original_head {
        return Err(refused());
    }
    let staged = sync_exact_control(&stage_path, staged_id, &facts.head)?;
    sync_parent(&facts.target, facts.target_id)?;
    require_named(&head_path, &parent)?;
    require_named(&stage_path, &staged)?;
    fs::rename(&stage_path, &head_path).map_err(io)?;
    #[cfg(test)]
    hooks.hit(V2TextPromotionTestCut::HeadRenamed)?;
    sync_parent(&facts.target, facts.target_id)?;
    require_named(&head_path, &staged)?;
    require_absent(&stage_path)
}

impl ReadyV2TextPromotionSelection {
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextPromotionObservation {
        self.original.clone()
    }
    pub(in crate::store) fn validate_ready_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.original
            .resources
            .require_same_original(guard.resources())?;
        self.work.validate_prepaid_account()?;
        self.original
            .resources
            .validate_original(guard.store(), guard.source())?;
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        if original_context(guard, &key)? != self.facts.context {
            return Err(refused());
        }
        let (target, _, id) = guard.target_directory_data();
        if target != self.facts.target || id != self.facts.target_id {
            return Err(refused());
        }
        let files = inspect_ready_content(&self.facts)?;
        require_exact_inventory(guard.directory_data(), &["pin.bin"], false)?;
        let target_names = [
            "origin",
            "source.seed",
            "source.bin",
            "records.bin",
            "manifest.bin",
            "head.bin",
            "text.seed",
            "text.pages",
            "head.stage",
        ];
        require_exact_inventory(
            guard.target_directory_data(),
            &target_names[..if files.parent.is_some() { 9 } else { 8 }],
            true,
        )?;
        guard.validate_original_namespace()?;
        // Repeat only held/named objects after source/ID3. All temporary Files stay local
        // and drop before this guard or any effect admission can escape its callback.
        files.require_names(&self.facts)?;
        guard.validate_source_only()
    }
}

impl SavedV2TextPromotion {
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.original.resources
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextPromotionObservation {
        self.original.clone()
    }
    pub(in crate::store) fn validate_selected_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.original
            .resources
            .require_same_original(guard.resources())?;
        self.original
            .resources
            .validate_original(guard.store(), guard.source())?;
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        if original_context(guard, &key)? != self.facts.context
            || self.head != TextHeadIdData::of_data(&self.facts.head)
        {
            return Err(refused());
        }
        let (target, _, id) = guard.target_directory_data();
        if target != self.facts.target || id != self.facts.target_id {
            return Err(refused());
        }
        let files = inspect_ready_content(&self.facts)?;
        // A Saved promotion requires the actual new O_G name, never only its complete stage.
        if files.parent.is_some() {
            return Err(refused());
        }
        guard.validate_original_namespace()?;
        files.require_names(&self.facts)?;
        guard.validate_source_only()
    }
    pub(in crate::store) fn head_id_data(&self) -> TextHeadIdData {
        self.head
    }
}

/// Temporary paid originals, kept through the final source/ID3 proof. All path allocation,
/// full-byte comparisons and directory walks precede that proof. Only held-name predicates
/// and cached negatives follow it, including the actual current O/predecessor File.
struct ReadyReadFiles {
    target: File,
    pages: File,
    pages_path: PathBuf,
    seed: OpenedTextControl,
    seed_path: PathBuf,
    seed_stage_path: PathBuf,
    archive: OpenedTextControl,
    archive_path: PathBuf,
    directory: OpenedTextControl,
    directory_path: PathBuf,
    selected_or_staged: OpenedTextControl,
    selected_or_staged_path: PathBuf,
    parent: Option<OpenedTextControl>,
    head_path: PathBuf,
    head_stage_path: PathBuf,
    wal_path: PathBuf,
    wal_stage_path: PathBuf,
}

fn inspect_ready_content(facts: &PromotionReadyFacts) -> Result<ReadyReadFiles, AppError> {
    let target = require_directory(&facts.target, facts.target_id)?;
    let pages_path = fixed_child(&facts.target, "text.pages")?;
    let pages = require_directory(&pages_path, facts.slots.pages.ok_or_else(refused)?)?;
    let seed_path = fixed_child(&facts.target, "text.seed")?;
    let seed = require_exact(
        &seed_path,
        facts.slots.seed.ok_or_else(refused)?,
        &facts.seed,
    )?;
    let seed_stage_path = fixed_child(&facts.target, "text.seed.stage.v1")?;
    require_absent(&seed_stage_path)?;
    let archive_path = archive_child(&pages_path, &facts.archive_id)?;
    let archive = require_exact(
        &archive_path,
        facts.slots.archive.ok_or_else(refused)?,
        &facts.archive,
    )?;
    let directory_path = page_child(&pages_path, &facts.directory_id)?;
    let directory = require_exact(
        &directory_path,
        facts.slots.directory.ok_or_else(refused)?,
        &facts.directory,
    )?;
    let head_path = fixed_child(&facts.target, "head.bin")?;
    let head_stage_path = fixed_child(&facts.target, "head.stage")?;
    let staged_id = facts.slots.head.ok_or_else(refused)?;
    let (selected_or_staged, selected_or_staged_path, parent) =
        if matches!(session_at(&head_path, staged_id), SessionAt::Ours) {
            require_absent(&head_stage_path)?;
            (
                require_exact(&head_path, staged_id, &facts.head)?,
                head_path.clone(),
                None,
            )
        } else {
            let parent = read_control(&head_path, facts.original_head.len())?;
            if parent.frame != facts.original_head {
                return Err(refused());
            }
            (
                require_exact(&head_stage_path, staged_id, &facts.head)?,
                head_stage_path.clone(),
                Some(parent),
            )
        };
    // New work cannot appear during an unsigned promotion's commit admission.
    let wal_path = fixed_child(&facts.target, "text.wal")?;
    let wal_stage_path = fixed_child(&facts.target, "text.wal.stage.v1")?;
    require_absent(&wal_path)?;
    require_absent(&wal_stage_path)?;
    Ok(ReadyReadFiles {
        target,
        pages,
        pages_path,
        seed,
        seed_path,
        seed_stage_path,
        archive,
        archive_path,
        directory,
        directory_path,
        selected_or_staged,
        selected_or_staged_path,
        parent,
        head_path,
        head_stage_path,
        wal_path,
        wal_stage_path,
    })
}

impl ReadyReadFiles {
    fn require_names(&self, facts: &PromotionReadyFacts) -> Result<(), AppError> {
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &facts.target,
            &self.target,
            facts.target_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &self.pages_path,
            &self.pages,
            facts.slots.pages.ok_or_else(refused)?,
        )?;
        for (path, control) in [
            (&self.seed_path, &self.seed),
            (&self.archive_path, &self.archive),
            (&self.directory_path, &self.directory),
            (&self.selected_or_staged_path, &self.selected_or_staged),
        ] {
            require_named(path, control)?;
        }
        if let Some(parent) = &self.parent {
            require_named(&self.head_path, parent)?;
        } else {
            require_absent(&self.head_stage_path)?;
        }
        require_absent(&self.seed_stage_path)?;
        require_absent(&self.wal_path)?;
        require_absent(&self.wal_stage_path)
    }
}

#[cfg(test)]
mod tests;
