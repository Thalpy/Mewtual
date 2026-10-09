//! One bounded pass through saved OWN ids per watched Open binding. Ordinary Save owns
//! validation, durability and sending; this scheduler neither signs nor stores another log.
use super::*;
use crate::studio::replay::{choose, deconflict, ordered, ReplayChoice};
use std::collections::BTreeSet;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct ReplayRuntime {
    active: Option<Pass>,
    completed: VecDeque<(StudioTarget, u128)>,
    context: Option<(Arc<()>, u64, u64)>,
    next_at: u64,
}
struct Pass {
    target: StudioTarget,
    epoch: u128,
    order: VecDeque<[u8; 32]>,
    manual: BTreeSet<[u8; 32]>,
    history_ids: Vec<[u8; 32]>,
    /// When this pass first asked the shared inventory job for its manual move's budget.
    inventory_since: Option<u64>,
}

/// How long a manual move waits on the shared inventory job before minting its budget the way it
/// did before C-3 step 2, from one synchronous receive-profile scan in the visit (review of step
/// 2, HIGH-1). The classifier (`6a79e6f8`) validates small records of the uncached families inline,
/// so what still parks is each cold Studio or Registry record and any record too large to validate
/// inline. Each parked record costs one replay turn, about six seconds each in a quiet actor, and a
/// five-family write between any two turns throws that progress away. (Written before the
/// classifier landed, when every uncached record parked.) Without this bound a steadily edited document, sustained receive or another actor's
/// writes could stall the move indefinitely, and while it waits its pass holds replay for every
/// other watched target. The fallback adds no custody class: it is the scan this site always ran,
/// under the same receive limits that every receive packet already pays.
const MANUAL_MOVE_PATIENCE_MS: u64 = 60_000;

/// The manual move's budget from one synchronous scan under the receive profile, inside this
/// visit: the pre-step-2 path, kept as the bounded way forward when the shared job cannot finish.
fn synchronous_receive_budget(
    store: &mut ServerStore,
    id: u64,
    group: &catcoms_mls::ServerGroup,
) -> Result<crate::store::EpochStudioBudget, AppError> {
    let mut scan = store.scan_studio_receive_inventory()?;
    while !scan.step()?.complete {}
    let inventory = scan.finish()?;
    store.studio_storage_budget(id, group, &inventory)
}
impl ReplayRuntime {
    fn complete(&mut self, target: StudioTarget, epoch: u128) {
        self.active = None;
        self.completed.retain(|b| *b != (target, epoch));
        if self.completed.len() == 16 {
            self.completed.pop_front();
        }
        self.completed.push_back((target, epoch));
    }
    pub(super) fn lifecycle(&mut self, store: &ServerStore, id: u64, mls: u64) {
        let mount = store.registry_mount();
        if self
            .context
            .as_ref()
            .is_none_or(|(m, s, e)| !Arc::ptr_eq(m, &mount) || *s != id || *e != mls)
        {
            *self = Self {
                context: Some((mount, id, mls)),
                ..Default::default()
            };
        }
    }
    /// Uninspected bindings are checked by the normal five-second idle cadence (or a fair
    /// turn during gossip). They are not immediately actionable work just because they were
    /// read: doing so would advertise absent/no-intent documents forever and bypass backoff.
    pub(super) fn pending(
        &self,
        watches: &VecDeque<(ServerStudioWatch, u128)>,
        now: u64,
        current: impl Fn(&ServerStudioWatch) -> bool,
    ) -> bool {
        now >= self.next_at
            && self.active.as_ref().is_some_and(|p| {
                watches
                    .iter()
                    .any(|(w, epoch)| w.target == p.target && *epoch == p.epoch && current(w))
            })
    }
    pub(super) fn has_work(&self, watches: &VecDeque<(ServerStudioWatch, u128)>) -> bool {
        self.active.is_some()
            || watches
                .iter()
                .any(|(w, e)| !self.completed.contains(&(w.target, *e)))
    }
}
impl StudioReceiver {
    #[cfg(test)]
    pub(crate) fn replay_state_for_test(&self) -> (bool, usize) {
        (self.replay.active.is_some(), self.replay.completed.len())
    }
    /// Which state the shared inventory job is in.
    #[cfg(test)]
    pub(crate) fn inventory_state_for_test(&self) -> &'static str {
        self.inventory.state_for_test()
    }
    /// How many budgets the shared inventory job has minted.
    #[cfg(test)]
    pub(crate) fn inventory_minted_for_test(&self) -> usize {
        self.inventory.minted_for_test()
    }
    /// How long a manual move waits on the shared job before its synchronous fallback.
    #[cfg(test)]
    pub(crate) fn manual_move_patience_for_test() -> u64 {
        MANUAL_MOVE_PATIENCE_MS
    }
    /// Hold catch-up's network-pass flag, to test `detach`'s ordering against it.
    #[cfg(test)]
    pub(crate) fn hold_catchup_in_flight_for_test(&mut self, in_flight: bool) {
        self.catchup.set_in_flight_for_test(in_flight);
    }
    /// Whether catch-up still believes a network pass is in flight.
    #[cfg(test)]
    pub(crate) fn catchup_in_flight_for_test(&self) -> bool {
        self.catchup.in_flight_for_test()
    }
    /// Whether the last `run` left its end-of-visit inventory mark at the vault's current token.
    #[cfg(test)]
    pub(crate) fn inventory_marked_for_test(&self, store: &ServerStore) -> bool {
        self.inventory.marked_for_test(store)
    }
    /// Put the shared inventory job into the backoff a restart storm ends in.
    #[cfg(test)]
    pub(crate) fn inventory_back_off_for_test(&mut self, now: u64) {
        self.inventory.back_off_for_test(now)
    }
    /// Isolate one production replay turn from network scheduling in deterministic regressions.
    #[cfg(test)]
    pub(crate) fn replay_step_for_test<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> Result<Option<(StudioSavedTransaction, Option<StudioTarget>)>, AppError> {
        let deadline = server
            .runtime_clock()
            .monotonic_ms()
            .saturating_add(super::inventory::INVENTORY_SLICE_MS);
        self.replay_step(server, store, id, deadline)
    }
    /// `deadline_ms` is the visit's absolute inventory deadline, sampled once by
    /// `background_step`; only the manual move's inventory turn spends it.
    pub(super) fn replay_step<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        deadline_ms: u64,
    ) -> Result<Option<(StudioSavedTransaction, Option<StudioTarget>)>, AppError> {
        let now = server.runtime_clock().monotonic_ms();
        if now < self.replay.next_at || !self.catchup.replay_ready() {
            return Ok(None);
        }
        self.replay.next_at = now.saturating_add(1_000);
        self.replay
            .completed
            .retain(|(t, e)| self.watches.iter().any(|(w, id)| w.target == *t && id == e));
        if self.replay.active.as_ref().is_some_and(|p| {
            !self
                .watches
                .iter()
                .any(|(w, e)| w.target == p.target && *e == p.epoch)
        }) {
            self.replay.active = None;
        }
        let target = self
            .replay
            .active
            .as_ref()
            .map(|p| (p.target, p.epoch))
            .or_else(|| {
                self.watches
                    .iter()
                    .map(|(w, e)| (w.target, *e))
                    .find(|b| !self.replay.completed.contains(b))
            });
        let Some((target, epoch)) = target else {
            return Ok(None);
        };
        if !self.catchup.prepare(server, store, id, target)? {
            return Ok(None);
        }
        let Some(evidence) = server.studio_replay_evidence(store, id, target, epoch)? else {
            self.replay.complete(target, epoch);
            return Ok(None);
        };
        if self
            .replay
            .active
            .as_ref()
            .is_some_and(|p| p.history_ids != evidence.history_ids)
        {
            // A new/evicted recovery version can change cross-branch ambiguity. Rebuild the
            // bounded graph; operations already replayed are now current and are not reapplied.
            self.replay.active = None;
        }
        if self.replay.active.is_none() {
            let mut choices = evidence
                .own
                .iter()
                .map(|(id, intent)| {
                    choose(
                        &evidence.current,
                        &evidence.signed,
                        &evidence.history,
                        intent,
                    )
                    .map(|c| (*id, c))
                })
                .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
            deconflict(&mut choices, &evidence.own)?;
            let (order, manual) = ordered(&choices);
            self.replay.active = Some(Pass {
                target,
                epoch,
                order,
                manual,
                history_ids: evidence.history_ids.clone(),
                inventory_since: None,
            });
        }
        // Every previously chosen id is screened AGAIN against current source and all recovery
        // slots. An intervening remote edit or deletion changes Ready to manual, never overwrite.
        let pass = self.replay.active.as_mut().expect("selected pass");
        if let Some(&op_id) = pass.order.front() {
            let mut choice = evidence
                .own
                .get(&op_id)
                .map(|intent| {
                    choose(
                        &evidence.current,
                        &evidence.signed,
                        &evidence.history,
                        intent,
                    )
                })
                .transpose()?;
            if choice == Some(ReplayChoice::Ready) && matches!(target, StudioTarget::Index { .. }) {
                if let IndexOp::PutObject { object, .. } =
                    IndexOp::decode(&evidence.own[&op_id].operation.body).map_err(invalid)?
                {
                    let exists = server
                        .sync
                        .with_registry_context(|g, d, _, _| {
                            let referenced = StudioTarget::Flipnote {
                                channel: target.channel(),
                                object,
                            };
                            // Background replay must not use explicit Restore's cold rebuild
                            // allowance. A large unprepared target goes to manual recovery;
                            // that deliberate action can open/fetch it under normal UI custody.
                            if !store.studio_source_is_warm(id, g, referenced, d)
                                && store.studio_receive_needs_preparation(
                                    id,
                                    &g.group_id(),
                                    referenced,
                                )?
                            {
                                return Ok(None);
                            }
                            store.with_studio_source(
                                id,
                                g,
                                StudioTarget::Flipnote {
                                    channel: target.channel(),
                                    object,
                                },
                                d,
                                |s| Ok(s.op_count() > 0 || s.epoch() > 0),
                            )
                        })?
                        .unwrap_or(false);
                    if !exists {
                        choice = Some(ReplayChoice::Manual);
                    }
                }
            }
            if choice == Some(ReplayChoice::Ready) {
                let intent = evidence.own.get(&op_id).expect("assessed intent");
                let result = server.studio_transaction_with_publication(
                    store,
                    id,
                    StudioRequest::Apply {
                        target,
                        epoch_id: epoch,
                        nonce: intent.operation.nonce,
                        body: intent.operation.body.clone(),
                    },
                );
                self.settlement
                    .note(target, StudioSettlementState::RefreshRequired);
                let saved = result?;
                pass.order.pop_front();
                self.settlement
                    .note(target, StudioSettlementState::RefreshRequired);
                return Ok(Some((saved, Some(target))));
            }
            if matches!(choice, Some(ReplayChoice::Manual | ReplayChoice::After(_))) {
                pass.manual.insert(op_id);
            }
            pass.order.pop_front();
            return Ok(None);
        }
        // Newly current envelopes (for example a normal retry during the pass) must stay in
        // the receipt-owned ledger. Evicted evidence leaves its entry pending, not deleted.
        pass.manual.retain(|id| {
            evidence.own.get(id).is_some_and(|i| {
                !evidence.signed.contains_key(id)
                    && evidence
                        .history
                        .iter()
                        .any(|r| r.operations().get(id) == Some(i))
            })
        });
        if !pass.manual.is_empty() {
            // C-3 step 2: the budget comes from the shared, resumable inventory job rather than a
            // whole synchronous scan. Every replay turn re-derives the evidence and re-screens
            // `pass.manual` before reaching here, so a turn that completes the job acts on
            // preconditions checked in that same visit. Until then the pass is kept, with its
            // order empty and its manual set intact, and the next replay turn resumes it.
            //
            // Two ways out that do not depend on the job finishing, both into the synchronous
            // scan this site ran before step 2: the vault would not hold still for it
            // (`Unstable`, including any turn inside the runtime's backoff), or this pass has
            // waited `MANUAL_MOVE_PATIENCE_MS`. See that constant for why both are needed.
            let since = *pass.inventory_since.get_or_insert(now);
            let turn = if now.saturating_sub(since) < MANUAL_MOVE_PATIENCE_MS {
                let pool = self.catchup.preparation_pool();
                let inventory = &mut self.inventory;
                Some(server.sync.with_registry_context(|g, _, c, _| {
                    inventory.budget_turn(store, id, g, c, deadline_ms, &pool)
                })?)
            } else {
                // Out of patience: whatever the job still holds (a parked body and its permit, or
                // a result) is dropped now rather than left for the idle drop. A backoff stays.
                self.inventory.abandon_job();
                None
            };
            let mut budget = match turn {
                Some(super::inventory::InventoryTurn::Ready(budget)) => budget,
                Some(super::inventory::InventoryTurn::NotYet) => return Ok(None),
                // The runtime keeps its backoff for its other owners; only this move goes ahead.
                Some(super::inventory::InventoryTurn::Unstable) | None => {
                    Box::new(server.sync.with_registry_context(|g, _, _, _| {
                        synchronous_receive_budget(store, id, g)
                    })?)
                }
            };
            let pass = self.replay.active.as_mut().expect("selected pass");
            let result = server.sync.with_registry_context(|g, d, c, rng| {
                store.move_studio_intents_to_recovery(
                    id,
                    g,
                    target,
                    d,
                    epoch,
                    &pass.manual,
                    c,
                    rng,
                    &mut budget,
                )
            });
            self.settlement
                .note(target, StudioSettlementState::RefreshRequired);
            result?;
            self.settlement
                .note(target, StudioSettlementState::RecoveryAvailable);
        }
        self.replay.complete(target, epoch);
        Ok(None)
    }
}
