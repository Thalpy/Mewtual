//! The Registry provider already owns detached verification and a bounded retained source.
//! Reuse it for service; do not reconstruct the same graph again for each head/seed query.
use super::*;
impl CatchupRuntime {
    #[cfg(test)]
    pub(in crate::studio::receiver) fn registry_cache_for_test(
        &mut self,
        provider: ServerRegistryPageProvider,
        until: u64,
    ) {
        self.registry_provider = Some(provider);
        self.registry_retained_until = until;
    }
    /// A retained read-only Registry graph still owns a process preparation permit. Bound
    /// its idle lifetime so four quiet servers cannot strand every other server's Studio
    /// receive. Do not extend this deadline per query or refund any running worker's slot.
    pub(super) fn expire_registry_source(&mut self, now: u64) {
        if now >= self.registry_retained_until
            && self
                .registry_provider
                .as_ref()
                .is_some_and(|p| p.has_prepared_source())
        {
            self.registry_provider = None;
        }
    }
    pub(super) fn drop_registry_service(&mut self, generation: &Arc<()>) {
        if self
            .service
            .as_ref()
            .is_some_and(|s| Arc::ptr_eq(&s.generation, generation))
        {
            self.service = None;
        }
    }
    pub(super) fn complete_registry_preparation<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
    ) -> Result<(), AppError> {
        if let Some((generation, result)) = self.registry_prepared.take() {
            let installed = result.and_then(|prepared| {
                let provider = self
                    .registry_provider
                    .as_mut()
                    .ok_or_else(|| invalid("Registry preparation superseded"))?;
                if !server
                    .attach_registry_page_preparation_if_current(store, provider, *prepared)?
                {
                    return Ok(false);
                }
                server.remember_registry_service_inventory(store, provider)?;
                Ok(true)
            });
            if let Err(error) = installed {
                if let Some(generation) = generation {
                    self.drop_registry_service(&generation);
                } else {
                    return Err(error);
                }
            } else if matches!(installed, Ok(true)) {
                self.registry_retained_until =
                    server.runtime_clock().monotonic_ms().saturating_add(30_000);
            } else {
                if let Some(generation) = generation {
                    self.drop_registry_service(&generation);
                }
                self.next_at = server.runtime_clock().monotonic_ms().saturating_add(5_000);
            }
        }
        Ok(())
    }
    /// Local discovery also needs a valid footprint for a large saved bucket, even when its
    /// adoption is deferred. Reuse exactly the provider's bounded detached preparation; no
    /// extra graph, inventory allowance or remote service token is minted by this local work.
    pub(super) fn prepare_registry_inventory<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        bucket: u8,
    ) -> Result<bool, AppError> {
        if self
            .registry_provider
            .as_ref()
            .is_none_or(|p| !server.registry_page_provider_matches(store, id, bucket, p))
        {
            if self.preparing
                || self.registry_preparation.is_some()
                || self.registry_prepared.is_some()
            {
                return Ok(false);
            }
            self.registry_provider = Some(server.begin_registry_page_provider(store, id, bucket)?);
        }
        let provider = self.registry_provider.as_mut().expect("provider");
        if server.registry_page_preparation_is_warm(store, provider)? {
            server.remember_registry_service_inventory(store, provider)?;
            return Ok(true);
        }
        if self.preparing
            || self.preparation.is_some()
            || self.prepared.is_some()
            || self.registry_preparation.is_some()
        {
            return Ok(false);
        }
        // A busy global pool is not corrupt storage. A future idle turn retries; any installed
        // Registry graphs release their slots on their fixed local-clock deadlines.
        let Ok(permit) = crate::registry_catchup::preparation_pool()
            .clone()
            .try_acquire_owned()
        else {
            return Ok(false);
        };
        if let Some(job) =
            server.begin_registry_page_preparation_reserved(store, provider, permit)?
        {
            self.registry_preparation = Some((job, None));
            Ok(false)
        } else {
            Ok(true)
        }
    }
    pub(super) fn serve_registry<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        mut work: ServiceWork,
    ) {
        let CheckpointTarget::Registry(bucket) = work.interest.target() else {
            return;
        };
        if self
            .registry_provider
            .as_ref()
            .is_none_or(|p| !server.registry_page_provider_matches(store, id, bucket, p))
        {
            // This cache is also the attachment target of an in-flight local preparation.
            // Queue the other key until attachment; replacing it early would strand that job.
            if self.preparing
                || self.registry_preparation.is_some()
                || self.registry_prepared.is_some()
            {
                self.service = Some(work);
                return;
            }
            self.registry_provider = server.begin_registry_page_provider(store, id, bucket).ok();
        }
        let Some(provider) = self.registry_provider.as_mut() else {
            return;
        };
        let warm = match server.registry_page_preparation_is_warm(store, provider) {
            Ok(warm) => warm,
            Err(_) => return,
        };
        if !warm {
            if self.preparing
                || self.preparation.is_some()
                || self.registry_preparation.is_some()
                || self.prepared.is_some()
            {
                self.service = Some(work);
                return;
            }
            if work.captured {
                return;
            }
            match server.begin_registry_page_preparation(store, provider) {
                Ok(Some(job)) => {
                    work.captured = true;
                    self.registry_preparation = Some((job, Some(work.generation.clone())));
                    self.service = Some(work);
                    return;
                }
                Err(_) => return,
                Ok(None) => {} // Exact absence will be checked against inventory in service.
            }
        }
        if !server
            .sync
            .epoch_service_interest_is_current(&work.interest)
        {
            return;
        }
        if let Ok(mut budget) = Self::inventory_budget(server, store, id) {
            let _ = server.serve_registry_interest(
                store,
                id,
                provider,
                &work.interest,
                self.owner_snapshot.as_ref(),
                &mut budget,
            );
        }
    }
}
