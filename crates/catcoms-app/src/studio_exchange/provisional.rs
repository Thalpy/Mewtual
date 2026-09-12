//! App mount/server custody for provisional head discovery. Scheduling, seed parsing and
//! native preview delivery are separate; these candidates are never canonical Studio views.
use super::*;
use catcoms_rt::PeerId;
use catcoms_sync::registry_seed::{
    CompletedProvisionalStudioDiscovery, PendingProvisionalStudioDiscovery, ProvisionalStudioHint,
    ProvisionalStudioHintUse,
};

pub struct ProvisionalStudioDiscoveryAttempt<T: MeshTransport> {
    inner: PendingProvisionalStudioDiscovery<T>,
    mount: Arc<()>,
    server: u64,
    target: StudioTarget,
}
pub struct ProvisionalStudioDiscoveryCompletion {
    inner: CompletedProvisionalStudioDiscovery,
    mount: Arc<()>,
    server: u64,
    target: StudioTarget,
}
pub struct ServerProvisionalStudioHint {
    inner: ProvisionalStudioHint,
    mount: Arc<()>,
    server: u64,
    target: StudioTarget,
}
impl<T: MeshTransport> ProvisionalStudioDiscoveryAttempt<T> {
    /// No Server or store borrow survives suspension; transport retains the charged seed slot.
    pub async fn fetch(self) -> ProvisionalStudioDiscoveryCompletion {
        ProvisionalStudioDiscoveryCompletion {
            inner: self.inner.fetch().await,
            mount: self.mount,
            server: self.server,
            target: self.target,
        }
    }
}
impl<T: MeshTransport> std::fmt::Debug for ProvisionalStudioDiscoveryAttempt<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProvisionalStudioDiscoveryAttempt { .. }")
    }
}
impl std::fmt::Debug for ProvisionalStudioDiscoveryCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProvisionalStudioDiscoveryCompletion { .. }")
    }
}
impl std::fmt::Debug for ServerProvisionalStudioHint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerProvisionalStudioHint { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    pub fn prepare_provisional_studio_discovery(
        &mut self,
        store: &ServerStore,
        server: u64,
        peer: PeerId,
        watch: &ServerStudioWatch,
    ) -> Result<ProvisionalStudioDiscoveryAttempt<T>, AppError> {
        if watch.server != server || !Arc::ptr_eq(&watch.mount, &store.registry_mount()) {
            return Err(invalid(
                "provisional discovery belongs to a replaced mount/server",
            ));
        }
        self.check_studio_channel(watch.target)?;
        Ok(ProvisionalStudioDiscoveryAttempt {
            inner: self
                .sync
                .prepare_provisional_studio_discovery(peer, &watch.inner)?,
            mount: watch.mount.clone(),
            server,
            target: watch.target,
        })
    }
    pub fn complete_provisional_studio_discovery(
        &mut self,
        store: &ServerStore,
        server: u64,
        completed: ProvisionalStudioDiscoveryCompletion,
    ) -> Result<Option<ServerProvisionalStudioHint>, AppError> {
        if completed.server != server || !Arc::ptr_eq(&completed.mount, &store.registry_mount()) {
            return Err(invalid(
                "provisional discovery belongs to a replaced mount/server",
            ));
        }
        self.check_studio_channel(completed.target)?;
        Ok(self
            .sync
            .complete_provisional_studio_discovery(completed.inner)?
            .map(|inner| ServerProvisionalStudioHint {
                inner,
                mount: completed.mount,
                server,
                target: completed.target,
            }))
    }
    /// Candidate metadata remains unconfirmed, including the receipt's claimed signer/tenure.
    /// This does not load a source, mutate a journal or grant ordinary Read/Apply authority.
    pub fn with_provisional_studio_hint<O>(
        &self,
        store: &ServerStore,
        server: u64,
        hint: &ServerProvisionalStudioHint,
        inspect: impl FnOnce(ProvisionalStudioHintUse<'_>) -> O,
    ) -> Result<O, AppError> {
        if hint.server != server || !Arc::ptr_eq(&hint.mount, &store.registry_mount()) {
            return Err(invalid(
                "provisional hint belongs to a replaced mount/server",
            ));
        }
        self.check_studio_channel(hint.target)?;
        Ok(self
            .sync
            .with_provisional_studio_hint(&hint.inner, inspect)?)
    }
}
