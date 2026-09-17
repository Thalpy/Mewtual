//! Scheduled automatic handoff (Flow H), stages H1 to H6.
//!
//! One job per actor, sharing the same per-actor admission and the same four-slot process-wide
//! preparation pool as local Save, because "one overlay operation is live per server at a time"
//! is a property of the actor, not of a particular flow.
//!
//! The stages alternate custody and detachment:
//!
//! ```text
//! H1 probe + capture   custody      cheap: bounded reads, structural decode, authority mint
//! H2 prepare           detached     full decode_vault, successor restore, change set
//! H3 sign slice        custody      bounded turns, repeated across background turns
//! H4 assemble          detached     finish, complete, snapshot, record encodings
//! H5 commit            custody      Prepared -> whole Source -> Completed
//! H6 notify            custody      settlement notice for the finished transfer
//! ```
//!
//! Only H3 is paged. H2 and H4 are single detached jobs, and H5 is the accepted durable
//! transaction, unchanged.
use super::*;
use crate::store::{StudioHandoffCapture, StudioHandoffCommit, StudioHandoffPlan};
use crate::studio::overlay::OverlayOwnership;
use std::collections::BTreeMap;

/// Where a job is, and who owns its bundle right now. `Detached` carries no ownership on purpose:
/// a worker holds it, so a cancelled waiter cannot take it back and admission stays occupied until
/// that worker actually ends.
pub(super) enum HandoffStage {
    /// H1 done; waiting for a background turn to detach H2.
    Captured(Box<StudioHandoffCapture>, OverlayOwnership),
    /// H2 or H4 is running on a worker, which owns the bundle.
    Detached,
    /// H3: signing, paged across visits.
    Signing(Box<StudioHandoffPlan>, OverlayOwnership),
    /// H4 done; waiting for a custody visit to run H5.
    Ready(Box<StudioHandoffCommit>, OverlayOwnership),
}

pub(super) struct HandoffJob {
    pub(super) target: StudioTarget,
    pub(super) stage: HandoffStage,
    /// The observed owner tenure H1 minted this job's authority under.
    pub(super) tenure: u64,
    /// The MLS epoch H1 minted it under.
    ///
    /// Tenure alone is **not** enough, and assuming it was is what made the first version of this
    /// check blind to the very failure it was written for. `observed_owner_tenure_start` reports
    /// when the current owner's tenure *began*, and a same-owner MLS commit preserves that value
    /// (`OwnerTenure::applied` takes the "same owner preserves knowledge" branch). But
    /// `StudioHandoffAuthority` pins `group.epoch()`, so that same commit makes the job
    /// permanently unsignable while the tenure comparison still matches.
    pub(super) mls: u64,
}

/// Per-actor handoff scheduling. Holds at most one job.
#[derive(Default)]
pub(super) struct HandoffRuntime {
    pub(super) job: Option<HandoffJob>,
    /// 7.2's memo: targets with no transferable overlay, valid only while the store's intent
    /// generation is unchanged, so a quiescent vault is not re-probed every background turn.
    quiet: Vec<StudioTarget>,
    quiet_generation: Option<Arc<()>>,
    /// Backoff after a refused probe or an abandoned job. **Per target**, because a shared scalar
    /// lets one permanently ineligible document starve every other document on the rail: the
    /// probe would keep selecting it, keep failing, and drive the shared hold to its maximum.
    /// Design 7.3's pacing: 30 s doubling to 300 s, reset on durable progress.
    next_at: BTreeMap<StudioTarget, u64>,
    hold_ms: BTreeMap<StudioTarget, u64>,
    /// Round-robin cursor over the watch rail, so selection rotates instead of always taking the
    /// first eligible target.
    selection: usize,
}

const FIRST_HOLD_MS: u64 = 30_000;
const MAX_HOLD_MS: u64 = 300_000;

impl HandoffRuntime {
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// True when a signing slice could run right now. Signing needs no new permit and no retained
    /// source, so it is allowed on any background turn, which is why the gate is about priority
    /// rather than about capacity.
    pub(super) fn can_sign(&self) -> bool {
        matches!(
            self.job.as_ref().map(|j| &j.stage),
            Some(HandoffStage::Signing(..))
        )
    }

    pub(super) fn can_commit(&self) -> bool {
        matches!(
            self.job.as_ref().map(|j| &j.stage),
            Some(HandoffStage::Ready(..))
        )
    }

    fn hold_target(&mut self, target: StudioTarget, now: u64) {
        let next = match self.hold_ms.get(&target) {
            Some(ms) => (ms * 2).min(MAX_HOLD_MS),
            None => FIRST_HOLD_MS,
        };
        self.hold_ms.insert(target, next);
        self.next_at.insert(target, now.saturating_add(next));
    }

    fn held(&self, target: StudioTarget, now: u64) -> bool {
        self.next_at.get(&target).is_some_and(|at| now < *at)
    }

    // There is deliberately no `hold(&mut self)` that reads `self.job`. Every caller that gives up
    // on a job has already removed it, so such a method silently records nothing — which is
    // exactly how an H5 refusal became an unpaced retry of the most expensive stage in the flow.
    // `hold_target` takes the target explicitly so the mistake cannot recur.

    /// Durable progress resets that target's pacing, so a server that transfers successfully does
    /// not inherit a long hold from an earlier refusal.
    fn progressed(&mut self, target: StudioTarget) {
        self.hold_ms.remove(&target);
        self.next_at.remove(&target);
    }

    /// The tenure and MLS epoch the live job was minted under, if there is one.
    fn authority(&self) -> Option<(u64, u64)> {
        self.job.as_ref().map(|job| (job.tenure, job.mls))
    }

    /// A job that could make progress this turn. `busy` alone is not that: a job held by backoff,
    /// or one waiting on a gate, cannot run, and treating it as pending keeps the driver awake at
    /// its active cadence for the whole life of the job.
    pub(super) fn runnable(&self, now: u64) -> bool {
        match self.job.as_ref().map(|job| (&job.stage, job.target)) {
            Some((HandoffStage::Detached, _)) => false,
            Some((_, target)) => !self.held(target, now),
            None => false,
        }
    }

    fn remaining(&self) -> Option<usize> {
        match self.job.as_ref().map(|job| &job.stage) {
            Some(HandoffStage::Signing(plan, _)) => Some(plan.remaining()),
            _ => None,
        }
    }

    /// Release a job that cannot advance because the receiver is paused.
    ///
    /// A paused receiver never reaches `background_step` and `detach` returns nothing, so a
    /// `Captured`, `Signing` or `Ready` job can never progress, while `pending` is false so the
    /// driver idles. Unlike Flow S, where a parked capture implies a user Save, Flow H creates
    /// these automatically on any background turn, so paused actors could otherwise exhaust the
    /// four-slot process-wide pool with no user action anywhere. `Detached` is left alone: its
    /// worker owns the bundle and releases it when it ends.
    pub(super) fn release_if_stalled(&mut self, now: u64) {
        if matches!(self.job.as_ref().map(|job| &job.stage), Some(stage) if !matches!(stage, HandoffStage::Detached))
        {
            self.abandon(now);
        }
    }

    /// Give up on the current job and release everything it holds.
    ///
    /// Every stage that can fail routes here. A handoff is opportunistic background work: nothing
    /// about it should pause the actor or keep capacity that can no longer be used. Signed work is
    /// cheap to redo from H1; a stranded admission token and a process-wide preparation permit are
    /// not cheap at all. `Detached` holds nothing, because a worker owns its bundle and releases
    /// it when it ends.
    fn abandon(&mut self, now: u64) {
        // The target must be read before the job is cleared. Clearing first leaves `hold` with
        // nothing to back off, and the probe then restarts the same job on the very next turn,
        // which is a hot loop rather than a release.
        if let Some(target) = self.job.take().map(|job| job.target) {
            self.hold_target(target, now);
        }
    }

    fn quiet_for(&mut self, generation: &Arc<()>, target: StudioTarget) {
        if !self
            .quiet_generation
            .as_ref()
            .is_some_and(|g| Arc::ptr_eq(g, generation))
        {
            self.quiet.clear();
            self.quiet_generation = Some(generation.clone());
        }
        if !self.quiet.contains(&target) {
            self.quiet.push(target);
        }
    }

    fn is_quiet(&self, generation: &Arc<()>, target: StudioTarget) -> bool {
        self.quiet_generation
            .as_ref()
            .is_some_and(|g| Arc::ptr_eq(g, generation))
            && self.quiet.contains(&target)
    }

    #[cfg(test)]
    pub(super) fn stage_for_test(&self) -> Option<&'static str> {
        self.job.as_ref().map(|job| match job.stage {
            HandoffStage::Captured(..) => "captured",
            HandoffStage::Detached => "detached",
            HandoffStage::Signing(..) => "signing",
            HandoffStage::Ready(..) => "ready",
        })
    }

    /// Move the job's recorded **owner tenure** away from the live one, which is what an owner
    /// change or an unobserved gap does.
    #[cfg(test)]
    pub(super) fn stale_tenure_for_test(&mut self) {
        if let Some(job) = self.job.as_mut() {
            job.tenure = u64::MAX;
        }
    }

    /// Move the job's recorded **MLS epoch** away from the live one, which is what any commit
    /// does: a member joining or leaving, or a key rotating.
    ///
    /// This is the case the first version of the authority check could not see, because a
    /// same-owner commit leaves the observed tenure start untouched. Only the precondition is
    /// simulated; the comparison, the abandonment and the receiver's reaction are production.
    #[cfg(test)]
    pub(super) fn stale_mls_for_test(&mut self) {
        if let Some(job) = self.job.as_mut() {
            job.mls = job.mls.wrapping_add(1);
        }
    }

    #[cfg(test)]
    pub(super) fn remaining_for_test(&self) -> Option<usize> {
        match self.job.as_ref().map(|job| &job.stage) {
            Some(HandoffStage::Signing(plan, _)) => Some(plan.remaining()),
            _ => None,
        }
    }
}

impl StudioReceiver {
    /// Abandon a job whose authority has moved, at **any** stage.
    ///
    /// A job's `StudioHandoffAuthority` pins **both** the owner tenure and the MLS epoch it was
    /// minted under, so both have to be compared. `observed_owner_tenure_start` alone is not a
    /// proxy for the pair: it reports when the current owner's tenure *began*, and a same-owner
    /// commit leaves that value untouched, so a check written on tenure alone cannot see the
    /// commonest way a job dies.
    ///
    /// Once either fact moves, the job can never be signed (H3 rechecks before every signature)
    /// and can never be committed (the H5 stamp carries the tenure). Parking it would strand this
    /// actor's admission and one of four process-wide preparation permits permanently, and letting
    /// the refusal surface as an error would pause the receiver's whole background loop.
    ///
    /// So: drop it, release the bundle, back that target off, and carry on. Signed work is cheap
    /// to redo from H1; the capacity is not.
    pub(super) fn handoff_check_authority<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
    ) {
        let Some((tenure, mls)) = self.handoff.authority() else {
            return;
        };
        // BOTH facts, not just tenure. Comparing tenure alone missed a same-owner MLS commit
        // entirely, which is the commonest way a job dies: a member joins or leaves, or a key
        // rotates, `OwnerTenure::applied` takes its "same owner preserves knowledge" branch so
        // the tenure start is unchanged, and the job is silently unsignable for ever. The
        // fallback that was meant to catch it, `sign_next` refusing, never runs on a priority
        // turn, because `sign_slice` yields before it signs.
        let live_mls = server.sync.with_registry_context(|g, _, _, _| g.epoch());
        if server.sync.observed_owner_tenure_start() != Some(tenure) || live_mls != mls {
            self.handoff.abandon(server.runtime_clock().monotonic_ms());
        }
    }

    /// H1. Find a target whose accepted local draft can now be transferred, and capture it.
    ///
    /// Round-robin over the watch rail, one target per turn. A target with no transferable branch
    /// is memoised against the store's intent generation (7.2), so a quiescent vault costs one
    /// cheap structural read per target and then nothing until an intent write rotates the token.
    ///
    /// A refusal is ordinary: a Closing document that has not rotated yet refuses every time, and
    /// per-target paced backoff rather than a hot loop is what keeps that cheap.
    ///
    /// Nothing here can fail the background turn. A probe is the least urgent work the receiver
    /// does, and no outcome of it justifies pausing catch-up, replay and receive.
    pub(super) fn handoff_probe<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) {
        if self.handoff.busy() || self.watches.is_empty() {
            return;
        }
        let now = server.runtime_clock().monotonic_ms();
        let generation = store.intent_generation();
        let group = server.group_id();
        let device = server
            .sync
            .with_registry_context(|_, d, _, _| d.device_id());

        // Round-robin from the cursor, so one ineligible document cannot monopolise selection.
        // A read error is recorded as an error, never silently memoised as "nothing here": doing
        // that would suppress every handoff on this actor until an unrelated intent write
        // happened to rotate the generation token.
        let rail: Vec<_> = self.watches.iter().map(|(w, _)| w.target).collect();
        let start = self.handoff.selection % rail.len();
        self.handoff.selection = start.wrapping_add(1);
        let mut found = None;
        // The target that actually failed to read, not the cursor's origin. Holding the origin
        // left the failing record to be re-read every turn while penalising an innocent,
        // eligible document, and because the cursor advances each turn a single bad record
        // walked the whole rail to the 300 s cap.
        let mut unreadable: Option<StudioTarget> = None;
        let mut quiet = Vec::new();
        for offset in 0..rail.len() {
            let target = rail[(start + offset) % rail.len()];
            if self.handoff.is_quiet(&generation, target) || self.handoff.held(target, now) {
                continue;
            }
            let Ok(logical) = target.document(&group) else {
                continue;
            };
            // Structural: the probe needs the branch's author and basis, never its projection.
            match store.load_epoch_intents_structural(id, &logical) {
                Ok(state) => match state.handoff_metadata().and_then(|m| m.overlay()) {
                    Some(overlay) if overlay.author() == device => {
                        found = Some((target, overlay.basis()));
                        break;
                    }
                    _ => quiet.push(target),
                },
                Err(_) => unreadable = unreadable.or(Some(target)),
            }
        }
        // Only targets actually read and found to have nothing are memoised.
        for target in quiet {
            self.handoff.quiet_for(&generation, target);
        }
        let Some((target, basis)) = found else {
            if let Some(bad) = unreadable {
                self.handoff.hold_target(bad, now);
            }
            return;
        };

        // 7.2: reserve admission and the shared slot before the first authorization read, and
        // release by dropping if H1 concludes there is nothing to schedule.
        let Some(ownership) = self.catchup.reserve_overlay() else {
            // Admission is busy, or the shared pool is full. Back this target off rather than
            // re-reading the rail on every turn while a Save holds the slot.
            self.handoff.hold_target(target, now);
            return;
        };
        // A transfer needs a live tenure to mint its authority. Without one there is nothing to
        // capture, and the reservation is released by dropping `ownership` on return.
        let Some(tenure) = server.sync.observed_owner_tenure_start() else {
            self.handoff.hold_target(target, now);
            return;
        };
        let Ok(mut budget) = self.handoff_budget(server, store, id) else {
            self.handoff.hold_target(target, now);
            return;
        };
        let started = server.sync.with_registry_context(|g, d, _, rng| {
            store.start_studio_handoff(id, g, target, d, basis, Some(tenure), rng, &mut budget)
        });
        match started {
            Ok(crate::store::StudioHandoffStart::Captured(capture)) => {
                self.handoff.job = Some(HandoffJob {
                    target,
                    stage: HandoffStage::Captured(capture, ownership),
                    tenure,
                    mls: server.sync.with_registry_context(|g, _, _, _| g.epoch()),
                });
            }
            Ok(crate::store::StudioHandoffStart::Settled(_)) => {
                // Already transferred and acknowledged. Durable progress, so pacing resets and
                // the reservation is released by dropping `ownership` here.
                self.handoff.progressed(target);
                self.settlement
                    .note(target, StudioSettlementState::RefreshRequired);
            }
            Err(_) => {
                // Not eligible yet, or not eligible at all. Both are ordinary; back off.
                self.handoff.hold_target(target, now);
            }
        }
    }

    /// Take whichever detached stage is ready, if any. H2 and H4 are single jobs; H3 is not here
    /// because signing never detaches.
    pub(super) fn handoff_detach<T: MeshTransport>(&mut self) -> Option<StudioBackgroundJob<T>> {
        let job = self.handoff.job.as_mut()?;
        let target = job.target;
        match std::mem::replace(&mut job.stage, HandoffStage::Detached) {
            HandoffStage::Captured(capture, ownership) => Some(
                StudioBackgroundJob::handoff_prepare(capture, ownership, target),
            ),
            // H4 detaches only once the whole branch is signed; a partly signed plan goes back.
            HandoffStage::Signing(plan, ownership) if plan.remaining() == 0 => Some(
                StudioBackgroundJob::handoff_assemble(plan, ownership, target),
            ),
            stage => {
                job.stage = stage;
                None
            }
        }
    }

    /// H3. One bounded signing slice, under custody, on a background turn.
    ///
    /// `priority` is 7.3's placement answer: authoritative service interest, inbound on any watch,
    /// or a parked background result. A yield signs zero and says so, which is a different event
    /// from a slice that hit its turn cap or its deadline with work left.
    pub(super) fn handoff_sign<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        priority: bool,
    ) -> Option<crate::store::SigningSlice> {
        if !self.handoff.can_sign() {
            return None;
        }
        // Nothing left to sign: H4 detaches next turn. Re-entering `sign_next` here would
        // recheck live authority for no signature and could abandon a finished plan.
        if self.handoff.remaining() == Some(0) {
            return None;
        }
        let tenure = server.sync.observed_owner_tenure_start()?;
        let job = self.handoff.job.as_mut()?;
        let HandoffStage::Signing(plan, _) = &mut job.stage else {
            return None;
        };
        let clock = server.runtime_clock();
        let slice = server.sync.with_registry_context(|group, device, _, _| {
            plan.sign_slice(
                device,
                group,
                tenure,
                priority,
                crate::store::MAX_SIGNING_TURNS_PER_VISIT,
                Some((&*clock, crate::store::SIGNING_SLICE_BUDGET_MS)),
            )
        });
        match slice {
            Ok(slice) => Some(slice),
            // A refused signature is an expected outcome, not a vault fault: the design says a
            // detached result can become a stale proposal that H3 declines to sign. Abandon the
            // job, release its bundle and back off. It must never reach the receiver's storage
            // pause, which would stop catch-up, replay and receive until the user happens to open
            // a Studio document.
            Err(_) => {
                let now = server.runtime_clock().monotonic_ms();
                self.handoff.abandon(now);
                None
            }
        }
    }

    /// H5 and H6. Commit the assembled transfer, then notify.
    ///
    /// Nothing here can fail the background turn. A refused stamp, a budget that will not build,
    /// a write that errors: all of them abandon the job, release its bundle and back off. Pausing
    /// the receiver for any of them would stop catch-up, replay and receive until the user next
    /// opened a Studio document, which is a far worse outcome than a retried transfer.
    pub(super) fn handoff_commit<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> Option<StudioTarget> {
        if !self.handoff.can_commit() {
            return None;
        }
        let now = server.runtime_clock().monotonic_ms();
        let target = self
            .handoff
            .job
            .as_ref()
            .expect("checked just above")
            .target;
        // The budget is built BEFORE the job is taken. Taking first meant any transient inventory
        // or generation failure discarded H1 to H4 entirely: a detached full vault decode plus
        // every signature, thrown away for a retryable error. Flow S gets this right by taking its
        // plan only when the commit is about to run.
        let Ok(mut budget) = self.handoff_budget(server, store, id) else {
            self.handoff.hold_target(target, now);
            return None;
        };
        let job = self.handoff.job.take().expect("checked just above");
        let HandoffStage::Ready(commit, ownership) = job.stage else {
            unreachable!("can_commit checked the stage")
        };
        let tenure = server.sync.observed_owner_tenure_start();
        let committed = server.sync.with_registry_context(|group, device, _, rng| {
            store.commit_studio_handoff(
                id,
                group,
                target,
                device,
                *commit,
                tenure,
                rng,
                &mut budget,
            )
        });
        // Explicit: admission and the shared slot are released only after the write attempt
        // returns, on success and on error alike.
        drop(ownership);
        match committed {
            Ok(_) => {
                // H6. Durable progress: pacing resets and the view is stale.
                self.handoff.progressed(target);
                self.settlement
                    .note(target, StudioSettlementState::RefreshRequired);
                Some(target)
            }
            Err(_) => {
                // `hold_target`, not `hold`: the job is already taken, so the job-reading variant
                // would record nothing and the probe would re-capture the same target on the very
                // next turn, replaying two inventory drains, a full detached decode and every
                // signature before failing identically. H5 has refusals H1 does not — the
                // three-write preflights and the reference check — so this is reachable and not
                // self-limiting.
                self.handoff.hold_target(target, now);
                None
            }
        }
    }

    /// A detached handoff stage came back.
    ///
    /// The three outcomes are deliberately **not** collapsed into one arm. A completion that does
    /// not belong to the job this actor is currently running must be ignored, never used to clear
    /// it: that job may be mid-signing with real work and a live `OverlayOwnership`, and dropping
    /// it there would discard both.
    pub(super) fn handoff_complete(&mut self, result: HandoffCompletion, now: u64) {
        let mine =
            |job: &Option<HandoffJob>, target| job.as_ref().is_some_and(|job| job.target == target);
        match result {
            HandoffCompletion::Prepared(target, Ok((plan, ownership)))
                if mine(&self.handoff.job, target) =>
            {
                self.handoff.job.as_mut().expect("checked").stage =
                    HandoffStage::Signing(plan, ownership);
            }
            HandoffCompletion::Assembled(target, Ok((commit, ownership)))
                if mine(&self.handoff.job, target) =>
            {
                self.handoff.job.as_mut().expect("checked").stage =
                    HandoffStage::Ready(commit, ownership);
            }
            // A stage this actor asked for refused. The worker already released its bundle.
            HandoffCompletion::Prepared(target, Err(_))
            | HandoffCompletion::Assembled(target, Err(_))
                if mine(&self.handoff.job, target) =>
            {
                self.handoff.abandon(now);
            }
            // The waiter was cancelled. The worker still owns its bundle and releases it when it
            // ends, so nothing is freed here; the job is simply no longer tracked.
            HandoffCompletion::Cancelled(target) if mine(&self.handoff.job, target) => {
                // `hold_target`, not `hold`: the job is cleared here, so the job-reading variant
                // would record no backoff at all.
                self.handoff.job = None;
                self.handoff.hold_target(target, now);
            }
            // A completion for something this actor is not working on. Ignore it entirely: the
            // current job, whatever stage it is in, is untouched.
            _ => {}
        }
    }

    /// A five-family inventory and the Studio budget this actor needs for a handoff visit, built
    /// the same way the receive path builds one.
    fn handoff_budget<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> Result<crate::store::EpochStudioBudget, AppError> {
        let mut scan = store.scan_epoch_storage_with_studio()?;
        while !scan.step()?.complete {}
        let inventory = scan.finish()?;
        server
            .sync
            .with_registry_context(|g, _, _, _| store.studio_storage_budget(id, g, &inventory))
    }
}
