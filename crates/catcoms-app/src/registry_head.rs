//! Cooperative keyed registry discovery. Network ticks queue small authenticated requests;
//! source service is explicit and bounded. Seed fetch/install and actor scheduling are separate.
use crate::store::epoch_budget::EpochStorageBudget;
use crate::{AppError, Server, ServerStore};
use catcoms_rt::{CryptoRngCore, MeshTransport, PeerId};
use catcoms_sync::receipt_head::{
    DurableOwnerSnapshot, ReceiptHeadAnswer, ReceiptHeadServed, RegistryHeadWatch,
};
use std::sync::Arc;

/// Local snapshot durability, additionally bound to the exact vault mount and numeric server.
/// Reusable across logical buckets, never across membership, runtime or mount replacement.
#[derive(Clone)]
pub struct ServerOwnerSnapshot {
    pub(crate) inner: DurableOwnerSnapshot,
    pub(crate) mount: Arc<()>,
    pub(crate) server: u64,
}
impl std::fmt::Debug for ServerOwnerSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerOwnerSnapshot { .. }")
    }
}
pub struct ServerRegistryHeadWatch {
    inner: RegistryHeadWatch,
    mount: Arc<()>,
    server: u64,
    bucket: u8,
}
impl std::fmt::Debug for ServerRegistryHeadWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerRegistryHeadWatch { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Explicitly drive one owner rotation from the checked saved source, preserving the exact
    /// close/receipt across crashes. This does not publish: pending publication must be completed
    /// separately before a later decision can replace it. No native scheduler is started here.
    pub fn rotate_registry_owner_step(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        bucket: u8,
        snapshot: &ServerOwnerSnapshot,
        budget: &mut EpochStorageBudget,
        intents: &mut crate::store::EpochIntentBudget,
    ) -> Result<
        (
            crate::store::RegistryOwnerRotationOutcome,
            crate::store::EpochRegistryState,
        ),
        AppError,
    > {
        if snapshot.server != server || !Arc::ptr_eq(&snapshot.mount, &store.registry_mount()) {
            return Err(AppError::Invalid(
                "owner snapshot belongs to another mount/server".into(),
            ));
        }
        let clock = self.runtime_clock();
        self.sync
            .with_durable_owner_snapshot(&snapshot.inner, |group, device, rng, tenure| {
                store.rotate_registry_owner(
                    server,
                    group,
                    bucket,
                    device,
                    tenure,
                    clock.as_ref(),
                    rng,
                    budget,
                    intents,
                )
            })?
    }

    /// Explicit local save of whole-server MLS/tenure evidence. This is not a remote-request
    /// callback: legacy whole-server serialization is outside the small discovery work budget.
    /// Call from local persistence lifecycle and re-prepare after membership changes, not per query.
    pub fn prepare_owner_head_snapshot(
        &mut self,
        store: &ServerStore,
        server: u64,
    ) -> Result<ServerOwnerSnapshot, AppError> {
        let inner = self.sync.prepare_receipt_head_snapshot(|snapshot, rng| {
            store.save_server(server, snapshot, rng)
        })??;
        Ok(ServerOwnerSnapshot {
            inner,
            mount: store.registry_mount(),
            server,
        })
    }
    /// Register a logical bucket, including an absent epoch zero. No disk read/create or routing
    /// subscription is needed; the requester need not know our current concrete epoch id.
    pub fn watch_registry_head(
        &mut self,
        store: &ServerStore,
        server: u64,
        bucket: u8,
    ) -> ServerRegistryHeadWatch {
        ServerRegistryHeadWatch {
            inner: self.sync.watch_registry_head(bucket),
            mount: store.registry_mount(),
            server,
            bucket,
        }
    }
    pub fn unwatch_registry_head(
        &mut self,
        watch: &ServerRegistryHeadWatch,
    ) -> Result<(), AppError> {
        Ok(self.sync.unwatch_registry_head(&watch.inner)?)
    }
    /// Drain at most one request. A stale/missing snapshot permit downgrades authority to hints;
    /// corrupt/missing indexed state and faults return errors, never a false absent head. The
    /// owner journal and source flush before signing. A checked local reply-channel handoff of
    /// an owner proof records exact journal completion before success; hints never do. This is
    /// not peer delivery. A completion error may follow a handoff and cannot retract the reply.
    pub fn serve_registry_head_step(
        &mut self,
        store: &mut ServerStore,
        watch: &ServerRegistryHeadWatch,
        snapshot: Option<&ServerOwnerSnapshot>,
        budget: &mut EpochStorageBudget,
    ) -> Result<Option<()>, AppError> {
        self.serve_registry_head_with_completion(
            store,
            watch,
            snapshot,
            budget,
            |store, server, receipt, rng, budget| {
                store
                    .mark_epoch_owner_receipt_published(
                        server,
                        &receipt.document,
                        receipt.hash(),
                        rng,
                        budget,
                    )
                    .map(|_| ())
            },
        )
    }

    // One transaction with a narrow completion seam for deterministic crash-boundary tests.
    // The public command always supplies the accounted durable journal writer above.
    pub(crate) fn serve_registry_head_with_completion(
        &mut self,
        store: &mut ServerStore,
        watch: &ServerRegistryHeadWatch,
        snapshot: Option<&ServerOwnerSnapshot>,
        budget: &mut EpochStorageBudget,
        complete: impl FnOnce(
            &mut ServerStore,
            u64,
            &catcoms_replication::Receipt,
            &mut R,
            &mut EpochStorageBudget,
        ) -> Result<(), AppError>,
    ) -> Result<Option<()>, AppError> {
        if !Arc::ptr_eq(&watch.mount, &store.registry_mount())
            || !self.sync.registry_head_watch_is_current(&watch.inner)
        {
            return Err(AppError::Invalid(
                "registry head watch belongs to a replaced mount or runtime".into(),
            ));
        }
        let snapshot = snapshot
            .filter(|p| Arc::ptr_eq(&p.mount, &watch.mount) && p.server == watch.server)
            .map(|p| &p.inner);
        let served = self
            .sync
            .serve_receipt_head_with_handoff(
                &watch.inner,
                snapshot,
                |group, device, rng, request| {
                    store.prepare_registry_head(
                        watch.server,
                        group,
                        watch.bucket,
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
            // No await or borrowed authority escapes between serving and this completion. The
            // same exclusive store/mount/server and budget are retained throughout the call.
            self.sync
                .with_receipt_head_handoff(handoff, |receipt, rng| {
                    complete(store, watch.server, receipt, rng, budget)
                })??;
        }
        Ok(Some(()))
    }
    /// Return query-bound hints or a current-owner selection proof; no checkpoint is installed.
    pub async fn request_registry_head(
        &mut self,
        peer: PeerId,
        bucket: u8,
    ) -> Result<Option<ReceiptHeadAnswer>, AppError> {
        Ok(self.sync.request_registry_head(peer, bucket).await?)
    }
}
