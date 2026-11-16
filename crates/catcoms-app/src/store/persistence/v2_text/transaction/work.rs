//! Exact finite first-A work. The consumed Factory marker owns the real same-store OS
//! lease and canonical control-write end until EVERY local File and cleanup has drained.
//! Between calls the Cell keeps only original frames, bounded native labels and DATA.
//! A reopened partial is preserved/refused; cold authenticated-WAL repair is a distinct
//! role, not an implicit renewal of this warm create-new permit.
use super::*;
use catcoms_storage::kept::{session_at, SessionAt};
use std::fs;

mod completed;
mod completed_continuation;
pub(super) mod io;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum V2TextRetainTestCut {
    WalCreated,
    WalStaged,
    WalRenamed,
    WalSaved,
    ArtifactCreated,
    ArtifactsSaved,
    HeadCreated,
    HeadStaged,
    ReadyCached,
    HeadRenamed,
    SavedCached,
}
#[cfg(test)]
#[derive(Default)]
pub(super) struct RetainTestHooks {
    fault: Option<V2TextRetainTestCut>,
    pause: Option<(V2TextRetainTestCut, Box<dyn FnOnce() + Send>)>,
}
#[cfg(test)]
impl RetainTestHooks {
    fn hit(&mut self, point: V2TextRetainTestCut) -> Result<(), AppError> {
        if self.pause.as_ref().is_some_and(|(cut, _)| *cut == point) {
            let (_, callback) = self.pause.take().expect("matched exact finite IO cut");
            callback();
        }
        if self.fault == Some(point) {
            self.fault = None;
            return Err(AppError::Io("injected retained-A IO failure".into()));
        }
        Ok(())
    }
}

/// Labels are negative filters only. After their original handle closes they do not
/// prove incarnation or permit partial append. Exact COMPLETE bytes are checked on reopen.
pub(super) struct RetainSlots {
    wal: Option<SessionId>,
    artifacts: [Option<SessionId>; MAX_A_ARTIFACTS],
    head: Option<SessionId>,
}
impl RetainSlots {
    pub(super) fn wal_native_data(&self) -> Option<SessionId> {
        self.wal
    }
    pub(super) fn no_dependents_data(&self) -> bool {
        self.head.is_none() && self.artifacts.iter().all(Option::is_none)
    }
    pub(super) fn clear_wal_scratch_data(&mut self) -> Result<(), AppError> {
        if !self.no_dependents_data() || self.wal.is_none() {
            return Err(refused());
        }
        self.wal = None;
        Ok(())
    }
    pub(super) fn empty() -> Self {
        Self {
            wal: None,
            artifacts: [None; MAX_A_ARTIFACTS],
            head: None,
        }
    }
}

#[derive(Clone, Copy)]
struct ArtifactFact {
    role: TextArtifactRoleData,
    id: [u8; 32],
    native: SessionId,
    len: usize,
}

/// Ready remains a paid active-work fact. Bounded admission checks metadata/names and
/// the small original/selected control bodies; the finite commit compares EVERY complete
/// WAL/artifact byte again before selector CAS. No large WAL scan runs under a writer.
pub(super) struct ReadyRetainFacts {
    pub(super) head: Zeroizing<Vec<u8>>,
    head_native: SessionId,
    wal_native: SessionId,
    wal_len: usize,
    artifacts: Vec<ArtifactFact>,
    // LAST: actual small original facts remain funded after the Cell is destroyed.
    pub(super) parent: Arc<RetainParent>,
}
impl ReadyRetainFacts {
    /// Bounded DATA returned only by the paired authenticated-WORK finite repair.
    /// This constructs no wrapper or permission; every File/barrier already drained.
    pub(super) fn from_recovery_data(
        parent: Arc<RetainParent>,
        head: &[u8],
        head_native: SessionId,
        wal_native: SessionId,
        wal_len: usize,
        outputs: &[(TextArtifactRoleData, [u8; 32], SessionId, usize)],
    ) -> Result<(Self, RetainSlots), AppError> {
        if head.len() > 1725 || outputs.len() > MAX_A_ARTIFACTS {
            return Err(refused());
        }
        let mut artifacts = Vec::with_capacity(MAX_A_ARTIFACTS);
        let mut slots = RetainSlots::empty();
        slots.wal = Some(wal_native);
        slots.head = Some(head_native);
        for (index, (role, id, native, len)) in outputs.iter().copied().enumerate() {
            artifacts.push(ArtifactFact {
                role,
                id,
                native,
                len,
            });
            slots.artifacts[index] = Some(native);
        }
        Ok((
            Self {
                head: Zeroizing::new(head.to_vec()),
                head_native,
                wal_native,
                wal_len,
                artifacts,
                parent,
            },
            slots,
        ))
    }
    /// Capture bounded paths and held objects before the caller's final original
    /// source/ID3 proof. These native labels are refusal filters, not an incarnation
    /// claim; the finite Commit still compares every full WAL/artifact before CAS.
    pub(super) fn capture_named_files<'guard>(
        &self,
        guard: &'guard V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<super::ready_read::ReadyNamedFiles<'guard>, AppError> {
        let pages = self.parent.pages()?;
        let mut held = super::ready_read::ReadyNamedFiles::capture(
            guard,
            &self.parent.target,
            self.parent.target_id,
            pages,
            self.parent.pages_id,
        )?;
        held.add_file(
            owned_path(&self.parent.wal_path)?,
            self.wal_native,
            self.wal_len,
        )?;
        held.add_staged_head(
            &self.parent.target,
            &self.parent.head_path,
            self.head_native,
            self.head.len(),
        )?;
        for fact in &self.artifacts {
            held.add_file(
                io::artifact_child(pages, fact.role, &fact.id)?,
                fact.native,
                fact.len,
            )?;
        }
        Ok(held)
    }
}

fn native_error(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

/// Only the small currently referenced first-A closure escapes into Saved. Exact full
/// authenticated output bytes were checked in the original verifier and again by commit;
/// these copies permit a fresh bounded actual H_A/T_A source proof without retaining WA,
/// action/cipher buffers, the original SDK packet, the16MiB worker or any obsolete File.
pub(super) struct SavedRetainFacts {
    archive_path: PathBuf,
    archive: Zeroizing<Vec<u8>>,
    archive_native: SessionId,
    token_path: PathBuf,
    token: Zeroizing<Vec<u8>>,
    token_native: SessionId,
    #[cfg(all(test, windows))]
    drop_notice: SavedFactsDropNotice,
}
impl SavedRetainFacts {
    /// Borrowed DATA only, used by the sealed same-Cell accepted-query source issuer.
    pub(super) fn borrow_query_data(
        &self,
    ) -> super::accepted_source::SavedRetainQueryFactsData<'_> {
        super::accepted_source::SavedRetainQueryFactsData {
            archive_path: &self.archive_path,
            archive: &self.archive,
            archive_native: self.archive_native,
            token: &self.token,
        }
    }
    #[cfg(all(test, windows))]
    pub(super) fn paths_data(&self) -> (&Path, &Path) {
        (&self.archive_path, &self.token_path)
    }
    pub(super) fn validate_selected(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        parent: &RetainParent,
        selected: &[u8],
    ) -> Result<(), AppError> {
        let pages_path = parent.pages()?;
        let target = io::require_directory(&parent.target, parent.target_id)?;
        let pages = io::require_directory(pages_path, parent.pages_id)?;
        let actual = io::require_exact(&parent.head_path, None, selected)?;
        let archive =
            io::require_exact(&self.archive_path, Some(self.archive_native), &self.archive)?;
        let token = io::require_exact(&self.token_path, Some(self.token_native), &self.token)?;
        guard.validate_original_namespace()?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            pages_path,
            &pages,
            parent.pages_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &parent.target,
            &target,
            parent.target_id,
        )?;
        require_named(&self.archive_path, &archive)?;
        require_named(&self.token_path, &token)?;
        require_named(&parent.head_path, &actual)?;
        guard.validate_source_only()
    }
}
#[cfg(all(test, windows))]
struct SavedFactsDropNotice(
    Option<(
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicBool>,
    )>,
);
#[cfg(all(test, windows))]
impl Drop for SavedFactsDropNotice {
    fn drop(&mut self) {
        if let Some((facts, fee)) = &self.0 {
            use std::sync::atomic::Ordering;
            assert!(
                !fee.load(Ordering::SeqCst),
                "actual transaction metadata refunded before selected A facts drained"
            );
            facts.store(true, Ordering::SeqCst);
        }
    }
}

impl PreparedV2TextRetainTransaction {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_pause_for_test(
        &mut self,
        cut: V2TextRetainTestCut,
        pause: Box<dyn FnOnce() + Send>,
    ) {
        self.hooks.pause = Some((cut, pause));
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_fault_for_test(&mut self, cut: V2TextRetainTestCut) {
        self.hooks.fault = Some(cut);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn observe_saved_facts_drop_for_test(
        &mut self,
        facts: Arc<std::sync::atomic::AtomicBool>,
        fee: Arc<std::sync::atomic::AtomicBool>,
    ) {
        assert!(self.saved.is_none());
        self.saved_drop_notice = Some((facts, fee));
    }

    /// Every operation below is covered by THIS pre-admitted same-store lease. Scope
    /// cancellation after admission does not strand an already admitted exact finite run.
    pub(in crate::store) fn run_stage(
        &mut self,
        admitted: AdmittedV2TextRetainStage,
    ) -> Result<(), AppError> {
        admitted.require_same_transaction(&self.observe_original())?;
        self.custody.work_resources().validate_prepaid_account()?;
        self.custody
            .metadata_resources()
            .validate_prepaid_account()?;
        if self.ready.is_some() {
            return Ok(());
        }
        // The temporary slots/hooks are DATA only; returning from the inner helper drains
        // all actual IO before its result or error is cached and before admitted drops.
        let mut slots = std::mem::replace(&mut self.slots, RetainSlots::empty());
        #[cfg(test)]
        let mut hooks = std::mem::take(&mut self.hooks);
        let result = self.stage_inner(
            &mut slots,
            #[cfg(test)]
            &mut hooks,
        );
        self.slots = slots;
        #[cfg(test)]
        {
            self.hooks = hooks;
        }
        let facts = result?;
        self.ready = Some(ReadyV2TextRetainSelection {
            original: self.observe_original(),
            facts: Arc::new(facts),
            work: Arc::clone(self.custody.work_resources()),
        });
        #[cfg(test)]
        self.hooks.hit(V2TextRetainTestCut::ReadyCached)?;
        // Explicitly keep the actual marker after ALL local IO/result cleanup.
        drop(admitted);
        Ok(())
    }

    fn stage_inner(
        &self,
        slots: &mut RetainSlots,
        #[cfg(test)] hooks: &mut RetainTestHooks,
    ) -> Result<ReadyRetainFacts, AppError> {
        let parent = &self.parent;
        let target = io::require_directory(&parent.target, parent.target_id)?;
        let pages_path = parent.pages()?;
        let pages = io::require_directory(pages_path, parent.pages_id)?;
        // Exact original O/H identity is a finite worker precondition as well as a fresh
        // Current admission proof. A foreign replacement is never adopted from an error.
        let old_o = io::require_exact(&parent.head_path, None, &parent.head_frame)?;
        let old_h = io::require_exact(&parent.archive_path, None, &parent.archive_frame)?;
        let wal = self.wal.as_ref().ok_or_else(refused)?;
        let candidate = self.candidate.as_ref().ok_or_else(refused)?;
        self.ensure_wal(
            slots,
            #[cfg(test)]
            hooks,
        )?;
        #[cfg(test)]
        hooks.hit(V2TextRetainTestCut::WalSaved)?;

        let artifacts = self.artifacts()?;
        let mut facts = Vec::with_capacity(MAX_A_ARTIFACTS);
        for (index, artifact) in artifacts.iter().copied().enumerate() {
            if index >= MAX_A_ARTIFACTS {
                return Err(refused());
            }
            let path = io::artifact_child(pages_path, artifact.role_data(), &artifact.id_data())?;
            io::write_complete_only(
                &path,
                artifact.frame_data(),
                &mut slots.artifacts[index],
                || {
                    #[cfg(test)]
                    hooks.hit(V2TextRetainTestCut::ArtifactCreated)?;
                    Ok(())
                },
            )?;
            facts.push(ArtifactFact {
                role: artifact.role_data(),
                id: artifact.id_data(),
                native: slots.artifacts[index].ok_or_else(refused)?,
                len: artifact.frame_data().len(),
            });
        }
        io::sync_parent(pages_path, parent.pages_id)?;
        #[cfg(test)]
        hooks.hit(V2TextRetainTestCut::ArtifactsSaved)?;
        // ALL immutable objects and their parent barrier precede the selected candidate.
        let head_stage = fixed_child(&parent.target, "head.stage")?;
        io::write_complete_only(&head_stage, candidate.frame_data(), &mut slots.head, || {
            #[cfg(test)]
            hooks.hit(V2TextRetainTestCut::HeadCreated)?;
            Ok(())
        })?;
        io::sync_parent(&parent.target, parent.target_id)?;
        #[cfg(test)]
        hooks.hit(V2TextRetainTestCut::HeadStaged)?;
        require_named(&parent.head_path, &old_o)?;
        require_named(&parent.archive_path, &old_h)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            pages_path,
            &pages,
            parent.pages_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &parent.target,
            &target,
            parent.target_id,
        )?;
        Ok(ReadyRetainFacts {
            head: Zeroizing::new(candidate.frame_data().to_vec()),
            head_native: slots.head.ok_or_else(refused)?,
            wal_native: slots.wal.ok_or_else(refused)?,
            wal_len: wal.frame_data().len(),
            artifacts: facts,
            parent: Arc::clone(parent),
        })
    }

    fn ensure_wal(
        &self,
        slots: &mut RetainSlots,
        #[cfg(test)] hooks: &mut RetainTestHooks,
    ) -> Result<(), AppError> {
        if self.parent.completed.is_some() {
            return self.ensure_completed_wal(
                slots,
                #[cfg(test)]
                hooks,
            );
        }
        let parent = &self.parent;
        let expected = self.wal.as_ref().ok_or_else(refused)?.frame_data();
        if slots
            .wal
            .is_some_and(|id| matches!(session_at(&parent.wal_path, id), SessionAt::Ours))
        {
            io::require_absent(&parent.wal_stage_path)?;
            let main = io::sync_exact(&parent.wal_path, slots.wal, expected)?;
            io::sync_parent(&parent.target, parent.target_id)?;
            return require_named(&parent.wal_path, &main);
        }
        io::require_absent(&parent.wal_path)?;
        io::write_complete_only(&parent.wal_stage_path, expected, &mut slots.wal, || {
            #[cfg(test)]
            hooks.hit(V2TextRetainTestCut::WalCreated)?;
            Ok(())
        })?;
        #[cfg(test)]
        hooks.hit(V2TextRetainTestCut::WalStaged)?;
        // Full WAL file and pre-rename barrier must exist before ANY dependent output.
        io::sync_parent(&parent.target, parent.target_id)?;
        let staged = io::require_exact(&parent.wal_stage_path, slots.wal, expected)?;
        io::require_absent(&parent.wal_path)?;
        require_named(&parent.wal_stage_path, &staged)?;
        fs::rename(&parent.wal_stage_path, &parent.wal_path).map_err(native_error)?;
        #[cfg(test)]
        hooks.hit(V2TextRetainTestCut::WalRenamed)?;
        // slots.wal already records the ambiguous rename before the post-parent barrier.
        io::sync_parent(&parent.target, parent.target_id)?;
        require_named(&parent.wal_path, &staged)?;
        io::require_absent(&parent.wal_stage_path)
    }

    pub(in crate::store) fn run_commit(
        &mut self,
        admitted: AdmittedV2TextRetainCommit,
    ) -> Result<(), AppError> {
        admitted.require_same_transaction(&self.observe_original())?;
        self.custody.work_resources().validate_prepaid_account()?;
        self.custody
            .metadata_resources()
            .validate_prepaid_account()?;
        if self.saved.is_some() {
            return Ok(());
        }
        // Assemble the small funded Saved DATA before any selector effect. All fallible
        // allocation/derivation then precedes the rename, and no SDK original escapes it.
        let selected = self.selected_facts()?;
        let selected_frame = Zeroizing::new(
            self.candidate
                .as_ref()
                .ok_or_else(refused)?
                .frame_data()
                .to_vec(),
        );
        #[cfg(test)]
        let mut hooks = std::mem::take(&mut self.hooks);
        let result = self.commit_inner(
            #[cfg(test)]
            &mut hooks,
        );
        #[cfg(test)]
        {
            self.hooks = hooks;
        }
        // commit_inner owns only borrowed original data and finite local Files; it always
        // drains them before the SAME admitted OSlease/write gate can be released.
        result?;
        #[cfg(all(test, windows))]
        let selected = {
            let mut selected = selected;
            selected.drop_notice.0 = self.saved_drop_notice.take();
            selected
        };
        let candidate = self.candidate.as_ref().ok_or_else(refused)?;
        self.saved = Some(Arc::new(SavedV2RetainedAction {
            head: TextHeadIdData::from_bytes(*candidate.id_data().as_bytes()),
            selected: selected_frame,
            selected_facts: selected,
            original: self.observe_original(),
            parent: Arc::clone(&self.parent),
        }));
        #[cfg(test)]
        self.hooks.hit(V2TextRetainTestCut::SavedCached)?;
        drop(admitted);
        Ok(())
    }

    fn selected_facts(&self) -> Result<SavedRetainFacts, AppError> {
        let artifacts = self.artifacts()?;
        let (archive_index, archive) = artifacts
            .iter()
            .copied()
            .enumerate()
            .find(|(_, a)| a.role_data() == TextArtifactRoleData::Archive)
            .ok_or_else(refused)?;
        let (token_index, token) = artifacts
            .iter()
            .copied()
            .enumerate()
            .find(|(_, a)| a.role_data() == TextArtifactRoleData::TokenIndex)
            .ok_or_else(refused)?;
        if archive.frame_data().len() > 1519 || token.frame_data().len() > 16470 {
            return Err(refused());
        }
        Ok(SavedRetainFacts {
            archive_path: io::artifact_child(
                self.parent.pages()?,
                archive.role_data(),
                &archive.id_data(),
            )?,
            archive: Zeroizing::new(archive.frame_data().to_vec()),
            archive_native: self.slots.artifacts[archive_index].ok_or_else(refused)?,
            token_path: io::artifact_child(
                self.parent.pages()?,
                token.role_data(),
                &token.id_data(),
            )?,
            token: Zeroizing::new(token.frame_data().to_vec()),
            token_native: self.slots.artifacts[token_index].ok_or_else(refused)?,
            #[cfg(all(test, windows))]
            drop_notice: SavedFactsDropNotice(None),
        })
    }

    fn commit_inner(&self, #[cfg(test)] hooks: &mut RetainTestHooks) -> Result<(), AppError> {
        let ready = self.ready.as_ref().ok_or_else(refused)?;
        ready
            .observe_original()
            .require_same_original(&self.observe_original())?;
        let parent = &self.parent;
        let target = io::require_directory(&parent.target, parent.target_id)?;
        let pages_path = parent.pages()?;
        let pages = io::require_directory(pages_path, parent.pages_id)?;
        let wal = io::sync_exact(
            &parent.wal_path,
            Some(ready.facts.wal_native),
            self.wal.as_ref().ok_or_else(refused)?.frame_data(),
        )?;
        io::require_absent(&parent.wal_stage_path)?;
        // Exact full expected bytes include all authentication/context/signature checks
        // performed by the original semantic WAL verifier, not just length/native labels.
        let artifacts = self.artifacts()?;
        if artifacts.len() != ready.facts.artifacts.len() {
            return Err(refused());
        }
        for (artifact, fact) in artifacts.iter().copied().zip(&ready.facts.artifacts) {
            if artifact.role_data() != fact.role
                || artifact.id_data() != fact.id
                || artifact.frame_data().len() != fact.len
            {
                return Err(refused());
            }
            let path = io::artifact_child(pages_path, fact.role, &fact.id)?;
            io::sync_exact(&path, Some(fact.native), artifact.frame_data())?;
        }
        io::sync_parent(pages_path, parent.pages_id)?;
        let old_h = io::require_exact(&parent.archive_path, None, &parent.archive_frame)?;
        let candidate = self.candidate.as_ref().ok_or_else(refused)?.frame_data();
        let head_stage = fixed_child(&parent.target, "head.stage")?;
        // Bounded actual bytes, rather than error-to-absence, select the two legitimate
        // states. Exact selected candidate permits same-plan lost-return completion.
        let current = read_control(&parent.head_path, 1725)?;
        if current.frame.as_slice() == parent.head_frame.as_slice() {
            let staged = io::sync_exact(&head_stage, Some(ready.facts.head_native), candidate)?;
            io::sync_parent(&parent.target, parent.target_id)?;
            require_named(&parent.head_path, &current)?;
            require_named(&head_stage, &staged)?;
            require_named(&parent.wal_path, &wal)?;
            require_named(&parent.archive_path, &old_h)?;
            // Cooperative private directory; the canonical actual write gate excludes
            // all participating control readers/writers through this native CAS edge.
            fs::rename(&head_stage, &parent.head_path).map_err(native_error)?;
            #[cfg(test)]
            hooks.hit(V2TextRetainTestCut::HeadRenamed)?;
            io::sync_parent(&parent.target, parent.target_id)?;
            require_named(&parent.head_path, &staged)?;
        } else if current.frame.as_slice() == candidate {
            io::require_absent(&head_stage)?;
            io::sync_exact(&parent.head_path, Some(ready.facts.head_native), candidate)?;
            io::sync_parent(&parent.target, parent.target_id)?;
        } else {
            return Err(refused());
        }
        io::require_absent(&head_stage)?;
        require_named(&parent.wal_path, &wal)?;
        require_named(&parent.archive_path, &old_h)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            pages_path,
            &pages,
            parent.pages_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &parent.target,
            &target,
            parent.target_id,
        )
    }
}
