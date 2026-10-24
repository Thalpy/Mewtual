//! LOW's original outgoing Offer is the only producer allowed to borrow an inert lane.
//! Generic packet/action lenders remain Active-only. These affine preparations own the
//! original bytes and table positions through every refusal, without publishing a nonce,
//! changing a role, or activating the exchange. Final native publication does all three.
use super::*;
use crate::voice_v2::crypto_actions::slots::{Slot, Slots};
use crate::voice_v2::packets::{PacketSlot, PacketSlots};
use crate::voice_v2::sdp::InitialSdpIntent;

// The private Answer intake prepays its complete two-copy packet graph before cloning.
pub(in crate::voice_v2) fn remote_packet_bytes() -> usize {
    PACKET_BYTES
}

pub(in crate::voice_v2) struct InitialOfferAction {
    pub history: Credit,
    pub payload: Credit,
    pub issue: ActionIssue,
    pub slot: Slot,
    pub engine_bytes: usize,
}
pub(in crate::voice_v2) struct InitialOfferPacket {
    pub credit: Credit,
    pub slot: PacketSlot,
    pub intent: Arc<InitialSdpIntent>,
    pub exchange: Arc<Exchange>,
}
impl Exchange {
    /// Pure identity/currency check usable again after final native callbacks. It grants
    /// no general inactive-phase access and cannot be satisfied with copied Ready labels.
    pub(in crate::voice_v2) fn initial_offer_current_at(
        &self,
        intent: &InitialSdpIntent,
        wall: u64,
        mono: u64,
    ) -> bool {
        !self.activated.load(Ordering::Acquire)
            && self.live.is_live()
            && intent.local_is_low()
            && intent.current_at(wall, mono)
            && self.successor.as_ref().is_some_and(|proof| {
                proof.binding().low && proof.drained() && intent.matches_successor(proof)
            })
    }
    /// Prepay all variable first-Offer owners from the SAME original lane. No fallible
    /// charge follows attachment of a PhaseLoan while State is held: dropping that loan
    /// re-enters State. Fixed history deliberately has no Exchange/PhaseLoan backedge.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::voice_v2) fn prepare_initial_offer(
        self: &Arc<Self>,
        intent: &Arc<InitialSdpIntent>,
        slots: &Arc<Slots>,
        tables: &PacketSlots,
        plaintext_bytes: usize,
        wall: u64,
        mono: u64,
    ) -> Result<(InitialOfferAction, InitialOfferPacket), VoiceError> {
        let ciphertext = plaintext_bytes.checked_add(16).ok_or(VoiceError::Bounds)?;
        let (payload_bytes, engine_bytes) =
            super::super::crypto_actions::sdp_bytes(MAX_AAD, ciphertext)?;
        let (_, _, action_max, _) = Stage::EncryptAction.contract();
        if payload_bytes + 1536 > action_max {
            return Err(VoiceError::Bounds);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !self.initial_offer_current_at(intent, wall, mono) {
            return Err(VoiceError::Unauthorized);
        }
        if state.used[0] != 0
            || state.used[4] != 0
            || state.action_preparing[0]
            || self.action_issued.load(Ordering::Acquire) & 1 != 0
        {
            return Err(VoiceError::Capacity);
        }
        let budget = ByteBudget::Reserved(self.reserve.clone());
        let history = budget.charge(&self.resources, self.group, 1536, ResourceKind::Retained)?;
        let payload = budget.charge(
            &self.resources,
            self.group,
            payload_bytes,
            ResourceKind::Retained,
        )?;
        let packet = budget.charge(
            &self.resources,
            self.group,
            PACKET_BYTES,
            ResourceKind::Retained,
        )?;
        let action_slot = self.action_slots.loan(slots)?;
        let packet_slot = state
            .packet_claims
            .as_ref()
            .ok_or(VoiceError::Capacity)?
            .outgoing_slot(tables)?;
        state.used[0] = 1;
        state.used[4] = 1;
        state.action_preparing[0] = true;
        drop(state);
        Ok((
            InitialOfferAction {
                history,
                payload: payload.with_custody(Arc::new(PhaseLoan {
                    exchange: self.clone(),
                    index: 4,
                })),
                issue: ActionIssue {
                    exchange: self.clone(),
                    direction: 0,
                    unpublished: true,
                    binding_issued: false,
                },
                slot: action_slot,
                engine_bytes,
            },
            InitialOfferPacket {
                credit: packet.with_custody(Arc::new(PhaseLoan {
                    exchange: self.clone(),
                    index: 0,
                })),
                slot: PacketSlot::successor(packet_slot, self.clone()),
                intent: intent.clone(),
                exchange: self.clone(),
            },
        ))
    }
}
