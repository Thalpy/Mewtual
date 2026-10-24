//! Original packet-table closing rights, independent of SDP byte/action capacity. Claims
//! remain protected while all old confirmation attempts return their actual table slots.
use super::*;
use crate::voice_v2::packets::{PacketSlot, PacketSlots};
use crate::voice_v2::EncryptedHeader;

impl Exchange {
    #[cfg(test)]
    pub(in crate::voice_v2) fn assert_inactive_packet_lenders_for_test(
        self: &Arc<Self>,
        tables: &PacketSlots,
    ) {
        tests::assert_inactive_lenders(self, tables);
    }
    /// Real Ready metadata, with its genuine inbox still held, must not imply activation.
    #[cfg(test)]
    pub(in crate::voice_v2) fn assert_inactive_round_lenders_for_test(
        self: &Arc<Self>,
        tables: &PacketSlots,
    ) {
        assert!(
            self.sdp_round().is_some(),
            "requires actual Ready publication/admission"
        );
        tests::assert_inactive_headers(self, tables);
    }
    pub(in crate::voice_v2) fn uses_successor_packet_claims(&self) -> bool {
        self.successor.is_some()
    }
    /// Capacity only, outside Group/Policy. A failure retains the original parents; no
    /// receive readiness or native action may be published until the same retry succeeds.
    pub(in crate::voice_v2) fn prepare_packet_claims(&self) -> Result<(), VoiceError> {
        let parents = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed || !self.live.is_live() || self.successor.is_none() {
                return Err(VoiceError::Unauthorized);
            }
            if state.packet_claims.is_some() {
                return Ok(());
            }
            let claims = state
                .packet_parents
                .as_ref()
                .ok_or(VoiceError::Unauthorized)?
                .subreserve_sdp()?;
            state.packet_claims = Some(claims);
            state
                .packet_parents
                .take()
                .expect("same original packet parents")
        };
        // Only surplus positions return. Both children still own their exact original
        // backing, so no ordinary caller can steal the promised SDP claims in between.
        parents.close();
        Ok(())
    }
    pub(in crate::voice_v2) fn incoming_sdp_slot(
        self: &Arc<Self>,
        header: &EncryptedHeader,
        slots: &PacketSlots,
    ) -> Result<PacketSlot, VoiceError> {
        self.sdp_packet_slot(header, slots, true)
    }
    pub(in crate::voice_v2) fn outgoing_sdp_slot(
        self: &Arc<Self>,
        header: &EncryptedHeader,
        slots: &PacketSlots,
    ) -> Result<PacketSlot, VoiceError> {
        self.sdp_packet_slot(header, slots, false)
    }
    fn sdp_packet_slot(
        self: &Arc<Self>,
        header: &EncryptedHeader,
        slots: &PacketSlots,
        incoming: bool,
    ) -> Result<PacketSlot, VoiceError> {
        let proof = self.successor.as_ref().ok_or(VoiceError::Scope)?;
        let binding = proof.binding();
        let offer = if incoming { !binding.low } else { binding.low };
        let round = self.sdp_round().ok_or(VoiceError::Unauthorized)?;
        let expected = if offer {
            EncryptedKind::OfferRound(round)
        } else {
            EncryptedKind::AnswerRound(round)
        };
        let (source_join, source_key, target, target_join, target_key) = if incoming {
            (
                binding.peer_join,
                binding.peer_key,
                binding.local,
                binding.local_join,
                binding.local_key,
            )
        } else {
            (
                binding.local_join,
                binding.local_key,
                binding.peer,
                binding.peer_join,
                binding.peer_key,
            )
        };
        if !self.is_live()
            || header.kind != expected
            || header.handshake != binding.handshake
            || header.source_join != source_join
            || header.source_key_ref != source_key
            || header.target != target
            || header.target_join != target_join
            || header.target_key_ref != target_key
            || header.low_to_high != if incoming { !binding.low } else { binding.low }
            || header.offer.is_none()
        {
            return Err(VoiceError::Unauthorized);
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !self.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let claims = state.packet_claims.as_ref().ok_or(VoiceError::Capacity)?;
        let slot = if incoming {
            claims.incoming_slot(slots)?
        } else {
            claims.outgoing_slot(slots)?
        };
        Ok(PacketSlot::successor(slot, self.clone()))
    }
}

#[cfg(test)]
mod tests;
