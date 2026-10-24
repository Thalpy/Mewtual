//! Independently quotaed Ready capacity, paid before affine extraction. This owner does
//! NOT authorize a Ready packet or a transport effect. Its eventual Source consumer must
//! additionally prove installed receive custody, old-Workflow drainage and exact publication.
//!
//! Ownership is deliberately one way: native Pair -> CacheCapacity -> Producer -> Exchange.
//! External credits retain Loan -> Producer. Neither Producer nor Exchange stores the cache
//! or a credited loan, which would form a permanent self-custody cycle.
use super::capacity;
use crate::voice_v2::negotiation::{Exchange, PaidReadyCache, Stage};
use crate::voice_v2::resources::{Credit, Resources};
use crate::voice_v2::VoiceError;
use std::sync::{Arc, Mutex};
mod artifact;

/// Occupancy, not an issuance budget. The irreversible constructor latch lives in Exchange;
/// dropping a credit/Producer cannot authorize a new Ready identity or reset retry attempts.
pub(in crate::voice_v2) struct Producer {
    used: Mutex<[usize; 4]>,
    // LAST: real credit returns and quota retirement must precede whole-plan drainage.
    exchange: Arc<Exchange>,
}
struct Loan {
    index: usize,
    producer: Arc<Producer>,
}
/// Unforgeable borrowed quota decision. No constructor/Clone exists outside this module;
/// the raw original-reserve lender requires it, so stage/byte/custody checks cannot be skipped.
pub(in crate::voice_v2) struct ChargePermit<'a> {
    producer: &'a Producer,
    stage: Stage,
    bytes: usize,
}
impl ChargePermit<'_> {
    pub(in crate::voice_v2) fn for_exchange(
        &self,
        exchange: &Exchange,
    ) -> Result<(Stage, usize), VoiceError> {
        if !std::ptr::eq(Arc::as_ptr(&self.producer.exchange), exchange) {
            return Err(VoiceError::Scope);
        }
        Ok((self.stage, self.bytes))
    }
}
/// One protected cache position, retained outside Producer by the actual native Pair.
/// Signing retains first ID/time/expiry in this same owner across retries. Its inert signed
/// artifact cannot publish before the separate receive/authority transaction exists.
pub(in crate::voice_v2) struct CacheCapacity {
    artifact: artifact::ArtifactState,
    _cache: Credit,
    // Last, independently of the credit's Loan, for future artifact-field destruction.
    producer: Arc<Producer>,
}
redacted_debug!(Producer, Loan, CacheCapacity);

impl Producer {
    /// Only `CacheCapacity::from_paid` can construct a Producer, after original payment.
    fn new(exchange: Arc<Exchange>) -> Arc<Self> {
        Arc::new(Self {
            used: Mutex::new([1, 0, 0, 0]),
            exchange,
        })
    }
    pub(in crate::voice_v2) fn current_for(&self, exchange: &Arc<Exchange>) -> bool {
        Arc::ptr_eq(&self.exchange, exchange) && self.exchange.ready_capacity_open()
    }
    /// Capacity only. These are independent of SDP stage quotas, but share its ORIGINAL
    /// Crypto lane and two group Lower positions. No ordinary fallback is permitted.
    pub(in crate::voice_v2) fn charge(
        self: &Arc<Self>,
        resources: &Arc<Resources>,
        group: [u8; 32],
        stage: Stage,
        bytes: usize,
    ) -> Result<Credit, VoiceError> {
        let (index, count, maximum) = match stage {
            Stage::LocalPacket => (0, 1, capacity::CACHE),
            Stage::Source => (1, 1, capacity::SOURCE),
            Stage::Lower => (2, 2, capacity::LOWER),
            Stage::Crypto => (3, 1, capacity::ACK_CRYPTO),
            _ => return Err(VoiceError::Scope),
        };
        if !self.exchange.uses_resources(resources, group) {
            return Err(VoiceError::Scope);
        }
        if bytes == 0 || bytes > maximum {
            return Err(VoiceError::Bounds);
        }
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        if !self.exchange.ready_capacity_open() {
            return Err(VoiceError::Unauthorized);
        }
        if used[index] >= count {
            return Err(VoiceError::Capacity);
        }
        let credit = self.exchange.ready_capacity_credit(&ChargePermit {
            producer: self,
            stage,
            bytes,
        })?;
        used[index] += 1;
        drop(used);
        // A refusal must never drop a Loan while its own quota mutex is still held.
        Ok(credit.with_custody(Arc::new(Loan {
            index,
            producer: self.clone(),
        })))
    }
}
impl CacheCapacity {
    pub(in crate::voice_v2) fn from_paid(paid: PaidReadyCache) -> Self {
        let (cache, exchange) = paid.into_parts();
        let producer = Producer::new(exchange);
        let cache = cache.with_custody(Arc::new(Loan {
            index: 0,
            producer: producer.clone(),
        }));
        Self {
            artifact: artifact::ArtifactState::default(),
            _cache: cache,
            producer,
        }
    }
    pub(in crate::voice_v2) fn current_for(&self, exchange: &Arc<Exchange>) -> bool {
        self.producer.current_for(exchange)
    }
    #[cfg(test)]
    pub(in crate::voice_v2) fn producer_for_test(&self) -> Arc<Producer> {
        self.producer.clone()
    }
}
impl Drop for Loan {
    fn drop(&mut self) {
        let mut used = self.producer.used.lock().unwrap_or_else(|e| e.into_inner());
        assert!(used[self.index] > 0, "actual Ready capacity custody");
        used[self.index] -= 1;
        // Guard drops before the final Producer field, whose Exchange may enter its State.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice_v2::resources::{Reservation, ResourceKind};
    use std::mem::size_of;

    #[test]
    fn producer_cache_and_all_actual_loan_wrappers_fit_the_original_ready_metadata() {
        // Structural application graph, not allocator slack, SDK heap or RSS. Packet/Source
        // and route wrappers retain their separately declared stage payments. Exchange and
        // original group lanes/binding have their own existing metadata payments.
        let graph = size_of::<Producer>()
            + size_of::<CacheCapacity>()
            + size_of::<capacity::ReadyCapacity>()
            + size_of::<Reservation>()
            + 5 * size_of::<Loan>()
            + 7 * 2 * size_of::<usize>();
        assert!(
            graph < capacity::METADATA,
            "Ready owner graph grew to {graph}"
        );
        assert_eq!(Stage::Crypto.resource_kind(), ResourceKind::Crypto);
    }
}
