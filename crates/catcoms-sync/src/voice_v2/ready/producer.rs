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
pub(in crate::voice_v2) use artifact::SignedReadyStamp;

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
    receive: Option<Arc<crate::voice_v2::negotiation::receive::HighReceive>>,
    // Plain history funding. This fee must never retain Producer/Exchange progress custody.
    receipt: Option<Arc<crate::voice_v2::negotiation::admission::ReceiptFunding>>,
    position: Option<crate::voice_v2::negotiation::namespace::ReceiptPosition>,
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
        let (cache, receipt, exchange) = paid.into_parts();
        let producer = Producer::new(exchange);
        let cache = cache.with_custody(Arc::new(Loan {
            index: 0,
            producer: producer.clone(),
        }));
        Self {
            artifact: artifact::ArtifactState::default(),
            receive: None,
            receipt: Some(crate::voice_v2::negotiation::admission::ReceiptFunding::new(receipt)),
            position: None,
            _cache: cache,
            producer,
        }
    }
    pub(in crate::voice_v2) fn current_for(&self, exchange: &Arc<Exchange>) -> bool {
        self.producer.current_for(exchange)
    }
    pub(in crate::voice_v2) fn install_receive(
        &mut self,
        tables: &crate::voice_v2::packets::PacketSlots,
        mono: u64,
    ) -> Result<(), VoiceError> {
        if self.receive.is_none() {
            self.receive = Some(self.producer.exchange.prepare_high_receive(tables, mono)?);
        }
        if self
            .receive
            .as_ref()
            .is_some_and(|receive| receive.current_for(&self.producer.exchange))
        {
            Ok(())
        } else {
            Err(VoiceError::Unauthorized)
        }
    }
    pub(in crate::voice_v2) fn receive(
        &self,
    ) -> Result<Arc<crate::voice_v2::negotiation::receive::HighReceive>, VoiceError> {
        self.receive.clone().ok_or(VoiceError::Unauthorized)
    }
    pub(in crate::voice_v2) fn receipt_funding(
        &self,
    ) -> Result<Arc<crate::voice_v2::negotiation::admission::ReceiptFunding>, VoiceError> {
        self.receipt.clone().ok_or(VoiceError::Unauthorized)
    }
    pub(in crate::voice_v2) fn commit_receipt(&mut self) {
        if let Some(position) = &mut self.position {
            position.consume();
        }
        drop(self.position.take());
        drop(self.receipt.take());
    }
    pub(in crate::voice_v2) fn has_position(
        &self,
        namespace: &crate::voice_v2::negotiation::namespace::ReceiptNamespace,
    ) -> bool {
        self.position
            .as_ref()
            .is_some_and(|position| namespace.owns(position))
    }
    pub(in crate::voice_v2) fn attach_position(
        &mut self,
        position: crate::voice_v2::negotiation::namespace::ReceiptPosition,
    ) -> Result<(), VoiceError> {
        if self.position.is_some() {
            return Err(VoiceError::Capacity);
        }
        self.position = Some(position);
        Ok(())
    }
    #[cfg(test)]
    pub(in crate::voice_v2) fn receive_for_test(
        &self,
    ) -> Arc<crate::voice_v2::negotiation::receive::HighReceive> {
        self.receive.as_ref().expect("installed inbox").clone()
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
            + 7 * 2 * size_of::<usize>()
            // Root's shared count allocation is independent of Root and can survive it.
            // Charge the whole allocation to each live Ready rather than rely on sharing.
            + size_of::<std::sync::atomic::AtomicUsize>()
            + 2 * size_of::<usize>()
            // Position's pending/cause/Drop observer is allocated only AFTER this native
            // metadata fee exists. Sources clone it; they never allocate another watch.
            + size_of::<crate::voice_v2::LeaseState>()
            + 2 * size_of::<usize>();
        assert!(
            graph < capacity::METADATA,
            "Ready owner graph grew to {graph}"
        );
        assert_eq!(Stage::Crypto.resource_kind(), ResourceKind::Crypto);
    }
}
