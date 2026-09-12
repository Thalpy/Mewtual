use super::*;
use crate::checkpoint_exchange::CheckpointTarget;
use crate::epoch_service::{EpochServiceInterest, EpochServiceKind};

impl From<PageScope> for CheckpointTarget {
    fn from(scope: PageScope) -> Self {
        match scope {
            PageScope::Registry(b) => Self::Registry(b),
            PageScope::Studio(t) => Self::Studio(t),
        }
    }
}
impl From<CheckpointTarget> for PageScope {
    fn from(scope: CheckpointTarget) -> Self {
        match scope {
            CheckpointTarget::Registry(b) => Self::Registry(b),
            CheckpointTarget::Studio(t) => Self::Studio(t),
        }
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    fn service_page_current(&self, p: &Pending) -> bool {
        p.auth.epoch == self.group.epoch()
            && self.clock.now_ms().abs_diff(p.auth.ts) <= MAX_REQUEST_AGE_MS
            && self.registry_page_member(&p.key)
            && self.registry_page_member(&self.device.public_key_bytes())
            && (self
                .page_watch_binding(p.query.scope)
                .is_some_and(|(id, g)| id == p.query.doc_id && Arc::ptr_eq(&g, &p.generation))
                || self.epoch_service_generation_is_current(&p.generation))
    }
    pub(crate) fn clear_page_service_requests(&mut self) {
        self.registry_pages.pending.clear();
    }
    pub(crate) fn has_page_service_interest(&self) -> bool {
        self.registry_pages.pending.iter().any(|p| {
            !p.preparing && self.clock.monotonic_ms() < p.expires && self.service_page_current(p)
        })
    }
    pub(crate) fn reserve_page_service_interest(&mut self) -> Option<EpochServiceInterest> {
        let now = self.registry_pages.expire(self.clock.monotonic_ms());
        let index = self
            .registry_pages
            .pending
            .iter()
            .position(|p| !p.preparing && self.service_page_current(p))?;
        if !self
            .registry_pages
            .service
            .get_or_insert_with(|| Rate::full(now, 4))
            .charge(now, 2, 4)
        {
            return None;
        }
        let instance = self.registry_instance();
        let p = &mut self.registry_pages.pending[index];
        p.preparing = true;
        Some(EpochServiceInterest {
            instance,
            enabled: self.epoch_service.generation.clone()?,
            request: p.id.clone(),
            generation: p.generation.clone(),
            target: p.query.scope.into(),
            kind: EpochServiceKind::Page,
            doc_id: Some(p.query.doc_id),
            expires: p.expires,
        })
    }
    pub(crate) fn page_service_interest_is_current(&self, interest: &EpochServiceInterest) -> bool {
        self.registry_pages.pending.iter().any(|p| {
            Arc::ptr_eq(&p.id, &interest.request)
                && CheckpointTarget::from(p.query.scope) == interest.target
                && self.service_page_current(p)
        })
    }
    /// Serves a single prepaid request using the ordinary signed page engine. The callback
    /// must compare the requested concrete id with its actual source before returning any ops.
    /// This right is unrelated to (and cannot create) an operation receive permit.
    pub fn serve_epoch_page_interest<E>(
        &mut self,
        interest: &EpochServiceInterest,
        serve: impl FnOnce(
            &ServerGroup,
            &MlsDevice,
            &mut R,
            RegistryPageRequest<'_>,
        ) -> Result<RegistryPageOutcome, E>,
    ) -> Result<Option<Result<(), E>>, SyncError> {
        if interest.kind != EpochServiceKind::Page
            || !self.epoch_service_interest_is_current(interest)
        {
            return Err(SyncError::Unauthorized);
        }
        self.serve_epoch_request_bound(
            interest.target.into(),
            interest.doc_id.ok_or(SyncError::Malformed)?,
            &interest.generation,
            Some(&interest.request),
            serve,
        )
    }
}
