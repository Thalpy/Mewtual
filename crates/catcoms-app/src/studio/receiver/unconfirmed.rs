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
//! No tenure is read anywhere here: an Unconfirmed branch has none (design 8.5).
use super::*;
use crate::store::{EpochStudioBudget, StudioOverlayMint, StudioOverlayStart};
use catcoms_replication::studio::StudioOverlaySave;
use catcoms_replication::DomainOp;

impl StudioReceiver {
    /// The two control actions of design 8.7, intercepted before the control transaction because
    /// only the receiver holds the ready preview.
    ///
    /// The same scope checks the transaction makes first: the request's own bounds, then a channel
    /// this server knows. The budget comes from a completed five-family inventory, as for every
    /// other lifecycle control. Nothing here reads a tenure (design 8.5).
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
        let mut budget = {
            let mut scan = store.scan_epoch_storage_with_studio()?;
            while !scan.step()?.complete {}
            let inventory = scan.finish()?;
            server.sync.with_registry_context(|g, _, _, _| {
                store.studio_storage_budget(id, g, &inventory)
            })?
        };
        match request.action {
            StudioControlAction::BeginUnconfirmedOverlaySave => {
                let (basis, branch) =
                    self.begin_unconfirmed_overlay_save(server, store, id, target, &mut budget)?;
                Ok(StudioControlResponse::UnconfirmedOverlaySaveTicket {
                    target,
                    basis,
                    branch,
                })
            }
            StudioControlAction::SaveUnconfirmedOverlay(save) => {
                let save = *save;
                let operation = crate::studio::domain(target, save.nonce, save.body);
                let visit = self.save_unconfirmed_overlay(
                    server,
                    store,
                    id,
                    target,
                    save.basis,
                    save.branch,
                    operation,
                    &mut budget,
                )?;
                let outcome = match visit {
                    StudioOverlaySaveVisit::Saved(saved) => match *saved {
                        StudioOverlaySave::Local(draft) => StudioUnconfirmedSaveOutcome::Saved {
                            basis: draft.basis(),
                            accepted: draft.accepted(),
                        },
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
                // A durable acceptance changes what the lifecycle row reports.
                if matches!(outcome, StudioUnconfirmedSaveOutcome::Saved { .. }) {
                    self.settlement
                        .note(target, StudioSettlementState::RefreshRequired);
                }
                Ok(StudioControlResponse::UnconfirmedOverlaySaved { target, outcome })
            }
            _ => Err(invalid("not an unconfirmed draft Save")),
        }
    }

    /// The ticket an Unconfirmed Save carries: the basis fingerprint and the branch the store names
    /// for it, both from one fresh mint in one custody visit. The Closing form is
    /// `Server::prepare_studio_closing_overlay`.
    ///
    /// Refuses when there is no live, tail-complete preview of `target`, with the mint's own
    /// reason. That is the only way to start new work. An exact retry of accepted work needs no
    /// ticket, because it carries the one it was saved with.
    pub(crate) fn begin_unconfirmed_overlay_save<T: MeshTransport + 'static, R: CryptoRngCore>(
        &self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        budget: &mut EpochStudioBudget,
    ) -> Result<([u8; 32], [u8; 32]), AppError> {
        let basis = self.catchup.preview.mint(server, store, id, target)?;
        let branch = server.sync.with_registry_context(|group, _, _, _| {
            store.studio_overlay_request_branch(id, group, target, &basis, budget)
        })?;
        Ok((basis.fingerprint(), branch))
    }

    /// One custody visit of the scheduled Unconfirmed Save. The same contract as the Closing
    /// `save_overlay`: at most one of committing a plan an earlier visit left ready, settling a
    /// classification terminally, or capturing new authoring and detaching it.
    ///
    /// The caller repeats the same request until it is `Saved`; it stays retryable and
    /// byte-stable in between. `budget` is the caller's, from a completed inventory, exactly as
    /// for the Closing form.
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
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioOverlaySaveVisit, AppError> {
        let request = request_fingerprint(target, &basis, &branch, &operation);
        // A plan this actor already produced is finished first, as for Closing: its transient
        // hold protects its pixels and it occupies admission until consumed. S3 re-enters the
        // live check with an attempt made in THIS visit, never one parked with the plan. A plan of
        // the other kind is refused by the store, by name, before it reads anything.
        if let Some((plan, ownership)) = self.catchup.take_planned_overlay(target) {
            // Whose plan this is. The slot holds one plan per target whichever request made it,
            // and the plan does not say. If it is another request's, it is still finished here.
            // Leaving it parked until its caller returns would hold the slot, and with it every
            // later Save, for as long as that caller stays away. But it is not reported as this
            // request's: that caller learns its outcome on its own retry, as an exact retry.
            let ours = self.unconfirmed_scheduled.take() == Some(request);
            let attempt = self.catchup.preview.mint(server, store, id, target);
            let committed = server.sync.with_registry_context(|group, device, _, rng| {
                store.commit_studio_overlay_with(
                    id,
                    group,
                    target,
                    device,
                    StudioOverlayMint::unconfirmed(attempt),
                    *plan,
                    rng,
                    budget,
                )
            });
            // Released only after the commit attempt returns, on success and on error alike.
            drop(ownership);
            if !ours {
                // Nothing of THIS request was saved, whatever the other commit did. Its success
                // or refusal belongs to its own caller's retry, not to this response.
                return Ok(StudioOverlaySaveVisit::Busy);
            }
            return committed.map(|draft| {
                StudioOverlaySaveVisit::Saved(Box::new(StudioOverlaySave::Local(draft)))
            });
        }
        // 7.2: reserve before the first bounded read, release by dropping if nothing is scheduled.
        let Some(ownership) = self.catchup.reserve_overlay() else {
            return Ok(StudioOverlaySaveVisit::Busy);
        };
        let attempt = self.catchup.preview.mint(server, store, id, target);
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
                    budget,
                )
            })?;
        match started {
            StudioOverlayStart::Settled(saved) => Ok(StudioOverlaySaveVisit::Saved(saved)),
            StudioOverlayStart::Captured(capture) => {
                self.catchup.schedule_overlay(*capture, ownership, target);
                self.unconfirmed_scheduled = Some(request);
                Ok(StudioOverlaySaveVisit::Scheduled)
            }
        }
    }
}

/// The identity of one Save request: everything the renderer sent, under a domain of its own. Two
/// requests are the same exactly when an exact retry of one would be the other. Used only to tell
/// whose parked plan a visit is finishing; never persisted and never authority.
fn request_fingerprint(
    target: StudioTarget,
    basis: &[u8; 32],
    branch: &[u8; 32],
    operation: &DomainOp,
) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-unconfirmed-save-request/v1");
    hash.update(&target.channel());
    match target {
        StudioTarget::Index { .. } => hash.update(&[0]),
        StudioTarget::Flipnote { object, .. } => hash.update(&[1]).update(&object),
    };
    hash.update(basis)
        .update(branch)
        .update(&operation.nonce)
        .update(&(operation.body.len() as u64).to_be_bytes())
        .update(&operation.body);
    *hash.finalize().as_bytes()
}
