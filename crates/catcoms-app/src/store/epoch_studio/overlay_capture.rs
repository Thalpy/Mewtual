//! Staged local acceptance: cheap classification and capture under custody, the expensive typed
//! append on a detached worker, then a stamp-checked commit. The synchronous adapter composes
//! these three stages in order, so there is one algorithm rather than a batch path that can drift
//! from the scheduled one.
//!
//! Ordering is load bearing and is the AG1-001/I3-001 boundary: classification is cheap,
//! structural and reaches no blob; media admission and the job-owned reference hold belong to the
//! new-authoring path only, and are taken before custody is released, never before an accepted
//! request has been recognised.
use super::*;
use crate::store::creative_references::CreativeHold;
use crate::store::epoch_intents::{self, EpochIntentState};
use catcoms_crypto::DeviceId;
use catcoms_replication::studio::{StudioClosingOverlayBasis, StudioLocalDraft};
use catcoms_replication::LocalIntent;

/// Record identity and public live context, captured under custody. It carries no plaintext, no
/// key, no store handle, no Server and no budget, so it is safe to hold across a detach.
pub(crate) struct StudioOverlayStamp {
    mount: Arc<()>,
    server: u64,
    document: LogicalDocument,
    target: StudioTarget,
    actor: DeviceId,
    actor_key: Vec<u8>,
    owner: DeviceId,
    mls: u64,
    /// Captured absence is explicit and must remain absence; it is not the same as unreadable.
    intent: Option<(blake3::Hash, u64)>,
}

impl std::fmt::Debug for StudioOverlayStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioOverlayStamp { .. }")
    }
}

/// Authenticated zeroizing plaintext, public context, the private Closing basis and the job-owned
/// media hold. The basis grants permission to prepare local draft data only; the commit re-derives
/// and re-matches it from actual durable state before anything is written.
pub(crate) struct StudioOverlayCapture {
    stamp: StudioOverlayStamp,
    intent_bytes: Option<Zeroizing<Vec<u8>>>,
    basis: StudioClosingOverlayBasis,
    intent: LocalIntent,
    ts: u64,
    pixels: Option<CreativeHold>,
}

impl std::fmt::Debug for StudioOverlayCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioOverlayCapture { .. }")
    }
}

/// A proposal, never a durable effect. It becomes durable only after a custody visit
/// reauthenticates the exact record it was derived from and re-mints the basis.
pub(crate) struct StudioOverlayPlan {
    stamp: StudioOverlayStamp,
    state: EpochIntentState,
    draft: StudioLocalDraft,
    pixels: Option<CreativeHold>,
}

impl std::fmt::Debug for StudioOverlayPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioOverlayPlan { .. }")
    }
}

impl StudioOverlayCapture {
    /// Runs on a blocking worker, off actor and store custody. It owns authenticated plaintext,
    /// public context, the basis and the media hold; it owns no store, Server, device key, MLS
    /// secret or source writer, and it writes nothing.
    ///
    /// This is the expensive stage: `EpochIntentState::decode` reconstructs any retained branch
    /// and `append` replays it again with the new operation, which is the cost the runtime exists
    /// to move off custody.
    pub(crate) fn plan(self) -> Result<StudioOverlayPlan, AppError> {
        let scope = epoch_intents::scope_bytes(self.stamp.server, &self.stamp.document)?;
        let mut state = match &self.intent_bytes {
            Some(bytes) => EpochIntentState::decode(bytes, &scope, &self.stamp.document)?,
            None => EpochIntentState {
                ledger: catcoms_replication::IntentLedger::new(self.stamp.document.clone()),
                overlay: None,
            },
        };
        let op_id = self.intent.operation.id(&self.intent.author);
        if let Some(metadata) = &state.overlay {
            if metadata.target() != self.stamp.target {
                return Err(invalid("overlay belongs to another channel"));
            }
        }
        // Repeated from the classification stage: a worker must not rely on a decision taken
        // against bytes it cannot itself see.
        if state.pending().any(|(id, _)| *id == op_id) {
            return Err(invalid("ordinary intent cannot become an accepted overlay"));
        }
        let mut overlay = state
            .overlay
            .clone()
            .unwrap_or_else(|| catcoms_replication::studio::StudioOverlayState::new(&self.basis));
        state
            .ledger
            .prepare(self.intent.author, self.intent.operation.clone())
            .map_err(invalid)?;
        let draft = overlay
            .append(&self.basis, &state.ledger, op_id, self.ts)
            .map_err(invalid)?;
        state.overlay = Some(overlay);
        Ok(StudioOverlayPlan {
            stamp: self.stamp,
            state,
            draft,
            pixels: self.pixels,
        })
    }
}

impl ServerStore {
    /// Capture under custody, after classification has already proved this is new authoring.
    /// The caller supplies the media hold it took for exactly that reason.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_studio_overlay_save(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        basis: StudioClosingOverlayBasis,
        intent: LocalIntent,
        ts: u64,
        pixels: Option<CreativeHold>,
    ) -> Result<StudioOverlayCapture, AppError> {
        current_member(group, device)?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let scope = epoch_intents::scope_bytes(server, &document)?;
        // Authenticate the bounded record without decoding it: the detached stage owns the decode.
        let record = self.read_scoped_intent_plain(&scope)?;
        Ok(StudioOverlayCapture {
            stamp: StudioOverlayStamp {
                mount: self.registry_mount(),
                server,
                document,
                target,
                actor: device.device_id(),
                actor_key: device.public_key_bytes(),
                owner: group
                    .designated_committer()
                    .ok_or_else(|| invalid("no current owner"))?,
                mls: group.epoch(),
                intent: record
                    .as_ref()
                    .map(|r| (blake3::hash(&r.plain), r.physical_bytes)),
            },
            intent_bytes: record.map(|r| r.plain),
            basis,
            intent,
            ts,
            pixels,
        })
    }

    /// Reacquired custody. Compares mount identity, numeric server, complete target, document,
    /// actor and key, designated owner, MLS epoch, and the complete authenticated plaintext digest
    /// with its physical size. Captured absence must remain absence. It does not decode.
    pub(crate) fn studio_overlay_is_current(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        stamp: &StudioOverlayStamp,
    ) -> Result<bool, AppError> {
        if !Arc::ptr_eq(&stamp.mount, &self.registry_mount())
            || stamp.document.server_id != group.group_id()
            || stamp.actor != device.device_id()
            || stamp.actor_key != device.public_key_bytes()
            || Some(stamp.owner) != group.designated_committer()
            || stamp.mls != group.epoch()
        {
            return Ok(false);
        }
        let scope = epoch_intents::scope_bytes(stamp.server, &stamp.document)?;
        let record = self.read_scoped_intent_plain(&scope)?;
        let version = record
            .as_ref()
            .map(|r| (blake3::hash(&r.plain), r.physical_bytes));
        Ok(version == stamp.intent)
    }

    /// Commit a planned acceptance. Re-mints the Closing basis from actual durable state and
    /// requires the same fingerprint, so the accepted rule "recheck the same source and authority
    /// immediately before the first durable acceptance" survives the detach. Then installs the
    /// ordinary conservative reference holds (I-3) and performs one accounted intent write.
    ///
    /// The plan's media hold is released only when this call returns, on success, error and
    /// unwinding alike.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_studio_overlay_save(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &catcoms_replication::CloseRecord,
        tenure: Option<u64>,
        plan: StudioOverlayPlan,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioLocalDraft, AppError> {
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        let StudioOverlayPlan {
            stamp,
            state,
            draft,
            pixels,
        } = plan;
        if stamp.server != server || stamp.target != target {
            return Err(invalid("overlay plan belongs to another target"));
        }
        if !self.studio_overlay_is_current(group, device, &stamp)? {
            return Err(invalid("overlay record or context changed; retry"));
        }
        let document = stamp.document.clone();
        // Re-derive the basis under the same custody as the write. A changed Closing source, a
        // replaced signed close, a changed owner or an Unknown tenure all discard the plan.
        let tenure =
            tenure.ok_or_else(|| invalid("Closing overlay needs observed owner tenure"))?;
        let (mut source, observed, _) =
            self.checked_studio_source(server, group, target, device, false, &mut budget.storage)?;
        if observed.is_none() {
            return Err(invalid("Closing overlay source is missing"));
        }
        let fresh = source
            .prepare_closing_overlay(close, group, tenure)
            .map_err(invalid)?;
        if fresh.fingerprint() != draft.basis() {
            return Err(invalid("Closing overlay basis changed"));
        }
        drop(source);
        // I-3, second half: the protection transfer. These run BEFORE the write attempt and while
        // the plan's job-owned hold is still alive, so dropping that hold below is safe whatever
        // the write does. A durable record alone does not repair a reference set that a complete
        // scan installed while this acceptance was in flight.
        self.hold_creative(
            &document.server_id,
            state
                .overlay()
                .ok_or_else(|| invalid("overlay base missing"))?
                .base_blob_cids()
                .map_err(invalid),
        );
        for (id, intent) in state.pending() {
            if state.is_overlay(id) {
                self.hold_creative_operation(&document, &intent.operation);
            }
        }
        let old = stamp.intent.map(|(_, physical)| physical);
        let written = self.write_prepared_intents(
            server,
            &document,
            state,
            old,
            false,
            rng,
            &mut budget.storage,
            &mut budget.intents,
            writer,
            sync,
        );
        // Explicit: the hold outlives the write attempt, including its error path.
        drop(pixels);
        written?;
        Ok(draft)
    }
}
