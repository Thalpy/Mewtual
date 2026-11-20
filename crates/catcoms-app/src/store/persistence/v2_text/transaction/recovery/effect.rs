//! Distinct finite mutations after authenticated WORK/read classification and a fresh
//! actual Current admission. No ordinary Stage, historical Saved or closed native ID
//! permits these appends/unlink. Each original expected byte comes from the same WAL.
use super::*;
use catcoms_storage::kept::{session_at, session_id, SessionAt};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

#[cfg(all(test, windows))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum V2TextRetainRecoveryTestCut {
    RepairOutputOpened,
    RepairReadyCached,
    ScratchUnlinked,
    ScratchCompleted,
}
#[cfg(all(test, windows))]
#[derive(Default)]
pub(super) struct RecoveryTestHooks {
    fault: Option<V2TextRetainRecoveryTestCut>,
    pause: Option<(V2TextRetainRecoveryTestCut, Box<dyn FnOnce() + Send>)>,
}
#[cfg(all(test, windows))]
impl RecoveryTestHooks {
    fn hit(&mut self, point: V2TextRetainRecoveryTestCut) -> Result<(), AppError> {
        if self.pause.as_ref().is_some_and(|(p, _)| *p == point) {
            let (_, run) = self.pause.take().expect("actual cut matched");
            run();
        }
        if self.fault == Some(point) {
            self.fault = None;
            return Err(io::io("injected same-original retained recovery failure"));
        }
        Ok(())
    }
}
impl PreparedV2TextRetainRecovery {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_fault_for_test(&mut self, cut: V2TextRetainRecoveryTestCut) {
        self.hooks.fault = Some(cut);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_pause_for_test(
        &mut self,
        cut: V2TextRetainRecoveryTestCut,
        hook: Box<dyn FnOnce() + Send>,
    ) {
        self.hooks.pause = Some((cut, hook));
    }

    pub(in crate::store) fn run_repair(
        &mut self,
        admitted: AdmittedV2TextRetainRepair,
    ) -> Result<(), AppError> {
        admitted.require_same_recovery(&self.observe_original())?;
        self.plan.claim.work.validate_prepaid_account()?;
        self.plan.claim.metadata.validate_prepaid_account()?;
        if self.repaired.is_some() || self.installed {
            return Ok(());
        }
        #[cfg(all(test, windows))]
        let mut hooks = std::mem::take(&mut self.hooks);
        let result = self.repair_inner(
            #[cfg(all(test, windows))]
            &mut hooks,
        );
        #[cfg(all(test, windows))]
        {
            self.hooks = hooks;
        }
        let (facts, slots) = result?;
        self.repaired = Some((
            ReadyV2TextRetainSelection {
                original: self.plan.original.clone(),
                facts: Arc::new(facts),
                work: Arc::clone(&self.plan.claim.work),
            },
            slots,
        ));
        #[cfg(all(test, windows))]
        self.hooks
            .hit(V2TextRetainRecoveryTestCut::RepairReadyCached)?;
        drop(admitted);
        Ok(())
    }
    fn repair_inner(
        &self,
        #[cfg(all(test, windows))] hooks: &mut RecoveryTestHooks,
    ) -> Result<(work::ReadyRetainFacts, work::RetainSlots), AppError> {
        let ready = self.ready_repair()?;
        let p = &self.plan.parent;
        let target = native::require_directory(&p.target, p.target_id)?;
        let pages = native::require_directory(p.pages()?, p.pages_id)?;
        let old = native::require_exact(&p.head_path, None, &p.head_frame)?;
        let archive = native::require_exact(&p.archive_path, None, &p.archive_frame)?;
        // Full exact MAIN match is checked FIRST on every admitted mutation retry.
        let wal_file = native::sync_exact(&p.wal_path, Some(ready.native), &self.plan.expected)?;
        native::require_absent(&p.wal_stage_path)?;
        let plain = self.plan.plain.get().ok_or_else(refused)?;
        let TextWalFrameData::Work(wal) = TextWalFrameData::decode_data(plain)? else {
            return Err(refused());
        };
        let mut outputs = Vec::with_capacity(MAX_A_ARTIFACTS);
        for artifact in wal.artifacts.entries_data() {
            let artifact = artifact?;
            let path =
                native::artifact_child(p.pages()?, artifact.role_data(), &artifact.id_data())?;
            let native = complete_authenticated_output(&path, artifact.frame_data(), || {
                #[cfg(all(test, windows))]
                hooks.hit(V2TextRetainRecoveryTestCut::RepairOutputOpened)?;
                Ok(())
            })?;
            outputs.push((
                artifact.role_data(),
                artifact.id_data(),
                native,
                artifact.frame_data().len(),
            ));
        }
        native::sync_parent(p.pages()?, p.pages_id)?;
        let head_stage = fixed_child(&p.target, "head.stage")?;
        let head = complete_authenticated_output(&head_stage, wal.candidate_frame, || {
            #[cfg(all(test, windows))]
            hooks.hit(V2TextRetainRecoveryTestCut::RepairOutputOpened)?;
            Ok(())
        })?;
        native::sync_parent(&p.target, p.target_id)?;
        require_named(&p.head_path, &old)?;
        require_named(&p.archive_path, &archive)?;
        require_named(&p.wal_path, &wal_file)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            p.pages()?,
            &pages,
            p.pages_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &p.target,
            &target,
            p.target_id,
        )?;
        work::ReadyRetainFacts::from_recovery_data(
            Arc::clone(p),
            wal.candidate_frame,
            head,
            ready.native,
            self.plan.expected.len(),
            &outputs,
        )
    }

    pub(in crate::store) fn run_scratch(
        &mut self,
        admitted: AdmittedV2TextRetainScratch,
    ) -> Result<(), AppError> {
        admitted.require_same_recovery(&self.observe_original())?;
        self.plan.claim.work.validate_prepaid_account()?;
        self.plan.claim.metadata.validate_prepaid_account()?;
        if self.scratch_completed || self.installed {
            return Ok(());
        }
        #[cfg(all(test, windows))]
        let mut hooks = std::mem::take(&mut self.hooks);
        let result = self.scratch_inner(
            #[cfg(all(test, windows))]
            &mut hooks,
        );
        #[cfg(all(test, windows))]
        {
            self.hooks = hooks;
        }
        result?;
        self.scratch_completed = true;
        #[cfg(all(test, windows))]
        self.hooks
            .hit(V2TextRetainRecoveryTestCut::ScratchCompleted)?;
        drop(admitted);
        Ok(())
    }
    fn scratch_inner(
        &self,
        #[cfg(all(test, windows))] hooks: &mut RecoveryTestHooks,
    ) -> Result<(), AppError> {
        let ready = self.ready_scratch()?;
        let p = &self.plan.parent;
        let target = native::require_directory(&p.target, p.target_id)?;
        let pages = native::require_directory(p.pages()?, p.pages_id)?;
        let old = native::require_exact(&p.head_path, None, &p.head_frame)?;
        let archive = native::require_exact(&p.archive_path, None, &p.archive_frame)?;
        native::require_absent(&p.wal_path)?;
        self.plan.require_no_dependents()?;
        if self.plan.scratch_unlinked.load(Ordering::Acquire) {
            // Only an actual cached same-plan unlink permits this absent continuation.
            native::require_absent(&p.wal_stage_path)?;
        } else {
            if ready.len >= self.plan.expected.len() || Some(ready.native) != self.plan.wal_native {
                return Err(refused());
            }
            let staged = open_exact_prefix(
                &p.wal_stage_path,
                ready.native,
                &self.plan.expected[..ready.len],
            )?;
            require_same_name(&p.wal_stage_path, &staged, ready.native)?;
            native::require_absent(&p.wal_path)?;
            self.plan.require_no_dependents()?;
            require_named(&p.head_path, &old)?;
            require_named(&p.archive_path, &archive)?;
            fs::remove_file(&p.wal_stage_path).map_err(io::io)?;
            // Cache BEFORE post-unlink parent barrier or injected lost return. This is
            // actual effect history, not a caller flag or parsed-WAL disposal permission.
            self.plan.scratch_unlinked.store(true, Ordering::Release);
            #[cfg(all(test, windows))]
            hooks.hit(V2TextRetainRecoveryTestCut::ScratchUnlinked)?;
        }
        native::sync_parent(&p.target, p.target_id)?;
        native::require_absent(&p.wal_stage_path)?;
        require_named(&p.head_path, &old)?;
        require_named(&p.archive_path, &archive)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            p.pages()?,
            &pages,
            p.pages_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &p.target,
            &target,
            p.target_id,
        )
    }
    /// Cache actual physical outcomes into ONLY the same owning original Cell. This is
    /// outside host writers: old metadata drops here, never a File/key/SDK extraction.
    /// Cancellation after admission does not erase an already completed physical result.
    pub(in crate::store) fn install_into(
        &mut self,
        cell: &mut PreparedV2TextRetainTransaction,
    ) -> Result<bool, AppError> {
        self.plan
            .original
            .require_same_original(&cell.observe_original())?;
        self.plan.claim.metadata.validate_prepaid_account()?;
        self.plan.claim.work.validate_prepaid_account()?;
        if self.installed {
            return Ok(true);
        }
        if cell.saved.is_some() {
            return Err(refused());
        }
        if let Some((ready, _)) = &self.repaired {
            if cell.ready.is_some() {
                return Err(refused());
            }
            ready
                .observe_original()
                .require_same_original(&cell.observe_original())?;
            let (ready, slots) = self
                .repaired
                .take()
                .expect("same checked actual repaired result");
            cell.slots = slots;
            cell.ready = Some(ready);
            self.installed = true;
            return Ok(true);
        }
        if self.scratch_completed {
            if cell.ready.is_some()
                || cell.slots.wal_native_data() != self.plan.wal_native
                || !cell.slots.no_dependents_data()
            {
                return Err(refused());
            }
            cell.slots.clear_wal_scratch_data()?;
            self.installed = true;
            return Ok(true);
        }
        Ok(false)
    }
}

/// Distinct authenticated-WORK exact repair. Existing regular named objects are NOT
/// adopted as original incarnations: the FULL existing prefix must equal independently
/// authenticated intended bytes, then only the missing suffix is appended. No truncate,
/// selected head/MAIN update, re-sign, re-seal or caller path can reach this private helper.
/// As with the original finite transaction, the actual canonical writer gate and OS
/// session exclude participating writers in a cooperative private directory. This is
/// not a claim of atomic safety against an external process rewriting an already-held
/// file concurrently; all observed wrong/oversize prefixes are preserved without writes.
fn complete_authenticated_output(
    path: &Path,
    expected: &[u8],
    opened: impl FnOnce() -> Result<(), AppError>,
) -> Result<SessionId, AppError> {
    let cap = 16470
        .max(catcoms_replication::chat::cold_actions::MAX_ACTION_CHUNK_PAGE_BYTES + 86)
        .max(super::super::super::retain::MAX_CIPHER_CHUNK_PLAIN + 86);
    if expected.is_empty() || expected.len() > cap {
        return Err(refused());
    }
    let mut file = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !regular(&metadata) || metadata.len() > expected.len() as u64 {
                return Err(refused());
            }
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(io::io)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(io::io)?,
        Err(error) => return Err(io::io(error)),
    };
    let native = session_id(&file).map_err(io::io)?;
    opened()?;
    let metadata = file.metadata().map_err(io::io)?;
    if !regular(&metadata) || metadata.len() > expected.len() as u64 {
        return Err(refused());
    }
    let used = metadata.len() as usize;
    require_prefix_bytes(&mut file, &expected[..used])?;
    require_same_name(path, &file, native)?;
    file.seek(SeekFrom::End(0)).map_err(io::io)?;
    file.write_all(&expected[used..]).map_err(io::io)?;
    file.sync_all().map_err(io::io)?;
    require_prefix_bytes(&mut file, expected)?;
    require_same_name(path, &file, native)?;
    Ok(native)
}
fn open_exact_prefix(path: &Path, native: SessionId, expected: &[u8]) -> Result<File, AppError> {
    let metadata = fs::symlink_metadata(path).map_err(io::io)?;
    if !regular(&metadata) || metadata.len() != expected.len() as u64 {
        return Err(refused());
    }
    let mut file = File::open(path).map_err(io::io)?;
    require_prefix_bytes(&mut file, expected)?;
    require_same_name(path, &file, native)?;
    Ok(file)
}
fn require_prefix_bytes(file: &mut File, expected: &[u8]) -> Result<(), AppError> {
    let metadata = file.metadata().map_err(io::io)?;
    if !regular(&metadata) || metadata.len() != expected.len() as u64 {
        return Err(refused());
    }
    file.seek(SeekFrom::Start(0)).map_err(io::io)?;
    let mut used = 0;
    let mut buffer = [0; 8192];
    while used < expected.len() {
        let cap = (expected.len() - used).min(buffer.len());
        match file.read(&mut buffer[..cap]) {
            Ok(0) => return Err(refused()),
            Ok(n) if buffer[..n] == expected[used..used + n] => used += n,
            Ok(_) => return Err(refused()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io::io(e)),
        }
    }
    loop {
        match file.read(&mut buffer[..1]) {
            Ok(0) => break,
            Ok(_) => return Err(refused()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io::io(e)),
        }
    }
    Ok(())
}
fn require_same_name(path: &Path, file: &File, native: SessionId) -> Result<(), AppError> {
    if session_id(file).map_err(io::io)? != native
        || !matches!(session_at(path, native), SessionAt::Ours)
    {
        return Err(refused());
    }
    Ok(())
}
fn regular(metadata: &fs::Metadata) -> bool {
    let plain = metadata.file_type().is_file() && !metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        plain && metadata.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    {
        plain
    }
}
