//! Native Ready Source provenance, separate from the SDP packet factory. Only the original
//! signed cache with an installed receiver and Root receipt position can construct it.
//! Transport ACK is not causal completion: the exact native first-Offer transaction consumes
//! that receipt position, which synchronously cancels all outstanding Ready work.
use super::*;
use crate::voice_v2::negotiation::receive::HighReceive;
use crate::voice_v2::negotiation::{Exchange, Stage};
use crate::voice_v2::resources::{Credit, Resources};
use catcoms_rt::Clock;
use futures::FutureExt;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ReadySourceAuthority {
    id: MessageId,
    hash: [u8; 32],
    signature: [u8; 64],
    first_mono: u64,
    deadline: u64,
    position: CallLease,
    clock: Arc<dyn Clock>,
    receive: Arc<HighReceive>,
    // Last: every actual input/route/ACK still owns its original Producer/Exchange custody.
    producer: Arc<producer::Producer>,
}
redacted_debug!(ReadySourceAuthority);
impl ReadySourceAuthority {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::voice_v2::ready) fn new(
        object: &SignedObject,
        hash: [u8; 32],
        first_mono: u64,
        deadline: u64,
        position: CallLease,
        clock: Arc<dyn Clock>,
        receive: Arc<HighReceive>,
        producer: Arc<producer::Producer>,
    ) -> Result<Self, VoiceError> {
        let authority = Self {
            id: object.id(),
            hash,
            signature: object.signature,
            first_mono,
            deadline,
            position,
            clock,
            receive,
            producer,
        };
        let (wall, mono) = authority.observation_time();
        if !authority.check_at(object, wall, mono) {
            return Err(VoiceError::Unauthorized);
        }
        Ok(authority)
    }
    pub(in crate::voice_v2) fn exchange(&self) -> Arc<Exchange> {
        self.receive.exchange()
    }
    pub(in crate::voice_v2) fn deadline(&self) -> u64 {
        self.deadline
    }
    pub(in crate::voice_v2) fn is_call(&self, call: &CallLease) -> bool {
        self.receive.exchange().ready_source_call(call)
    }
    pub(in crate::voice_v2) fn hash(&self) -> [u8; 32] {
        self.hash
    }
    pub(in crate::voice_v2) fn observation_time(&self) -> (u64, u64) {
        (self.clock.now_ms(), self.clock.monotonic_ms())
    }
    pub(in crate::voice_v2) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        let exchange = self.receive.exchange();
        self.position.is_live()
            && mono >= self.first_mono
            && mono < self.deadline
            && self.producer.current_for(&exchange)
            && self.receive.current_for(&exchange)
            && exchange.ready_source_current_at(self.id, self.hash, wall, mono)
    }
    pub(in crate::voice_v2) fn check_at(
        &self,
        object: &SignedObject,
        wall: u64,
        mono: u64,
    ) -> bool {
        matches!(object.body(), Body::PairReady { .. })
            && object.id() == self.id
            && object.signature == self.signature
            && self.authorized_at(wall, mono)
    }
    pub(in crate::voice_v2) fn same(&self, other: &Self) -> bool {
        self.id == other.id
            && self.hash == other.hash
            && self.signature == other.signature
            && self.first_mono == other.first_mono
            && self.deadline == other.deadline
            && self.position.is_same(&other.position)
            && Arc::ptr_eq(&self.clock, &other.clock)
            && Arc::ptr_eq(&self.receive, &other.receive)
            && Arc::ptr_eq(&self.producer, &other.producer)
    }
    pub(in crate::voice_v2) fn charge(
        &self,
        resources: &Arc<Resources>,
        group: [u8; 32],
        stage: Stage,
        bytes: usize,
    ) -> Result<Credit, VoiceError> {
        self.producer.charge(resources, group, stage, bytes)
    }
    pub(in crate::voice_v2) fn require_current_at(
        &self,
        resources: &Arc<Resources>,
        group: [u8; 32],
        wall: u64,
        mono: u64,
    ) -> Result<(), VoiceError> {
        let exchange = self.receive.exchange();
        if !exchange.uses_resources(resources, group)
            || !self.authorized_at(wall, mono)
            || !resources
                .exchange(group)
                .as_ref()
                .is_some_and(|actual| Arc::ptr_eq(actual, &exchange))
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    pub(in crate::voice_v2) async fn revoked(&self) {
        let (wall, mono) = self.observation_time();
        if !self.authorized_at(wall, mono) {
            return;
        }
        // The position lease closes on both successful native Offer admission and abandoned
        // cache cleanup. Never wait on Exchange::revoked: generic Exchange is inactive here.
        let exchange = self.receive.exchange();
        futures::future::select_all(vec![
            self.position.wait_revoked().boxed(),
            exchange.ready_source_closed().boxed(),
            self.clock
                .sleep(std::time::Duration::from_millis(
                    self.deadline.saturating_sub(self.clock.monotonic_ms()),
                ))
                .boxed(),
        ])
        .await;
    }
}
