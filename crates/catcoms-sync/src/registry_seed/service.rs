use super::*;
use crate::epoch_service::{EpochServiceInterest, EpochServiceKind};

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub(crate) fn clear_seed_service_requests(&mut self) {
        self.registry_seeds.pending.clear();
    }
    fn service_seed_binding(&self, item: &Pending) -> bool {
        self.registry_seeds
            .watches
            .get(&item.query.target)
            .is_some_and(|g| Arc::ptr_eq(g, &item.generation))
            || self.epoch_service_generation_is_current(&item.generation)
    }
    pub(crate) fn has_seed_service_interest(&self) -> bool {
        self.registry_seeds.pending.iter().any(|p| {
            !p.preparing
                && self.clock.monotonic_ms() < p.expires
                && self.seed_request_current(p)
                && self.service_seed_binding(p)
        })
    }
    pub(crate) fn reserve_seed_service_interest(&mut self) -> Option<EpochServiceInterest> {
        let now = self.registry_seeds.expire(self.clock.monotonic_ms());
        let index = self.registry_seeds.pending.iter().position(|p| {
            !p.preparing && self.seed_request_current(p) && self.service_seed_binding(p)
        })?;
        if !self
            .registry_seeds
            .service
            .get_or_insert_with(|| Rate::full(now, 2))
            .charge(now, 1, 2)
        {
            return None;
        }
        let instance = self.registry_instance();
        let p = &mut self.registry_seeds.pending[index];
        p.preparing = true;
        Some(EpochServiceInterest {
            instance,
            enabled: self.epoch_service.generation.clone()?,
            request: p.id.clone(),
            generation: p.generation.clone(),
            target: p.query.target,
            kind: EpochServiceKind::Seed,
            doc_id: Some(p.query.doc_id),
            expires: p.expires,
        })
    }
    pub(crate) fn seed_service_interest_is_current(&self, interest: &EpochServiceInterest) -> bool {
        self.registry_seeds.pending.iter().any(|p| {
            Arc::ptr_eq(&p.id, &interest.request)
                && p.query.target == interest.target
                && self.seed_request_current(p)
                && self.service_seed_binding(p)
        })
    }
    /// Serves only the exact expected-hash seed; it does not generate a checkpoint or grant
    /// authority to apply one. Shares the explicit adapter's seal, signature and expiry checks.
    pub fn serve_epoch_seed_interest<E>(
        &mut self,
        interest: &EpochServiceInterest,
        serve: impl FnOnce(&ServerGroup, &MlsDevice, u128, [u8; 32]) -> Result<Option<Vec<u8>>, E>,
    ) -> Result<Option<Result<(), E>>, SyncError> {
        if interest.kind != EpochServiceKind::Seed
            || !self.epoch_service_interest_is_current(interest)
        {
            return Err(SyncError::Unauthorized);
        }
        let watch = RegistrySeedWatch {
            instance: self.registry_instance(),
            target: interest.target,
            generation: interest.generation.clone(),
            request: Some(interest.request.clone()),
        };
        self.serve_registry_seed(&watch, serve)
    }
}
