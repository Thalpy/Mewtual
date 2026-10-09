//! Runtime ownership around the existing watched inbox and durable receive adapter. No second
//! inbox, document, membership snapshot or persistence coordinator lives here.

use super::*;
use crate::studio_exchange::ServerStudioWatch;
use catcoms_replication::Admission;
use std::collections::VecDeque;
use std::sync::Arc;
mod catchup;
#[cfg(test)]
pub(crate) use catchup::PreviewHarness;
mod handoff;
mod inventory;
mod replay;
mod unconfirmed;
pub(crate) use catchup::{HandoffCompletion, StudioBackgroundJob, StudioBackgroundResult};

/// Recently accessed targets, bounded by the existing sync watch rail. Reopening the same exact
/// source preserves its inbox; eviction explicitly revokes the old subscription and queued work.
#[derive(Default)]
pub(crate) struct StudioReceiver {
    watches: VecDeque<(ServerStudioWatch, u128)>,
    paused: bool,
    pause_notice: bool,
    catchup: catchup::CatchupRuntime,
    gossip_runs: usize,
    settlement: SettlementNotices,
    replay: replay::ReplayRuntime,
    replay_turn: bool,
    handoff: handoff::HandoffRuntime,
    /// C-3's one shared, turn-based inventory job for background owners (C-3 runtime design 2.2).
    inventory: inventory::InventoryRuntime,
    /// Which Unconfirmed Save request scheduled the plan now parked, if one did (design 8.7). A plan
    /// does not record its request, and the overlay slot holds one plan for a target whichever
    /// request made it. So a later visit must know whether that plan is its own before reporting
    /// it saved.
    unconfirmed_scheduled: Option<[u8; 32]>,
    /// The exact Closing request that owns the one overlay job/plan slot. The target alone is not
    /// enough: two operations can name the same document, and neither may report the other's
    /// detached plan as its own successful Save.
    closing_scheduled: Option<[u8; 32]>,
}
/// What one custody visit of the scheduled Flow S concluded.
pub(crate) enum StudioOverlaySaveVisit {
    /// Terminal and already durable: an acknowledgement, an exact retry, or a commit that this
    /// visit completed from a plan an earlier visit left ready.
    Saved(Box<catcoms_replication::studio::StudioOverlaySave>),
    /// New authoring captured and handed to the background runtime. The caller asks again after
    /// a later visit; the request stays retryable and byte-stable in the meantime.
    Scheduled,
    /// Retryable, for one of two reasons.
    ///
    /// **Admission or the shared four-slot pool is full.** Then nothing was read, promoted or held:
    /// 7.2 reserves before the first body read precisely so this costs nothing.
    ///
    /// **The receiver is paused** (design 18.3 review, F4). This one is not free. The visit ran
    /// classification and S1b (the mint and media admission, which can promote a frame's PIX, and
    /// for the Unconfirmed Save the budget's inventory scan, which the Closing Save's caller pays
    /// instead) before it dropped the capture. The dropped hold leaves any promoted PIX unprotected,
    /// which RT-001 already accepts for a refused plan.
    ///
    /// **A caller cannot yet tell the two apart**, and that is a known gap, not a contract a caller
    /// can follow:
    /// - `studio-receive-paused` is a one-shot transition event, not a queryable state;
    /// - neither Save path clears the pause; only a successful explicit Studio document access
    ///   through `run` does (a Read, an ordinary Save, Apply or ApplyOverlayCopy);
    /// - the native Unconfirmed mapping answers `Busy` with "send the identical request again".
    ///
    /// So a caller resending on `Busy` while paused repeats that work each time. The fix proposed to
    /// Agent 2, whose outcome type and native mapping this is, is a distinct `Paused` outcome from
    /// the same refusal point. Native Save registration must not ship before it (re-review of the
    /// batch fixes, M-1).
    Busy,
}

/// Correlate a detached Flow S result with the complete bounded request envelope.
///
/// `DomainOp::id` intentionally omits the body because it identifies an idempotency slot. That is
/// too weak here: conflicting bodies with one nonce must not claim each other's parked work. The
/// canonical operation encoding covers nonce, document type, logical key and body; the remaining
/// fields distinguish the two basis kinds and the exact branch ticket presented by the caller.
fn overlay_request_fingerprint(
    provenance: u8,
    target: StudioTarget,
    basis: &[u8; 32],
    branch: &[u8; 32],
    operation: &catcoms_replication::DomainOp,
) -> Result<[u8; 32], AppError> {
    let encoded = operation.encode().map_err(invalid)?;
    let mut hash = blake3::Hasher::new_derive_key("mewtual/studio-flow-s-request/v1");
    hash.update(&[provenance]);
    hash.update(&target.channel());
    match target {
        StudioTarget::Index { .. } => {
            hash.update(&[0]);
        }
        StudioTarget::Flipnote { object, .. } => {
            hash.update(&[1]);
            hash.update(&object);
        }
    }
    hash.update(basis);
    hash.update(branch);
    hash.update(&(encoded.len() as u64).to_be_bytes());
    hash.update(&encoded);
    Ok(*hash.finalize().as_bytes())
}

impl StudioReceiver {
    pub(crate) fn clear_previews(&mut self) {
        self.catchup.preview.reset();
        // The UI-lock reset: a parked inventory body is authenticated plaintext, and no visit
        // runs while locked, so it must not stay resident (C-3 runtime design 4, review M6).
        self.inventory.release();
    }

    /// Scheduled local Save (Flow S), under the actor's custody lease.
    ///
    /// One visit does at most one of: commit a plan a previous visit left ready, settle a
    /// classification terminally, or capture new authoring and detach it. The expensive
    /// reconstruction never runs here; that is the whole point.
    ///
    /// `close` and `budget` are parameters, exactly as they are on the existing explicit
    /// `Server::save_studio_closing_overlay`. Acquiring them is the caller's job: the saved close
    /// comes from the owner journal and the budget from a completed five-family inventory, and
    /// deciding when to pay for both is the manual lifecycle Agent 2 owns. That command is what
    /// will call this, so no production caller exists yet.
    //
    // Consumed by the native/actor overlay lifecycle, which is gated on Agent 2's P5 (still
    // false) and on C-3 making the per-visit inventory affordable. The seam is exercised by its
    // own tests today and exposes no command; delete this marker with that commit.
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn save_overlay<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        close: &catcoms_replication::CloseRecord,
        basis: [u8; 32],
        branch: [u8; 32],
        operation: catcoms_replication::DomainOp,
        budget: &mut crate::store::EpochStudioBudget,
    ) -> Result<StudioOverlaySaveVisit, AppError> {
        let request = overlay_request_fingerprint(0, target, &basis, &branch, &operation)?;
        // Read, never required here: S1b and S3 require it at their own points, and everything
        // before them - retries and acknowledgements - must keep working without it (V8).
        let tenure = server.observed_owner_tenure();
        // A plan this actor already produced is finished first. Its transient hold is the only
        // thing protecting its pixels, and it occupies admission until it is consumed.
        if let Some((plan, ownership)) = self.catchup.take_planned_overlay(target) {
            // Whatever plan this takes is no longer another request's to claim. Commit it to
            // release its transient media hold and shared slot, but only its exact scheduling
            // request may receive the outcome as its own.
            let ours = self.closing_scheduled == Some(request);
            self.closing_scheduled = None;
            self.unconfirmed_scheduled = None;
            let committed = server.sync.with_registry_context(|group, device, _, rng| {
                store.commit_studio_overlay(
                    id, group, target, device, close, tenure, *plan, rng, budget,
                )
            });
            // Explicit: admission and the shared slot are released only after the commit attempt
            // returns, on success and on error alike.
            drop(ownership);
            if !ours {
                if let Err(error) = committed {
                    tracing::warn!(%error, "a parked overlay plan of another request did not commit");
                }
                return Ok(StudioOverlaySaveVisit::Busy);
            }
            return committed.map(|draft| {
                StudioOverlaySaveVisit::Saved(Box::new(
                    catcoms_replication::studio::StudioOverlaySave::Local(draft),
                ))
            });
        }
        // 7.2. Reserve before the first bounded read, and release by dropping if there is nothing
        // to schedule. Nothing below this line reaches a blob until S1b.
        let Some(ownership) = self.catchup.reserve_overlay() else {
            return Ok(if self.closing_scheduled == Some(request) {
                StudioOverlaySaveVisit::Scheduled
            } else {
                StudioOverlaySaveVisit::Busy
            });
        };
        let started = server
            .sync
            .with_registry_context(|group, device, clock, rng| {
                store.start_studio_closing_overlay(
                    id,
                    group,
                    target,
                    device,
                    close,
                    tenure,
                    basis,
                    branch,
                    operation,
                    clock.now_ms(),
                    rng,
                    budget,
                )
            })?;
        match started {
            // Terminal, and the reservation is released by dropping `ownership` on return.
            crate::store::StudioOverlayStart::Settled(saved) => {
                Ok(StudioOverlaySaveVisit::Saved(saved))
            }
            crate::store::StudioOverlayStart::Captured(capture) => {
                if !self.queue_capture_unless_paused(*capture, ownership, target) {
                    return Ok(StudioOverlaySaveVisit::Busy);
                }
                self.closing_scheduled = Some(request);
                Ok(StudioOverlaySaveVisit::Scheduled)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn observe_hints_for_test(
        &mut self,
        observer: tokio::sync::watch::Sender<
            Option<crate::studio_exchange::discovery::StudioHintObservation>,
        >,
    ) {
        self.catchup.hint_observer = Some(observer);
    }
    /// Recovery writes deliberately reuse ordinary Save's watch, storage and one-shot
    /// publication path. Read-only controls carry no fake document or saved packets.
    pub(crate) fn control<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        request: StudioControlRequest,
    ) -> Result<
        (
            StudioSavedTransaction,
            Option<StudioTarget>,
            Option<StudioControlResponse>,
        ),
        AppError,
    > {
        // Both applies publish through the ordinary Save path rather than writing in the control
        // transaction, so both are intercepted here. Copy's `target` is the SOURCE it copied from;
        // the request it produces publishes to the destination.
        if let StudioControlAction::RepairFault(decision) = request.action {
            // Only the catch-up runtime holds the durable owner snapshot issuance requires.
            let response =
                self.catchup
                    .repair_fault(server, store, id, request.target, *decision)?;
            Ok((StudioSavedTransaction::empty(), None, Some(response)))
        } else if let StudioControlAction::RepairRegistryFault(decision) = request.action {
            let response =
                self.catchup
                    .repair_registry_fault(server, store, id, request.target, *decision)?;
            Ok((StudioSavedTransaction::empty(), None, Some(response)))
        } else if let StudioControlAction::Apply(apply) = request.action {
            let (edit, already_saved) =
                server.prepare_studio_recovery_apply(store, id, request.target, *apply)?;
            let (saved, updated) = self.run(server, store, id, Some(edit))?;
            Ok((
                saved,
                updated,
                Some(StudioControlResponse::Applied {
                    target: request.target,
                    already_saved,
                }),
            ))
        } else if let StudioControlAction::ApplyOverlayCopy(apply) = request.action {
            let destination = apply.destination;
            let (edit, already_saved) =
                server.prepare_studio_copy_apply(store, id, request.target, *apply)?;
            let (saved, updated) = self.run(server, store, id, Some(edit))?;
            Ok((
                saved,
                updated,
                Some(StudioControlResponse::OverlayCopyApplied {
                    // The destination is what changed, so it is what the caller is told about.
                    destination,
                    already_saved,
                }),
            ))
        } else if matches!(
            request.action,
            StudioControlAction::BeginUnconfirmedOverlaySave
                | StudioControlAction::SaveUnconfirmedOverlay(_)
        ) {
            // Design 8.7. Only the receiver holds the ready preview each stage mints from.
            let response = self.unconfirmed_save_control(server, store, id, request)?;
            Ok((StudioSavedTransaction::empty(), None, Some(response)))
        } else {
            let target = request.target;
            // Every action that can leave the vault different from what the renderer last read.
            // Release and dispose are here for the reason the comment below gives and more
            // sharply: release closes both budgets and unlinks a record, and a failure *after*
            // the unlink is precisely the case where the caller must reconcile rather than
            // resend. A destructive action that emitted no refresh would leave the renderer
            // showing an archive that is gone.
            let changing = matches!(
                request.action,
                StudioControlAction::Acknowledge { .. }
                    | StudioControlAction::RestorePointer
                    | StudioControlAction::FinishOverlayArchive(_)
                    | StudioControlAction::ReleaseOverlayArchive(_)
                    | StudioControlAction::DisposeOverlay(_)
            );
            let result = server.studio_control_transaction(store, id, request);
            if changing {
                // Even an error may follow a successful first durability barrier. Read-only
                // List/Preview never emit, preventing a notification -> refresh -> event loop.
                self.settlement
                    .note(target, StudioSettlementState::RefreshRequired);
            }
            if let Ok(StudioControlResponse::Acknowledged(list)) = &result {
                if let Some(source) = &list.source {
                    self.settlement.note(target, source.phase.into());
                }
                self.settlement.note(
                    target,
                    if list.eviction_pending.is_some() {
                        StudioSettlementState::RecoveryEvictionPending
                    } else {
                        StudioSettlementState::RecoveryAvailable
                    },
                );
            }
            result.map(|r| (StudioSavedTransaction::empty(), None, Some(r)))
        }
    }
    pub(crate) fn take_settlement_notices(&mut self) -> Vec<(StudioTarget, StudioSettlementState)> {
        for (target, state) in self.catchup.settlement.take() {
            self.settlement.note(target, state);
        }
        self.settlement.take()
    }
    #[cfg(test)]
    pub(crate) fn hold_registry_page_for_test(
        &mut self,
        target: StudioTarget,
        pass: crate::registry_catchup::ServerRegistryReceive,
    ) {
        self.catchup.hold_registry_page_for_test(target, pass);
    }
    #[cfg(test)]
    pub(crate) fn has_registry_page_for_test(&self) -> bool {
        self.catchup.has_registry_page_for_test()
    }
    /// Test scheduling with a page obtained through the real authenticated fetch adapters.
    #[cfg(test)]
    pub(crate) fn hold_page_for_test(
        &mut self,
        target: StudioTarget,
        pass: crate::studio_exchange::ServerStudioReceive,
    ) {
        self.catchup.hold_page_for_test(target, pass);
    }
    #[cfg(test)]
    pub(crate) fn retaining_registry_for_test(
        provider: crate::registry_catchup::ServerRegistryPageProvider,
        until: u64,
    ) -> Self {
        let mut receiver = Self::default();
        receiver.catchup.registry_cache_for_test(provider, until);
        receiver
    }
    /// Exercise the pre-install router with a transport-produced pass while keeping all of the
    /// otherwise private runtime custody explicit in the test. The returned pass survives only
    /// when the router permits the ordinary installer to continue.
    #[cfg(test)]
    pub(crate) fn route_registry_checkpoint_for_test<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        pass: crate::studio_exchange::discovery::ServerCheckpointFetch,
    ) -> Result<
        (
            bool,
            Option<crate::studio_exchange::discovery::ServerCheckpointFetch>,
        ),
        AppError,
    > {
        self.catchup
            .route_registry_checkpoint_for_test(server, store, id, pass)
    }
    #[cfg(test)]
    pub(crate) fn hold_checkpoint_for_test(
        &mut self,
        pass: crate::studio_exchange::discovery::ServerCheckpointFetch,
    ) {
        self.catchup.hold_checkpoint_for_test(pass);
    }
    #[cfg(test)]
    pub(crate) fn take_checkpoint_for_test(
        &mut self,
    ) -> Option<crate::studio_exchange::discovery::ServerCheckpointFetch> {
        self.catchup.take_checkpoint_for_test()
    }
    fn catchup_step<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> Result<Option<StudioTarget>, AppError> {
        let updated = self.catchup.run(server, store, id, &self.watches)?;
        if let Some((target, epoch)) = self.catchup.take_binding() {
            // Retarget only AFTER the saved source has crossed recovery/install barriers.
            // The old subscription and queued concrete-epoch packets are revoked together.
            self.observe(server, store, id, target, epoch)?;
        }
        Ok(updated.or_else(|| self.catchup.preview.take_notice()))
    }
    /// The same fair background turn services catch-up and at most one own replay operation.
    /// Sustained gossip must not postpone recovery indefinitely, and replay must not steal a
    /// source that a fetched page/seed still needs.
    fn background_step<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> Result<(StudioSavedTransaction, Option<StudioTarget>), AppError> {
        // Flow H, in stage order, behind design 7.3's placement gate.
        //
        // RT-002's rule applies to every heavy stage, and H5 and H1 are heavy: H5 drains an
        // inventory, reads the whole source and performs three accounted writes with flushes; H1
        // drains an inventory too. Both wait for `replay_ready()`, so authoritative catch-up is
        // never starved by transfer work. H3 is the documented exception: a signing slice needs
        // no permit and no retained source, so 7.3 lets it run on any turn, subject to the
        // priority answer, which is a yield rather than a gate.
        //
        // Among themselves the order is H5, then H3, then H1: an assembled transfer is holding
        // admission, a shared slot and a signed candidate, and finishing it frees all three,
        // while starting new work is the least urgent.
        // Both gates take the clock, so the scheduler and `pending` consult the same backoff.
        // A refusal that records a hold the next turn ignores is not pacing at all.
        let handoff_now = server.runtime_clock().monotonic_ms();
        let ready = self.catchup.replay_ready();
        if ready && self.handoff.can_commit(handoff_now) {
            let updated = self.handoff_commit(server, store, id);
            return Ok((StudioSavedTransaction::empty(), updated));
        }
        if self.handoff.can_sign(handoff_now) {
            let priority = self.handoff_priority(server);
            // A yield consumes no turn: it signed nothing, so the turn goes to whatever it
            // yielded to. A slice that signed at least one operation has used the turn, whether
            // it stopped at its turn cap, at its deadline, or by finishing the branch.
            if let Some(slice) = self.handoff_sign(server, store, priority) {
                if !slice.yielded() && slice.signed() != 0 {
                    return Ok((StudioSavedTransaction::empty(), None));
                }
            }
        }
        if ready {
            self.handoff_probe(server, store, id);
        }
        self.replay_turn = !self.replay_turn;
        if self.replay_turn {
            // One inventory slice per visit, from this visit's one clock sample (C-3 S-3): what
            // the H-stages above spent comes out of replay's share rather than adding to it.
            let deadline = handoff_now.saturating_add(inventory::INVENTORY_SLICE_MS);
            if let Some(saved) = self.replay_step(server, store, id, deadline)? {
                return Ok(saved);
            }
        }
        let updated = self.catchup_step(server, store, id)?;
        Ok((StudioSavedTransaction::empty(), updated))
    }

    /// Queue a fresh Save capture for its detached plan, unless the receiver is paused. Returns
    /// whether it was queued; the caller answers `Busy` if not and records no scheduling request.
    ///
    /// Design 18.3 review, F4, in the shape Agent 2 asked for. A paused receiver hands out no work,
    /// and only an unrelated explicit access clears the pause, so a capture queued now would hold
    /// this actor's admission, one of the four process-wide preparation permits and its transient
    /// media hold for as long as the pause lasts. Dropping it releases all three. Nothing of the
    /// Save is durable; media admission may have promoted the frame's PIX, which the dropped hold
    /// no longer protects, the cost RT-001 already accepts for a refused plan. The paused `Busy`
    /// is therefore not free, and `StudioOverlaySaveVisit::Busy` says how a caller must treat it.
    ///
    /// The check sits here, after classification, not at either Save's entry: an exact retry of
    /// accepted work is answered at S1 before any capture, so it is not refused merely because the
    /// receiver is paused. (It can still answer `Busy` while the overlay slot is held by other work,
    /// as before this change; the batch review's LOW-4 narrowed this claim.)
    fn queue_capture_unless_paused(
        &mut self,
        capture: crate::store::StudioOverlayCapture,
        ownership: crate::studio::overlay::OverlayOwnership,
        target: StudioTarget,
    ) -> bool {
        if self.paused {
            drop(capture);
            drop(ownership);
            return false;
        }
        self.catchup.schedule_overlay(capture, ownership, target);
        true
    }

    /// 7.3's placement answer for a signing slice: yield immediately to authoritative service
    /// interest, to inbound on any watch, to a background result already parked, or to a reserved
    /// service request still owed its answer.
    ///
    /// The last term is not in 7.3's list; the actor-level test for design 18.3 review F2 found it
    /// missing. A request is queued interest only until catch-up reserves it, and a parked result
    /// only until its source is installed. In the turns after that it is still unanswered, and
    /// catch-up serves it only on a turn no slice has signed in.
    fn handoff_priority<T: MeshTransport, R: CryptoRngCore>(&self, server: &Server<T, R>) -> bool {
        server.sync.has_epoch_service_interest()
            || self
                .watches
                .iter()
                .any(|(w, _)| server.sync.studio_has_inbound(&w.inner))
            || self.catchup.result_parked()
            || self.catchup.service_owed(server)
    }
    /// Notify before any bounded event-channel await: native work never waits on the event
    /// consumer, and event backpressure must not conceal an already-queued inbox packet.
    pub(crate) fn signal<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
        signal: &tokio::sync::watch::Sender<bool>,
    ) {
        self.signal_and_wake(server, signal);
    }

    /// Publish "is there work now" and return "when is there work next", from **one** clock read.
    ///
    /// These two answers are about the same deadline, and taking them from separate reads is a
    /// capacity strand waiting to happen: sample `pending` at `D - 1` and it says not runnable,
    /// let the clock cross `D`, and a `wake_in` that only publishes future deadlines then says
    /// there is nothing to wait for either. No timer is armed, `studio_pending` stays false, and a
    /// `Ready` job holds admission and a process-wide permit until unrelated work happens by.
    ///
    /// With a single sample the two are exhaustive by construction: at `now >= D` the job is
    /// runnable so `pending` is true, and below `D` the returned delay is strictly positive, so
    /// the timer that fires is followed by a fresh sample that sees it runnable. There is no
    /// third case, which is the property the previous shape lacked.
    pub(crate) fn signal_and_wake<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
        signal: &tokio::sync::watch::Sender<bool>,
    ) -> Option<u64> {
        let now = server.runtime_clock().monotonic_ms();
        signal.send_if_modified(|pending| {
            let next = self.pending_at(server, now);
            if *pending == next {
                false
            } else {
                *pending = next;
                true
            }
        });
        self.wake_in_at(now)
    }
    /// Production reads this through `signal_and_wake`, which pairs it with `wake_in_at` under a
    /// single clock sample. This convenience takes its own sample and so must not be used to
    /// decide anything alongside a separately sampled deadline.
    #[cfg(test)]
    pub(crate) fn pending<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
    ) -> bool {
        self.pending_at(server, server.runtime_clock().monotonic_ms())
    }

    fn pending_at<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
        now: u64,
    ) -> bool {
        (!self.paused
            && ((self.catchup.replay_ready()
                && self
                    .replay
                    .pending(&self.watches, now, |w| {
                        server.sync.studio_watch_is_current(&w.inner)
                    }))
                || self.watches.iter().any(|(watch, _)| {
                    server.sync.studio_has_inbound(&watch.inner)
                        || server.sync.studio_has_page_request(&watch.inner)
                })
                // A parked handoff job holds this actor's admission and one of four process-wide
                // preparation permits, so the driver must keep scheduling turns while one exists.
                // `runnable`, not `busy`: a job held by backoff or already detached cannot make
                // progress this turn, and reporting it as pending would hold the driver at its
                // active cadence for the whole life of the job. Every other term here is time- or
                // state-gated for the same reason.
                || self.handoff.runnable(now)
                // With no job there is nothing for `runnable` to report, so a due probe deadline
                // has to reach the driver by its own term or the timer that fired for it is the
                // last one this actor ever arms.
                || self
                    .handoff
                    .probe_due(now, &self.rail())
                // A retained Registry source owns a process-wide preparation permit, and only a
                // custody visit can drop it. Without this term a quiet actor with no watches
                // never schedules that visit, so the thirty-second bound the code claims is not
                // a bound at all: four such actors strand the whole pool indefinitely.
                || self.catchup.registry_expiry_due(now)
                // A parked inventory body holds a pool permit and plaintext until a visit detaches
                // it. Only that: the job's other states wait on an owner's own paced turn.
                || self.inventory.pending()
                || self.catchup.pending(server, &self.watches)))
            // A parked Save plan past its deadline (design 8.7), outside the pause gate: a paused
            // receiver must still release the admission and process-wide permit a plan holds,
            // and `run` drops it before it honours the pause (review of `265b0756`, LOW-1).
            || self.catchup.overlay_park_expiry_due(now)
    }
    /// Milliseconds until this receiver has time-gated work to do, if any.
    ///
    /// `pending` answers "is there work right now"; this answers "when is there work next", and a
    /// held handoff job needs both. Without it a paced refusal is indistinguishable from a stall:
    /// the job reports not-runnable, `pending` goes false, and a quiescent actor schedules no
    /// further Studio turn while the job still holds admission and a process-wide permit.
    /// As `pending`: a separately sampled convenience for tests. Production pairs the two.
    #[cfg(test)]
    pub(crate) fn wake_in<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
    ) -> Option<u64> {
        self.wake_in_at(server.runtime_clock().monotonic_ms())
    }

    fn wake_in_at(&self, now: u64) -> Option<u64> {
        // A parked plan's deadline is published even while paused, for the reason its pending
        // term sits outside the pause gate.
        if self.paused {
            return self.catchup.overlay_park_wake_in(now);
        }
        // Every deadline, from the one sample the caller took. They are independent resources,
        // so the earliest wins; nothing here dominates anything else the way the capacity gate
        // dominates per-target pacing.
        [
            self.handoff.wake_in(now, &self.rail()),
            self.catchup.registry_wake_in(now),
            self.catchup.overlay_park_wake_in(now),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// The watched targets, which is the only set a handoff deadline may speak for. A deadline
    /// remembered for a target that is no longer watched must not wake the actor.
    fn rail(&self) -> Vec<StudioTarget> {
        self.watches.iter().map(|(w, _)| w.target).collect()
    }
    /// A private preparation pool for this receiver, so a test that parks plans does not contend
    /// with every other test in the process on the one global four-slot semaphore.
    #[cfg(test)]
    pub(crate) fn inject_overlay_pool_for_test(
        &mut self,
        permits: usize,
    ) -> Arc<tokio::sync::Semaphore> {
        self.catchup.inject_overlay_pool_for_test(permits)
    }

    /// Put the receiver in the state a storage fault leaves it in.
    ///
    /// Only the precondition is simulated. What a completion arriving in that state does, and
    /// whether the bundle it carries is released, are production paths.
    /// Sets the flag only, for a test that needs the state without the transition.
    #[cfg(test)]
    pub(crate) fn pause_for_test(&mut self) {
        self.paused = true;
    }

    /// The whole production transition, which is what an errored step performs.
    #[cfg(test)]
    pub(crate) fn pause_at_for_test<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &Server<T, R>,
    ) {
        self.pause(server);
    }

    /// Enter the paused state, releasing anything that can no longer make progress.
    ///
    /// The release has to happen **at the transition**, not on the next visit. A paused receiver
    /// publishes neither pending work nor a wake, so "the next `run`" is precisely the event this
    /// state prevents: a background pass that owns a non-detached bundle when an unrelated step
    /// errors would hold this actor's admission and one of four process-wide permits until a user
    /// happened to open a Studio document successfully. `release_if_stalled` at the top of `run`
    /// is a backstop for a pause that arrived some other way, not the primary path.
    fn pause<T: MeshTransport, R: CryptoRngCore>(&mut self, server: &Server<T, R>) {
        self.paused = true;
        self.pause_notice = true;
        self.handoff
            .release_if_stalled(server.runtime_clock().monotonic_ms());
        // A paused receiver runs no turn, so a held inventory job and its plaintext are released
        // now rather than left resident for the whole pause.
        self.inventory.release();
        // Likewise a Save capture still queued for its plan (design 18.3 review, F4): it would
        // hold admission, a pool permit and its media hold for the whole pause. The slot holds at
        // most one of a queued capture, a detached job and a parked plan, so the request it
        // belonged to is whichever of the two is set; forget both.
        if self.catchup.release_queued_overlay() {
            self.closing_scheduled = None;
            self.unconfirmed_scheduled = None;
        }
    }
    pub(crate) fn take_pause_notice(&mut self) -> bool {
        std::mem::take(&mut self.pause_notice)
    }

    fn observe<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
        target: StudioTarget,
        epoch: u128,
    ) -> Result<(), AppError> {
        // A logical object cannot acquire extra slots by changing its claimed channel.
        let logical = target.document(&server.group_id()).map_err(invalid)?;
        if let Some(i) = self.watches.iter().position(|(w, _)| {
            w.target.document(&server.group_id()).ok().as_ref() == Some(&logical)
        }) {
            let (old, held_epoch) = self.watches.remove(i).expect("watch index");
            if old.target == target
                && old.server == id
                && held_epoch == epoch
                && Arc::ptr_eq(&old.mount, &store.registry_mount())
                && server.sync.studio_watch_is_current(&old.inner)
            {
                self.watches.push_back((old, held_epoch));
                return Ok(());
            }
            let _ = server.unwatch_studio_epoch(&old);
        }
        if self.watches.len() == 16 {
            let (old, _) = self.watches.pop_front().expect("full watch rail");
            let _ = server.unwatch_studio_epoch(&old);
        }
        // The existing transaction already validated channel/member and loaded this concrete
        // epoch (or proved absence). Do not replay the full source again just to subscribe.
        let inner = server.sync.watch_studio(target, epoch)?;
        self.watches.push_back((
            ServerStudioWatch {
                inner,
                mount: store.registry_mount(),
                server: id,
                target,
            },
            epoch,
        ));
        Ok(())
    }

    /// Executed only inside the same off-executor Server/vault/native lease as ordinary Save.
    /// A busy/locked caller never reaches this method; one pass consumes at most one packet.
    pub(crate) fn run<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        request: Option<StudioRequest>,
    ) -> Result<(StudioSavedTransaction, Option<StudioTarget>), AppError> {
        // C-3 N-M1: compare the vault's inventory token with how the last visit left it before
        // anything here can write, and mark how this one leaves it on every exit, so the next
        // visit can tell this actor's writes from anyone else's.
        self.inventory.begin_visit(store);
        let result = self.run_visit(server, store, id, request);
        self.inventory.end_visit(store);
        result
    }

    fn run_visit<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        request: Option<StudioRequest>,
    ) -> Result<(StudioSavedTransaction, Option<StudioTarget>), AppError> {
        // A background pass may not reinterpret existing watches under a different vault or
        // numeric server, including lifecycle snapshot writes. Explicit access rebinds them.
        if request.is_none()
            && self
                .watches
                .iter()
                .any(|(w, _)| w.server != id || !Arc::ptr_eq(&w.mount, &store.registry_mount()))
        {
            for (watch, _) in &self.watches {
                let _ = server.unwatch_studio_epoch(watch);
            }
            return Err(invalid("Studio receive mount or numeric server changed"));
        }
        self.catchup.lifecycle(server, store, id);
        self.inventory
            .lifecycle(store, id, server.runtime_clock().monotonic_ms());
        // A parked Save plan whose caller never came back holds this actor's admission and a
        // process-wide preparation permit; only a Save visit consumes it. Bounded like the
        // retained Registry source, and its request forgotten with it (design 8.7).
        if self
            .catchup
            .expire_parked_overlay(server.runtime_clock().monotonic_ms())
        {
            self.closing_scheduled = None;
            self.unconfirmed_scheduled = None;
        }
        self.catchup.preview.maintain(server, store, id);
        let mls = server.sync.with_registry_context(|g, _, _, _| g.epoch());
        self.replay.lifecycle(store, id, mls);
        // A lifecycle step, not a scheduling one: a handoff job whose authority has moved can
        // never be signed or committed again, and releasing it must not depend on which branch
        // this turn happens to take. Holding it would strand this actor's admission and one of
        // four process-wide preparation permits indefinitely.
        self.handoff_check_authority(server);
        if let Some(request) = request {
            let read_target = (!request.changes_state()).then(|| request.target());
            let updated = request.changes_state().then(|| request.target());
            let result = server.studio_transaction_with_publication(store, id, request);
            if let Some(target) = updated {
                self.settlement.note(
                    target,
                    match &result {
                        Ok(saved) => saved
                            .view
                            .as_ref()
                            .map(|v| v.phase.into())
                            .unwrap_or(StudioSettlementState::RefreshRequired),
                        Err(_) => StudioSettlementState::RefreshRequired,
                    },
                );
            }
            let mut saved = result?;
            // Only explicit successful access retries a failed/over-budget background pass.
            // More inbound traffic cannot repeatedly restart expensive failed disk work.
            self.paused = false;
            self.catchup
                .explicit_retry(server.runtime_clock().monotonic_ms());
            for &(target, epoch) in &saved.observed {
                // A trusted cooperative caller can occupy the sync watch rail separately. A
                // watch refusal must not misreport an already-durable Save as failed.
                if self.observe(server, store, id, target, epoch).is_err() {
                    tracing::warn!("Studio watch admission unavailable; saved state unchanged");
                }
            }
            if let Some(target) = read_target {
                // An absent Index's synthetic empty epoch zero must not hide a ready preview.
                // A stored source always wins, including a stored but empty document.
                if self
                    .catchup
                    .preview
                    .read(server, store, id, target)
                    .is_some()
                {
                    let absent = server.sync.with_registry_context(|g, d, _, _| {
                        store
                            .load_studio_epoch(id, g, target, d)
                            .map(|source| source.is_none())
                    })?;
                    if absent {
                        saved.preview = self.catchup.preview.read(server, store, id, target);
                    }
                }
            }
            return Ok((saved, updated));
        }
        let empty = || StudioSavedTransaction {
            view: None,
            preview: None,
            packets: vec![],
            observed: vec![],
        };
        // Subscription/channel churn is not a vault fault. Revoke stale interests before
        // catch-up can select them, without touching a newer external watch of the same key.
        self.watches.retain(|(watch, _)| {
            let keep = watch.server == id
                && Arc::ptr_eq(&watch.mount, &store.registry_mount())
                && server.sync.studio_watch_is_current(&watch.inner)
                && server
                    .channels()
                    .iter()
                    .any(|c| c.id == u128::from_be_bytes(watch.target.channel()));
            if !keep {
                let _ = server.unwatch_studio_epoch(watch);
            }
            keep
        });
        if self.paused {
            // A paused receiver never reaches `background_step`, so a held handoff job can never
            // advance. Release it rather than letting it hold admission and a process-wide
            // preparation permit until the user happens to open a Studio document.
            self.handoff
                .release_if_stalled(server.runtime_clock().monotonic_ms());
            return Ok((empty(), None));
        }
        let serving = server.sync.has_epoch_service_interest()
            || self
                .watches
                .iter()
                .any(|(w, _)| server.sync.studio_has_page_request(&w.inner));
        if (serving && self.gossip_runs >= 1)
            || (self.gossip_runs >= 4
                && (self.replay.has_work(&self.watches)
                    || self.catchup.pending(server, &self.watches)))
        {
            self.gossip_runs = 0;
            return match self.background_step(server, store, id) {
                Ok(saved) => Ok(saved),
                Err(error) => {
                    self.pause(server);
                    Err(error)
                }
            };
        }
        let Some(index) = self
            .watches
            .iter()
            .position(|(w, _)| server.sync.studio_has_inbound(&w.inner))
        else {
            let result = self.background_step(server, store, id);
            return match result {
                Ok(saved) => Ok(saved),
                Err(error) => {
                    self.pause(server);
                    Err(error)
                }
            };
        };
        let (watch, epoch) = self.watches.remove(index).expect("pending watch index");
        self.gossip_runs = self.gossip_runs.saturating_add(1);
        // Round-robin work selection: one busy document cannot monopolize every receive pass.
        self.watches.push_back((watch, epoch));
        let watch = &self.watches.back().expect("selected watch").0;
        if watch.server != id || !Arc::ptr_eq(&watch.mount, &store.registry_mount()) {
            let _ = server.unwatch_studio_epoch(watch);
            return Err(invalid("Studio receive mount or numeric server changed"));
        }
        if !server
            .channels()
            .iter()
            .any(|c| c.id == u128::from_be_bytes(watch.target.channel()))
        {
            let _ = server.unwatch_studio_epoch(watch);
            return Err(invalid("Studio receive channel was removed"));
        }
        // Persist the current membership/device state before admitting a source that needs it on
        // restart. Exclusive native snapshot ordering remains held by the caller throughout.
        if !server.sync.check_studio_inbound(&watch.inner)? {
            return Ok((empty(), None));
        }
        let received = (|| {
            // Serving another watched target can displace this graph. Prepare the existing
            // source off-actor before draining; a healthy cold target is not a storage failure.
            // Actual absence needs no rebuild; inventory still verifies that fact before Save.
            let large_cold = server.sync.with_registry_context(|g, d, _, _| {
                if store.studio_source_is_warm(id, g, watch.target, d) {
                    Ok(false)
                } else {
                    store.studio_receive_needs_preparation(id, &g.group_id(), watch.target)
                }
            })?;
            if large_cold && !self.catchup.prepare(server, store, id, watch.target)? {
                return Ok(None);
            }
            // A warm candidate permits only bounded authentication, not stale-source use.
            // The store takes ownership and checks exact bytes again before actual ingest.
            server.sync.with_registry_context(|group, device, _, _| {
                if !store.studio_source_is_warm(id, group, watch.target, device) {
                    store.check_studio_receive_source_bound(id, &group.group_id(), watch.target)?;
                }
                Ok::<_, AppError>(())
            })?;
            // Limit before ANY source reconstruction or snapshot write. Directory order can
            // affect which small records are inspected, never produce a partial successful budget.
            let mut scan = store.scan_studio_receive_inventory()?;
            while !scan.step()?.complete {}
            let inventory = scan.finish()?;
            let snapshot = server.snapshot()?;
            server
                .sync
                .with_registry_context(|_, _, _, rng| store.save_server(id, &snapshot, rng))?;
            let mut budget = server.sync.with_registry_context(|g, _, _, _| {
                store.studio_storage_budget(id, g, &inventory)
            })?;
            server.receive_studio_step_reusing(store, watch, &mut budget)
        })();
        let received = match received {
            Ok(received) => received,
            Err(error) => {
                // A pre-drain failure retains its packet; an admission failure may consume it.
                // Neither is delivery. Hold BOTH cases instead of a peer-driven disk retry loop.
                self.pause(server);
                return Err(error);
            }
        };
        let updated = if let Some(received) = received {
            let updated = (received.admission == Admission::Accepted).then_some(watch.target);
            server.sync.with_registry_context(|group, device, _, _| {
                store.retain_received_studio_source(group, device, received.state);
            });
            updated
        } else {
            None
        };
        Ok((empty(), updated))
    }
}
