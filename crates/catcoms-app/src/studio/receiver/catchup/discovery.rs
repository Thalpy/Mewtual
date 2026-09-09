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
}
impl CatchupRuntime {
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
        });
        self.discovery_plan = Some(DiscoveryPlan {
            mount: store.registry_mount(),
            server,
            target: CheckpointTarget::Registry(bucket),
            peer,
        });
        self.checkpoint_retry = 0;
        self.registry_attempts = 1;
    }
    pub(in crate::studio::receiver) fn take_binding(&mut self) -> Option<(StudioTarget, u128)> {
        self.binding.take()
    }
    pub(super) fn retry_discovery(&mut self, now: u64) {
        self.checkpoint = None;
        self.discovery_plan = None;
        self.after_registry = None;
        self.checkpoint_sealed = false;
        self.discovery_needed = self.target;
        self.next_at = now.saturating_add(5_000);
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
            let registry = matches!(target, CheckpointTarget::Registry(_));
            let result = server.complete_checkpoint_discovery(store, id, *completed);
            match result {
                Ok(Some(ServerCheckpointDiscovery::Selected(pass))) => {
                    self.pass = None;
                    self.checkpoint = Some(pass);
                    self.checkpoint_sealed = false;
                }
                Ok(Some(ServerCheckpointDiscovery::Hint(_))) if registry => {
                    self.discovery_plan = self.after_registry.take()
                }
                Ok(Some(ServerCheckpointDiscovery::Hint(_))) => {
                    self.next_at = now.saturating_add(5_000);
                }
                _ if registry && self.registry_attempts < 3 => {
                    // Cold verification may outlive the original request. Keep its checked
                    // preparation useful via a NEW nonce/charge, never extend an expired proof.
                    // Optional Registry bootstrap must not indefinitely block the known key.
                    self.registry_attempts += 1;
                    if let Some(next) = &self.after_registry {
                        self.discovery_plan = Some(DiscoveryPlan {
                            mount: next.mount.clone(),
                            server: next.server,
                            peer: next.peer,
                            target,
                        });
                        self.checkpoint_retry = now.saturating_add(5_000);
                    }
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
            if !store.registry_receive_source_fits(id, &server.group_id(), bucket)? {
                if !self.prepare_registry_inventory(server, store, id, bucket)? {
                    return Ok(None);
                }
                // Registry bootstrap is optional for a key already learned from StudioIndex.
                // A healthy large saved bucket needs a prepared receiver (Gate 4); do not
                // confuse this local work rail with corruption and globally pause Studio.
                self.checkpoint = None;
                self.discovery_plan = self.after_registry.take();
                return Ok(None);
            }
            let mut budget = Self::budget(server, store, id)?;
            let outcome = server.install_registry_seed_for_studio(
                store,
                id,
                self.checkpoint.as_ref().expect("pass"),
                &mut budget,
            )?;
            if outcome == StudioAdoptionOutcome::AwaitingSeed {
                self.checkpoint_sealed = true;
                self.checkpoint_retry = now;
            } else {
                self.checkpoint = None;
                self.discovery_plan = self.after_registry.take();
            }
            return Ok(None);
        };
        if !self.prepare(server, store, id, target)? {
            return Ok(None);
        }
        let mut budget = Self::budget(server, store, id)?;
        let (outcome, state) = server.install_studio_seed_step(
            store,
            id,
            self.checkpoint.as_ref().expect("pass"),
            &mut budget,
        )?;
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
                self.next_at = now;
            }
        }
        Ok(Some(target))
    }
}
