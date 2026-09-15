//! Same-epoch catch-up admission: one page, one checked owned source, one atomic save/flush.
//! The caller advances its provider cursor only after success. This is not a network scheduler.
use super::*;
use catcoms_replication::studio::catchup::{
    StudioFrontier, StudioPageOutcome, StudioPageProvider, StudioPageRequest,
    MAX_STUDIO_PAGE_BYTES, MAX_STUDIO_PAGE_OPS,
};

/// Locally durable page counts and the resulting verified frontier. Completion of a provider's
/// prefix is not owner settlement, peer delivery or proof of global currency. No intent retires.
#[derive(Debug)]
pub struct StudioPageAdmission {
    pub accepted: usize,
    pub duplicates: usize,
    pub frontier: StudioFrontier,
}

impl ServerStore {
    /// Cooperative trusted-local provider adapter. The caller authenticates requester transport
    /// identity and owns provider lifetime/rate limits/native custody; this registers no handler.
    /// Check membership/MAC/expiry before I/O, then serve only the exact warm owned source.
    /// Missing/stale/cold sources refuse locally, never report an empty remote document or rebuild
    /// inside an admitted request. The provider secret must be reminted on runtime/mount restart.
    #[allow(clippy::too_many_arguments)]
    pub fn serve_studio_page(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        provider: &mut StudioPageProvider,
        request: StudioPageRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<StudioPageOutcome, AppError> {
        if !provider
            .preflight_request(group, device, target, &request)
            .map_err(invalid)?
        {
            return Ok(StudioPageOutcome::Restart);
        }
        self.with_prepared_studio_source(server, group, target, device, |state| {
            provider
                .page(&state.unit, group, device, request, rng)
                .map_err(invalid)
        })
    }

    /// Admit a complete bounded page under exclusive store/current-actor custody. Every entry
    /// uses the existing author/DAG/domain/preflight gate; a bad middle entry saves no prefix.
    /// Empty/duplicate pages verify the exact Open epoch and flush existing bytes too. Actual
    /// absence at epoch zero stays absent. No raw seed or snapshot is accepted by this API.
    ///
    /// An I/O error may leave either the OLD source or the COMPLETE new page after atomic rename.
    /// Reconcile uncertain accounting, then retry the same page without advancing its cursor.
    /// Changed sources and previously warm sources return to the existing one-slot policy for
    /// the next page. An unchanged cold flush has no warm stamp and need not populate the slot.
    /// Cold sources retain the local 256-KiB rail; explicit preparation is separate, never an
    /// implicit expensive retry under a remote request deadline.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest_studio_page(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        expected_doc_id: u128,
        device: &MlsDevice,
        operations: &[SealedOp],
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioPageAdmission, AppError> {
        self.ingest_studio_page_with_io(
            server,
            group,
            target,
            expected_doc_id,
            device,
            operations,
            rng,
            budget,
            atomic_write,
            sync_studio,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn ingest_studio_page_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        expected_doc_id: u128,
        device: &MlsDevice,
        operations: &[SealedOp],
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioPageAdmission, AppError> {
        if operations.len() > MAX_STUDIO_PAGE_OPS {
            return Err(invalid("Studio page exceeds operation cap"));
        }
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        let mut bytes = 0;
        for op in operations {
            if op.doc_type != logical.doc_type
                || op.doc_id != expected_doc_id
                || op.epoch != group.epoch()
                || op.blob.ciphertext.len() > MAX_INBOUND_CIPHERTEXT
            {
                return Err(invalid("Studio page operation scope or size is invalid"));
            }
            // Fixed SealedOp framing plus the page entry's u32 length; no encoding allocation.
            bytes += op.blob.ciphertext.len() + 62;
            if bytes > MAX_STUDIO_PAGE_BYTES {
                return Err(invalid("Studio page exceeds byte cap"));
            }
        }
        let source::CheckedReceiveSource {
            mut unit,
            observed,
            before,
            version,
        } = self.checked_studio_receive_source(server, group, target, device, budget)?;
        if unit.doc_id() != expected_doc_id || unit.phase() != EpochPhase::Open {
            return Err(invalid("Studio page target is replaced or not open"));
        }
        let mut accepted = 0;
        let mut duplicates = 0;
        for op in operations {
            match unit.ingest(op, group, device).map_err(invalid)? {
                Admission::Accepted => accepted += 1,
                Admission::Duplicate => duplicates += 1,
                _ => return Err(invalid("Studio page was not admitted")),
            }
        }
        if observed.is_none() && operations.is_empty() {
            return Ok(StudioPageAdmission {
                accepted,
                duplicates,
                frontier: unit.catchup_frontier(),
            });
        }
        let mut state = self.save_studio_source_reusing(
            server,
            unit,
            observed,
            &before,
            WritePurpose::Ordinary,
            rng,
            &mut budget.storage,
            writer,
            sync,
            version,
        )?;
        let result = StudioPageAdmission {
            accepted,
            duplicates,
            frontier: state.unit.catchup_frontier(),
        };
        self.retain_received_studio_source(group, device, state);
        Ok(result)
    }
}
