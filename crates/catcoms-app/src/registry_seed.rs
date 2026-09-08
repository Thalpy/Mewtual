//! Cooperative seed transport. Fetching does not create or replace a local epoch; the future
//! newcomer installer must preserve existing provisional content under its recovery-first gate.
use crate::store::epoch_budget::EpochStorageBudget;
use crate::{AppError, Server, ServerStore};
use catcoms_rt::{CryptoRngCore, MeshTransport, PeerId};
use catcoms_sync::{
    receipt_head::ReceiptHeadAnswer,
    registry_seed::{RegistrySeedDiscovery, RegistrySeedFetch, RegistrySeedWatch},
};
use std::sync::Arc;

pub struct ServerRegistrySeedWatch {
    inner: RegistrySeedWatch,
    mount: Arc<()>,
    server: u64,
    bucket: u8,
}
impl std::fmt::Debug for ServerRegistrySeedWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerRegistrySeedWatch { .. }")
    }
}
/// No Clone and no publicly replaceable receipt, bytes or scope. Physical vault replacement
/// invalidates use, while the underlying sync slot stays charged until the handle is dropped.
pub struct ServerRegistrySeedFetch {
    pub(crate) inner: RegistrySeedFetch,
    pub(crate) mount: Arc<()>,
    pub(crate) server: u64,
}
impl std::fmt::Debug for ServerRegistrySeedFetch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerRegistrySeedFetch")
            .field("fetch", &self.inner)
            .finish_non_exhaustive()
    }
}
pub enum ServerRegistrySeedDiscovery {
    Hint(ReceiptHeadAnswer),
    Selected(ServerRegistrySeedFetch),
}
impl std::fmt::Debug for ServerRegistrySeedDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Hint(_) => "ServerRegistrySeedDiscovery::Hint",
            Self::Selected(_) => "ServerRegistrySeedDiscovery::Selected",
        })
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    pub fn watch_registry_seed(
        &mut self,
        store: &ServerStore,
        server: u64,
        bucket: u8,
    ) -> ServerRegistrySeedWatch {
        ServerRegistrySeedWatch {
            inner: self.sync.watch_registry_seed(bucket),
            mount: store.registry_mount(),
            server,
            bucket,
        }
    }
    pub fn unwatch_registry_seed(
        &mut self,
        watch: &ServerRegistrySeedWatch,
    ) -> Result<(), AppError> {
        Ok(self.sync.unwatch_registry_seed(&watch.inner)?)
    }
    /// Source and membership checks happen before I/O and again before responder handoff.
    /// Serving reads a saved, checked seed; it never publishes or installs an owner decision.
    pub fn serve_registry_seed_step(
        &mut self,
        store: &mut ServerStore,
        watch: &ServerRegistrySeedWatch,
        budget: &mut EpochStorageBudget,
    ) -> Result<Option<()>, AppError> {
        if !Arc::ptr_eq(&watch.mount, &store.registry_mount())
            || !self.sync.registry_seed_watch_is_current(&watch.inner)
        {
            return Err(AppError::Invalid(
                "registry seed watch belongs to a replaced mount or runtime".into(),
            ));
        }
        self.sync
            .serve_registry_seed(&watch.inner, |group, device, id, hash| {
                store.read_registry_seed(
                    watch.server,
                    group,
                    watch.bucket,
                    device,
                    id,
                    hash,
                    budget,
                )
            })?
            .transpose()
    }
    /// This first performs fresh keyed head discovery; raw answer fields cannot mint a pass.
    /// The store borrow captures the mount only. No vault file is read or changed by discovery.
    pub async fn discover_registry_seed(
        &mut self,
        store: &ServerStore,
        server: u64,
        peer: PeerId,
        bucket: u8,
    ) -> Result<Option<ServerRegistrySeedDiscovery>, AppError> {
        let mount = store.registry_mount();
        Ok(self
            .sync
            .discover_registry_seed(peer, bucket)
            .await?
            .map(|value| match value {
                RegistrySeedDiscovery::Hint(answer) => ServerRegistrySeedDiscovery::Hint(answer),
                RegistrySeedDiscovery::Selected(inner) => {
                    ServerRegistrySeedDiscovery::Selected(ServerRegistrySeedFetch {
                        inner,
                        mount,
                        server,
                    })
                }
            }))
    }
    /// Fetch at most one attempt, retaining verified bytes only. No disk borrow across network
    /// is needed; mount authority is rechecked synchronously before any future installation.
    pub async fn fetch_registry_seed_step(
        &mut self,
        pass: &mut ServerRegistrySeedFetch,
        peer: PeerId,
    ) -> Result<bool, AppError> {
        Ok(self.sync.fetch_registry_seed(&mut pass.inner, peer).await?)
    }
    /// Current fetch evidence, not settlement status. False also covers runtime/mount replacement.
    pub fn registry_seed_ready(
        &self,
        store: &ServerStore,
        server: u64,
        pass: &ServerRegistrySeedFetch,
    ) -> bool {
        pass.server == server
            && Arc::ptr_eq(&pass.mount, &store.registry_mount())
            && pass.inner.is_fetched()
            && self.sync.registry_seed_fetch_is_current(&pass.inner)
    }
}
