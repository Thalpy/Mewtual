//! LOW reserves the future Answer's Root namespace position BEFORE publishing its Offer.
//! Packet-table claims alone do not protect the separate shared256-receipt namespace.
//! Fixed receipt funding has no Exchange/PhaseLoan backedge; the real incoming bytes will.
use super::admission::ReceiptFunding;
use super::namespace::ReceiptPosition;
use super::*;
use crate::voice_v2::OfferId;

pub(in crate::voice_v2) struct AnswerPosition {
    pub position: ReceiptPosition,
    pub funding: Arc<ReceiptFunding>,
    pub offer: OfferId,
}
impl Exchange {
    /// A plain retained history fee from LOW's unused original Ready envelope. Never
    /// split PhaseLoan-backed input to pay history: that would retain live progress.
    pub(in crate::voice_v2) fn prepare_answer_funding(
        &self,
        intent: &super::super::sdp::InitialSdpIntent,
        wall: u64,
        mono: u64,
    ) -> Result<Arc<ReceiptFunding>, VoiceError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !self.initial_offer_current_at(intent, wall, mono) {
            return Err(VoiceError::Unauthorized);
        }
        let ready = self.ready.as_ref().ok_or(VoiceError::Unauthorized)?;
        let credit = ByteBudget::Reserved(ready.reserve.clone()).charge(
            &self.resources,
            self.group,
            super::super::ready::capacity::RECEIPT,
            ResourceKind::Retained,
        )?;
        drop(state);
        Ok(ReceiptFunding::new(credit))
    }
}
