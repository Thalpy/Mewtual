//! One fixed original Open result and its separate, irreversible RTC admission. Plaintext
//! stays in the browser's already paid inbox; Root never retains or returns another copy.
//! Neither this record nor a historical receipt owns a live action, Exchange or Phase.
use super::action::ActionBinding;
use super::*;
use crate::voice_v2::crypto_actions::{CryptoAction, CryptoResultReceipt, RecordFunding};
use crate::voice_v2::negotiation::admission::SdpAdmission;
use crate::voice_v2::packets::InboundPacket;
use std::sync::atomic::{AtomicBool, Ordering};

/// Closed result from the original private SDK computation. A failed AEAD operation has
/// no plaintext or invented padding. The native adapter must authenticate the actual row,
/// original SDK/phase/action and output purpose before calling Root with either variant.
#[derive(Clone, Copy)]
pub enum SdpOpenOutput<'a> {
    Plaintext {
        bytes: &'a [u8],
        claimed_tag: [u8; 32],
    },
    AuthenticationFailed,
}

/// Recoverable native metadata only. A successful Open creates its delivery observation in
/// the SAME transaction as token consumption; losing the reply cannot require another Open.
pub struct SdpOpenResult {
    pub receipt: CryptoResultReceipt,
    pub delivery: Option<Arc<SdpDelivery>>,
}

/// Original accepted Open/Offer/round/PC binding. The browser must latch SDK-started before
/// invoking its exact original PC; repeated admission replies name that one logical effect,
/// and must never repeat setRemoteDescription. Closing the PC does not erase held strings:
/// the independent SDK custodian remains paid until all actual references/continuations drop.
pub struct SdpDelivery {
    binding: Arc<ActionBinding>,
    admission: Arc<SdpAdmission>,
    output_tag: [u8; 32],
    admitted: AtomicBool,
    // LAST: this plain fixed action fee survives Action/LocalJoin destruction. In particular
    // it must not be a Stage::Result/PhaseLoan credit, which would retain a live Exchange.
    _funding: Arc<RecordFunding>,
}
redacted_debug!(SdpDelivery, SdpOpenResult, SdpOpenOutput<'_>);

impl SdpDelivery {
    pub(in crate::voice_v2) fn prepare(
        action: &CryptoAction,
        admission: Arc<SdpAdmission>,
        packet: &InboundPacket,
        output_tag: [u8; 32],
    ) -> Result<Arc<Self>, VoiceError> {
        let binding = action.sdp_binding().ok_or(VoiceError::Unauthorized)?;
        if binding.is_seal()
            || !binding.matches_packet(packet)
            || admission.token != packet.token
            || admission.hash != packet.object_hash()
            || admission.issuer != packet.issuer
            || packet.header.offer != Some(admission.offer)
            || packet.header.kind.round() != Some(admission.round)
            || binding.offer() != admission.offer
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(Arc::new(Self {
            binding: binding.clone(),
            admission,
            output_tag,
            admitted: AtomicBool::new(false),
            _funding: action.history_funding(),
        }))
    }
    pub(in crate::voice_v2) fn same_admission(&self, admission: &Arc<SdpAdmission>) -> bool {
        Arc::ptr_eq(&self.admission, admission)
    }
    pub(in crate::voice_v2) fn current_at(&self, wall: u64, mono: u64) -> bool {
        self.binding.current_at(wall, mono) && self.admission.authorized_at(wall, mono)
    }
    /// Call only in the actual original actor's callback-free origin publish tail. Retrying
    /// the same record never creates another identity, refreshes time, or resets its latch.
    pub(in crate::voice_v2) fn admit(&self) {
        self.admitted.store(true, Ordering::Release);
    }
    /// Historical status only; true cannot substitute for current native delivery admission.
    pub fn was_admitted(&self) -> bool {
        self.admitted.load(Ordering::Acquire)
    }
    pub fn output_tag(&self) -> [u8; 32] {
        self.output_tag
    }
    pub fn offer(&self) -> OfferId {
        self.binding.offer()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixed_sdp_action_terminal_and_delivery_share_the_original_history_fee() {
        // Count both live Action and independently retained delivery together. Neither
        // admission/PC observer's separately paid allocation is charged again here.
        let bytes = crate::voice_v2::crypto_actions::fixed_record_layout_bytes()
            + std::mem::size_of::<super::SdpDelivery>()
            + 2 * std::mem::size_of::<usize>();
        assert!(bytes <= 1536, "complete SDP fixed history graph: {bytes}");
    }
}
