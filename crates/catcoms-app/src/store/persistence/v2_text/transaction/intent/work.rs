//! Finite first-I IO under the distinct SAME-store Stage/Commit markers. Every helper
//! File is local to one call, including uncertain rename/barrier errors. No Saved/Ready
//! retains an obsolete O/WAL File. Scope stop after admission cannot erase cached work.
//! SAME-original Stage may complete a cached-created exact prefix under warm.rs's
//! narrow contract; general A/cold/unknown-file work stays complete-only.
use super::super::work::io as native;
use super::*;
use catcoms_storage::kept::{session_at, SessionAt};
use std::fs;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum V2TextIntentTestCut {
    WalCreated,
    WalStaged,
    WalReplaced,
    WalSaved,
    IntentCreated,
    IntentSaved,
    HeadCreated,
    HeadStaged,
    ReadyCached,
    HeadRenamed,
    SavedCached,
}
#[cfg(test)]
#[derive(Default)]
pub(super) struct IntentTestHooks {
    fault: Option<V2TextIntentTestCut>,
    partial: Option<(V2TextIntentTestCut, usize)>,
    pause: Option<(V2TextIntentTestCut, Box<dyn FnOnce() + Send>)>,
}
#[cfg(test)]
impl IntentTestHooks {
    /// Inject an actual proper-prefix write at the real create-new seam, while the
    /// original worker File/native label and consumed Stage lease/End are still held.
    /// Tests then retry the SAME frozen candidate through a new genuine Stage.
    fn hit_created(
        &mut self,
        point: V2TextIntentTestCut,
        path: &Path,
        expected: &[u8],
    ) -> Result<(), AppError> {
        self.hit(point)?;
        if self.partial.as_ref().is_some_and(|(cut, _)| *cut == point) {
            use std::io::Write;
            let (_, len) = self.partial.take().expect("actual prefix fault");
            if len >= expected.len() {
                return Err(refused());
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(failed)?;
            file.write_all(&expected[..len]).map_err(failed)?;
            file.sync_all().map_err(failed)?;
            return Err(failed("injected original I partial write"));
        }
        Ok(())
    }
    fn hit(&mut self, point: V2TextIntentTestCut) -> Result<(), AppError> {
        if self.pause.as_ref().is_some_and(|(cut, _)| *cut == point) {
            let (_, run) = self.pause.take().expect("actual I IO cut");
            run();
        }
        if self.fault == Some(point) {
            self.fault = None;
            return Err(failed("injected unsigned-I IO failure"));
        }
        Ok(())
    }
}
fn failed(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

pub(super) struct IntentSlots {
    wal: Option<SessionId>,
    intent: Option<SessionId>,
    head: Option<SessionId>,
}
impl IntentSlots {
    pub(super) fn empty() -> Self {
        Self {
            wal: None,
            intent: None,
            head: None,
        }
    }
}
pub(super) struct IntentReadyFacts {
    pub(super) head: Zeroizing<Vec<u8>>,
    intent: Zeroizing<Vec<u8>>,
    intent_path: PathBuf,
    head_native: SessionId,
    intent_native: SessionId,
    wal_native: SessionId,
    wal_len: usize,
    // LAST: this actual metadata/read closure outlives all saved small facts above.
    pub(super) parent: Arc<IntentParent>,
}
impl IntentReadyFacts {
    /// Open/compare the small intent and hold all Ready names BEFORE final source
    /// validation. No later path construction or fresh File may follow that proof.
    pub(super) fn capture_named_files<'guard>(
        &'guard self,
        guard: &'guard V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<super::super::ready_read::ReadyNamedFiles<'guard>, AppError> {
        let mut held = super::super::ready_read::ReadyNamedFiles::capture(
            guard,
            &self.parent.target,
            self.parent.target_id,
            self.parent.pages()?,
            self.parent.pages_id,
        )?;
        held.add_file(
            owned_path(&self.parent.wal_path)?,
            self.wal_native,
            self.wal_len,
        )?;
        held.add_exact_file(
            owned_path(&self.intent_path)?,
            self.intent_native,
            &self.intent,
        )?;
        held.add_staged_head(
            &self.parent.target,
            &self.parent.head_path,
            self.head_native,
            self.head.len(),
        )?;
        held.add_absence(owned_path(&self.parent.wal_stage_path)?)?;
        Ok(held)
    }
}

/// This literal first-I namespace was frozen with the paired consumer. No caller name,
/// prefix override, page role or alternative source folder is accepted.
fn intent_child(base: &Path, id: &[u8; 32]) -> Result<PathBuf, AppError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut leaf = String::with_capacity(70);
    leaf.push_str("i-");
    for byte in id {
        leaf.push(HEX[(byte >> 4) as usize] as char);
        leaf.push(HEX[(byte & 15) as usize] as char);
    }
    leaf.push_str(".bin");
    let cap = base
        .as_os_str()
        .len()
        .checked_add(71)
        .filter(|n| *n <= MAX_PATH)
        .ok_or_else(refused)?;
    let mut path = PathBuf::with_capacity(cap);
    path.push(base);
    path.push(leaf);
    if path.capacity() > MAX_PATH {
        return Err(refused());
    }
    Ok(path)
}
impl PreparedV2TextUnsignedIntent {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_partial_write_fault_for_test(
        &mut self,
        point: V2TextIntentTestCut,
        len: usize,
    ) {
        assert!(matches!(
            point,
            V2TextIntentTestCut::WalCreated
                | V2TextIntentTestCut::IntentCreated
                | V2TextIntentTestCut::HeadCreated
        ));
        self.hooks.partial = Some((point, len));
    }
    /// Test DATA copied from this actual original, never a path/frame adopter.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn original_output_for_test(
        &self,
        point: V2TextIntentTestCut,
    ) -> Result<(PathBuf, Zeroizing<Vec<u8>>), AppError> {
        let (path, frame) = match point {
            V2TextIntentTestCut::WalCreated => (
                owned_path(&self.parent.wal_stage_path)?,
                self.wal.as_ref().ok_or_else(refused)?.frame_data(),
            ),
            V2TextIntentTestCut::IntentCreated => {
                let intent = self.intent.as_ref().ok_or_else(refused)?;
                (
                    intent_child(self.parent.pages()?, intent.id_data().as_bytes())?,
                    intent.frame_data(),
                )
            }
            V2TextIntentTestCut::HeadCreated => (
                fixed_child(&self.parent.target, "head.stage")?,
                self.candidate.as_ref().ok_or_else(refused)?.frame_data(),
            ),
            _ => return Err(refused()),
        };
        Ok((path, Zeroizing::new(frame.to_vec())))
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_pause_for_test(
        &mut self,
        point: V2TextIntentTestCut,
        hook: Box<dyn FnOnce() + Send>,
    ) {
        self.hooks.pause = Some((point, hook));
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_fault_for_test(&mut self, point: V2TextIntentTestCut) {
        self.hooks.fault = Some(point);
    }

    pub(in crate::store) fn run_stage(
        &mut self,
        admitted: AdmittedV2TextIntentStage,
    ) -> Result<(), AppError> {
        admitted.require_same_intent(&self.observe_original())?;
        self.retained
            .custody
            .work_resources()
            .validate_prepaid_account()?;
        self.parent.metadata.validate_prepaid_account()?;
        self.unsigned.validate_accounts_only()?;
        self.unsigned.require_original_plan_data(&self.retained)?;
        if self.ready.is_some() {
            return Ok(());
        }
        let mut slots = std::mem::replace(&mut self.slots, IntentSlots::empty());
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
        self.ready = Some(ReadyV2TextUnsignedIntentSelection {
            original: self.observe_original(),
            facts: Arc::new(facts),
            work: Arc::clone(self.retained.custody.work_resources()),
        });
        #[cfg(test)]
        self.hooks.hit(V2TextIntentTestCut::ReadyCached)?;
        drop(admitted);
        Ok(())
    }
    fn stage_inner(
        &self,
        slots: &mut IntentSlots,
        #[cfg(test)] hooks: &mut IntentTestHooks,
    ) -> Result<IntentReadyFacts, AppError> {
        let p = &self.parent;
        let target = native::require_directory(&p.target, p.target_id)?;
        let pages = native::require_directory(p.pages()?, p.pages_id)?;
        let old = native::require_exact(&p.head_path, None, &p.head_frame)?;
        let archive = native::require_exact(&p.archive_path, None, &p.archive_frame)?;
        // Every new attempt first proves FULL original W_A, or this SAME fully replaced
        // W_I after a prior uncertain rename. A MAC/metadata-only match is insufficient.
        self.ensure_wal(
            slots,
            #[cfg(test)]
            hooks,
        )?;
        #[cfg(test)]
        hooks.hit(V2TextIntentTestCut::WalSaved)?;
        let intent = self.intent.as_ref().ok_or_else(refused)?;
        let path = intent_child(p.pages()?, intent.id_data().as_bytes())?;
        super::warm::write_original(&path, intent.frame_data(), &mut slots.intent, || {
            #[cfg(test)]
            hooks.hit_created(
                V2TextIntentTestCut::IntentCreated,
                &path,
                intent.frame_data(),
            )?;
            Ok(())
        })?;
        native::sync_parent(p.pages()?, p.pages_id)?;
        #[cfg(test)]
        hooks.hit(V2TextIntentTestCut::IntentSaved)?;
        let candidate = self.candidate.as_ref().ok_or_else(refused)?;
        let head_stage = fixed_child(&p.target, "head.stage")?;
        super::warm::write_original(&head_stage, candidate.frame_data(), &mut slots.head, || {
            #[cfg(test)]
            hooks.hit_created(
                V2TextIntentTestCut::HeadCreated,
                &head_stage,
                candidate.frame_data(),
            )?;
            Ok(())
        })?;
        native::sync_parent(&p.target, p.target_id)?;
        #[cfg(test)]
        hooks.hit(V2TextIntentTestCut::HeadStaged)?;
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
        )?;
        Ok(IntentReadyFacts {
            head: Zeroizing::new(candidate.frame_data().to_vec()),
            intent: Zeroizing::new(intent.frame_data().to_vec()),
            intent_path: path,
            head_native: slots.head.ok_or_else(refused)?,
            intent_native: slots.intent.ok_or_else(refused)?,
            wal_native: slots.wal.ok_or_else(refused)?,
            wal_len: self.wal.as_ref().ok_or_else(refused)?.frame_data().len(),
            parent: Arc::clone(p),
        })
    }
    fn ensure_wal(
        &self,
        slots: &mut IntentSlots,
        #[cfg(test)] hooks: &mut IntentTestHooks,
    ) -> Result<(), AppError> {
        let p = &self.parent;
        let expected = self.wal.as_ref().ok_or_else(refused)?.frame_data();
        if slots
            .wal
            .is_some_and(|id| matches!(session_at(&p.wal_path, id), SessionAt::Ours))
        {
            native::require_absent(&p.wal_stage_path)?;
            let main = native::sync_exact(&p.wal_path, slots.wal, expected)?;
            native::sync_parent(&p.target, p.target_id)?;
            return require_named(&p.wal_path, &main);
        }
        // Before first/retried pre-MAIN WI work, neither recorded nor physical
        // dependent can exist. A complete own MAIN takes the separate branch above;
        // dirty/foreign/unknown stages never become an excuse to append a prefix.
        if slots.intent.is_some() || slots.head.is_some() {
            return Err(refused());
        }
        let intended = self.intent.as_ref().ok_or_else(refused)?;
        let intended_path = intent_child(p.pages()?, intended.id_data().as_bytes())?;
        native::require_absent(&intended_path)?;
        native::require_absent(&fixed_child(&p.target, "head.stage")?)?;
        native::require_absent(&fixed_child(&p.target, "text.seed.stage.v1")?)?;
        let original = self.retained.wal.as_ref().ok_or_else(refused)?.frame_data();
        // Full exact original authenticated WA BEFORE any WI-stage content write.
        let previous = native::sync_exact(&p.wal_path, None, original)?;
        super::warm::write_original(&p.wal_stage_path, expected, &mut slots.wal, || {
            #[cfg(test)]
            hooks.hit_created(V2TextIntentTestCut::WalCreated, &p.wal_stage_path, expected)?;
            Ok(())
        })?;
        #[cfg(test)]
        hooks.hit(V2TextIntentTestCut::WalStaged)?;
        native::sync_parent(&p.target, p.target_id)?;
        let stage = native::require_exact(&p.wal_stage_path, slots.wal, expected)?;
        require_named(&p.wal_path, &previous)?;
        require_named(&p.wal_stage_path, &stage)?;
        // Canonical same-group writer gate and OS session exclude participating writers.
        // This is the dedicated exact replacement edge, never a generic fallback unlink.
        fs::rename(&p.wal_stage_path, &p.wal_path).map_err(failed)?;
        #[cfg(test)]
        hooks.hit(V2TextIntentTestCut::WalReplaced)?;
        native::sync_parent(&p.target, p.target_id)?;
        require_named(&p.wal_path, &stage)?;
        native::require_absent(&p.wal_stage_path)
    }
    pub(in crate::store) fn run_commit(
        &mut self,
        admitted: AdmittedV2TextIntentCommit,
    ) -> Result<(), AppError> {
        admitted.require_same_intent(&self.observe_original())?;
        self.retained
            .custody
            .work_resources()
            .validate_prepaid_account()?;
        self.parent.metadata.validate_prepaid_account()?;
        self.unsigned.validate_accounts_only()?;
        self.unsigned.require_original_plan_data(&self.retained)?;
        if self.saved.is_some() {
            return Ok(());
        }
        // All fallible derivation/allocation precedes the selector effect. The real
        // wrapper is constructed only after completed file and parent barriers below.
        let intent = self.intent.as_ref().ok_or_else(refused)?;
        let intent_path = intent_child(self.parent.pages()?, intent.id_data().as_bytes())?;
        let intent_frame = Zeroizing::new(intent.frame_data().to_vec());
        let intent_plain = Zeroizing::new(self.intent_plain.as_ref().ok_or_else(refused)?.to_vec());
        let selected = Zeroizing::new(
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
        result?;
        let saved = SavedV2UnsignedAppendIntent {
            head: TextHeadIdData::from_bytes(
                *self
                    .candidate
                    .as_ref()
                    .ok_or_else(refused)?
                    .id_data()
                    .as_bytes(),
            ),
            intent: TextIntentIdData::from_bytes(
                *self
                    .intent
                    .as_ref()
                    .ok_or_else(refused)?
                    .id_data()
                    .as_bytes(),
            ),
            selected,
            intent_frame,
            intent_plain,
            intent_path,
            intent_native: self.slots.intent.ok_or_else(refused)?,
            wal_native: self.slots.wal.ok_or_else(refused)?,
            wal_len: self.wal.as_ref().ok_or_else(refused)?.frame_data().len(),
            original: self.observe_original(),
            parent: Arc::clone(&self.parent),
        };
        self.saved = Some(Arc::new(saved));
        #[cfg(test)]
        self.hooks.hit(V2TextIntentTestCut::SavedCached)?;
        drop(admitted);
        Ok(())
    }
    fn commit_inner(&self, #[cfg(test)] hooks: &mut IntentTestHooks) -> Result<(), AppError> {
        let ready = self.ready.as_ref().ok_or_else(refused)?;
        ready
            .original
            .require_same_original(&self.observe_original())?;
        let p = &self.parent;
        let target = native::require_directory(&p.target, p.target_id)?;
        let pages = native::require_directory(p.pages()?, p.pages_id)?;
        let wal = native::sync_exact(
            &p.wal_path,
            Some(ready.facts.wal_native),
            self.wal.as_ref().ok_or_else(refused)?.frame_data(),
        )?;
        native::require_absent(&p.wal_stage_path)?;
        let intent = native::sync_exact(
            &ready.facts.intent_path,
            Some(ready.facts.intent_native),
            self.intent.as_ref().ok_or_else(refused)?.frame_data(),
        )?;
        native::sync_parent(p.pages()?, p.pages_id)?;
        let archive = native::require_exact(&p.archive_path, None, &p.archive_frame)?;
        let current = read_control(&p.head_path, 1725)?;
        let expected = self.candidate.as_ref().ok_or_else(refused)?.frame_data();
        let head_stage = fixed_child(&p.target, "head.stage")?;
        if current.frame.as_slice() == p.head_frame.as_slice() {
            let stage = native::sync_exact(&head_stage, Some(ready.facts.head_native), expected)?;
            native::sync_parent(&p.target, p.target_id)?;
            require_named(&p.head_path, &current)?;
            require_named(&head_stage, &stage)?;
            require_named(&p.wal_path, &wal)?;
            require_named(&ready.facts.intent_path, &intent)?;
            require_named(&p.archive_path, &archive)?;
            fs::rename(&head_stage, &p.head_path).map_err(failed)?;
            #[cfg(test)]
            hooks.hit(V2TextIntentTestCut::HeadRenamed)?;
            native::sync_parent(&p.target, p.target_id)?;
            require_named(&p.head_path, &stage)?;
        } else if current.frame.as_slice() == expected {
            native::require_absent(&head_stage)?;
            native::sync_exact(&p.head_path, Some(ready.facts.head_native), expected)?;
            native::sync_parent(&p.target, p.target_id)?;
        } else {
            return Err(refused());
        }
        native::require_absent(&head_stage)?;
        require_named(&p.wal_path, &wal)?;
        require_named(&ready.facts.intent_path, &intent)?;
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
}
