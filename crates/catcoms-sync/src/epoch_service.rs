//! Local opt-in service of saved logical documents, independent of UI receive subscriptions.
//! Interests remain in the existing bounded queues. A private token charges source preparation
//! before the app probes the vault; it grants neither receive authority nor document existence.
use super::*;
use crate::checkpoint_exchange::CheckpointTarget;

#[derive(Default)]
pub(super) struct EpochService {
    pub(super) generation: Option<Arc<()>>,
    turn: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpochServiceKind {
    Head,
    Seed,
    Page,
}

/// One preparation right for one authenticated queued request. Not Clone: cancellation loses
/// this right without refunding service debt. It owns no body, source, vault key or responder.
pub struct EpochServiceInterest {
    pub(super) instance: RegistrySyncInstance,
    pub(super) enabled: Arc<()>,
    pub(super) request: Arc<()>,
    pub(super) generation: Arc<()>,
    pub(super) target: CheckpointTarget,
    pub(super) kind: EpochServiceKind,
    pub(super) doc_id: Option<u128>,
    pub(super) expires: u64,
}
impl fmt::Debug for EpochServiceInterest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EpochServiceInterest")
            .field("kind", &self.kind)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}
impl EpochServiceInterest {
    pub fn target(&self) -> CheckpointTarget {
        self.target
    }
    pub fn kind(&self) -> EpochServiceKind {
        self.kind
    }
    pub fn doc_id(&self) -> Option<u128> {
        self.doc_id
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Call under local unlocked runtime custody, not in response to untrusted query contents.
    /// Idempotent; enabling adds no watches/subscriptions and admits no operation or disk state.
    pub fn enable_epoch_service(&mut self) {
        self.epoch_service
            .generation
            .get_or_insert_with(|| Arc::new(()));
    }
    /// Revokes preparation/results without changing receive watches or forgiving rate debt.
    pub fn disable_epoch_service(&mut self) {
        self.epoch_service.generation = None;
        self.clear_head_service_requests();
        self.clear_seed_service_requests();
        self.clear_page_service_requests();
    }
    pub(super) fn epoch_service_generation_is_current(&self, generation: &Arc<()>) -> bool {
        self.epoch_service
            .generation
            .as_ref()
            .is_some_and(|g| Arc::ptr_eq(g, generation))
    }
    pub(super) fn epoch_service_context_is_current(&self, interest: &EpochServiceInterest) -> bool {
        self.matches_registry_instance(&interest.instance)
            && self.epoch_service_generation_is_current(&interest.enabled)
            && self.clock.monotonic_ms() < interest.expires
    }
    /// Current metadata only. No lookup/rebuild is allowed on this hint alone.
    pub fn has_epoch_service_interest(&self) -> bool {
        self.epoch_service.generation.is_some()
            && (self.has_head_service_interest()
                || self.has_seed_service_interest()
                || self.has_page_service_interest())
    }
    /// Fairly reserves one family using that family's EXISTING service token bucket. The
    /// queued request is marked once before returning; repeated polling/cancellation cannot
    /// mint another preparation right. It prepays serving only that exact request; a new
    /// request for the same key must pay again, even if the first one prepared a reusable graph.
    pub fn reserve_epoch_service_interest(&mut self) -> Option<EpochServiceInterest> {
        self.epoch_service.generation.as_ref()?;
        for _ in 0..3 {
            let turn = self.epoch_service.turn % 3;
            self.epoch_service.turn = self.epoch_service.turn.wrapping_add(1);
            let next = match turn {
                0 => self.reserve_head_service_interest(),
                1 => self.reserve_seed_service_interest(),
                _ => self.reserve_page_service_interest(),
            };
            if next.is_some() {
                return next;
            }
        }
        None
    }
    /// A detached preparation must recheck the exact queued request, membership and expiry
    /// before source use. New requests for the same key never inherit an old preparation right.
    pub fn epoch_service_interest_is_current(&self, interest: &EpochServiceInterest) -> bool {
        self.epoch_service_context_is_current(interest)
            && match interest.kind {
                EpochServiceKind::Head => self.head_service_interest_is_current(interest),
                EpochServiceKind::Seed => self.seed_service_interest_is_current(interest),
                EpochServiceKind::Page => self.page_service_interest_is_current(interest),
            }
    }
}
