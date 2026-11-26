//! Cover the one exact Head2 candidate while the original Head remains selected.
use super::*;
use selected_head::GovernedHeadData;

mod selection;

pub(super) struct PreparedHeadStage {
    id: TextHeadIdData,
    record: PreparedSmallControl,
    selection_entered: bool,
}

impl PreparedHeadStage {
    pub(super) fn from_frame(frame: Zeroizing<Vec<u8>>) -> Self {
        Self {
            id: TextHeadIdData::of_data(&frame),
            record: PreparedSmallControl::new(frame),
            selection_entered: false,
        }
    }
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
        let parent = self.held.writer_parent(selected)?;
        let (context, head, before_archive) = (parent.context, parent.head, parent.archive);
        let archive = self.pending_archive()?;
        if !archive.record.covered
            || archive.labels.before_head != head
            || archive.labels.before_archive != before_archive
            || archive.labels.context != context
        {
            return Err(refused());
        }
        if archive.head.is_none() {
            let head = GovernedHeadData::after_first_archive(
                archive.labels,
                archive.id,
                selected.head_data().generation,
            )?;
            let key = Zeroizing::new(store.keys.db_key()?);
            let frame = head.seal_data(&key, rng)?;
            archive.head = Some(PreparedHeadStage::from_frame(frame));
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
