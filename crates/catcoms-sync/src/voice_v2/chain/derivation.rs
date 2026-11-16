//! Row zero belongs to the original native-selected derivation. It cannot be reconstructed from
//! a cached Derived marker or reissued after input/history compaction in a replacement WebView.
use super::*;
use crate::voice_v2::runtime::LocalOrigin;

impl RowIssue {
    /// Borrow, rather than move, the unpublished issue into Group/Policy. Refusal leaves its
    /// destructor outside native guards because Drop reacquires History to abandon its ticket.
    pub(in crate::voice_v2) fn publish_derivation(
        &mut self,
        origin: LocalOrigin<'_>,
        mono: u64,
        publish: impl FnOnce(),
    ) -> Result<(), VoiceError> {
        let mut state = match self.plan.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        if self.row != 0
            || self.key != Key::Derive(self.plan.binding.handshake)
            || state.closed
            || state.active_row != 0
            || state.serial != self.ticket
            || state.issued & 1 != 0
            || !self.plan.live.is_live()
            || !self.plan.binding.live()
            || origin.epoch() != self.plan.binding.epoch
            || mono >= self.plan.binding.deadline
        {
            return Err(VoiceError::Unauthorized);
        }
        origin.publish(|| {
            state.issued |= 1;
            state.active_row = NO_ROW;
            self.unpublished = false;
            publish();
        })
    }
}
