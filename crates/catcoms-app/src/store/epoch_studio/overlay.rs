//! No source writes accompany local acceptance. Eligibility is checked under the same
//! exclusive store/group borrow as the shared intent transaction, with no detached gap.
use super::*;
use catcoms_replication::studio::{StudioClosingOverlayBasis, StudioOverlaySave};
use catcoms_replication::{CloseRecord, LocalIntent};

/// The pixel reference a frame operation carries, with its declared size. Index operations and
/// header edits carry none. This only reads the request; possession is a separate question.
fn frame_pixels(
    logical: &LogicalDocument,
    operation: &DomainOp,
) -> Result<Option<(catcoms_storage::Cid, u64)>, AppError> {
    Ok(
        match FlipnoteOp::decode_domain(logical, operation).map_err(invalid)? {
            FlipnoteOp::InsertFrame { cid, bytes, .. }
            | FlipnoteOp::ReplaceFrame { cid, bytes, .. } => {
                Some((catcoms_storage::Cid::from_bytes(cid), bytes))
            }
            _ => None,
        },
    )
}

impl ServerStore {
    /// S1b media admission, for new authoring only. A new local acceptance may not name pixels
    /// the vault does not hold: `operation_blob_cid` extracts an address and the typed layer
    /// checks declared sizes, but neither establishes that the bytes exist. Validate and promote
    /// them into the durable namespace here, before the transient hold and before any detached
    /// work, exactly as the ordinary Apply path does.
    fn admit_studio_frame_pixels(
        &self,
        group: &[u8],
        cid: &catcoms_storage::Cid,
        bytes: u64,
    ) -> Result<(), AppError> {
        let mut blobs = self.blob_store(&hex::encode(group))?;
        let pixels = blobs
            .get_bounded(cid, bytes as usize)?
            .ok_or_else(|| invalid("publish the frame PIX before saving its reference"))?;
        if pixels.len() as u64 != bytes {
            return Err(invalid("frame byte declaration differs from PIX"));
        }
        crate::creative::validate_pix(&pixels)?;
        if pixels[4] != 191 || pixels[5] != 143 {
            return Err(invalid("Flipnote frames must be 192x144"));
        }
        blobs.put_staged(&pixels)?;
        if !blobs.promote_staged_bounded(cid, pixels.len())? {
            return Err(invalid("frame promotion failed"));
        }
        Ok(())
    }

    /// S3 possession recheck, immediately before the first durable acceptance. The transient hold
    /// is a liveness claim over an address, not proof the bytes are still present: external
    /// deletion or storage damage during the detached stage must refuse here rather than produce
    /// a durable draft naming pixels the store does not possess.
    pub(super) fn check_studio_frame_pixels(
        &self,
        group: &[u8],
        cid: &catcoms_storage::Cid,
        bytes: u64,
    ) -> Result<(), AppError> {
        let blobs = self.blob_store(&hex::encode(group))?;
        match blobs.get_bounded(cid, bytes as usize)? {
            Some(pixels) if pixels.len() as u64 == bytes => Ok(()),
            Some(_) => Err(invalid("frame byte declaration differs from PIX")),
            None => Err(invalid(
                "accepted frame PIX is no longer held; republish it",
            )),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn save_studio_closing_overlay(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &CloseRecord,
        tenure: Option<u64>,
        basis: [u8; 32],
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioOverlaySave, AppError> {
        self.save_studio_closing_overlay_with_io(
            server,
            group,
            target,
            device,
            close,
            tenure,
            basis,
            operation,
            ts,
            rng,
            budget,
            atomic_write,
            super::super::epoch_intents::sync_intent,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_studio_closing_overlay(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &CloseRecord,
        tenure: Option<u64>,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioClosingOverlayBasis, AppError> {
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        let tenure =
            tenure.ok_or_else(|| invalid("Closing overlay needs observed owner tenure"))?;
        let (mut source, observed, _) =
            self.checked_studio_source(server, group, target, device, false, &mut budget.storage)?;
        if observed.is_none() {
            return Err(invalid("Closing overlay source is missing"));
        }
        source
            .prepare_closing_overlay(close, group, tenure)
            .map_err(invalid)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn save_studio_closing_overlay_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &CloseRecord,
        tenure: Option<u64>,
        basis: [u8; 32],
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioOverlaySave, AppError> {
        current_member(group, device)?;
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        // Bound the caller's public Vec before making a LocalIntent copy.
        match target {
            StudioTarget::Index { .. } => {
                IndexOp::decode_domain(&logical, &operation, &device.device_id())
                    .map_err(invalid)?;
            }
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::decode_domain(&logical, &operation).map_err(invalid)?;
            }
        }
        self.enter_studio_budget(server, group, budget)?;
        let state = self.checked_epoch_replay_state(
            server,
            &logical,
            &mut budget.storage,
            &mut budget.intents,
        )?;
        let intent = LocalIntent {
            author: device.device_id(),
            operation: operation.clone(),
        };
        if let Some(metadata) = state.handoff_metadata() {
            if let Some(outcome) = metadata
                .completed_retry(target, basis, &intent)
                .map_err(invalid)?
            {
                let scope = super::super::epoch_intents::scope_bytes(server, &logical)?;
                let (_, old) = self.read_epoch_intent_record(&scope, &logical)?;
                self.write_prepared_intents(
                    server,
                    &logical,
                    state,
                    old,
                    true,
                    rng,
                    &mut budget.storage,
                    &mut budget.intents,
                    writer,
                    sync,
                )?;
                return Ok(StudioOverlaySave::HandedOff(outcome));
            }
        }
        let exact = match state.overlay() {
            Some(overlay) if overlay.target() == target => {
                overlay.exact_retry(basis, &intent).map_err(invalid)?
            }
            Some(_) => return Err(invalid("overlay belongs to another channel")),
            None => false,
        };
        // Equal nonce and body from an ordinary failed Save is not accepted local draft evidence.
        // That is classification rather than authoring, so it also precedes media admission; the
        // writer keeps its own copy of this check as defence in depth.
        if !exact
            && state
                .pending()
                .any(|(id, _)| *id == intent.operation.id(&intent.author))
        {
            return Err(invalid("ordinary intent cannot become an accepted overlay"));
        }
        // Recognize accepted retries BEFORE first/append eligibility. An installed successor,
        // a later fault or Unknown tenure cannot turn a saved exact request into a new append.
        if exact {
            return self
                .write_studio_overlay_intent(
                    server,
                    &logical,
                    target,
                    device,
                    group,
                    basis,
                    None,
                    operation,
                    ts,
                    rng,
                    &mut budget.storage,
                    &mut budget.intents,
                    writer,
                    sync,
                )
                .map(StudioOverlaySave::Local);
        }
        // Everything from here is new authoring, so this is where media admission belongs. An
        // already accepted request must never be refused because the reference rails are full or
        // because its pixels were legitimately reclaimed after retirement; classification above is
        // deliberately cheap and reaches no blob.
        //
        // I-3, first half: until this acceptance is durable, nothing in the vault names the
        // operation's pixels, so a complete reference scan would install a set without them and
        // they would become reclaimable. The hold is owned by the capture, carried through the
        // detached stage, and released only when the commit returns.
        let frame = match target {
            StudioTarget::Flipnote { .. } => frame_pixels(&logical, &operation)?,
            StudioTarget::Index { .. } => None,
        };
        // S1b: validate and promote the referenced pixels into the durable namespace BEFORE the
        // hold, so a reference to bytes the vault never held is refused here rather than becoming
        // a durable local draft. Only new authoring reaches this.
        if let Some((cid, bytes)) = &frame {
            self.admit_studio_frame_pixels(&logical.server_id, cid, *bytes)?;
        }
        let pixels: std::collections::BTreeSet<[u8; 32]> =
            catcoms_replication::studio::operation_blob_cid(&operation)
                .map_err(invalid)?
                .into_iter()
                .collect();
        let pixels = if pixels.is_empty() {
            None
        } else {
            Some(self.hold_creative_transient(&logical.server_id, pixels)?)
        };
        let tenure_value =
            tenure.ok_or_else(|| invalid("Closing overlay needs observed owner tenure"))?;
        let (mut source, observed, _) =
            self.checked_studio_source(server, group, target, device, false, &mut budget.storage)?;
        if observed.is_none() {
            return Err(invalid("Closing overlay source is missing"));
        }
        let fresh = source
            .prepare_closing_overlay(close, group, tenure_value)
            .map_err(invalid)?;
        if fresh.fingerprint() != basis {
            return Err(invalid("Closing overlay basis changed"));
        }
        drop(source);
        // The three staged seams, composed inline. A scheduled caller runs the same three in the
        // same order with custody released around `plan`; there is no second algorithm.
        let capture = self.capture_studio_overlay_save(
            server, group, target, device, fresh, intent, ts, pixels, frame,
        )?;
        let plan = capture.plan()?;
        self.commit_studio_overlay_save(
            server, group, target, device, close, tenure, plan, rng, budget, writer, sync,
        )
        .map(StudioOverlaySave::Local)
    }
}
