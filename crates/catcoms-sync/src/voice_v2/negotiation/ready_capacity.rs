//! Capacity-only Ready owner construction and original-lane access. Generic Exchange
//! liveness deliberately remains Active-only; these methods cannot publish or route Ready.
use super::*;
use crate::voice_v2::ready::{
    capacity,
    producer::{CacheCapacity, ChargePermit},
};

/// Affine payment proof; only this module can construct it, under the original Exchange
/// State fence after full cache payment. No public raw Producer/cache constructor exists.
pub(in crate::voice_v2) struct PaidReadyCache {
    cache: Credit,
    // LAST through unpublished destruction. This token grants no packet/effect authority.
    exchange: Arc<Exchange>,
}
impl PaidReadyCache {
    pub(in crate::voice_v2) fn into_parts(self) -> (Credit, Arc<Exchange>) {
        (self.cache, self.exchange)
    }
}

impl Exchange {
    #[cfg(test)]
    pub(in crate::voice_v2) fn occupy_unissued_ready_bytes_for_test(&self) -> Credit {
        let state = self.state.lock().unwrap();
        assert!(!state.ready_producer_issued);
        ByteBudget::Reserved(self.ready.as_ref().unwrap().reserve.clone())
            .charge(
                &self.resources,
                self.group,
                capacity::BYTES - capacity::METADATA,
                ResourceKind::Retained,
            )
            .unwrap()
    }
    /// Copy immutable native fields only after the cache caller pays its signing scratch.
    /// Public FinishEvidence or a renderer-provided ReadyHeader cannot construct this path.
    pub(in crate::voice_v2) fn original_ready_header(
        &self,
        mono: u64,
    ) -> Result<(super::super::ReadyHeader, u64), VoiceError> {
        let proof = self.successor.as_ref().ok_or(VoiceError::Unauthorized)?;
        let until = proof.until().ok_or(VoiceError::Unauthorized)?;
        if !self.ready_capacity_open()
            || !proof.current_at(mono)
            || mono < until - super::super::chain::AWAIT_READY_MS
        {
            return Err(VoiceError::Unauthorized);
        }
        let binding = proof.binding();
        let finish = proof.finish_identity().ok_or(VoiceError::Unauthorized)?;
        Ok((
            super::super::ReadyHeader {
                epoch: binding.epoch,
                target: binding.peer,
                source_join: binding.local_join,
                target_join: binding.peer_join,
                source_key_ref: binding.local_key,
                target_key_ref: binding.peer_key,
                handshake: binding.handshake,
                finish_message: finish.message,
                finish_object_hash: finish.hash,
                helpers: self.helpers().to_vec(),
            },
            until,
        ))
    }
    pub(in crate::voice_v2) fn ready_artifact_current_at(&self, mono: u64) -> bool {
        self.ready_capacity_open()
            && self
                .successor
                .as_ref()
                .is_some_and(|proof| proof.current_at(mono))
    }
    pub(in crate::voice_v2) fn ready_scope_and_local(
        &self,
    ) -> Result<(Scope, DeviceId), VoiceError> {
        let proof = self.successor.as_ref().ok_or(VoiceError::Unauthorized)?;
        Ok((proof.binding().scope, proof.binding().local))
    }
    pub(in crate::voice_v2) fn ready_resources(&self) -> &Arc<Resources> {
        &self.resources
    }
    pub(in crate::voice_v2) fn ready_capacity_open(&self) -> bool {
        self.live.is_live()
            && self.ready.is_some()
            && self
                .successor
                .as_ref()
                .is_some_and(|proof| !proof.binding().low && proof.live())
    }
    /// Native Pair must retain this exact returned owner before any later fallible work.
    /// Cache payment and construction precede the irreversible latch. This is not a Ready
    /// artifact: no ID, deadline, Source, receive installation or transmission is issued.
    pub(in crate::voice_v2) fn prepare_ready_cache(
        self: &Arc<Self>,
    ) -> Result<Box<CacheCapacity>, VoiceError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !self.ready_capacity_open() {
            return Err(VoiceError::Unauthorized);
        }
        if state.ready_producer_issued {
            return Err(VoiceError::Capacity);
        }
        let ready = self.ready.as_ref().ok_or(VoiceError::Unauthorized)?;
        let cache = ByteBudget::Reserved(ready.reserve.clone()).charge(
            &self.resources,
            self.group,
            capacity::CACHE,
            ResourceKind::Retained,
        )?;
        let paid = PaidReadyCache {
            cache,
            exchange: self.clone(),
        };
        state.ready_producer_issued = true;
        drop(state);
        // No recoverably failing operation after issuance. Native Pair retains this exact
        // owner before its next final-fence/registry/signing operation can refuse.
        // The body belongs to the existing2KiB cache payment, rather than inflating every
        // native Pair's fixed6KiB record before it even has a successor or Ready artifact.
        Ok(Box::new(CacheCapacity::from_paid(paid)))
    }
    /// Private prepaid bytes. Producer supplies independent occupancy quotas and its actual
    /// last-owner custody; this method never manufactures a new Crypto or Forward position.
    pub(in crate::voice_v2) fn ready_capacity_credit(
        &self,
        permit: &ChargePermit<'_>,
    ) -> Result<Credit, VoiceError> {
        let (stage, bytes) = permit.for_exchange(self)?;
        if !self.ready_capacity_open() {
            return Err(VoiceError::Unauthorized);
        }
        let ready = self.ready.as_ref().ok_or(VoiceError::Unauthorized)?;
        match stage {
            Stage::LocalPacket | Stage::Source => ByteBudget::Reserved(ready.reserve.clone())
                .charge(&self.resources, self.group, bytes, ResourceKind::Retained),
            Stage::Crypto => ByteBudget::ReservedLane {
                reservation: ready.reserve.clone(),
                lane: self.lane.clone(),
            }
            .charge(&self.resources, self.group, bytes, ResourceKind::Crypto),
            Stage::Lower => {
                let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.closed || !self.ready_capacity_open() {
                    return Err(VoiceError::Unauthorized);
                }
                state
                    .lower_lanes
                    .as_ref()
                    .ok_or(VoiceError::Unauthorized)?
                    .charge(&self.resources, self.group, &ready.reserve, bytes)
            }
            _ => Err(VoiceError::Scope),
        }
    }
}
