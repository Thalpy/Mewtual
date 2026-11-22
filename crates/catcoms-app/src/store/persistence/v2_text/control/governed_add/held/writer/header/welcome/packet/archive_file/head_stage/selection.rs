//! Physical CAS of the one staged Head2 and late reproof after any lost
//! rename success. The fixed Add journal is retained through installation.
use super::*;
use catcoms_sync::owner_v2::V2InstalledFounderAddData;

impl PreparedGovernedAddFence {
    pub(in crate::store) fn head_selection_entered(&mut self) -> bool {
        self.pending_archive()
            .ok()
            .and_then(|archive| archive.head.as_ref())
            .is_some_and(|head| head.selection_entered)
    }

    pub(in crate::store) fn select_head2(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
        installed: V2InstalledFounderAddData,
    ) -> Result<TextHeadIdData, AppError> {
        self.held
            .resources
            .require_same_original(selected.resources())?;
        self.held
            .resources
            .require_same_original(guard.resources())?;
        guard.require_governed_native_invite_parent()?;
        guard.require_text_control_io_quiescent()?;
        let parent = selected.physical_query_data();
        let (target, directory, id) = guard.target_directory_data();
        if parent.target != target || parent.target_id != id {
            return Err(refused());
        }
        let mut current = read_control(
            parent.head_path,
            crate::store::persistence::v2_text::head::MAX_TEXT_HEAD_FRAME,
        )?;
        let parent_is_selected = current.frame.as_slice() == parent.head_frame;
        if parent_is_selected {
            self.cover_head_stage(guard, selected)?;
        }
        let stage = &self.held.welcome_paths.head_stage;
        let archive = self
            .header
            .as_mut()
            .and_then(|header| {
                header
                    .welcome_signing
                    .as_mut()?
                    .response
                    .as_mut()?
                    .archive
                    .as_mut()
            })
            .ok_or_else(refused)?;
        if archive.labels.before_head != selected.head_id_data()
            || archive.labels.before_archive != selected.head_data().archive
            || archive.labels.context != selected.head_data().context.id_data()?
            || installed.header != archive.labels.header
            || installed.state != archive.labels.state
            || installed.epoch != archive.labels.epoch
            || installed.authenticator != archive.labels.authenticator
        {
            return Err(refused());
        }
        let head = archive.head.as_mut().ok_or_else(refused)?;
        archive.record.require_covered(&archive.path)?;
        if parent_is_selected {
            head.record.require_covered(stage)?;
            crate::store::persistence::v2_genesis_activation::require_directory_name(
                target, directory, id,
            )?;
            require_exact_held(parent.head_path, &mut current)?;
            guard.validate_source_only()?;
            head.selection_entered = true;
            fs::rename(stage, parent.head_path).map_err(io)?;
        } else if !head.selection_entered
            || current.frame.as_slice() != head.record.covered_frame()?
        {
            return Err(refused());
        }
        require_absent(stage)?;
        head.record.require_covered(parent.head_path)?;
        crate::store::persistence::sync_directory(target).map_err(io)?;
        let saved = head.id;
        self.require_selected_head2(guard, selected, saved)?;
        Ok(saved)
    }

    pub(in crate::store) fn require_selected_head2(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
        saved: TextHeadIdData,
    ) -> Result<(), AppError> {
        self.held
            .resources
            .require_same_original(selected.resources())?;
        self.held
            .resources
            .require_same_original(guard.resources())?;
        guard.require_governed_native_invite_parent()?;
        guard.require_text_control_io_quiescent()?;
        require_absences(&self.other_paths)?;
        let parent = selected.physical_query_data();
        let (target, directory, id) = guard.target_directory_data();
        if parent.target != target || parent.target_id != id {
            return Err(refused());
        }
        let stage = &self.held.welcome_paths.head_stage;
        let archive = self
            .header
            .as_mut()
            .and_then(|header| {
                header
                    .welcome_signing
                    .as_mut()?
                    .response
                    .as_mut()?
                    .archive
                    .as_mut()
            })
            .ok_or_else(refused)?;
        if archive.labels.before_head != selected.head_id_data()
            || archive.labels.before_archive != selected.head_data().archive
        {
            return Err(refused());
        }
        let head = archive
            .head
            .as_mut()
            .filter(|head| head.selection_entered && head.id == saved)
            .ok_or_else(refused)?;
        require_absent(stage)?;
        archive.record.require_covered(&archive.path)?;
        head.record.require_covered(parent.head_path)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()
    }
}
