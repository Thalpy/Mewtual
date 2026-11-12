//! Finite first-S IO under the distinct SAME-store Stage/Commit markers. Every helper
//! File is local to one call, including uncertain rename/barrier errors. No Saved/Ready
//! retains an obsolete O/WAL File. Scope stop after admission cannot erase cached work.
//! SAME-original S Stage may complete its recorded exact prefix under warm.rs;
//! fresh original WI/ownWS proof, current admission and original signature remain
//! mandatory. Generic native/cold/unknown output helpers stay complete-only.
use super::super::super::work::io as native;
use super::*;
use catcoms_storage::kept::{session_at, SessionAt};
use std::fs;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum V2TextRawSignatureTestCut {
    WalCreated,
    WalStaged,
    WalReplaced,
    WalSaved,
    SignatureCreated,
    SignatureSaved,
    HeadCreated,
    HeadStaged,
    ReadyCached,
    HeadRenamed,
    SavedCached,
}
#[cfg(test)]
#[derive(Default)]
pub(super) struct RawTestHooks {
    fault: Option<V2TextRawSignatureTestCut>,
    partial: Option<(V2TextRawSignatureTestCut, usize)>,
    pause: Option<(V2TextRawSignatureTestCut, Box<dyn FnOnce() + Send>)>,
}
#[cfg(test)]
impl RawTestHooks {
    /// Actual test prefix write at genuine create-new/native caching while its
    /// original File and real consumed Stage OSlease/End are still alive.
    fn hit_created(
        &mut self,
        point: V2TextRawSignatureTestCut,
        path: &Path,
        expected: &[u8],
    ) -> Result<(), AppError> {
        self.hit(point)?;
        if self.partial.as_ref().is_some_and(|(cut, _)| *cut == point) {
            use std::io::Write;
            let (_, len) = self.partial.take().expect("actual S prefix cut");
            if len >= expected.len() {
                return Err(refused());
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(failed)?;
            file.write_all(&expected[..len]).map_err(failed)?;
            file.sync_all().map_err(failed)?;
            return Err(failed("injected original S partial write"));
        }
        Ok(())
    }
    fn hit(&mut self, point: V2TextRawSignatureTestCut) -> Result<(), AppError> {
        if self.pause.as_ref().is_some_and(|(cut, _)| *cut == point) {
            let (_, run) = self.pause.take().expect("actual S IO cut");
            run();
        }
        if self.fault == Some(point) {
            self.fault = None;
            return Err(failed("injected raw-S IO failure"));
        }
        Ok(())
    }
}
fn failed(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

pub(super) struct RawSlots {
    wal: Option<SessionId>,
    signature: Option<SessionId>,
    head: Option<SessionId>,
}
impl RawSlots {
    pub(super) fn empty() -> Self {
        Self {
            wal: None,
            signature: None,
            head: None,
        }
    }
}
pub(super) struct RawReadyFacts {
    pub(super) head: Zeroizing<Vec<u8>>,
    signature: Zeroizing<Vec<u8>>,
    signature_path: PathBuf,
    head_native: SessionId,
    signature_native: SessionId,
    wal_native: SessionId,
    wal_len: usize,
    // LAST: this actual metadata/read closure outlives all saved small facts above.
    pub(super) parent: Arc<RawParent>,
}
impl RawReadyFacts {
    /// Open/compare the small signature and hold all Ready names BEFORE final source
    /// validation. No later path construction or fresh File may follow that proof.
    pub(super) fn capture_named_files<'guard>(
        &'guard self,
        guard: &'guard V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<super::super::super::ready_read::ReadyNamedFiles<'guard>, AppError> {
        let mut held = super::super::super::ready_read::ReadyNamedFiles::capture(
            guard,
            &self.parent.base.target,
            self.parent.base.target_id,
            self.parent.base.pages()?,
            self.parent.base.pages_id,
        )?;
        held.add_file(
            owned_path(&self.parent.base.wal_path)?,
            self.wal_native,
            self.wal_len,
        )?;
        held.add_exact_file(
            owned_path(&self.signature_path)?,
            self.signature_native,
            &self.signature,
        )?;
        held.add_staged_head(
            &self.parent.base.target,
            &self.parent.base.head_path,
            self.head_native,
            self.head.len(),
        )?;
        held.add_absence(owned_path(&self.parent.base.wal_stage_path)?)?;
        Ok(held)
    }
}

/// This literal first-S namespace was frozen with the paired consumer. No caller name,
/// prefix override, page role or alternative source folder is accepted.
fn signature_child(base: &Path, id: &[u8; 32]) -> Result<PathBuf, AppError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut leaf = String::with_capacity(70);
    leaf.push_str("s-");
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
impl PreparedV2TextRawSignature {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_partial_write_fault_for_test(
        &mut self,
        point: V2TextRawSignatureTestCut,
        len: usize,
    ) {
        assert!(matches!(
            point,
            V2TextRawSignatureTestCut::WalCreated
                | V2TextRawSignatureTestCut::SignatureCreated
                | V2TextRawSignatureTestCut::HeadCreated
        ));
        self.hooks.partial = Some((point, len));
    }
    /// Only this actual original's cached cipher DATA, never an output/path adopter.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn original_output_for_test(
        &self,
        point: V2TextRawSignatureTestCut,
    ) -> Result<(PathBuf, Zeroizing<Vec<u8>>), AppError> {
        let (path, frame) = match point {
            V2TextRawSignatureTestCut::WalCreated => (
                owned_path(&self.parent.base.wal_stage_path)?,
                self.wal.as_ref().ok_or_else(refused)?.frame_data(),
            ),
            V2TextRawSignatureTestCut::SignatureCreated => {
                let s = self.signature.as_ref().ok_or_else(refused)?;
                (
                    signature_child(self.parent.base.pages()?, s.id_data().as_bytes())?,
                    s.frame_data(),
                )
            }
            V2TextRawSignatureTestCut::HeadCreated => (
                fixed_child(&self.parent.base.target, "head.stage")?,
                self.candidate.as_ref().ok_or_else(refused)?.frame_data(),
            ),
            _ => return Err(refused()),
        };
        Ok((path, Zeroizing::new(frame.to_vec())))
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_pause_for_test(
        &mut self,
        point: V2TextRawSignatureTestCut,
        hook: Box<dyn FnOnce() + Send>,
    ) {
        self.hooks.pause = Some((point, hook));
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_fault_for_test(&mut self, point: V2TextRawSignatureTestCut) {
        self.hooks.fault = Some(point);
    }

    pub(in crate::store) fn run_stage(
        &mut self,
        admitted: AdmittedV2TextRawSignatureStage,
    ) -> Result<(), AppError> {
        admitted.require_same_signature(&self.observe_original())?;
        self.intent
            .retained
            .custody
            .work_resources()
            .validate_prepaid_account()?;
        self.parent.base.metadata.validate_prepaid_account()?;
        self.intent.unsigned.validate_accounts_only()?;
        self.require_original_raw_data()?;
        if self.ready.is_some() {
            return Ok(());
        }
        let mut slots = std::mem::replace(&mut self.slots, RawSlots::empty());
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
        self.ready = Some(ReadyV2TextRawSignatureSelection {
            original: self.observe_original(),
            facts: Arc::new(facts),
            work: Arc::clone(self.intent.retained.custody.work_resources()),
        });
        #[cfg(test)]
        self.hooks.hit(V2TextRawSignatureTestCut::ReadyCached)?;
        drop(admitted);
        Ok(())
    }
    fn stage_inner(
        &self,
        slots: &mut RawSlots,
        #[cfg(test)] hooks: &mut RawTestHooks,
    ) -> Result<RawReadyFacts, AppError> {
        let p = &self.parent;
        let target = native::require_directory(&p.base.target, p.base.target_id)?;
        let pages = native::require_directory(p.base.pages()?, p.base.pages_id)?;
        let old = native::require_exact(&p.base.head_path, None, &p.head_frame)?;
        let archive = native::require_exact(&p.base.archive_path, None, &p.base.archive_frame)?;
        let original_i =
            native::require_exact(&p.intent_path, Some(p.intent_native), &p.intent_frame)?;
        // Every new attempt first proves FULL original W_I, or this SAME fully replaced
        // W_S after a prior uncertain rename. A MAC/metadata-only match is insufficient.
        self.ensure_wal(
            slots,
            #[cfg(test)]
            hooks,
        )?;
        #[cfg(test)]
        hooks.hit(V2TextRawSignatureTestCut::WalSaved)?;
        let signature = self.signature.as_ref().ok_or_else(refused)?;
        let path = signature_child(p.base.pages()?, signature.id_data().as_bytes())?;
        super::warm::write_original(&path, signature.frame_data(), &mut slots.signature, || {
            #[cfg(test)]
            hooks.hit_created(
                V2TextRawSignatureTestCut::SignatureCreated,
                &path,
                signature.frame_data(),
            )?;
            Ok(())
        })?;
        native::sync_parent(p.base.pages()?, p.base.pages_id)?;
        #[cfg(test)]
        hooks.hit(V2TextRawSignatureTestCut::SignatureSaved)?;
        let candidate = self.candidate.as_ref().ok_or_else(refused)?;
        let head_stage = fixed_child(&p.base.target, "head.stage")?;
        super::warm::write_original(&head_stage, candidate.frame_data(), &mut slots.head, || {
            #[cfg(test)]
            hooks.hit_created(
                V2TextRawSignatureTestCut::HeadCreated,
                &head_stage,
                candidate.frame_data(),
            )?;
            Ok(())
        })?;
        native::sync_parent(&p.base.target, p.base.target_id)?;
        #[cfg(test)]
        hooks.hit(V2TextRawSignatureTestCut::HeadStaged)?;
        require_named(&p.base.head_path, &old)?;
        require_named(&p.intent_path, &original_i)?;
        require_named(&p.base.archive_path, &archive)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            p.base.pages()?,
            &pages,
            p.base.pages_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &p.base.target,
            &target,
            p.base.target_id,
        )?;
        Ok(RawReadyFacts {
            head: Zeroizing::new(candidate.frame_data().to_vec()),
            signature: Zeroizing::new(signature.frame_data().to_vec()),
            signature_path: path,
            head_native: slots.head.ok_or_else(refused)?,
            signature_native: slots.signature.ok_or_else(refused)?,
            wal_native: slots.wal.ok_or_else(refused)?,
            wal_len: self.wal.as_ref().ok_or_else(refused)?.frame_data().len(),
            parent: Arc::clone(p),
        })
    }
    fn ensure_wal(
        &self,
        slots: &mut RawSlots,
        #[cfg(test)] hooks: &mut RawTestHooks,
    ) -> Result<(), AppError> {
        let p = &self.parent;
        let expected = self.wal.as_ref().ok_or_else(refused)?.frame_data();
        if slots
            .wal
            .is_some_and(|id| matches!(session_at(&p.base.wal_path, id), SessionAt::Ours))
        {
            native::require_absent(&p.base.wal_stage_path)?;
            let main = native::sync_exact(&p.base.wal_path, slots.wal, expected)?;
            native::sync_parent(&p.base.target, p.base.target_id)?;
            return require_named(&p.base.wal_path, &main);
        }
        // Before first/retried pre-MAIN WS work, neither recorded nor physical
        // dependent can exist. A complete own MAIN takes the separate branch above;
        // dirty/foreign/unknown stages never become an excuse to append a prefix.
        if slots.signature.is_some() || slots.head.is_some() {
            return Err(refused());
        }
        let intended = self.signature.as_ref().ok_or_else(refused)?;
        let intended_path = signature_child(p.base.pages()?, intended.id_data().as_bytes())?;
        native::require_absent(&intended_path)?;
        native::require_absent(&fixed_child(&p.base.target, "head.stage")?)?;
        native::require_absent(&fixed_child(&p.base.target, "text.seed.stage.v1")?)?;
        let original = self.intent.wal.as_ref().ok_or_else(refused)?.frame_data();
        // Full exact original authenticated WI BEFORE any WS-stage content write.
        let previous = native::sync_exact(&p.base.wal_path, None, original)?;
        super::warm::write_original(&p.base.wal_stage_path, expected, &mut slots.wal, || {
            #[cfg(test)]
            hooks.hit_created(
                V2TextRawSignatureTestCut::WalCreated,
                &p.base.wal_stage_path,
                expected,
            )?;
            Ok(())
        })?;
        #[cfg(test)]
        hooks.hit(V2TextRawSignatureTestCut::WalStaged)?;
        native::sync_parent(&p.base.target, p.base.target_id)?;
        let stage = native::require_exact(&p.base.wal_stage_path, slots.wal, expected)?;
        require_named(&p.base.wal_path, &previous)?;
        require_named(&p.base.wal_stage_path, &stage)?;
        // Canonical same-group writer gate and OS session exclude participating writers.
        // This is the dedicated exact replacement edge, never a generic fallback unlink.
        fs::rename(&p.base.wal_stage_path, &p.base.wal_path).map_err(failed)?;
        #[cfg(test)]
        hooks.hit(V2TextRawSignatureTestCut::WalReplaced)?;
        native::sync_parent(&p.base.target, p.base.target_id)?;
        require_named(&p.base.wal_path, &stage)?;
        native::require_absent(&p.base.wal_stage_path)
    }
    pub(in crate::store) fn run_commit(
        &mut self,
        admitted: AdmittedV2TextRawSignatureCommit,
    ) -> Result<(), AppError> {
        admitted.require_same_signature(&self.observe_original())?;
        self.intent
            .retained
            .custody
            .work_resources()
            .validate_prepaid_account()?;
        self.parent.base.metadata.validate_prepaid_account()?;
        self.intent.unsigned.validate_accounts_only()?;
        self.require_original_raw_data()?;
        if self.saved.is_some() {
            return Ok(());
        }
        // All fallible derivation/allocation precedes the selector effect. The real
        // wrapper is constructed only after completed file and parent barriers below.
        let signature = self.signature.as_ref().ok_or_else(refused)?;
        let signature_path =
            signature_child(self.parent.base.pages()?, signature.id_data().as_bytes())?;
        let signature_frame = Zeroizing::new(signature.frame_data().to_vec());
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
        let saved = SavedV2RawAppendSignature {
            head: TextHeadIdData::from_bytes(
                *self
                    .candidate
                    .as_ref()
                    .ok_or_else(refused)?
                    .id_data()
                    .as_bytes(),
            ),
            signature: TextRawSignatureIdData::from_bytes(
                *self
                    .signature
                    .as_ref()
                    .ok_or_else(refused)?
                    .id_data()
                    .as_bytes(),
            ),
            selected,
            signature_frame,
            signature_path,
            signature_native: self.slots.signature.ok_or_else(refused)?,
            #[cfg(all(test, windows))]
            drop_notice: SavedRawFactsDropNotice::default(),
            original: self.observe_original(),
            parent: Arc::clone(&self.parent),
        };
        #[cfg(all(test, windows))]
        let saved = {
            let mut saved = saved;
            saved.drop_notice.notice = self.saved_drop_notice.take();
            saved
        };
        self.saved = Some(Arc::new(saved));
        #[cfg(test)]
        self.hooks.hit(V2TextRawSignatureTestCut::SavedCached)?;
        drop(admitted);
        Ok(())
    }
    fn commit_inner(&self, #[cfg(test)] hooks: &mut RawTestHooks) -> Result<(), AppError> {
        let ready = self.ready.as_ref().ok_or_else(refused)?;
        ready
            .original
            .require_same_original(&self.observe_original())?;
        let p = &self.parent;
        let target = native::require_directory(&p.base.target, p.base.target_id)?;
        let pages = native::require_directory(p.base.pages()?, p.base.pages_id)?;
        let wal = native::sync_exact(
            &p.base.wal_path,
            Some(ready.facts.wal_native),
            self.wal.as_ref().ok_or_else(refused)?.frame_data(),
        )?;
        native::require_absent(&p.base.wal_stage_path)?;
        let signature = native::sync_exact(
            &ready.facts.signature_path,
            Some(ready.facts.signature_native),
            self.signature.as_ref().ok_or_else(refused)?.frame_data(),
        )?;
        native::sync_parent(p.base.pages()?, p.base.pages_id)?;
        let archive = native::require_exact(&p.base.archive_path, None, &p.base.archive_frame)?;
        let original_i =
            native::require_exact(&p.intent_path, Some(p.intent_native), &p.intent_frame)?;
        let current = read_control(&p.base.head_path, 1725)?;
        let expected = self.candidate.as_ref().ok_or_else(refused)?.frame_data();
        let head_stage = fixed_child(&p.base.target, "head.stage")?;
        if current.frame.as_slice() == p.head_frame.as_slice() {
            let stage = native::sync_exact(&head_stage, Some(ready.facts.head_native), expected)?;
            native::sync_parent(&p.base.target, p.base.target_id)?;
            require_named(&p.base.head_path, &current)?;
            require_named(&head_stage, &stage)?;
            require_named(&p.base.wal_path, &wal)?;
            require_named(&ready.facts.signature_path, &signature)?;
            require_named(&p.intent_path, &original_i)?;
            require_named(&p.base.archive_path, &archive)?;
            fs::rename(&head_stage, &p.base.head_path).map_err(failed)?;
            #[cfg(test)]
            hooks.hit(V2TextRawSignatureTestCut::HeadRenamed)?;
            native::sync_parent(&p.base.target, p.base.target_id)?;
            require_named(&p.base.head_path, &stage)?;
        } else if current.frame.as_slice() == expected {
            native::require_absent(&head_stage)?;
            native::sync_exact(&p.base.head_path, Some(ready.facts.head_native), expected)?;
            native::sync_parent(&p.base.target, p.base.target_id)?;
        } else {
            return Err(refused());
        }
        native::require_absent(&head_stage)?;
        require_named(&p.base.wal_path, &wal)?;
        require_named(&ready.facts.signature_path, &signature)?;
        require_named(&p.intent_path, &original_i)?;
        require_named(&p.base.archive_path, &archive)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            p.base.pages()?,
            &pages,
            p.base.pages_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &p.base.target,
            &target,
            p.base.target_id,
        )
    }
}
