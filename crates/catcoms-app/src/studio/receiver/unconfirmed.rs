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
//! **A refused plan is answered, not re-planned** (MAP K12). Its refusal is kept against the stamp of
//! the record it was planned from, and the identical retry hears it again, with no capture, while
//! that record is unchanged and for at most the park bound.
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
                if let Err(error) = refuse_closing_draft(server, store, id, target) {
                    return unconfirmed_refusal(target, error);
                }
                let (basis, branch) =
                    match self.begin_unconfirmed_overlay_save(server, store, id, target) {
                        Ok(ticket) => ticket,
                        Err(error) => return unconfirmed_refusal(target, error),
                    };
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
                if let Err(error) = refuse_closing_draft(server, store, id, target) {
                    return unconfirmed_refusal(target, error);
                }
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
                let visit = match visit {
                    UnconfirmedSaveVisit::Paused => {
                        let outcome = StudioUnconfirmedSaveOutcome::Paused;
                        return Ok(StudioControlResponse::UnconfirmedOverlaySaved {
                            target,
                            outcome,
                        });
                    }
                    UnconfirmedSaveVisit::Visit(visit) => visit,
                };
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
                    StudioOverlaySaveVisit::Refused { reason, message } => {
                        StudioUnconfirmedSaveOutcome::Refused { reason, message }
                    }
                    StudioOverlaySaveVisit::Scheduled => StudioUnconfirmedSaveOutcome::Scheduled,
                    StudioOverlaySaveVisit::Busy => StudioUnconfirmedSaveOutcome::Busy,
                    StudioOverlaySaveVisit::Paused => StudioUnconfirmedSaveOutcome::Paused,
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
            return Err(AppError::StudioRefused {
                reason: catcoms_replication::studio::StudioOverlaySaveRefusal::NotMember,
                message: "Studio: this device is no longer a member".into(),
            });
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
    ) -> Result<UnconfirmedSaveVisit, AppError> {
        use UnconfirmedSaveVisit::Visit;
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
            let (context, plan, ownership) = self
                .catchup
                .take_any_planned_overlay()
                .expect("checked just above, in the same custody visit");
            let (planned, provenance, committed) =
                self.commit_planned_overlay(server, store, id, context, *plan, &mut budget);
            // Whose plan this is. The slot holds one plan whichever request made it, and the plan
            // does not say, so the receiver remembers which request scheduled it.
            let ours = planned == target
                && provenance == OverlayCommitKind::Unconfirmed
                && self.unconfirmed_scheduled == Some(request);
            self.closing_scheduled = None;
            self.unconfirmed_scheduled = None;
            // Released only after the commit attempt returns, on success and on error alike.
            drop(ownership);
            if !ours {
                // Nothing of THIS request was saved, whatever the other commit did. Its success or
                // refusal belongs to its own caller's retry, not to this response, so it is logged
                // rather than returned.
                if let Err(error) = committed {
                    tracing::warn!(%error, "a parked overlay plan of another request did not commit");
                }
                return Ok(Visit(StudioOverlaySaveVisit::Busy));
            }
            return match committed {
                Ok(draft) => Ok(Visit(StudioOverlaySaveVisit::Saved(Box::new(
                    StudioOverlaySave::Local(draft),
                )))),
                Err(error) => refused_save_visit(error).map(Visit),
            };
        }
        // MAP K12: this request's plan was refused, and the record it was planned from is unchanged,
        // so the refusal is answered again. It goes before the reservation and every stage after it:
        // the point is that a Save that cannot succeed costs no scan, no media admission, no permit
        // and no detached decode on each retry. The stamp is proved current under custody first,
        // which costs one authenticated read of the intent record, beside the structural read every
        // Save makes at the control boundary. Until the refusal carrier lands it is the store's own
        // `Err`, as on the first attempt.
        //
        // The numeric server first. The fingerprint does not name it, and the stamp check reads the
        // record at the stamp's server rather than this one, so a refusal kept under another would
        // otherwise answer for a record it never read (review of K12, LOW-1). The commit compares
        // it for the same reason.
        let now = server.runtime_clock().monotonic_ms();
        if let Some(refusal) = self.catchup.overlay_refusal_for(request, now) {
            let current = refusal.stamp().server() == id
                && server.sync.with_registry_context(|group, device, _, _| {
                    store.studio_overlay_is_current(group, device, refusal.stamp())
                })?;
            if current {
                return Ok(Visit(StudioOverlaySaveVisit::Refused {
                    reason: refusal.reason(),
                    message: refusal.message().to_owned(),
                }));
            }
            // The record moved on, by a commit, a disposal or anything else. Decide afresh.
            self.catchup.forget_overlay_refusal();
        }
        // 7.2: reserve before the first bounded read, release by dropping if nothing is scheduled.
        let Some(ownership) = self.catchup.reserve_overlay() else {
            // The slot is busy with work in flight. If that work is this very request's, say so:
            // "nothing of this request was saved" would be untrue of a plan already running
            // (review of `b35e23d2`, LOW-1).
            return Ok(Visit(if self.unconfirmed_scheduled == Some(request) {
                StudioOverlaySaveVisit::Scheduled
            } else {
                StudioOverlaySaveVisit::Busy
            }));
        };
        let attempt = self.catchup.preview.mint(server, store, id, target);
        let mut budget = save_budget(server, store, id)?;
        let started = match server
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
            }) {
            Ok(started) => started,
            Err(error) => {
                let refused = refused_save_visit(error)?;
                if matches!(
                    refused,
                    StudioOverlaySaveVisit::Refused { reason, .. }
                        if super::refused_save_requires_refresh(reason)
                ) {
                    self.settlement
                        .note(target, StudioSettlementState::RefreshRequired);
                }
                return Ok(Visit(refused));
            }
        };
        match started {
            StudioOverlayStart::Settled(saved) => {
                // An acknowledgement or a disposal manifest. Neither changed the vault, but a
                // durable acceptance on a retry still changes what the row should show.
                self.settlement
                    .note(target, StudioSettlementState::RefreshRequired);
                Ok(Visit(StudioOverlaySaveVisit::Saved(saved)))
            }
            StudioOverlayStart::Captured(capture) => {
                // Refused at the capture, not at the door: everything above answers an exact
                // retry, so only new authoring reaches this (design 18.3 review, F4).
                if !self.queue_capture_unless_paused(
                    *capture,
                    ownership,
                    target,
                    OverlayCommit::Unconfirmed,
                ) {
                    return Ok(UnconfirmedSaveVisit::Paused);
                }
                self.unconfirmed_scheduled = Some(request);
                Ok(Visit(StudioOverlaySaveVisit::Scheduled))
            }
        }
    }
}

/// Lift a classified refusal into the stable control response without laundering any other error.
/// Invariant, I/O and uncertain-write failures must remain on the error channel; only the app's
/// typed `StudioRefused` variant is safe to present as proof that this request wrote nothing.
fn unconfirmed_refusal(
    target: StudioTarget,
    error: AppError,
) -> Result<StudioControlResponse, AppError> {
    let StudioOverlaySaveVisit::Refused { reason, message } = refused_save_visit(error)? else {
        unreachable!("the refusal converter returned a non-refusal visit")
    };
    Ok(StudioControlResponse::UnconfirmedOverlaySaved {
        target,
        outcome: StudioUnconfirmedSaveOutcome::Refused { reason, message },
    })
}

impl StudioReceiver {
    /// Whether a refused plan's answer is kept (MAP K12), for a test that must see it was not.
    #[cfg(test)]
    pub(crate) fn has_overlay_refusal_for_test(&self) -> bool {
        self.catchup.has_overlay_refusal_for_test()
    }
}

/// One visit of the Unconfirmed Save: what the Flow S visit concluded, or new work refused because
/// the receiver is paused.
///
/// The refusal has its own variant so the caller can be told to wait for the pause to end rather
/// than to resend, which `Busy` means. A resend does not end a pause, and each one pays for the
/// inventory scan, the mint and media admission again (re-review of the F4 batch, M-1).
pub(crate) enum UnconfirmedSaveVisit {
    Visit(StudioOverlaySaveVisit),
    Paused,
}

/// A budget from a completed five-family inventory, as every lifecycle control takes one. Called
/// only by a stage that will spend it, so a visit that finds the slot busy costs no scan.
pub(super) fn save_budget<T: MeshTransport + 'static, R: CryptoRngCore>(
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
            return Err(crate::store::studio_refused(
                catcoms_replication::studio::StudioOverlaySaveRefusal::OtherKindDraft,
                "this document's live draft was not made on a preview; it cannot take an \
                 unconfirmed Save",
            ));
        }
    }
    Ok(())
}
