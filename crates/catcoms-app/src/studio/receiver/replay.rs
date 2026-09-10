//! One bounded pass through saved OWN ids per watched Open binding. Ordinary Save owns
//! validation, durability and sending; this scheduler neither signs nor stores another log.
use super::*;
use crate::studio::replay::{choose, deconflict, ordered, ReplayChoice};
use std::collections::BTreeSet;

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
    pub(super) fn pending(&self, watches: &VecDeque<(ServerStudioWatch, u128)>) -> bool {
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
    /// Isolate one production replay turn from network scheduling in deterministic regressions.
    #[cfg(test)]
    pub(crate) fn replay_step_for_test<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> Result<Option<(StudioSavedTransaction, Option<StudioTarget>)>, AppError> {
        self.replay_step(server, store, id)
    }
    pub(super) fn replay_step<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
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
            let mut scan = store.scan_studio_receive_inventory()?;
            while !scan.step()?.complete {}
            let inventory = scan.finish()?;
            let result = server.sync.with_registry_context(|g, d, c, rng| {
                let mut budget = store.studio_storage_budget(id, g, &inventory)?;
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
