//! Cover the one exact Head2 candidate while the original Head remains selected.
use super::*;
use selected_head::GovernedHeadData;

pub(super) struct PreparedHeadStage {
    id: TextHeadIdData,
    record: PreparedSmallControl,
}

impl PreparedGovernedAddFence {
    pub(in crate::store) fn prepare_head_stage(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        self.held
            .resources
            .require_same_original(selected.resources())?;
        let archive = self.pending_archive()?;
        if !archive.record.covered
            || archive.labels.before_head != selected.head_id_data()
            || archive.labels.before_archive != selected.head_data().archive
            || archive.labels.context != selected.head_data().context.id_data()?
        {
            return Err(refused());
        }
        if archive.head.is_none() {
            let head = GovernedHeadData::after_archive(
                archive.labels,
                archive.id,
                selected.head_data().generation,
            )?;
            let key = Zeroizing::new(store.keys.db_key()?);
            let frame = head.seal_data(&key, rng)?;
            archive.head = Some(PreparedHeadStage {
                id: TextHeadIdData::of_data(&frame),
                record: PreparedSmallControl::new(frame),
            });
        }
        self.held
            .resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn cover_head_stage(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<TextHeadIdData, AppError> {
        self.cover_archive(guard, selected)?;
        let paths = &self.held.welcome_paths;
        let head = self
            .header
            .as_mut()
            .and_then(|header| {
                header
                    .welcome_signing
                    .as_mut()?
                    .response
                    .as_mut()?
                    .archive
                    .as_mut()?
                    .head
                    .as_mut()
            })
            .ok_or_else(refused)?;
        if !head.record.covered {
            head.record
                .cover(&paths.head_stage, selected_head::FRAME_LEN, guard)?;
            self.cover_archive(guard, selected)?;
        }
        let paths = &self.held.welcome_paths;
        let head = self
            .header
            .as_mut()
            .and_then(|header| {
                header
                    .welcome_signing
                    .as_mut()?
                    .response
                    .as_mut()?
                    .archive
                    .as_mut()?
                    .head
                    .as_mut()
            })
            .ok_or_else(refused)?;
        head.record.require_covered(&paths.head_stage)?;
        guard.validate_source_only()?;
        Ok(head.id)
    }
}
