//! Typed app custody around the existing shared discovery engine. Detached work owns no vault
//! or Server. Fresh response provenance and exact native mount/server are rechecked on return.
use super::*;
use crate::registry_head::ServerOwnerSnapshot;
use crate::registry_seed::{ServerRegistrySeedDiscovery, ServerRegistrySeedFetch};
use crate::store::StudioAdoptionOutcome;
use catcoms_rt::PeerId;
use catcoms_sync::checkpoint_exchange::CheckpointTarget;
use catcoms_sync::receipt_head::{ReceiptHeadServed, RegistryHeadWatch};
use catcoms_sync::registry_seed::{
    CompletedCheckpointDiscovery, CompletedCheckpointSeed, PendingCheckpointDiscovery,
    PendingCheckpointSeed, RegistrySeedDiscovery, RegistrySeedWatch,
};

/// Both document families share these opaque fetch handles and their four retained slots.
pub type ServerCheckpointFetch = ServerRegistrySeedFetch;
pub type ServerCheckpointDiscovery = ServerRegistrySeedDiscovery;

pub struct CheckpointDiscoveryAttempt<T: MeshTransport> {
    inner: PendingCheckpointDiscovery<T>,
    mount: Arc<()>,
    server: u64,
    target: CheckpointTarget,
}
pub struct CheckpointDiscoveryCompletion {
    inner: CompletedCheckpointDiscovery,
    mount: Arc<()>,
    server: u64,
    target: CheckpointTarget,
}
impl<T: MeshTransport> CheckpointDiscoveryAttempt<T> {
    pub async fn fetch(self) -> CheckpointDiscoveryCompletion {
        CheckpointDiscoveryCompletion {
            inner: self.inner.fetch().await,
            mount: self.mount,
            server: self.server,
            target: self.target,
        }
    }
}
impl<T: MeshTransport> std::fmt::Debug for CheckpointDiscoveryAttempt<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CheckpointDiscoveryAttempt { .. }")
    }
}
impl std::fmt::Debug for CheckpointDiscoveryCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CheckpointDiscoveryCompletion { .. }")
    }
}

/// Logical source service, independent of the user's receive watch and its concrete epoch.
/// Explicit revocation is required on eviction/lock; dropping alone is not unregistration.
pub struct ServerStudioCheckpointWatch {
    head: RegistryHeadWatch,
    seed: RegistrySeedWatch,
    mount: Arc<()>,
    server: u64,
    target: StudioTarget,
}
impl std::fmt::Debug for ServerStudioCheckpointWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerStudioCheckpointWatch { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    fn check_checkpoint_target(&self, target: CheckpointTarget) -> Result<(), AppError> {
        if let CheckpointTarget::Studio(target) = target {
            self.check_studio_channel(target)?;
        }
        Ok(())
    }
    /// Captures a physical mount without keeping any store borrow across network suspension.
    pub fn prepare_checkpoint_discovery(
        &mut self,
        store: &ServerStore,
        server: u64,
        peer: PeerId,
        target: CheckpointTarget,
    ) -> Result<CheckpointDiscoveryAttempt<T>, AppError> {
        self.check_checkpoint_target(target)?;
        Ok(CheckpointDiscoveryAttempt {
            inner: self.sync.prepare_checkpoint_discovery(peer, target)?,
            mount: store.registry_mount(),
            server,
            target,
        })
    }
    pub fn complete_checkpoint_discovery(
        &mut self,
        store: &ServerStore,
        server: u64,
        completed: CheckpointDiscoveryCompletion,
    ) -> Result<Option<ServerCheckpointDiscovery>, AppError> {
        if completed.server != server || !Arc::ptr_eq(&completed.mount, &store.registry_mount()) {
            return Err(invalid(
                "checkpoint discovery belongs to a replaced mount/server",
            ));
        }
        self.check_checkpoint_target(completed.target)?;
        Ok(self
            .sync
            .complete_checkpoint_discovery(completed.inner)?
            .map(|value| match value {
                RegistrySeedDiscovery::Hint(answer) => ServerCheckpointDiscovery::Hint(answer),
                RegistrySeedDiscovery::Selected(inner) => {
                    ServerCheckpointDiscovery::Selected(ServerCheckpointFetch {
                        inner,
                        mount: completed.mount,
                        server,
                    })
                }
            }))
    }
    /// Returning a job spends one retry. It can be awaited without the Server or vault lease.
    pub fn prepare_checkpoint_seed_fetch(
        &mut self,
        pass: &mut ServerCheckpointFetch,
        peer: PeerId,
    ) -> Result<Option<PendingCheckpointSeed<T>>, AppError> {
        self.check_checkpoint_target(pass.inner.target())?;
        Ok(self.sync.prepare_checkpoint_seed(&mut pass.inner, peer)?)
    }
    pub fn complete_checkpoint_seed_fetch(
        &mut self,
        pass: &mut ServerCheckpointFetch,
        completed: CompletedCheckpointSeed,
    ) -> Result<bool, AppError> {
        self.check_checkpoint_target(pass.inner.target())?;
        Ok(self
            .sync
            .complete_checkpoint_seed(&mut pass.inner, completed)?)
    }
    /// Save Closing/Fault even before fetching. Replacement requires typed recovery durable
    /// first; no intent retires on adoption and an exact retry preserves later successor edits.
    /// Runtime callers must retain the returned owned source and replace their concrete watch.
    pub fn install_studio_seed_step(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        pass: &ServerCheckpointFetch,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioAdoptionOutcome, EpochStudioState), AppError> {
        if pass.server != server || !Arc::ptr_eq(&pass.mount, &store.registry_mount()) {
            return Err(invalid(
                "Studio checkpoint selection belongs to a replaced mount/server",
            ));
        }
        let CheckpointTarget::Studio(target) = pass.inner.target() else {
            return Err(invalid("Studio installer requires a Studio selection"));
        };
        self.check_studio_channel(target)?;
        let clock = self.runtime_clock();
        self.sync
            .with_checkpoint_seed_selection(&pass.inner, |group, device, rng, selected| {
                store.adopt_studio_checkpoint(
                    server,
                    group,
                    target,
                    device,
                    selected.receipt,
                    selected.checkpoint.map(|s| s.bytes()),
                    selected.tenure,
                    clock.as_ref(),
                    rng,
                    budget,
                )
            })?
    }
    pub fn watch_studio_checkpoint(
        &mut self,
        store: &ServerStore,
        server: u64,
        target: StudioTarget,
    ) -> Result<ServerStudioCheckpointWatch, AppError> {
        self.check_studio_channel(target)?;
        let scope = CheckpointTarget::Studio(target);
        let head = self.sync.watch_checkpoint_head(scope)?;
        let seed = match self.sync.watch_checkpoint_seed(scope) {
            Ok(seed) => seed,
            Err(error) => {
                self.sync.unwatch_registry_head(&head)?;
                return Err(error.into());
            }
        };
        Ok(ServerStudioCheckpointWatch {
            head,
            seed,
            mount: store.registry_mount(),
            server,
            target,
        })
    }
    pub fn unwatch_studio_checkpoint(
        &mut self,
        watch: &ServerStudioCheckpointWatch,
    ) -> Result<(), AppError> {
        // Revoke both even if one was already replaced independently.
        let head = self.sync.unwatch_registry_head(&watch.head);
        let seed = self.sync.unwatch_registry_seed(&watch.seed);
        head?;
        seed?;
        Ok(())
    }
    pub fn serve_studio_head_step(
        &mut self,
        store: &mut ServerStore,
        watch: &ServerStudioCheckpointWatch,
        snapshot: Option<&ServerOwnerSnapshot>,
        budget: &mut EpochStudioBudget,
    ) -> Result<Option<()>, AppError> {
        self.serve_studio_head_with_completion(
            store,
            watch,
            snapshot,
            budget,
            |store, server, receipt, rng, budget| {
                store.complete_studio_head(server, receipt, rng, budget)
            },
        )
    }
    pub(crate) fn serve_studio_head_with_completion(
        &mut self,
        store: &mut ServerStore,
        watch: &ServerStudioCheckpointWatch,
        snapshot: Option<&ServerOwnerSnapshot>,
        budget: &mut EpochStudioBudget,
        complete: impl FnOnce(
            &mut ServerStore,
            u64,
            &catcoms_replication::Receipt,
            &mut R,
            &mut EpochStudioBudget,
        ) -> Result<(), AppError>,
    ) -> Result<Option<()>, AppError> {
        if !Arc::ptr_eq(&watch.mount, &store.registry_mount()) {
            return Err(invalid(
                "Studio checkpoint watch belongs to a replaced mount",
            ));
        }
        self.check_studio_channel(watch.target)?;
        let snapshot = snapshot
            .filter(|s| s.server == watch.server && Arc::ptr_eq(&s.mount, &watch.mount))
            .map(|s| &s.inner);
        let served = self
            .sync
            .serve_receipt_head_with_handoff(
                &watch.head,
                snapshot,
                |group, device, rng, request| {
                    store.prepare_studio_head(
                        watch.server,
                        group,
                        watch.target,
                        device,
                        request.tenure,
                        rng,
                        budget,
                    )
                },
            )?
            .transpose()?;
        let Some(served) = served else {
            return Ok(None);
        };
        if let ReceiptHeadServed::Owner(handoff) = served {
            self.sync
                .with_receipt_head_handoff(handoff, |receipt, rng| {
                    complete(store, watch.server, receipt, rng, budget)
                })??;
        }
        Ok(Some(()))
    }
    pub fn serve_studio_seed_step(
        &mut self,
        store: &mut ServerStore,
        watch: &ServerStudioCheckpointWatch,
        budget: &mut EpochStudioBudget,
    ) -> Result<Option<()>, AppError> {
        if !Arc::ptr_eq(&watch.mount, &store.registry_mount()) {
            return Err(invalid(
                "Studio checkpoint watch belongs to a replaced mount",
            ));
        }
        self.check_studio_channel(watch.target)?;
        self.sync
            .serve_registry_seed(&watch.seed, |group, device, id, hash| {
                store.read_studio_seed(watch.server, group, watch.target, device, id, hash, budget)
            })?
            .transpose()
    }
}
