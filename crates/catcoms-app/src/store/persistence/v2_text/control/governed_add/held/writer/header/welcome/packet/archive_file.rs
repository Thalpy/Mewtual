//! Cover the immutable archive while the original selected parent and full
//! Add journal are still held. The archive ID alone cannot select a head.
use super::*;
use crate::store::persistence::v2_text::control::governed_add::archive::{
    self, GovernedArchiveFrames, GovernedArchiveLabels,
};
use crate::store::persistence::v2_text::io::archive_child;
use catcoms_replication::chat::cold_manifest::ArchiveHeadId;

mod head_stage;

pub(super) struct PreparedArchive {
    record: PreparedSmallControl,
    path: PathBuf,
    id: ArchiveHeadId,
    labels: GovernedArchiveLabels,
    head: Option<head_stage::PreparedHeadStage>,
    _fee: CatalogReadReservation,
}

impl PreparedGovernedAddFence {
    pub(in crate::store) fn prepare_archive(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        outcome: &V2FounderAddOutcomeData<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        self.held
            .resources
            .require_same_original(selected.resources())?;
        let original = self
            .outcome
            .as_ref()
            .filter(|original| original.covered)
            .ok_or_else(refused)?;
        let unsigned = outcome.unsigned_data();
        if original.header_intent != unsigned.intent_commitment_data()
            || unsigned.header_spec_data().parent != selected.archive_data().spec_data().header.id()
            || self.held.parent
                != (
                    selected.head_data().context.id_data()?,
                    selected.head_id_data(),
                )
        {
            return Err(refused());
        }
        let header = self.header.as_mut().ok_or_else(refused)?;
        let (id, governing) = header.governing.as_ref().ok_or_else(refused)?;
        let (_, raw_header) = header.raw.as_ref().ok_or_else(refused)?;
        let (_, welcome_intent) = header.welcome.as_ref().ok_or_else(refused)?;
        let welcome = header.welcome_signing.as_mut().ok_or_else(refused)?;
        let (_, raw_welcome) = welcome.raw.as_ref().ok_or_else(refused)?;
        let response = welcome.response.as_mut().ok_or_else(refused)?;
        let labels = GovernedArchiveLabels {
            context: selected.head_data().context.id_data()?,
            before_head: selected.head_id_data(),
            before_archive: selected.head_data().archive,
            header: *id,
            state: unsigned.header_spec_data().state,
            epoch: outcome.result_data().after_epoch_data(),
            authenticator: *outcome.result_data().after_authenticator_data(),
        };
        if let Some(prepared) = &response.archive {
            if prepared.labels != labels {
                return Err(refused());
            }
        } else {
            let frames = GovernedArchiveFrames {
                fence: &self
                    .held
                    .original
                    .as_ref()
                    .filter(|_| self.held.authenticated)
                    .ok_or_else(refused)?
                    .frame,
                request: self.request.covered_frame()?,
                entry: self.entry.as_ref().ok_or_else(refused)?.covered_frame()?,
                outcome: &original.original.as_ref().ok_or_else(refused)?.frame,
                header_attempt: header.attempt.covered_frame()?,
                raw_header: raw_header.covered_frame()?,
                governing: governing.covered_frame()?,
                welcome_intent: welcome_intent.covered_frame()?,
                welcome_attempt: welcome.attempt.covered_frame()?,
                raw_welcome: raw_welcome.covered_frame()?,
                response: response.record.covered_frame()?,
            };
            let bytes = 5 * frames.frame_bytes()?
                + MAX_TEXT_PATH
                + 6 * selected_head::FRAME_LEN
                + 4 * crate::store::persistence::v2_text::head::MAX_TEXT_HEAD_FRAME
                + 4 * std::mem::size_of::<PreparedArchive>()
                + 4096;
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            let fee = pool.try_reserve(bytes)?;
            fee.validate(pool, bytes)?;
            let key = Zeroizing::new(store.keys.db_key()?);
            let frame = archive::seal_data(&key, labels, &frames, rng)?;
            let id = archive::id_data(&frame);
            let path = archive_child(selected.physical_query_data().target, id.as_bytes())?;
            response.archive = Some(PreparedArchive {
                record: PreparedSmallControl::new(frame),
                path,
                id,
                labels,
                head: None,
                _fee: fee,
            });
        }
        self.held
            .resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn cover_archive(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<ArchiveHeadId, AppError> {
        self.cover_response(guard, selected)?;
        let head_stage = &self.held.welcome_paths.head_stage;
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
        if !archive.record.covered {
            require_absent(head_stage)?;
            archive
                .record
                .cover(&archive.path, archive::MAX_BYTES, guard)?;
            self.cover_response(guard, selected)?;
        }
        let archive = self.pending_archive()?;
        archive.record.require_covered(&archive.path)?;
        guard.validate_source_only()?;
        Ok(archive.id)
    }

    fn pending_archive(&mut self) -> Result<&mut PreparedArchive, AppError> {
        self.header
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
            .ok_or_else(refused)
    }
}
