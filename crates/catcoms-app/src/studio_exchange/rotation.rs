//! Owner rotation consumes the same private MLS-snapshot permit as Registry. This adapter
//! performs no network wait while holding the sole mutable server and mounted store.
use super::*;
use crate::registry_head::ServerOwnerSnapshot;
use crate::store::StudioRotationOutcome;

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    pub(crate) fn maintain_studio_registry_owner(
        &mut self,
        store: &mut ServerStore,
        id: u64,
        bucket: u8,
        snapshot: &ServerOwnerSnapshot,
        budget: &mut EpochStudioBudget,
    ) -> Result<
        Option<(
            crate::store::RegistryOwnerRotationOutcome,
            crate::store::EpochRegistryState,
        )>,
        AppError,
    > {
        if snapshot.server != id || !Arc::ptr_eq(&snapshot.mount, &store.registry_mount()) {
            return Err(invalid("owner snapshot belongs to another mount/server"));
        }
        let clock = self.runtime_clock();
        self.sync
            .with_durable_owner_snapshot(&snapshot.inner, |g, d, rng, tenure| {
                store.maintain_studio_registry_owner(
                    id,
                    g,
                    bucket,
                    d,
                    tenure,
                    clock.as_ref(),
                    rng,
                    budget,
                )
            })?
    }
    /// Finish local publication after retaining the installed source. This is also valid with
    /// no remote peers: the saved head/seed are discoverable, not claimed remotely delivered.
    /// Failed completion leaves the exact decision pending; never regenerate it on retry.
    pub fn complete_studio_owner_availability(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        snapshot: &ServerOwnerSnapshot,
        budget: &mut EpochStudioBudget,
    ) -> Result<(), AppError> {
        self.check_studio_channel(target)?;
        if snapshot.server != server || !Arc::ptr_eq(&snapshot.mount, &store.registry_mount()) {
            return Err(invalid("owner snapshot belongs to another mount/server"));
        }
        self.sync
            .with_durable_owner_snapshot(&snapshot.inner, |g, d, rng, tenure| {
                store.complete_studio_installed_head(server, g, target, d, tenure, rng, budget)
            })?
    }
    /// Drive one durable owner checkpoint. The caller must own native lifecycle custody and
    /// an accounted source preparation. Local installation is not publication or delivery.
    pub fn rotate_studio_owner_step(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        snapshot: &ServerOwnerSnapshot,
        budget: &mut EpochStudioBudget,
    ) -> Result<(StudioRotationOutcome, EpochStudioState), AppError> {
        self.check_studio_channel(target)?;
        if snapshot.server != server || !Arc::ptr_eq(&snapshot.mount, &store.registry_mount()) {
            return Err(invalid("owner snapshot belongs to another mount/server"));
        }
        let clock = self.runtime_clock();
        self.sync
            .with_durable_owner_snapshot(&snapshot.inner, |group, device, rng, tenure| {
                store.rotate_studio_owner(
                    server,
                    group,
                    target,
                    device,
                    tenure,
                    clock.as_ref(),
                    rng,
                    budget,
                )
            })?
    }
}
