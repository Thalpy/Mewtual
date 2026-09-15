use super::*;
use crate::epoch_service::{EpochServiceInterest, EpochServiceKind};

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub(crate) fn clear_head_service_requests(&mut self) {
        self.receipt_heads.pending.clear();
    }
    fn service_head_binding(&self, item: &Pending) -> bool {
        self.receipt_heads
            .watches
            .get(&item.target)
            .is_some_and(|g| Arc::ptr_eq(g, &item.generation))
            || self.epoch_service_generation_is_current(&item.generation)
    }
    pub(crate) fn has_head_service_interest(&self) -> bool {
        self.receipt_heads.pending.iter().any(|p| {
            !p.preparing
                && self.clock.monotonic_ms() < p.expires
                && self.head_request_current(p)
                && self.service_head_binding(p)
        })
    }
    pub(crate) fn reserve_head_service_interest(&mut self) -> Option<EpochServiceInterest> {
        let now = self.receipt_heads.expire(self.clock.monotonic_ms());
        let index = self.receipt_heads.pending.iter().position(|p| {
            !p.preparing && self.head_request_current(p) && self.service_head_binding(p)
        })?;
        if !self
            .receipt_heads
            .service
            .get_or_insert_with(|| Rate::full(now, 4))
            .charge(now, 2, 4)
        {
            return None;
        }
        let instance = self.registry_instance();
        let p = &mut self.receipt_heads.pending[index];
        p.preparing = true;
        Some(EpochServiceInterest {
            instance,
            enabled: self.epoch_service.generation.clone()?,
            request: p.id.clone(),
            generation: p.generation.clone(),
            target: p.target,
            kind: EpochServiceKind::Head,
            doc_id: None,
            expires: p.expires,
        })
    }
    pub(crate) fn head_service_interest_is_current(&self, interest: &EpochServiceInterest) -> bool {
        self.receipt_heads.pending.iter().any(|p| {
            Arc::ptr_eq(&p.id, &interest.request)
                && p.target == interest.target
                && self.head_request_current(p)
                && self.service_head_binding(p)
        })
    }
    /// Same proof, durable-source and handoff contract as explicit head service. The private
    /// interest has prepaid this exact request. Do not charge again and let preparation
    /// traffic consume all refill tokens before any already-prepared request can be answered.
    pub fn serve_epoch_head_interest<E>(
        &mut self,
        interest: &EpochServiceInterest,
        snapshot: Option<&DurableOwnerSnapshot>,
        serve: impl FnOnce(
            &ServerGroup,
            &MlsDevice,
            &mut R,
            ReceiptHeadSource<'_>,
        ) -> Result<ReceiptHeadSelection, E>,
    ) -> Result<Option<Result<ReceiptHeadServed, E>>, SyncError> {
        if interest.kind != EpochServiceKind::Head
            || !self.epoch_service_interest_is_current(interest)
        {
            return Err(SyncError::Unauthorized);
        }
        let watch = RegistryHeadWatch {
            instance: self.registry_instance(),
            target: interest.target,
            generation: interest.generation.clone(),
            request: Some(interest.request.clone()),
        };
        self.serve_receipt_head_with_handoff(&watch, snapshot, serve)
    }
}
