//! Metadata first under genuine Current, authenticated complete seed/WC read only
//! in the finite paid worker. Parsed WC cannot select its source or create a parent;
//! all context, original resident, selected roots/header/counters and Plan ID come
//! from the actual closed original inputs prepared before this read.
use super::*;
impl CapturedV2TextCompletedWorkspace {
    pub(super) fn empty(plan: Arc<CompletedPlan>) -> Self {
        Self {
            target: HeldWorkspaceDirectory::empty(),
            pages: HeldWorkspaceDirectory::empty(),
            head: HeldWorkspaceControl::empty(),
            archive: HeldWorkspaceControl::empty(),
            seed: HeldWorkspaceControl::empty(),
            wal: HeldWorkspaceControl::empty(),
            seed_stage: HeldWorkspaceControl::empty(),
            wal_stage: HeldWorkspaceControl::empty(),
            head_stage: HeldWorkspaceControl::empty(),
            key: None,
            plan,
        }
    }
    pub(super) fn require_same_plan(&self, plan: &Arc<CompletedPlan>) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.plan, plan) {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn capture_metadata(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.plan.validate_original(guard)?;
        if self.key.is_none() {
            self.key = Some(Zeroizing::new(guard.store().keys.db_key()?));
        }
        let p = &self.plan.base;
        self.target.capture(&p.target, p.target_id)?;
        self.pages.capture(&p.pages, p.pages_id)?;
        self.head
            .capture_metadata(&p.paths.head, MAX_TEXT_HEAD_FRAME)?;
        self.archive
            .capture_metadata(&p.paths.archive, MAX_ARCHIVE_PLAIN + 40)?;
        self.head.read_complete(&p.paths.head)?;
        self.archive.read_complete(&p.paths.archive)?;
        self.seed
            .capture_metadata(&p.paths.seed, MAX_WORKSPACE_SEED_FRAME)?;
        // Only complete WC can be a completed source. Present oversized WORK/WD,
        // unsupported data and every stage are preserved/refused, never Clear.
        self.wal
            .capture_metadata(&p.paths.wal, MAX_COMPLETION_WAL_PLAIN + 40)?;
        self.seed_stage
            .capture_metadata(&p.paths.seed_stage, MAX_WORKSPACE_SEED_FRAME)?;
        self.wal_stage
            .capture_metadata(&p.paths.wal_stage, MAX_WORKSPACE_WAL_FRAME)?;
        self.head_stage
            .capture_metadata(&p.paths.head_stage, MAX_TEXT_HEAD_FRAME)?;
        self.require_presence()
    }
    fn require_presence(&self) -> Result<(), AppError> {
        let p = &self.plan.base;
        if p.parsed.pending != TextPendingData::None
            || !self.seed.is_present_data()
            || !self.wal.is_present_data()
        {
            return Err(refused());
        }
        self.seed_stage.require_absent(&p.paths.seed_stage)?;
        self.wal_stage.require_absent(&p.paths.wal_stage)?;
        self.head_stage.require_absent(&p.paths.head_stage)
    }
    pub(super) fn require_all_unchanged(&self) -> Result<(), AppError> {
        let p = &self.plan.base;
        if self.head.raw()? != p.head.as_slice() || self.archive.raw()? != p.archive.as_slice() {
            return Err(refused());
        }
        self.head.require_unchanged(&p.paths.head)?;
        self.archive.require_unchanged(&p.paths.archive)?;
        for (path, control) in [
            (&p.paths.seed, &self.seed),
            (&p.paths.wal, &self.wal),
            (&p.paths.seed_stage, &self.seed_stage),
            (&p.paths.wal_stage, &self.wal_stage),
            (&p.paths.head_stage, &self.head_stage),
        ] {
            control.require_metadata(path)?;
        }
        self.target.require_name(&p.target, p.target_id)?;
        self.pages.require_name(&p.pages, p.pages_id)
    }
    pub(in crate::store) fn validate_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.plan.validate_original(guard)?;
        if original_context(guard, self.key.as_ref().ok_or_else(refused)?)?
            != self.plan.base.context
        {
            return Err(refused());
        }
        self.require_presence()?;
        self.require_all_unchanged()?;
        guard.validate_original_namespace()?;
        self.require_all_unchanged()?;
        self.plan.base.resident.validate_negative()?;
        guard.validate_source_only()
    }
    /// Nonextracting finite DATA method. Factory retains this entire Source, its
    /// exact same-vault OS lease, native lane End and cap through every return.
    pub(in crate::store) fn prove_completed_for_original_data(
        &mut self,
        original: &OriginalV2RetainObservation,
    ) -> Result<[u8; 32], AppError> {
        self.plan.base.original.require_same_original(original)?;
        self.plan.base.metadata.validate()?;
        self.plan.work.validate_prepaid_account()?;
        self.require_presence()?;
        self.require_all_unchanged()?;
        let p = &self.plan.base;
        self.seed.read_complete(&p.paths.seed)?;
        seed::verify_anchor(p, self.key.as_ref().ok_or_else(refused)?, self.seed.raw()?)?;
        self.wal.read_complete(&p.paths.wal)?;
        let raw = self.wal.raw()?;
        if TextControlKindData::Wal.id_data(raw).as_bytes() != &p.expected_completion {
            return Err(refused());
        }
        let opened = open_control_data(
            self.key.as_ref().ok_or_else(refused)?,
            TextControlKindData::Wal.id_data(raw),
            raw,
        )?;
        let TextWalFrameData::Completion(wc) = TextWalFrameData::decode_data(&opened)? else {
            return Err(refused());
        };
        let plan = RetirementPlanData::decode_data(wc.plan)?;
        if wc.context != p.context.id_data()?
            || wc.transaction != p.expected_transaction
            || wc.selected != p.selected
            || wc.plan_commit != p.expected_plan
            || plan.id_data()? != p.expected_plan
            || wc.public_header != p.parsed_archive.spec_data().header.id()
            || wc.reclaimed != p.reclaimed
            || wc.reclaimed != plan.carry_data().totals_data()?
        {
            return Err(refused());
        }
        p.parsed
            .live_bytes
            .checked_sub(wc.reclaimed.bytes)
            .ok_or_else(refused)?;
        p.parsed
            .live_objects
            .checked_sub(wc.reclaimed.objects)
            .ok_or_else(refused)?;
        // Names come ONLY from this complete authenticated matching actual WC.
        // Unknown input/error never supplies an empty list, quota grant or source.
        let retired = plan
            .carry_data()
            .entries_data()
            .iter()
            .map(|entry| retired_path(&p.pages, entry.kind, &entry.id))
            .collect::<Result<Vec<_>, AppError>>()?;
        for path in &retired {
            require_absent(path)?;
        }
        let candidate = Arc::new(CompletedV2TextParentData {
            base: p.clone(),
            completion: Zeroizing::new(raw.to_vec()),
            completion_native: self.wal.native_data()?,
            plan,
            retired,
            seed: *blake3::hash(self.seed.raw()?).as_bytes(),
            metadata: p.metadata.clone(),
        });
        if let Some(old) = self.plan.completed.get() {
            old.require_source(self)?;
        } else {
            self.plan.completed.set(candidate).map_err(|_| refused())?;
        }
        // Real completion is cached before final name checks/lost return, but Ready
        // is created only by run_read. A retry still needs its own actual admission.
        self.require_all_unchanged()?;
        self.seed.require_unchanged(&p.paths.seed)?;
        self.wal.require_unchanged(&p.paths.wal)?;
        require_inventory(&p.target)?;
        self.plan
            .completed
            .get()
            .ok_or_else(refused)?
            .require_absences()?;
        Ok(p.expected_completion)
    }
}
impl CompletedV2TextParentData {
    pub(super) fn require_source(
        &self,
        source: &CapturedV2TextCompletedWorkspace,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.base, &source.plan.base)
            || source.wal.native_data()? != self.completion_native
            || source.wal.raw()? != self.completion.as_slice()
            || *blake3::hash(source.seed.raw()?).as_bytes() != self.seed
        {
            return Err(refused());
        }
        self.require_absences()
    }
    pub(super) fn require_absences(&self) -> Result<(), AppError> {
        for path in &self.retired {
            require_absent(path)?;
        }
        Ok(())
    }
    pub(in crate::store) fn observe_completed_original(
        &self,
    ) -> OriginalV2TextCompletedWorkspaceObservation {
        self.base.observe_completed()
    }
    pub(in crate::store) fn effective_baseline_data(&self) -> Result<RetiredTotalsData, AppError> {
        self.metadata.validate()?;
        self.plan
            .carry_data()
            .totals_data()?
            .effective_baseline_data(
                &self.plan,
                self.base.selected,
                RetiredTotalsData {
                    bytes: self.base.parsed.live_bytes,
                    objects: self.base.parsed.live_objects,
                },
            )
    }
    pub(in crate::store) fn completion_frame_data(&self) -> &[u8] {
        &self.completion
    }
    pub(in crate::store) fn original_resident(&self) -> &OriginalV2TextResidentObservation {
        &self.base.resident
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.base.resources
    }
}
pub(super) fn retired_path(
    base: &Path,
    kind: RetirementKindData,
    id: &[u8; 32],
) -> Result<PathBuf, AppError> {
    let prefix = match kind {
        RetirementKindData::AcceptedIndex
        | RetirementKindData::TokenIndex
        | RetirementKindData::PublicationIndex => "p-",
        RetirementKindData::Intent => "i-",
        RetirementKindData::Signature => "s-",
    };
    let mut name = String::with_capacity(70);
    name.push_str(prefix);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for b in id {
        name.push(HEX[(b >> 4) as usize] as char);
        name.push(HEX[(b & 15) as usize] as char);
    }
    name.push_str(".bin");
    owned_child(base, &name)
}
fn require_absent(path: &Path) -> Result<(), AppError> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(refused()),
    }
}
