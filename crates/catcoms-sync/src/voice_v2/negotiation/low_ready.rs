//! A transferred LOW lane may verify an exact lost Ready ACK, without opening generic SDP
//! or borrowing ordinary capacity. Normal request work/signatures/native admission still run.
use super::*;
impl Exchange {
    pub(crate) fn low_ready_retry_open(&self) -> bool {
        self.live.is_live()
            && self
                .successor
                .as_ref()
                .is_some_and(|proof| proof.live() && proof.binding().low)
            && self.low_ready.is_some()
    }
    pub(crate) fn lends_low_ready_retry_verification(
        &self,
        requester: DeviceId,
        frame: &[u8],
        wall: u64,
        mono: u64,
    ) -> bool {
        let Some(receipt) = &self.low_ready else {
            return false;
        };
        let Ok(peek) = super::super::Envelope::peek_ready(frame) else {
            return false;
        };
        self.low_ready_retry_open()
            && receipt.matches_retry(&peek, wall, mono)
            && ((peek.envelope_kind == 1 && requester == peek.issuer)
                || (peek.envelope_kind == 3 && self.allows_helper(requester)))
    }
    pub(crate) fn charge_low_ready_retry_verification(
        self: &Arc<Self>,
        bytes: usize,
    ) -> Result<Credit, VoiceError> {
        let (index, maximum_count, maximum_bytes, kind) = Stage::Crypto.contract();
        if bytes == 0 || bytes > maximum_bytes {
            return Err(VoiceError::Bounds);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !self.low_ready_retry_open() || state.used[index] >= maximum_count {
            return Err(VoiceError::Capacity);
        }
        let credit = ByteBudget::ReservedLane {
            reservation: self.reserve.clone(),
            lane: self.lane.clone(),
        }
        .charge(&self.resources, self.group, bytes, kind)?;
        state.used[index] += 1;
        Ok(credit.with_custody(Arc::new(PhaseLoan {
            exchange: self.clone(),
            index,
        })))
    }
}
