//! Automatic joining reuses the private head selection and recovery-first installer. Network
//! completion is not a disk commit: current mount/native custody is regained for every step.
use super::*;
use crate::store::StudioAdoptionOutcome;
use crate::studio_exchange::discovery::ServerCheckpointDiscovery;

pub(super) struct DiscoveryPlan {
    pub mount: Arc<()>,
    pub server: u64,
    pub peer: PeerId,
    pub target: CheckpointTarget,
    /// W-1: this device's own frozen pair for `target`, attached to its head query. Evidence for
    /// the owner to attest or ignore; it chooses nothing and asserts no authority.
    pub fault_report: Option<[catcoms_replication::Receipt; 2]>,
}
impl CatchupRuntime {
    /// Schedule discovery and attach the warm Studio source's frozen pair, if it has one, so a
    /// faulted peer both reports its evidence and receives the owner's repair in the answer.
    pub(super) fn schedule_reporting_discovery<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        node: &mut Server<T, R>,
        store: &ServerStore,
        server: u64,
        watch: &ServerStudioWatch,
        peer: PeerId,
    ) {
        self.schedule_discovery(store, server, watch, peer);
        let target = watch.target;
        let report = node
            .sync
            .with_registry_context(|g, d, _, _| store.warm_studio_fault_pair(server, g, target, d));
        if let Some(plan) = self.after_registry.as_mut() {
            plan.fault_report = report;
        }
    }
    pub(super) fn schedule_discovery(
        &mut self,
        store: &ServerStore,
        server: u64,
        watch: &ServerStudioWatch,
        peer: PeerId,
    ) {
        let target = watch.target;
        self.discovery_watch = Some(watch.inner.copy_binding());
        self.target = Some(target);
        self.checkpoint_peer = Some(peer);
        let logical = target
            .document(b"type-and-key-only")
            .expect("validated Studio target");
        let bucket =
            catcoms_replication::registry::PointerKey::new(logical.doc_type, logical.logical_key)
                .expect("Studio key")
                .bucket();
        self.after_registry = Some(DiscoveryPlan {
            mount: store.registry_mount(),
            server,
            target: CheckpointTarget::Studio(target),
            peer,
            fault_report: None,
        });
        self.discovery_plan = Some(DiscoveryPlan {
            mount: store.registry_mount(),
            server,
            target: CheckpointTarget::Registry(bucket),
            peer,
            fault_report: None,
        });
        self.checkpoint_retry = 0;
    }
    pub(in crate::studio::receiver) fn take_binding(&mut self) -> Option<(StudioTarget, u128)> {
        self.binding.take()
    }
    pub(super) fn retry_discovery(&mut self, now: u64) {
        self.checkpoint = None;
        self.repair_failure_target = None;
        // Registry failure/expiry yields the paired Studio target; Studio failure yields the
        // next watched target. Retrying one unresponsive key must not reset the rotation.
        self.discovery_plan = self.after_registry.take();
        self.checkpoint_sealed = false;
        self.discovery_needed = None;
        self.checkpoint_retry = now.saturating_add(5_000);
        self.next_at = now.saturating_add(5_000);
    }
    /// A verified receipt/installation supersedes any tail selected before discovery. Old
    /// concrete-epoch pages must never enter the newly Open checkpoint even when their transport
    /// watch and peer credentials are otherwise still current.
    pub(super) fn reset_registry_tail<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        now: u64,
    ) {
        self.registry_pass = None;
        if let Some(old) = self.registry_watch.take() {
            let _ = server.unwatch_registry_epoch(&old);
        }
        self.registry_watch_id = None;
        self.registry_target = None;
        self.registry_next_at = now.saturating_add(5_000);
    }
    /// A verified receipt is saved even before its seed is available. A fetched seed has
    /// priority over more source service just like a held operation page, so it cannot starve.
    pub(super) fn advance_checkpoint<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> Result<Option<StudioTarget>, AppError> {
        let now = server.runtime_clock().monotonic_ms();
        if self
            .discovery_watch
            .as_ref()
            .is_some_and(|w| !server.sync.studio_watch_is_current(w))
            || self.target.is_some_and(|t| {
                !server
                    .channels()
                    .iter()
                    .any(|c| c.id == u128::from_be_bytes(t.channel()))
            })
        {
            // User navigation/channel removal supersedes background relevance, not storage.
            // Never re-add an evicted watch or turn normal lifecycle churn into global pause.
            self.head_result = None;
            self.checkpoint = None;
            self.discovery_plan = None;
            self.after_registry = None;
            self.discovery_needed = None;
            self.discovery_watch = None;
            self.next_at = now.saturating_add(5_000);
            return Ok(None);
        }
        if let Some(completed) = self.head_result.take() {
            let target = completed.target();
            let peer = completed.peer;
            let registry = matches!(target, CheckpointTarget::Registry(_));
            let result = server.complete_checkpoint_discovery(store, id, *completed);
            match result {
                Ok(Some(ServerCheckpointDiscovery::Selected(pass))) => {
                    // Flow D before the seed. A repair this answer carries is offered to the
                    // repair job; when the job takes it, this pass is dropped rather than raced:
                    // the job's S3 applies the repair to a rebuilt source and, if a replacement is
                    // then owed, fetches its selected seed through a repaired pass made under this
                    // device's authoring tenure, never under this proof's own claim (6.3).
                    let mut keep = true;
                    if let Some(repair) = pass.inner.fault_repair().cloned() {
                        let offered = pass.inner.selected_receipt().clone();
                        match target {
                            CheckpointTarget::Studio(studio) => {
                                if self.offer_repair(
                                    server,
                                    store,
                                    id,
                                    studio,
                                    &repair,
                                    Some(&offered),
                                    true,
                                ) {
                                    keep = false;
                                    self.retry_discovery(now);
                                }
                            }
                            CheckpointTarget::Registry(bucket) => {
                                // The same rule for a bucket. When the job does not take it, the
                                // pass reaches the router, which defers it for any owed or held
                                // repair the prepared provider shows, or while it is unknown.
                                let failure_target = self.target;
                                if self.offer_registry_repair(
                                    server,
                                    store,
                                    id,
                                    bucket,
                                    failure_target,
                                    &repair,
                                    Some(&offered),
                                    true,
                                ) {
                                    keep = false;
                                    self.retry_discovery(now);
                                }
                            }
                        }
                    }
                    if keep {
                        self.pass = None;
                        self.checkpoint = Some(pass);
                        self.checkpoint_sealed = false;
                        self.repair_failure_target = None;
                    }
                }
                Ok(Some(ServerCheckpointDiscovery::Hint(answer))) if registry => {
                    // A faulted bucket's only way out is the repair the owner's answer carries. A
                    // hint carries no pass to drop; the job, or the owed seed fetch, does the rest.
                    if let (CheckpointTarget::Registry(bucket), Some(repair)) =
                        (target, answer.repair.as_ref())
                    {
                        let failure_target = self.target;
                        self.offer_registry_repair(
                            server,
                            store,
                            id,
                            bucket,
                            failure_target,
                            repair,
                            answer.receipt.as_ref(),
                            false,
                        );
                    }
                    if self.checkpoint.is_none() {
                        self.discovery_plan = self.after_registry.take();
                    }
                }
                Ok(Some(ServerCheckpointDiscovery::Hint(answer))) => {
                    #[cfg(test)]
                    if let Some(observer) = &self.hint_observer {
                        // Emitted only after the existing watch/mount/current-member/request
                        // authentication checks and actual classification as a Studio Hint.
                        observer.send_replace(Some(
                            crate::studio_exchange::discovery::StudioHintObservation {
                                target,
                                peer,
                                provider: server
                                    .sync
                                    .registry_page_peer_device(peer)
                                    .expect("completed discovery authenticated this member"),
                                receipt: answer.receipt.clone(),
                                proof_absent: answer.proof.is_none(),
                            },
                        ));
                    }
                    // An unproven answer may still deliver a repair; verification is the app's.
                    if let (CheckpointTarget::Studio(studio), Some(repair)) =
                        (target, answer.repair.as_ref())
                    {
                        // A hint carries no pass to drop; the job, or the owed seed fetch, does
                        // everything else.
                        self.offer_repair(
                            server,
                            store,
                            id,
                            studio,
                            repair,
                            answer.receipt.as_ref(),
                            false,
                        );
                    }
                    if let (CheckpointTarget::Studio(target), Some(inner)) =
                        (target, &self.discovery_watch)
                    {
                        let watch = ServerStudioWatch {
                            inner: inner.copy_binding(),
                            mount: store.registry_mount(),
                            server: id,
                            target,
                        };
                        self.preview.queue(&watch, peer, now);
                    }
                    self.next_at = now.saturating_add(5_000);
                }
                _ if registry => self.discovery_plan = self.after_registry.take(),
                _ => self.retry_discovery(now),
            }
        }
        let Some(pass) = self.checkpoint.as_ref() else {
            return Ok(None);
        };
        if !server.sync.registry_seed_fetch_is_current(&pass.inner) {
            self.retry_discovery(now);
            return Ok(None);
        }
        if self.checkpoint_sealed && !pass.inner.is_fetched() {
            return Ok(None);
        }
        let CheckpointTarget::Studio(target) = pass.inner.target() else {
            let CheckpointTarget::Registry(bucket) = pass.inner.target() else {
                unreachable!()
            };
            if self.repair_claimed(CheckpointTarget::Registry(bucket)) {
                // Same rule for a bucket a repair job owns: drop before any preparation.
                self.checkpoint = None;
                self.retry_discovery(now);
                return Ok(None);
            }
            // The router classifies an owed or held repair from the prepared provider, never from
            // a restore under custody, so it needs an exact local classification before an
            // ordinary pass can install. Prepare even a small source; otherwise a fresh receiver
            // would repeatedly discard the pass as unknown without ever scheduling the detached
            // work that can resolve it. Checked absence is retained separately from cold state.
            if !self.prepare_registry_inventory(server, store, id, bucket)? {
                return Ok(None);
            }
            // A repaired bucket pass reports to the target it was minted for; an owner's may have
            // been minted with no discovery scheduled at all.
            let failure_target = self.repair_failure_target.or(self.target);
            if let Some(updated) =
                self.route_checkpoint_install(server, store, id, failure_target)?
            {
                return Ok(updated);
            }
            let mut budget = Self::budget(server, store, id)?;
            let outcome = server.install_registry_seed_for_studio(
                store,
                id,
                self.checkpoint.as_ref().expect("pass"),
                self.registry_provider.as_mut(),
                &mut budget,
            )?;
            self.reset_registry_tail(server, now);
            if outcome == StudioAdoptionOutcome::AwaitingSeed {
                self.checkpoint_sealed = true;
                self.checkpoint_retry = now;
            } else {
                self.checkpoint = None;
                self.discovery_plan = self.after_registry.take();
            }
            return Ok(None);
        };
        if self.repair_claimed(CheckpointTarget::Studio(target)) {
            // Before `prepare`, which refuses a claimed target and would leave this pass parked
            // (blocking `replay_ready`) for the job's whole length. The job is what unblocks it.
            self.checkpoint = None;
            self.retry_discovery(now);
            return Ok(None);
        }
        if !self.prepare(server, store, id, target)? {
            return Ok(None);
        }
        if let Some(updated) = self.route_checkpoint_install(server, store, id, Some(target))? {
            return Ok(updated);
        }
        let mut budget = Self::budget(server, store, id)?;
        let result = server.install_studio_seed_step(
            store,
            id,
            self.checkpoint.as_ref().expect("pass"),
            &mut budget,
        );
        self.settlement
            .note(target, StudioSettlementState::RefreshRequired);
        let (outcome, state) = result?;
        self.settlement.note(target, state.phase().into());
        if outcome == StudioAdoptionOutcome::RecoveryPending {
            self.settlement
                .note(target, StudioSettlementState::RecoveryEvictionPending);
        }
        let doc_id = state.doc_id();
        server
            .sync
            .with_registry_context(|g, d, _, _| store.retain_received_studio_source(g, d, state));
        self.binding = Some((target, doc_id));
        match outcome {
            StudioAdoptionOutcome::AwaitingSeed => {
                self.checkpoint_sealed = true;
                self.checkpoint_retry = now;
            }
            StudioAdoptionOutcome::RecoveryPending => {
                self.retry_discovery(now);
                self.next_at = now.saturating_add(60_000);
            }
            _ => {
                self.checkpoint = None;
                self.discovery_needed = None;
                // Installation intentionally replaces the old concrete-epoch watch. Its
                // completed discovery binding must not cancel the new epoch's first tail
                // pass or give unrelated maintenance a turn before that pass can start.
                self.discovery_watch = None;
                self.next_at = now;
            }
        }
        Ok(Some(target))
    }
}
