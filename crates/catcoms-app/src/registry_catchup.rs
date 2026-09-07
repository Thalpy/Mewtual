//! Cooperative read-only serving of durable registry operation pages. This does not expose a
//! network request kind: future routing must authenticate the requester and bound aggregate work.

use std::sync::Arc;

use catcoms_replication::registry_epoch::catchup::{
    RegistryPageOutcome, RegistryPageProvider, RegistryPageRequest,
};
use catcoms_rt::{CryptoRngCore, MeshTransport};
use catcoms_sync::RegistrySyncInstance;

use crate::{AppError, Server, ServerStore};

/// One provider-local key tied to an exact Server, physical vault mount, captured local server
/// id and registry bucket. No operation bodies or requester-specific sessions are retained.
/// Drop on lifecycle replacement: this is not a native UI-unlock lease or a remote permission.
pub struct ServerRegistryPageProvider {
    inner: RegistryPageProvider,
    instance: RegistrySyncInstance,
    mount: Arc<()>,
    server: u64,
    bucket: u8,
}
impl std::fmt::Debug for ServerRegistryPageProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerRegistryPageProvider { .. }")
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Mint a constant-sized, restart-local provider key using the actual Server RNG and clock.
    /// Does not load/create a registry record, subscribe a topic or establish remote currency.
    pub fn begin_registry_page_provider(
        &mut self,
        store: &ServerStore,
        server: u64,
        bucket: u8,
    ) -> Result<ServerRegistryPageProvider, AppError> {
        let clock = self.runtime_clock();
        let inner = self.sync.with_registry_context(|group, device, _, rng| {
            if group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
            {
                return Err(AppError::Invalid(
                    "registry provider is not a current member".into(),
                ));
            }
            Ok(RegistryPageProvider::new(device.device_id(), clock, rng))
        })?;
        Ok(ServerRegistryPageProvider {
            inner,
            instance: self.sync.registry_instance(),
            mount: store.registry_mount(),
            server,
            bucket,
        })
    }

    /// Read one bounded page from the actual saved source. The trusted network adapter MUST
    /// derive `request.requester` from its authenticated request, never from a body field. Runtime/
    /// mount binding, current membership, field caps, continuation MAC and expiry reject before
    /// source I/O. Matching the current concrete document id requires loading the captured bucket.
    /// Source corruption is an error; absence is Restart, never proof that a document is empty.
    /// A full source rebuild is bounded by the epoch cap but synchronous; no live network
    /// handler may call this without an aggregate rate/concurrency rail. No writes or acks occur.
    pub fn serve_registry_page(
        &mut self,
        store: &ServerStore,
        provider: &mut ServerRegistryPageProvider,
        request: RegistryPageRequest<'_>,
    ) -> Result<RegistryPageOutcome, AppError> {
        if !self.sync.matches_registry_instance(&provider.instance)
            || !Arc::ptr_eq(&provider.mount, &store.registry_mount())
        {
            return Err(AppError::Invalid(
                "registry page provider belongs to a replaced server or mount".into(),
            ));
        }
        self.sync.with_registry_context(|group, device, _, rng| {
            if !provider
                .inner
                .preflight_request(group, device, provider.bucket, &request)
                .map_err(|e| AppError::Invalid(e.to_string()))?
            {
                return Ok(RegistryPageOutcome::Restart);
            }
            let Some(state) =
                store.load_registry_epoch(provider.server, group, provider.bucket, device)?
            else {
                return Ok(RegistryPageOutcome::Restart);
            };
            state.catchup_page(&mut provider.inner, group, device, request, rng)
        })
    }
}

#[cfg(test)]
mod tests;
