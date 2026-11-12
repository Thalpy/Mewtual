//! Borrowed exact predecessor used ONLY before replacing WC with the new WA.
//! After actual WA admission/rename, the new owning cell's own continuation wins;
//! this old-parent check must not be called through an obsolete WC afterward.
use super::*;
impl OriginalV2TextCompletedWorkspaceObservation {
    pub(in crate::store) fn require_original_retained(
        &self,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)
    }
}
impl CompletedV2TextParentData {
    pub(in crate::store) fn require_selected_data(
        &self,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.base
            .resources
            .require_same_original(selected.resources())?;
        let p = selected.physical_query_data();
        if p.target != self.base.target
            || p.target_id != self.base.target_id
            || p.pages != self.base.pages
            || p.pages_id != self.base.pages_id
            || p.head_frame != self.base.head.as_slice()
            || p.archive_frame != self.base.archive.as_slice()
            || selected.head_id_data() != self.base.selected
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn validate_prepaid_account(&self) -> Result<(), AppError> {
        self.metadata.validate()
    }
    pub(in crate::store) fn original_head_data(&self) -> &TextHeadData {
        &self.base.parsed
    }
    pub(in crate::store) fn completion_native_data(&self) -> SessionId {
        self.completion_native
    }
    /// FULL actual old WC/O/H proof under the SAME Current, before final ID3. No
    /// account, decoder, path allocation or payload open follows the final seam.
    pub(in crate::store) fn validate_before_retain_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.validate_before_retain_inner(guard, None)
    }
    pub(in crate::store::persistence::v2_text) fn validate_owned_retain_stage_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        stage: &crate::store::persistence::v2_text::transaction::CompletedRetainStageSourceData<'_>,
    ) -> Result<(), AppError> {
        stage.require_parent(self)?;
        self.validate_before_retain_inner(guard, Some(stage))
    }
    fn validate_before_retain_inner(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        stage: Option<
            &crate::store::persistence::v2_text::transaction::CompletedRetainStageSourceData<'_>,
        >,
    ) -> Result<(), AppError> {
        self.base
            .resources
            .require_same_original(guard.resources())?;
        guard.require_text_control_write_idle_for_query()?;
        self.metadata.validate()?;
        self.base
            .resources
            .validate_original(guard.store(), guard.source())?;
        self.base.resident.validate_negative()?;
        let p = &self.base;
        let mut target = HeldWorkspaceDirectory::empty();
        let mut pages = HeldWorkspaceDirectory::empty();
        target.capture(&p.target, p.target_id)?;
        pages.capture(&p.pages, p.pages_id)?;
        let mut head = HeldWorkspaceControl::empty();
        let mut archive = HeldWorkspaceControl::empty();
        let mut seed = HeldWorkspaceControl::empty();
        let mut wal = HeldWorkspaceControl::empty();
        head.capture_metadata(&p.paths.head, MAX_TEXT_HEAD_FRAME)?;
        head.read_complete(&p.paths.head)?;
        archive.capture_metadata(&p.paths.archive, MAX_ARCHIVE_PLAIN + 40)?;
        archive.read_complete(&p.paths.archive)?;
        wal.capture_metadata(&p.paths.wal, MAX_COMPLETION_WAL_PLAIN + 40)?;
        wal.read_complete(&p.paths.wal)?;
        seed.capture_metadata(&p.paths.seed, MAX_WORKSPACE_SEED_FRAME)?;
        seed.read_complete(&p.paths.seed)?;
        if head.raw()? != p.head.as_slice()
            || archive.raw()? != p.archive.as_slice()
            || wal.raw()? != self.completion.as_slice()
            || wal.native_data()? != self.completion_native
            || *blake3::hash(seed.raw()?).as_bytes() != self.seed
        {
            return Err(refused());
        }
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        if original_context(guard, &key)? != p.context {
            return Err(refused());
        }
        let mut own_stage = HeldWorkspaceControl::empty();
        let stage_id = if let Some(proof) = stage {
            let (id, cap) = proof.recorded_stage_data()?;
            own_stage.capture_metadata(&p.paths.wal_stage, cap)?;
            match id {
                Some(id) => {
                    if !own_stage.is_present_data() || own_stage.native_data()? != id {
                        return Err(refused());
                    }
                    Some(id)
                }
                None => {
                    own_stage.require_absent(&p.paths.wal_stage)?;
                    None
                }
            }
        } else {
            require_absent(&p.paths.wal_stage)?;
            None
        };
        self.require_absences()?;
        require_absent(&p.paths.seed_stage)?;
        if stage.is_none() {
            require_absent(&p.paths.wal_stage)?;
        }
        require_absent(&p.paths.head_stage)?;
        if stage_id.is_some() {
            require_owned_stage_inventory(&p.target)?;
        } else {
            require_inventory(&p.target)?;
        }
        guard.validate_original_namespace()?;
        head.require_unchanged(&p.paths.head)?;
        archive.require_unchanged(&p.paths.archive)?;
        wal.require_unchanged(&p.paths.wal)?;
        seed.require_unchanged(&p.paths.seed)?;
        target.require_name(&p.target, p.target_id)?;
        pages.require_name(&p.pages, p.pages_id)?;
        self.require_absences()?;
        require_absent(&p.paths.seed_stage)?;
        if stage.is_some() {
            own_stage.require_metadata(&p.paths.wal_stage)?;
            if let Some(id) = stage_id {
                if own_stage.native_data()? != id {
                    return Err(refused());
                }
            }
        } else {
            require_absent(&p.paths.wal_stage)?;
        }
        require_absent(&p.paths.head_stage)?;
        self.base.resident.validate_negative()?;
        guard.validate_source_only()
    }
}
fn require_absent(path: &Path) -> Result<(), AppError> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(refused()),
    }
}

impl CompletedV2TextParentData {
    pub(in crate::store) fn original_head_frame_data(&self) -> &[u8] {
        &self.base.head
    }
    pub(in crate::store) fn original_head_id_data(&self) -> TextHeadIdData {
        self.base.selected
    }
}

/// Only the sealed same-A stage proof reaches this inventory. General completed
/// preCreate still calls the exact nine-name inventory and refuses every stage.
fn require_owned_stage_inventory(target: &Path) -> Result<(), AppError> {
    const NAMES: &[&str] = &[
        "origin",
        "source.seed",
        "source.bin",
        "records.bin",
        "manifest.bin",
        "head.bin",
        "text.seed",
        "text.pages",
        "text.wal",
        "text.wal.stage.v1",
    ];
    let mut seen = 0u16;
    for (index, entry) in std::fs::read_dir(target).map_err(io::io)?.enumerate() {
        if index >= NAMES.len() {
            return Err(refused());
        }
        let name = entry.map_err(io::io)?.file_name();
        let index = NAMES.iter().position(|n| name == *n).ok_or_else(refused)?;
        let bit = 1u16 << index;
        if seen & bit != 0 {
            return Err(refused());
        }
        seen |= bit;
    }
    if seen != (1u16 << NAMES.len()) - 1 {
        return Err(refused());
    }
    Ok(())
}
