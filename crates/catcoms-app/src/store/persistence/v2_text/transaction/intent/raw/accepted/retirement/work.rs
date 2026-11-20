//! Two DISTINCT finite physical runs. The first persists W_D before any unlink,
//! drains every removed handle and caches ReadyWC. Only a NEW Current admission
//! may then run W_C. Stop between them leaves honest durable deletion debt.
use super::super::super::super::super::work::io as native;
use super::data::*;
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextAcceptedRetirementCompletion, AdmittedV2TextAcceptedRetirementDeletion,
};
use std::fs;

#[cfg(all(test, windows))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum V2TextRetirementTestCut {
    DeletionCreated,
    DeletionStaged,
    DeletionRenamed,
    DeletionSaved,
    ObjectOpened,
    /// Actual name has been unlinked while the original File is STILL owned.
    ObjectUnlinkedHeld,
    ObjectUnlinked,
    PagesSynced,
    ReadyCached,
    CompletionCreated,
    CompletionStaged,
    CompletionRenamed,
    CompletionSaved,
}
#[cfg(all(test, windows))]
#[derive(Default)]
pub(super) struct RetirementTestHooks {
    fault: Option<V2TextRetirementTestCut>,
    partial: Option<(V2TextRetirementTestCut, usize)>,
    pause: Option<(V2TextRetirementTestCut, Box<dyn FnOnce() + Send>)>,
}
#[cfg(test)]
#[cfg(windows)]
impl RetirementTestHooks {
    fn hit_created(
        &mut self,
        cut: V2TextRetirementTestCut,
        path: &Path,
        expected: &[u8],
    ) -> Result<(), AppError> {
        self.hit(cut)?;
        if self.partial.as_ref().is_some_and(|(at, _)| *at == cut) {
            use std::io::Write;
            let (_, len) = self
                .partial
                .take()
                .expect("same exact retirement partial cut");
            if len >= expected.len() {
                return Err(refused());
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(failed)?;
            file.write_all(&expected[..len]).map_err(failed)?;
            file.sync_all().map_err(failed)?;
            return Err(failed("injected exact retirement partial creation"));
        }
        Ok(())
    }
    fn hit(&mut self, cut: V2TextRetirementTestCut) -> Result<(), AppError> {
        if self.pause.as_ref().is_some_and(|(at, _)| *at == cut) {
            self.pause.take().expect("same retirement cut").1();
        }
        if self.fault == Some(cut) {
            self.fault = None;
            return Err(failed("injected retirement cut"));
        }
        Ok(())
    }
}
impl PreparedV2TextAcceptedCreate {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_retirement_partial_write_fault_for_test(
        &mut self,
        cut: V2TextRetirementTestCut,
        len: usize,
    ) {
        assert!(matches!(
            cut,
            V2TextRetirementTestCut::DeletionCreated | V2TextRetirementTestCut::CompletionCreated
        ));
        self.retirement_work.hooks.partial = Some((cut, len));
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn original_retirement_output_for_test(
        &self,
        cut: V2TextRetirementTestCut,
    ) -> Result<(PathBuf, Zeroizing<Vec<u8>>), AppError> {
        let facts = self.retirement_work.facts.as_ref().ok_or_else(refused)?;
        let frame = match cut {
            V2TextRetirementTestCut::DeletionCreated => &facts.deletion,
            V2TextRetirementTestCut::CompletionCreated => &facts.completion,
            _ => return Err(refused()),
        };
        Ok((
            owned_path(&facts.stage_path)?,
            Zeroizing::new(frame.to_vec()),
        ))
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_retirement_fault_for_test(&mut self, cut: V2TextRetirementTestCut) {
        self.retirement_work.hooks.fault = Some(cut);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_retirement_pause_for_test(
        &mut self,
        cut: V2TextRetirementTestCut,
        run: Box<dyn FnOnce() + Send>,
    ) {
        self.retirement_work.hooks.pause = Some((cut, run));
    }
    fn validate_retirement_work_accounts(&self, facts: &RetirementFacts) -> Result<(), AppError> {
        facts
            .original
            .require_same_original(&self.observe_original())?;
        facts.metadata.validate_prepaid_account()?;
        self.signature
            .intent
            .retained
            .custody
            .work_resources()
            .validate_prepaid_account()?;
        self.signature.intent.unsigned.validate_accounts_only()?;
        facts
            .basis
            .require_same_original(self.accepted_final_basis_data()?)
    }
    pub(in crate::store) fn run_retirement_deletion(
        &mut self,
        admitted: AdmittedV2TextAcceptedRetirementDeletion,
    ) -> Result<(), AppError> {
        let facts = Arc::clone(self.retirement_work.facts.as_ref().ok_or_else(refused)?);
        admitted.require_same_retirement(&self.observe_original(), &facts.plan_id)?;
        self.validate_retirement_work_accounts(&facts)?;
        if self.retirement_work.ready_completion.is_some() {
            return Ok(());
        }
        #[cfg(all(test, windows))]
        let mut hooks = std::mem::take(&mut self.retirement_work.hooks);
        let result = (|| {
            // Cache durable WD BEFORE attempting even the first unlink. A failure
            // below preserves this original receipt and the exact outstanding Plan.
            if self.retirement_work.saved_deletion.is_none() {
                let native = ensure_deletion_journal(
                    self,
                    &facts,
                    #[cfg(all(test, windows))]
                    &mut hooks,
                )?;
                self.retirement_work.saved_deletion =
                    Some(Arc::new(SavedV2TextRetirementDeletion {
                        native,
                        facts: Arc::clone(&facts),
                    }));
                #[cfg(all(test, windows))]
                hooks.hit(V2TextRetirementTestCut::DeletionSaved)?;
            }
            let wd = self
                .retirement_work
                .saved_deletion
                .as_ref()
                .ok_or_else(refused)?;
            deletion_inner(
                &facts,
                wd.native,
                #[cfg(all(test, windows))]
                &mut hooks,
            )?;
            // deletion_inner returned only AFTER ALL local Files have closed. This
            // owning DATA has no open-unlinked inode, OS lease or permission hidden.
            self.retirement_work.ready_completion = Some(ReadyV2TextRetirementCompletion {
                deletion: Arc::clone(wd),
                completion_native: std::sync::OnceLock::new(),
                facts: Arc::clone(&facts),
            });
            #[cfg(all(test, windows))]
            hooks.hit(V2TextRetirementTestCut::ReadyCached)?;
            Ok(())
        })();
        #[cfg(all(test, windows))]
        {
            self.retirement_work.hooks = hooks;
        }
        // Every syscall/helper destructor above precedes the SAME marker's OS lease
        // and actual writer End. No cleanup IO runs in Prepared/Ready/Saved Drop.
        drop(admitted);
        result
    }
    pub(in crate::store) fn run_retirement_completion(
        &mut self,
        admitted: AdmittedV2TextAcceptedRetirementCompletion,
    ) -> Result<(), AppError> {
        let facts = Arc::clone(self.retirement_work.facts.as_ref().ok_or_else(refused)?);
        admitted.require_same_retirement(&self.observe_original(), &facts.plan_id)?;
        self.validate_retirement_work_accounts(&facts)?;
        if self.retirement_work.saved_completion.is_some() {
            return Ok(());
        }
        let ready = self
            .retirement_work
            .ready_completion
            .as_ref()
            .ok_or_else(refused)?;
        #[cfg(all(test, windows))]
        let mut hooks = std::mem::take(&mut self.retirement_work.hooks);
        let result = completion_inner(
            &facts,
            ready.deletion.native,
            &mut self.retirement_work.completion_native,
            #[cfg(all(test, windows))]
            &mut hooks,
        );
        // Cache a created native refusal filter even after a failed write/rename.
        // It does NOT authorize adoption: every retry still compares full originals.
        if let Some(id) = self.retirement_work.completion_native {
            if let Some(previous) = ready.completion_native.get() {
                if *previous != id {
                    return Err(refused());
                }
            } else {
                let _ = ready.completion_native.set(id);
            }
        }
        #[cfg(all(test, windows))]
        {
            self.retirement_work.hooks = hooks;
        }
        let native = result?;
        self.retirement_work.saved_completion =
            Some(Arc::new(SavedV2TextRetirementCompletion { native, facts }));
        #[cfg(all(test, windows))]
        self.retirement_work
            .hooks
            .hit(V2TextRetirementTestCut::CompletionSaved)?;
        drop(admitted);
        Ok(())
    }
}
fn ensure_deletion_journal(
    cell: &mut PreparedV2TextAcceptedCreate,
    facts: &RetirementFacts,
    #[cfg(all(test, windows))] hooks: &mut RetirementTestHooks,
) -> Result<SessionId, AppError> {
    let flat = &facts.selected.facts;
    let _target = native::require_directory(&flat.target, flat.target_id)?;
    let _pages = native::require_directory(&flat.pages, flat.pages_id)?;
    native::require_absent(&facts.head_stage)?;
    native::require_absent(&facts.seed_stage)?;
    let main = read_control(&facts.wal_path, FIRST_B_WORK_WAL_FRAME_BYTES)?;
    if main.frame.as_slice() == facts.deletion.as_slice() {
        let id = cell.retirement_work.deletion_native.ok_or_else(refused)?;
        if main.id != id {
            return Err(refused());
        }
        native::require_absent(&facts.stage_path)?;
        let exact = native::sync_exact(&facts.wal_path, Some(id), &facts.deletion)?;
        native::sync_parent(&flat.target, flat.target_id)?;
        require_named(&facts.wal_path, &exact)?;
        return Ok(id);
    }
    let ready = cell.ready.as_ref().ok_or_else(refused)?;
    if main.id != ready.wal_native
        || main.frame.as_slice() != cell.wal.as_ref().ok_or_else(refused)?.frame_data()
    {
        return Err(refused());
    }
    // Complete original WB is durably compared before staging WD. This grants no
    // permission by itself: only the consumed typed deletion marker starts this run.
    let previous = native::sync_exact(&facts.wal_path, Some(ready.wal_native), &main.frame)?;
    super::continuation::write_original(
        &facts.stage_path,
        &facts.deletion,
        &mut cell.retirement_work.deletion_native,
        || {
            #[cfg(all(test, windows))]
            hooks.hit_created(
                V2TextRetirementTestCut::DeletionCreated,
                &facts.stage_path,
                &facts.deletion,
            )?;
            Ok(())
        },
    )?;
    #[cfg(all(test, windows))]
    hooks.hit(V2TextRetirementTestCut::DeletionStaged)?;
    native::sync_parent(&flat.target, flat.target_id)?;
    let stage = native::require_exact(
        &facts.stage_path,
        cell.retirement_work.deletion_native,
        &facts.deletion,
    )?;
    require_named(&facts.wal_path, &previous)?;
    require_named(&facts.stage_path, &stage)?;
    fs::rename(&facts.stage_path, &facts.wal_path).map_err(failed)?;
    #[cfg(all(test, windows))]
    hooks.hit(V2TextRetirementTestCut::DeletionRenamed)?;
    native::sync_parent(&flat.target, flat.target_id)?;
    require_named(&facts.wal_path, &stage)?;
    native::require_absent(&facts.stage_path)?;
    Ok(stage.id)
}
/// Full NEW selected closure and FULL ownWD before the first deletion. Only the four
/// producer-authenticated private entries are eligible; no public history kind exists.
fn deletion_inner(
    facts: &RetirementFacts,
    wd_native: SessionId,
    #[cfg(all(test, windows))] hooks: &mut RetirementTestHooks,
) -> Result<(), AppError> {
    let flat = &facts.selected.facts;
    let target = native::require_directory(&flat.target, flat.target_id)?;
    let pages = native::require_directory(&flat.pages, flat.pages_id)?;
    let wal = native::sync_exact(&facts.wal_path, Some(wd_native), &facts.deletion)?;
    native::require_absent(&facts.stage_path)?;
    native::require_absent(&facts.head_stage)?;
    native::require_absent(&facts.seed_stage)?;
    let selected = native::require_exact(&flat.head_path, Some(flat.head_native), &flat.selected)?;
    let mut artifacts = Vec::with_capacity(flat.artifacts.len());
    for a in &flat.artifacts {
        artifacts.push(native::require_exact(&a.path, Some(a.native), &a.frame)?);
    }
    for object in &facts.retired {
        // Real absence is idempotent ONLY after this exact durable actual WD. An
        // unreadable/nonregular/torn/wrong object remains intact and returns Err.
        match fs::symlink_metadata(&object.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(failed(e)),
            Ok(_) => {
                let original = read_control(&object.path, object.entry.frame_bytes as usize)?;
                if original.frame.len() != object.entry.frame_bytes as usize {
                    return Err(refused());
                }
                let role = match object.entry.kind {
                    RetirementKindData::AcceptedIndex => TextArtifactRoleData::AcceptedIndex,
                    RetirementKindData::TokenIndex => TextArtifactRoleData::TokenIndex,
                    RetirementKindData::PublicationIndex => TextArtifactRoleData::PublicationIndex,
                    RetirementKindData::Intent => TextArtifactRoleData::Intent,
                    RetirementKindData::Signature => TextArtifactRoleData::Signature,
                };
                TextWalArtifactData::from_data(role, object.entry.id, &original.frame)?;
                #[cfg(all(test, windows))]
                hooks.hit(V2TextRetirementTestCut::ObjectOpened)?;
                // Actual cached immutable ID+frame size, no unique-incarnation claim.
                // Cooperative private-directory writer exclusion remains required.
                require_same_bytes(&original.file, &original.frame)?;
                require_named(&object.path, &original)?;
                require_same_bytes(&wal.file, &facts.deletion)?;
                require_named(&facts.wal_path, &wal)?;
                fs::remove_file(&object.path).map_err(failed)?;
                #[cfg(all(test, windows))]
                hooks.hit(V2TextRetirementTestCut::ObjectUnlinkedHeld)?;
                // Close the REAL unlinked File before the pages barrier/ReadyWC.
                drop(original);
                #[cfg(all(test, windows))]
                hooks.hit(V2TextRetirementTestCut::ObjectUnlinked)?;
                native::require_absent(&object.path)?;
            }
        }
    }
    native::sync_parent(&flat.pages, flat.pages_id)?;
    #[cfg(all(test, windows))]
    hooks.hit(V2TextRetirementTestCut::PagesSynced)?;
    for object in &facts.retired {
        native::require_absent(&object.path)?;
    }
    require_named(&facts.wal_path, &wal)?;
    require_named(&flat.head_path, &selected)?;
    for (a, held) in flat.artifacts.iter().zip(&artifacts) {
        require_named(&a.path, held)?;
    }
    crate::store::persistence::v2_genesis_activation::require_directory_name(
        &flat.pages,
        &pages,
        flat.pages_id,
    )?;
    crate::store::persistence::v2_genesis_activation::require_directory_name(
        &flat.target,
        &target,
        flat.target_id,
    )
}
fn completion_inner(
    facts: &RetirementFacts,
    wd_native: SessionId,
    recorded: &mut Option<SessionId>,
    #[cfg(all(test, windows))] hooks: &mut RetirementTestHooks,
) -> Result<SessionId, AppError> {
    let flat = &facts.selected.facts;
    let target = native::require_directory(&flat.target, flat.target_id)?;
    let pages = native::require_directory(&flat.pages, flat.pages_id)?;
    native::require_absent(&facts.head_stage)?;
    native::require_absent(&facts.seed_stage)?;
    for object in &facts.retired {
        native::require_absent(&object.path)?;
    }
    let selected = native::require_exact(&flat.head_path, Some(flat.head_native), &flat.selected)?;
    let mut artifacts = Vec::with_capacity(flat.artifacts.len());
    for a in &flat.artifacts {
        artifacts.push(native::require_exact(&a.path, Some(a.native), &a.frame)?);
    }
    let current = read_control(
        &facts.wal_path,
        crate::store::persistence::v2_text::wal::MAX_DELETION_WAL_PLAIN + 40,
    )?;
    if current.frame.as_slice() == facts.completion.as_slice() {
        let id = recorded.ok_or_else(refused)?;
        if current.id != id {
            return Err(refused());
        }
        native::require_absent(&facts.stage_path)?;
        drop(native::sync_exact(
            &facts.wal_path,
            Some(id),
            &facts.completion,
        )?);
        native::sync_parent(&flat.target, flat.target_id)?;
        require_named(&facts.wal_path, &current)?;
    } else {
        if current.id != wd_native || current.frame.as_slice() != facts.deletion.as_slice() {
            return Err(refused());
        }
        let wd = native::sync_exact(&facts.wal_path, Some(wd_native), &facts.deletion)?;
        native::sync_parent(&flat.pages, flat.pages_id)?;
        super::continuation::write_original(
            &facts.stage_path,
            &facts.completion,
            recorded,
            || {
                #[cfg(all(test, windows))]
                hooks.hit_created(
                    V2TextRetirementTestCut::CompletionCreated,
                    &facts.stage_path,
                    &facts.completion,
                )?;
                Ok(())
            },
        )?;
        #[cfg(all(test, windows))]
        hooks.hit(V2TextRetirementTestCut::CompletionStaged)?;
        native::sync_parent(&flat.target, flat.target_id)?;
        let stage = native::require_exact(&facts.stage_path, *recorded, &facts.completion)?;
        require_named(&facts.wal_path, &wd)?;
        require_named(&facts.stage_path, &stage)?;
        fs::rename(&facts.stage_path, &facts.wal_path).map_err(failed)?;
        #[cfg(all(test, windows))]
        hooks.hit(V2TextRetirementTestCut::CompletionRenamed)?;
        native::sync_parent(&flat.target, flat.target_id)?;
        require_named(&facts.wal_path, &stage)?;
    }
    native::require_absent(&facts.stage_path)?;
    for object in &facts.retired {
        native::require_absent(&object.path)?;
    }
    require_named(&flat.head_path, &selected)?;
    for (a, held) in flat.artifacts.iter().zip(&artifacts) {
        require_named(&a.path, held)?;
    }
    crate::store::persistence::v2_genesis_activation::require_directory_name(
        &flat.pages,
        &pages,
        flat.pages_id,
    )?;
    crate::store::persistence::v2_genesis_activation::require_directory_name(
        &flat.target,
        &target,
        flat.target_id,
    )?;
    recorded.ok_or_else(refused)
}
fn failed(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

/// Bounded SAME held-File recheck after the real IO hook and before unlink. The
/// complete original was authenticated above; no stale in-memory copy can hide a
/// same-native/same-length mutation while a deterministic fault test parks the run.
fn require_same_bytes(file: &std::fs::File, expected: &[u8]) -> Result<(), AppError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut stream = file;
    stream.seek(SeekFrom::Start(0)).map_err(failed)?;
    let mut at = 0;
    let mut buffer = [0; 8192];
    while at < expected.len() {
        let cap = (expected.len() - at).min(buffer.len());
        match stream.read(&mut buffer[..cap]) {
            Ok(0) => return Err(refused()),
            Ok(n) if buffer[..n] == expected[at..at + n] => at += n,
            Ok(_) => return Err(refused()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(failed(e)),
        }
    }
    loop {
        match stream.read(&mut buffer[..1]) {
            Ok(0) => return Ok(()),
            Ok(_) => return Err(refused()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(failed(e)),
        }
    }
}
