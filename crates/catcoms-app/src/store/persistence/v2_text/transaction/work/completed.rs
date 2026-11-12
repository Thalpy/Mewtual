//! Finite completed-source WA replacement. Actual sameWA continuation is first;
//! the previous exact WC is required only before this original WA reaches MAIN.
//! No missing journal, decoder error or parsed WC supplies permission.
use super::io as native;
use super::*;
use catcoms_storage::kept::{session_at, SessionAt};
impl PreparedV2TextRetainTransaction {
    pub(in crate::store::persistence::v2_text::transaction) fn ensure_completed_wal(
        &self,
        slots: &mut RetainSlots,
        #[cfg(test)] hooks: &mut RetainTestHooks,
    ) -> Result<(), AppError> {
        let completed = self.parent.completed.as_ref().ok_or_else(refused)?;
        completed.validate_prepaid_account()?;
        let expected = self.wal.as_ref().ok_or_else(refused)?.frame_data();
        if slots
            .wal_native_data()
            .is_some_and(|id| matches!(session_at(&self.parent.wal_path, id), SessionAt::Ours))
        {
            native::require_absent(&self.parent.wal_stage_path)?;
            let main =
                native::sync_exact(&self.parent.wal_path, slots.wal_native_data(), expected)?;
            native::sync_parent(&self.parent.target, self.parent.target_id)?;
            return require_named(&self.parent.wal_path, &main);
        }
        if !slots.no_dependents_data() {
            return Err(refused());
        }
        let previous = native::sync_exact(
            &self.parent.wal_path,
            Some(completed.completion_native_data()),
            completed.completion_frame_data(),
        )?;
        super::completed_continuation::write_original(
            &self.parent.wal_stage_path,
            expected,
            &mut slots.wal,
            || {
                #[cfg(test)]
                hooks.hit(V2TextRetainTestCut::WalCreated)?;
                Ok(())
            },
        )?;
        #[cfg(test)]
        hooks.hit(V2TextRetainTestCut::WalStaged)?;
        native::sync_parent(&self.parent.target, self.parent.target_id)?;
        let stage = native::require_exact(
            &self.parent.wal_stage_path,
            slots.wal_native_data(),
            expected,
        )?;
        require_named(&self.parent.wal_path, &previous)?;
        require_named(&self.parent.wal_stage_path, &stage)?;
        std::fs::rename(&self.parent.wal_stage_path, &self.parent.wal_path)
            .map_err(|e| AppError::Io(e.to_string()))?;
        #[cfg(test)]
        hooks.hit(V2TextRetainTestCut::WalRenamed)?;
        native::sync_parent(&self.parent.target, self.parent.target_id)?;
        require_named(&self.parent.wal_path, &stage)?;
        native::require_absent(&self.parent.wal_stage_path)
    }
}
