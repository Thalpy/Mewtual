//! Cooperative read-only serving of durable registry operation pages. The network adapter drains
//! a bounded authenticated request through the same mounted source; UI and scheduling stay separate.

use std::sync::{Arc, OnceLock};

use crate::registry_ingress::ServerRegistryWatch;
use catcoms_replication::registry_epoch::catchup::{
    RegistryPageOutcome, RegistryPageProvider, RegistryPageRequest, RegistryPageSource,
};
use catcoms_rt::{CryptoRngCore, MeshTransport, PeerId};
use catcoms_sync::registry_catchup::RegistryPageQuery;
use catcoms_sync::RegistrySyncInstance;

use crate::store::{RegistrySourceCapture, RegistrySourceStamp};
use crate::{AppError, Server, ServerStore};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

// Process-wide, not per Server/mount: dropping a cancelled caller or remounting must not refund
// a worker that still owns plaintext. A permit covers capture, queued/running work AND retention.
const MAX_PREPARED_REGISTRY_SOURCES: usize = 4;
pub(crate) fn preparation_pool() -> &'static Arc<Semaphore> {
    static POOL: OnceLock<Arc<Semaphore>> = OnceLock::new();
    POOL.get_or_init(|| Arc::new(Semaphore::new(MAX_PREPARED_REGISTRY_SOURCES)))
}

struct PreparedSource {
    stamp: RegistrySourceStamp,
    source: RegistryPageSource,
    _permit: OwnedSemaphorePermit,
}

/// Captured, bounded plaintext work. Owns no store, Server, device key or MLS state. Release
/// store/actor locks before awaiting `rebuild`, then reacquire them to install the result.
pub struct ServerRegistryPagePreparation {
    capture: RegistrySourceCapture,
    permit: OwnedSemaphorePermit,
    generation: Arc<()>,
}

/// Detached verification result, not permission to serve. Installation rechecks the mount,
/// runtime, preparation generation, current membership and exact saved record before attachment.
pub struct ServerPreparedRegistryPageSource {
    inner: PreparedSource,
    generation: Arc<()>,
}

impl std::fmt::Debug for ServerRegistryPagePreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerRegistryPagePreparation { .. }")
    }
}
impl std::fmt::Debug for ServerPreparedRegistryPageSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerPreparedRegistryPageSource { .. }")
    }
}
impl ServerRegistryPagePreparation {
    /// Reconstruct on a Tokio blocking worker. Dropping this future cannot cancel running work;
    /// its slot stays owned by that work/result until actual destruction. No state is installed.
    pub async fn rebuild(self) -> Result<ServerPreparedRegistryPageSource, AppError> {
        self.rebuild_with(RegistrySourceCapture::rebuild).await
    }

    async fn rebuild_with(
        self,
        rebuild: impl FnOnce(
                RegistrySourceCapture,
            ) -> Result<(RegistrySourceStamp, RegistryPageSource), AppError>
            + Send
            + 'static,
    ) -> Result<ServerPreparedRegistryPageSource, AppError> {
        tokio::task::spawn_blocking(move || {
            let (stamp, source) = rebuild(self.capture)?;
            Ok(ServerPreparedRegistryPageSource {
                inner: PreparedSource {
                    stamp,
                    source,
                    _permit: self.permit,
                },
                generation: self.generation,
            })
        })
        .await
        .map_err(|_| AppError::Invalid("registry page preparation worker failed".into()))?
    }
}

fn preparation_required() -> AppError {
    AppError::Invalid("registry page source requires local preparation".into())
}

mod receive;
pub use receive::{RegistryReceiveProgress, RegistryReceiveState, ServerRegistryReceive};

/// One provider-local key tied to an exact Server, physical vault mount, captured local server
/// id and registry bucket. At most one prepared read-only source is retained; all providers and
/// running preparations share four process-wide slots. No requester-specific sessions remain.
/// Drop on lifecycle replacement: this is not a native UI-unlock lease or a remote permission.
pub struct ServerRegistryPageProvider {
    inner: RegistryPageProvider,
    instance: RegistrySyncInstance,
    mount: Arc<()>,
    server: u64,
    bucket: u8,
    prepared: Option<PreparedSource>,
    preparation_generation: Arc<()>,
}
impl ServerRegistryPageProvider {
    /// Local cache status only. True is not a currency/authority claim; service rechecks the file.
    pub fn has_prepared_source(&self) -> bool {
        self.prepared.is_some()
    }

    fn check_source(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let prepared = self.prepared.take().ok_or_else(preparation_required)?;
        // Taking first means every I/O error also releases the stale body instead of allowing
        // a later caller to accidentally fall back to it. Never normalize/rebuild for this check.
        if !store.registry_page_source_is_current(&prepared.stamp)? {
            return Err(preparation_required());
        }
        self.prepared = Some(prepared);
        Ok(())
    }
}
impl std::fmt::Debug for ServerRegistryPageProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerRegistryPageProvider { .. }")
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Capture a saved source under lifecycle/store custody, then release that custody BEFORE
    /// awaiting the job's `rebuild`. No network request triggers preparation. None means actual
    /// absence, not a proof of empty history. Capacity/corruption are errors. Refresh drops the
    /// old cache but keeps the cursor MAC; any previously captured completion is superseded.
    pub fn begin_registry_page_preparation(
        &mut self,
        store: &ServerStore,
        provider: &mut ServerRegistryPageProvider,
    ) -> Result<Option<ServerRegistryPagePreparation>, AppError> {
        self.begin_registry_page_preparation_with(store, provider, preparation_pool())
    }

    fn begin_registry_page_preparation_with(
        &mut self,
        store: &ServerStore,
        provider: &mut ServerRegistryPageProvider,
        pool: &Arc<Semaphore>,
    ) -> Result<Option<ServerRegistryPagePreparation>, AppError> {
        self.check_page_provider(store, provider)?;
        // Refresh keeps the cursor MAC but releases the previous full source BEFORE reserving.
        provider.prepared = None;
        provider.preparation_generation = Arc::new(());
        let permit = pool.clone().try_acquire_owned().map_err(|_| {
            AppError::Invalid("registry page preparation capacity exhausted".into())
        })?;
        let capture = store.capture_registry_page_source(
            provider.server,
            &self.group_id(),
            provider.bucket,
        )?;
        let Some(capture) = capture else {
            return Ok(None);
        };
        Ok(Some(ServerRegistryPagePreparation {
            capture,
            permit,
            generation: provider.preparation_generation.clone(),
        }))
    }

    /// Attach only under reacquired current lifecycle/store custody. Results from a different
    /// provider or superseded preparation cannot replace a newer cache even for identical bytes.
    pub fn finish_registry_page_preparation(
        &mut self,
        store: &ServerStore,
        provider: &mut ServerRegistryPageProvider,
        prepared: ServerPreparedRegistryPageSource,
    ) -> Result<(), AppError> {
        self.check_page_provider(store, provider)?;
        if !Arc::ptr_eq(&provider.preparation_generation, &prepared.generation) {
            return Err(AppError::Invalid(
                "registry page preparation was replaced".into(),
            ));
        }
        if !store.registry_page_source_is_current(&prepared.inner.stamp)? {
            return Err(preparation_required());
        }
        provider.prepared = Some(prepared.inner);
        Ok(())
    }

    // Convenience only for tests without actor/store mutexes. Production uses the split API
    // above so unrelated saves and Server ticks continue while verification runs off-executor.
    #[cfg(test)]
    async fn prepare_registry_page_source_with(
        &mut self,
        store: &ServerStore,
        provider: &mut ServerRegistryPageProvider,
        pool: &Arc<Semaphore>,
        rebuild: impl FnOnce(
                RegistrySourceCapture,
            ) -> Result<(RegistrySourceStamp, RegistryPageSource), AppError>
            + Send
            + 'static,
    ) -> Result<bool, AppError> {
        let Some(job) = self.begin_registry_page_preparation_with(store, provider, pool)? else {
            return Ok(false);
        };
        let result = job.rebuild_with(rebuild).await?;
        self.finish_registry_page_preparation(store, provider, result)?;
        Ok(true)
    }

    fn check_page_provider(
        &mut self,
        store: &ServerStore,
        provider: &ServerRegistryPageProvider,
    ) -> Result<(), AppError> {
        if !self.sync.matches_registry_instance(&provider.instance)
            || !Arc::ptr_eq(&provider.mount, &store.registry_mount())
        {
            return Err(AppError::Invalid(
                "registry page provider belongs to a replaced server or mount".into(),
            ));
        }
        self.sync.with_registry_context(|group, device, _, _| {
            if group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
            {
                return Err(AppError::Invalid(
                    "registry provider is not a current member".into(),
                ));
            }
            Ok(())
        })
    }

    /// Fetch a transport-verified page from an already proven member endpoint. No store write,
    /// cursor advancement, epoch installation or delivery/finality acknowledgement occurs here.
    /// The durable receive driver must gate and save every op before following a continuation.
    pub async fn request_registry_page(
        &mut self,
        peer: PeerId,
        query: RegistryPageQuery<'_>,
    ) -> Result<Option<RegistryPageOutcome>, AppError> {
        Ok(self.sync.request_registry_page(peer, query).await?)
    }

    /// Answer one authenticated network request using only the durable registry bound to this
    /// watch/provider pair. Mount and scope checks precede even queue consumption; sync checks
    /// current requester authority, watch generation and aggregate service debt before vault I/O.
    /// None means no eligible work (including throttling), not an empty or settled document.
    pub fn serve_registry_request_step(
        &mut self,
        store: &ServerStore,
        provider: &mut ServerRegistryPageProvider,
        watch: &ServerRegistryWatch,
    ) -> Result<Option<()>, AppError> {
        self.check_registry_watch(store, watch)?;
        if !self.sync.matches_registry_instance(&provider.instance)
            || !Arc::ptr_eq(&provider.mount, &store.registry_mount())
            || provider.server != watch.server
            || provider.bucket != watch.bucket
        {
            return Err(AppError::Invalid(
                "registry page provider/watch scope mismatch".into(),
            ));
        }
        self.sync
            .serve_registry_request(&watch.inner, |group, device, rng, request| {
                if !provider
                    .inner
                    .preflight_request(group, device, provider.bucket, &request)
                    .map_err(|e| AppError::Invalid(e.to_string()))?
                {
                    return Ok(RegistryPageOutcome::Restart);
                }
                provider.check_source(store)?;
                provider
                    .inner
                    .page_prepared(
                        &provider.prepared.as_ref().expect("checked source").source,
                        group,
                        device,
                        request,
                        rng,
                    )
                    .map_err(|e| AppError::Invalid(e.to_string()))
            })?
            .transpose()
    }

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
            prepared: None,
            preparation_generation: Arc::new(()),
        })
    }

    /// Read one bounded page from the actual saved source. The trusted network adapter MUST
    /// derive `request.requester` from its authenticated request, never from a body field. Runtime/
    /// mount binding, current membership, field caps, continuation MAC and expiry reject before
    /// source I/O. Matching the current concrete document id requires loading the captured bucket.
    /// Cold, missing or changed sources require explicit local preparation; corruption is an
    /// error. Neither is a wire claim about remote history. No replay occurs here, but the bounded
    /// saved record is reread/unsealed/hashed each call. No writes or acks occur.
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
            provider.check_source(store)?;
            provider
                .inner
                .page_prepared(
                    &provider.prepared.as_ref().expect("checked source").source,
                    group,
                    device,
                    request,
                    rng,
                )
                .map_err(|e| AppError::Invalid(e.to_string()))
        })
    }
}

#[cfg(test)]
mod tests;

// Fixture-local pools prevent unrelated parallel tests from competing for production capacity.
// Dedicated lifecycle tests use a shared four-slot pool and the exact implementation above.
#[cfg(test)]
pub(crate) async fn prepare_test_source<T: MeshTransport, R: CryptoRngCore>(
    server: &mut Server<T, R>,
    store: &ServerStore,
    provider: &mut ServerRegistryPageProvider,
) -> Result<bool, AppError> {
    server
        .prepare_registry_page_source_with(
            store,
            provider,
            &Arc::new(Semaphore::new(MAX_PREPARED_REGISTRY_SOURCES)),
            RegistrySourceCapture::rebuild,
        )
        .await
}
