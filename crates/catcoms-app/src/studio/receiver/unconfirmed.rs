//! Design 8.7: Save on an awaiting-tenure preview, scheduled through the actor.
//!
//! Agent 1's Flow S, unchanged, with one substitution: the basis is an Unconfirmed mint from this
//! actor's current ready preview instead of a Closing one from the installed source. Classification,
//! acknowledgements, exact retries and the ordinary-pending refusal all run inside the store before
//! the mint attempt is opened, so a preview that has expired, been evicted or vanished on restart
//! blocks new authoring and nothing else.
//!
//! **Each stage mints its own attempt.** The begin, the first custody visit (S1b) and the commit
//! visit (S3) each call [`PreviewRuntime::mint`](super::catchup) at the moment they run. Nothing
//! carries a minted basis from one visit to the next. The store's MLS-epoch guard catches a basis
//! kept across a membership change, but not one kept past its preview's expiry within an epoch, so
//! this is the half of that freshness that only the caller can provide.
//!
//! **The overlay slot is one per actor.** It holds at most one parked plan, for one target, and
//! refuses every reservation while it does. So a visit finishes whatever plan is parked, for any
//! target and any request, before it does its own work. A caller who never returns therefore
//! cannot hold the slot. A visit reports only its own request's outcome; another request's finished
//! work is that request's to learn on its own retry.
//!
//! No tenure is read anywhere here: an Unconfirmed branch has none (design 8.5).
use super::*;
use crate::store::{EpochStudioBudget, StudioOverlayMint, StudioOverlayStart};
use catcoms_replication::studio::{StudioOverlayProvenance, StudioOverlaySave};
use catcoms_replication::DomainOp;

impl StudioReceiver {
    /// The two control actions of design 8.7, intercepted before the control transaction because
    /// only the receiver holds the ready preview.
    ///
    /// The same scope checks the transaction makes first: the request's own bounds, then a channel
    /// this server knows. The budget (a completed five-family inventory, as for every other
    /// lifecycle control) is taken only by a stage that will use it. A visit that finds the slot
    /// busy costs no scan; one that finishes another request's parked plan pays that commit's.
    pub(super) fn unconfirmed_save_control<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        request: StudioControlRequest,
    ) -> Result<StudioControlResponse, AppError> {
        request.validate()?;
        let target = request.target;
        if !server
            .channels()
            .iter()
            .any(|c| c.id == u128::from_be_bytes(target.channel()))
        {
            return Err(invalid("unknown Studio channel"));
        }
        match request.action {
            StudioControlAction::BeginUnconfirmedOverlaySave => {
                // Every Save under a ticket for a Closing draft would be refused anyway; naming
                // the reason here, before a ticket exists, is the truthful place for it.
                refuse_closing_draft(server, store, id, target)?;
                let (basis, branch) =
                    self.begin_unconfirmed_overlay_save(server, store, id, target)?;
                Ok(StudioControlResponse::UnconfirmedOverlaySaveTicket {
                    target,
                    basis,
                    branch,
                })
            }
            StudioControlAction::SaveUnconfirmedOverlay(save) => {
                let save = *save;
                // A Closing draft's accepted operation, resent through this action, would be
                // acknowledged by the store's kind-blind exact retry and then reported here as
                // Unconfirmed work. Refused first instead, by what the document's live draft is
                // (review of `b35e23d2`, MEDIUM-1).
                refuse_closing_draft(server, store, id, target)?;
                let operation = crate::studio::domain(target, save.nonce, save.body);
                let visit = self.save_unconfirmed_overlay(
                    server,
                    store,
                    id,
                    target,
                    save.basis,
                    save.branch,
                    operation,
                )?;
                let outcome = match visit {
                    StudioOverlaySaveVisit::Saved(saved) => match *saved {
                        StudioOverlaySave::Local(draft) => StudioUnconfirmedSaveOutcome::Saved {
                            basis: draft.basis(),
                            accepted: draft.accepted(),
                        },
                        // An exact retry: the same two facts, read from the stored branch rather
                        // than from a rebuilt draft (design 6.2, S1a).
                        StudioOverlaySave::Acknowledged { basis, accepted } => {
                            StudioUnconfirmedSaveOutcome::Saved { basis, accepted }
                        }
                        StudioOverlaySave::Disposed(manifest) => {
                            StudioUnconfirmedSaveOutcome::Disposed(manifest)
                        }
                        StudioOverlaySave::HandedOff(outcome) => {
                            StudioUnconfirmedSaveOutcome::HandedOff(outcome)
                        }
                    },
                    StudioOverlaySaveVisit::Scheduled => StudioUnconfirmedSaveOutcome::Scheduled,
                    StudioOverlaySaveVisit::Busy => StudioUnconfirmedSaveOutcome::Busy,
                };
                Ok(StudioControlResponse::UnconfirmedOverlaySaved { target, outcome })
            }
            _ => Err(invalid("not an unconfirmed draft Save")),
        }
    }

    /// The ticket an Unconfirmed Save carries: the basis fingerprint and the branch the store names
    /// for it, both from one fresh mint in one custody visit. The Closing form is
    /// `Server::prepare_studio_closing_overlay`.
    ///
    /// Refuses when this device is no longer a member, or when there is no live, tail-complete
    /// preview of `target` (with the mint's own reason). That is the only way to start new work. An
    /// exact retry of accepted work needs no ticket, because it carries the one it was saved with.
    pub(crate) fn begin_unconfirmed_overlay_save<T: MeshTransport + 'static, R: CryptoRngCore>(
        &self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
    ) -> Result<([u8; 32], [u8; 32]), AppError> {
        // The mint checks the provider's membership, not this device's, and naming a branch writes
        // nothing that would. A removed device must not be handed a ticket it cannot use.
        if !server
            .sync
            .with_registry_context(|g, d, _, _| g.contains_device(&d.device_id()))
        {
            return Err(invalid("this device is no longer a member"));
        }
        let basis = self.catchup.preview.mint(server, store, id, target)?;
        let mut budget = save_budget(server, store, id)?;
        let branch = server.sync.with_registry_context(|group, _, _, _| {
            store.studio_overlay_request_branch(id, group, target, &basis, &mut budget)
        })?;
        Ok((basis.fingerprint(), branch))
    }

    /// One custody visit of the scheduled Unconfirmed Save. At most one of: finishing a parked
    /// plan, settling a classification terminally, or capturing new authoring and detaching it.
    ///
    /// The caller repeats the identical request until it is `Saved`; it stays retryable and
    /// byte-stable in between.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn save_unconfirmed_overlay<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        basis: [u8; 32],
        branch: [u8; 32],
        operation: DomainOp,
    ) -> Result<StudioOverlaySaveVisit, AppError> {
        let request = overlay_request_fingerprint(1, target, &basis, &branch, &operation)?;
        // Any parked plan is finished first, whatever its target: its transient hold protects its
        // pixels, and while it is parked the slot refuses every reservation on this actor. S3
        // re-enters the live check with an attempt made in THIS visit, for the plan's own target,
        // never one parked with the plan. A Closing plan is refused by the store, by name, before
        // it reads anything; RT-001 holds for that as for any refusal: nothing is parked, and its
        // request reclassifies from durable state on its own retry.
        if self.catchup.has_planned_overlay() {
            // The budget before the take: a failed scan then leaves the plan parked for the next
            // visit, instead of destroying a committable plan (re-review of `5ccc4647`, LOW-2).
            let mut budget = save_budget(server, store, id)?;
            let (planned, plan, ownership) = self
                .catchup
                .take_any_planned_overlay()
                .expect("checked just above, in the same custody visit");
            // Whose plan this is. The slot holds one plan whichever request made it, and the plan
            // does not say, so the receiver remembers which request scheduled it.
            let ours = planned == target && self.unconfirmed_scheduled == Some(request);
            self.closing_scheduled = None;
            self.unconfirmed_scheduled = None;
            let attempt = self.catchup.preview.mint(server, store, id, planned);
            let committed = server.sync.with_registry_context(|group, device, _, rng| {
                store.commit_studio_overlay_with(
                    id,
                    group,
                    planned,
                    device,
                    StudioOverlayMint::unconfirmed(attempt),
                    *plan,
                    rng,
                    &mut budget,
                )
            });
            // Released only after the commit attempt returns, on success and on error alike.
            drop(ownership);
            // The commit's write is its last step, so even an error may follow a durable change;
            // the lifecycle row is refreshed either way, as the control transaction does for every
            // action that can change the vault (review of `b35e23d2`, MEDIUM-2).
            self.settlement
                .note(planned, StudioSettlementState::RefreshRequired);
            if !ours {
                // Nothing of THIS request was saved, whatever the other commit did. Its success or
                // refusal belongs to its own caller's retry, not to this response, so it is logged
                // rather than returned.
                if let Err(error) = committed {
                    tracing::warn!(%error, "a parked overlay plan of another request did not commit");
                }
                return Ok(StudioOverlaySaveVisit::Busy);
            }
            return committed.map(|draft| {
                StudioOverlaySaveVisit::Saved(Box::new(StudioOverlaySave::Local(draft)))
            });
        }
        // 7.2: reserve before the first bounded read, release by dropping if nothing is scheduled.
        let Some(ownership) = self.catchup.reserve_overlay() else {
            // The slot is busy with work in flight. If that work is this very request's, say so:
            // "nothing of this request was saved" would be untrue of a plan already running
            // (review of `b35e23d2`, LOW-1).
            return Ok(if self.unconfirmed_scheduled == Some(request) {
                StudioOverlaySaveVisit::Scheduled
            } else {
                StudioOverlaySaveVisit::Busy
            });
        };
        let attempt = self.catchup.preview.mint(server, store, id, target);
        let mut budget = save_budget(server, store, id)?;
        let started = server
            .sync
            .with_registry_context(|group, device, clock, rng| {
                store.start_studio_overlay(
                    id,
                    group,
                    target,
                    device,
                    StudioOverlayMint::unconfirmed(attempt),
                    basis,
                    branch,
                    operation,
                    clock.now_ms(),
                    rng,
                    &mut budget,
                )
            })?;
        match started {
            StudioOverlayStart::Settled(saved) => {
                // An acknowledgement or a disposal manifest. Neither changed the vault, but a
                // durable acceptance on a retry still changes what the row should show.
                self.settlement
                    .note(target, StudioSettlementState::RefreshRequired);
                Ok(StudioOverlaySaveVisit::Saved(saved))
            }
            StudioOverlayStart::Captured(capture) => {
                if !self.queue_capture_unless_paused(*capture, ownership, target) {
                    return Ok(StudioOverlaySaveVisit::Busy);
                }
                self.unconfirmed_scheduled = Some(request);
                Ok(StudioOverlaySaveVisit::Scheduled)
            }
        }
    }
}

/// A budget from a completed five-family inventory, as every lifecycle control takes one. Called
/// only by a stage that will spend it, so a visit that finds the slot busy costs no scan.
fn save_budget<T: MeshTransport + 'static, R: CryptoRngCore>(
    server: &mut Server<T, R>,
    store: &mut ServerStore,
    id: u64,
) -> Result<EpochStudioBudget, AppError> {
    let mut scan = store.scan_epoch_storage_with_studio()?;
    while !scan.step()?.complete {}
    let inventory = scan.finish()?;
    server
        .sync
        .with_registry_context(|g, _, _, _| store.studio_storage_budget(id, g, &inventory))
}

/// Refuse when the document's live draft was not made on a preview. Read structurally, from the
/// intent record only. An Unconfirmed branch, no branch, or only terminal records all pass: an
/// exact retry of disposed Unconfirmed work must still be answered.
fn refuse_closing_draft<T: MeshTransport + 'static, R: CryptoRngCore>(
    server: &Server<T, R>,
    store: &ServerStore,
    id: u64,
    target: StudioTarget,
) -> Result<(), AppError> {
    let logical = target.document(&server.group_id()).map_err(invalid)?;
    let state = store.load_epoch_intents_structural(id, &logical)?;
    if let Some(metadata) = state.handoff_metadata() {
        if metadata.overlay().is_some()
            && matches!(metadata.provenance(), StudioOverlayProvenance::Closing)
        {
            return Err(invalid(
                "this document's live draft was not made on a preview; it cannot take an \
                 unconfirmed Save",
            ));
        }
    }
    Ok(())
}
