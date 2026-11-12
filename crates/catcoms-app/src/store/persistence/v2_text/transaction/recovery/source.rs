//! Readonly classification from actual held controls. Presence is never inferred from a
//! parser/decryption/read error. Large WAL bytes and crypto are outside all host writers.
use super::*;
use catcoms_storage::kept::{session_at, SessionAt};
use std::fs;

pub(in crate::store) struct CapturedV2TextRetainRecovery {
    target: HeldRetainRecoveryDirectory,
    pages: HeldRetainRecoveryDirectory,
    head: HeldRetainRecoveryControl,
    archive: HeldRetainRecoveryControl,
    main: HeldRetainRecoveryControl,
    stage: HeldRetainRecoveryControl,
    key: Option<Zeroizing<[u8; 32]>>,
    // LAST: coupled active lease owns this object before the first File/key can open.
    plan: Arc<RecoveryPlan>,
}
impl CapturedV2TextRetainRecovery {
    pub(in crate::store) fn require_same_recovery(
        &self,
        original: &OriginalV2TextRetainRecoveryObservation,
    ) -> Result<(), AppError> {
        observe(&self.plan).require_same_original(original)
    }
    pub(super) fn empty(plan: Arc<RecoveryPlan>) -> Self {
        Self {
            target: HeldRetainRecoveryDirectory::empty(),
            pages: HeldRetainRecoveryDirectory::empty(),
            head: HeldRetainRecoveryControl::empty(),
            archive: HeldRetainRecoveryControl::empty(),
            main: HeldRetainRecoveryControl::empty(),
            stage: HeldRetainRecoveryControl::empty(),
            key: None,
            plan,
        }
    }
    pub(super) fn require_same_plan(&self, plan: &Arc<RecoveryPlan>) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.plan, plan) {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn capture_metadata(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        let p = &self.plan.parent;
        self.target.capture(&p.target, p.target_id)?;
        self.pages.capture(p.pages()?, p.pages_id)?;
        self.head.capture_metadata(&p.head_path, 1725)?;
        self.head.read_complete(&p.head_path)?;
        self.archive.capture_metadata(&p.archive_path, 1519)?;
        self.archive.read_complete(&p.archive_path)?;
        self.main
            .capture_metadata(&p.wal_path, self.plan.expected.len())?;
        self.stage
            .capture_metadata(&p.wal_stage_path, self.plan.expected.len())?;
        if self.key.is_none() {
            self.key = Some(Zeroizing::new(guard.store().keys.db_key()?));
        }
        self.validate_under_current_guard(guard)
    }
    fn require_small_and_names(&self) -> Result<(), AppError> {
        let p = &self.plan.parent;
        if self.head.raw()? != p.head_frame.as_slice()
            || self.archive.raw()? != p.archive_frame.as_slice()
        {
            return Err(refused());
        }
        self.target.require_name(&p.target, p.target_id)?;
        self.pages.require_name(p.pages()?, p.pages_id)?;
        self.head.require_unchanged(&p.head_path)?;
        self.archive.require_unchanged(&p.archive_path)?;
        self.main.require_metadata(&p.wal_path)?;
        self.stage.require_metadata(&p.wal_stage_path)
    }
    pub(in crate::store) fn validate_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.plan
            .parent
            .resources
            .require_same_original(guard.resources())?;
        // This is the SAME finite writer gate already held by this Source. The full
        // original proof repeats without an idle precheck against its own actual End.
        self.plan.validate_parent(guard)?;
        self.require_small_and_names()?;
        guard.validate_original_namespace()?;
        self.require_small_and_names()?;
        guard.validate_source_only()
    }
    /// The original complete expected WA was produced/semantic-checked by the actual Cell.
    /// Decrypt it only with the genuine held Current source key, then repeat ALL first-A
    /// semantic checks. This cache is DATA; it neither imports an SDK nor selects a head.
    fn cache_expected_plain(&self) -> Result<(), AppError> {
        if self.plan.plain.get().is_none() {
            let key = self.key.as_ref().ok_or_else(refused)?;
            let plain = open_control_data(
                key,
                TextControlKindData::Wal.id_data(&self.plan.expected),
                &self.plan.expected,
            )?;
            if plain.len() > MAX_RECOVERY_WAL_FRAME || plain.capacity() > MAX_RECOVERY_WAL_FRAME {
                return Err(refused());
            }
            let TextWalFrameData::Work(wal) = TextWalFrameData::decode_data(&plain)? else {
                return Err(refused());
            };
            wal.verify_first_retained_a_parent_data(
                key,
                super::super::super::wal::retain::RetainWalParentData {
                    head: &self.plan.parent.head,
                    archive: &self.plan.parent.archive,
                    id: TextHeadIdData::of_data(&self.plan.parent.head_frame),
                    frame: &self.plan.parent.head_frame,
                },
            )?;
            self.plan.plain.set(plain).map_err(|_| refused())?;
        }
        Ok(())
    }
    pub(in crate::store) fn classify_outside_writers(
        &mut self,
    ) -> Result<(V2TextRetainWarmRecoveryClassData, Option<SessionId>, usize), AppError> {
        self.require_small_and_names()?;
        self.cache_expected_plain()?;
        let p = &self.plan.parent;
        let outcome = if self.main.is_present_data() {
            self.stage.require_absent(&p.wal_stage_path)?;
            self.main.read_complete(&p.wal_path)?;
            if self.main.raw()? != self.plan.expected.as_slice() {
                return Err(refused());
            }
            (
                V2TextRetainWarmRecoveryClassData::CompleteMain,
                Some(self.main.native_data()?),
                self.main.length_data()?,
            )
        } else {
            self.main.require_absent(&p.wal_path)?;
            if self.stage.is_absent_data() {
                // Only an actual never-started original Cell may turn these genuine
                // absences into Clean DATA. A lost recorded attempt remains an error.
                if self.plan.wal_native.is_some() {
                    return Err(refused());
                }
                self.plan.require_no_dependents()?;
                self.require_small_and_names()?;
                return Ok((V2TextRetainWarmRecoveryClassData::CleanUnstarted, None, 0));
            }
            self.stage.read_complete(&p.wal_stage_path)?;
            if Some(self.stage.native_data()?) != self.plan.wal_native {
                return Err(refused());
            }
            let raw = self.stage.raw()?;
            // Missing MAIN after any recorded/actual dependent output is NOT first-stage
            // scratch or a renewed stage. Preserve this inconsistent state for cold repair.
            self.plan.require_no_dependents()?;
            if raw == self.plan.expected.as_slice() {
                (
                    V2TextRetainWarmRecoveryClassData::CompleteStage,
                    Some(self.stage.native_data()?),
                    raw.len(),
                )
            } else {
                if raw.len() >= self.plan.expected.len() || !self.plan.expected.starts_with(raw) {
                    return Err(refused());
                }
                preserve_authenticated_stage(self.key.as_ref().ok_or_else(refused)?, raw)?;
                (
                    V2TextRetainWarmRecoveryClassData::FirstWalScratch,
                    Some(self.stage.native_data()?),
                    raw.len(),
                )
            }
        };
        self.require_small_and_names()?;
        if self.main.is_present_data() {
            self.main.require_unchanged(&p.wal_path)?;
        }
        if self.stage.is_present_data() {
            self.stage.require_unchanged(&p.wal_stage_path)?;
        }
        Ok(outcome)
    }
}

/// Reserved stage.v1 can discard only the actual original expected proper-prefix attempt.
/// Preserve every MAC under ANY known control KDF before interpreting kind/tag/version/C.
/// Plaintexts drain sequentially; unknown future KDF authenticity cannot be detected, so
/// a future writer must reserve a refusal/version change before changing stage.v1 meaning.
fn preserve_authenticated_stage(key: &[u8; 32], raw: &[u8]) -> Result<(), AppError> {
    const PAGE: &[u8] = b"catcoms/private-chat-sealed-page/v1\0";
    if raw.len() >= 4 + PAGE.len()
        && raw[..4] == (PAGE.len() as u32).to_be_bytes()
        && &raw[4..4 + PAGE.len()] == PAGE
    {
        return Err(refused());
    }
    if raw.len() < 40 {
        return Ok(());
    }
    let sealed = crate::store::unframe(raw)?;
    for domain in [
        "catcoms/private-chat-text-head-key/v1",
        "catcoms/private-chat-text-archive-key/v1",
        "catcoms/private-chat-text-retain-key/v1",
        "catcoms/private-chat-text-intent-key/v1",
        "catcoms/private-chat-text-signature-key/v1",
        "catcoms/private-chat-text-publication-key/v1",
        "catcoms/private-chat-text-seed-key/v1",
        "catcoms/private-chat-text-wal-key/v1",
    ] {
        let derived = Zeroizing::new(blake3::derive_key(domain, key));
        if let Ok(plain) = catcoms_crypto::unseal(&derived, &sealed) {
            drop(Zeroizing::new(plain));
            return Err(refused());
        }
    }
    Ok(())
}

fn require_metadata(path: &Path, native: SessionId, len: usize) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path).map_err(io::io)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != len as u64
        || !matches!(session_at(path, native), SessionAt::Ours)
    {
        return Err(refused());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(refused());
        }
    }
    Ok(())
}
impl ReadyV2TextRetainRepair {
    pub(in crate::store) fn validate_repair_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.plan.validate_parent(guard)?;
        require_metadata(&self.plan.parent.wal_path, self.native, self.len)?;
        native::require_absent(&self.plan.parent.wal_stage_path)?;
        guard.validate_original_namespace()?;
        require_metadata(&self.plan.parent.wal_path, self.native, self.len)?;
        guard.validate_source_only()
    }
}
impl ReadyV2TextRetainScratch {
    pub(in crate::store) fn validate_scratch_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.plan.validate_parent(guard)?;
        native::require_absent(&self.plan.parent.wal_path)?;
        self.plan.require_no_dependents()?;
        self.require_stage_state()?;
        guard.validate_original_namespace()?;
        self.require_stage_state()?;
        guard.validate_source_only()
    }
    fn require_stage_state(&self) -> Result<(), AppError> {
        if self.plan.scratch_unlinked.load(Ordering::Acquire) {
            // This bit is cached ONLY immediately after the actual same-plan unlink.
            // Absence cannot manufacture it or authorize de-novo scratch disposal.
            native::require_absent(&self.plan.parent.wal_stage_path)
        } else {
            require_metadata(&self.plan.parent.wal_stage_path, self.native, self.len)
        }
    }
}
