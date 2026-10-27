//! Actual-Actor current engine admission, distinct from input/terminal/RTC authority.
use super::*;
use crate::voice_v2::crypto_actions::{ObservedSdpEngine, SdpEngineRead};

/// One lifetime-bound final engine opportunity from the actual ChannelSync call.
/// Native must first hold its original UI/Session/SDK/PC/row publication fences.
/// Capacity and original Clock callbacks have already finished. No part may await,
/// reenter Root or escape the callback; an immutable intent alone is insufficient.
pub struct SdpEngineAdmission<'a> {
    observed: ObservedSdpEngine<'a>,
    current: Result<InputCurrent<'a>, VoiceError>,
}
redacted_debug!(SdpEngineAdmission<'_>);
impl<'a> SdpEngineAdmission<'a> {
    /// Pure original allocation identity for native's final indexed row check.
    /// A same-intent Action or equal serialized serial/tag is not a substitute.
    pub fn belongs_to_action(
        &self,
        action: &Arc<crate::voice_v2::crypto_actions::CryptoAction>,
    ) -> bool {
        self.observed.belongs_to(action)
    }
    pub(crate) fn new(
        observed: ObservedSdpEngine<'a>,
        current: Result<InputCurrent<'a>, VoiceError>,
    ) -> Self {
        Self { observed, current }
    }
    /// Terminal/settled history wins over stale current context. Otherwise retain
    /// Engine then try Group/Policy until the original prepaid permit is installed.
    /// This does not execute SDK/WebCrypto, consume a token, sign or authorize RTC.
    pub fn commit(self) -> Result<SdpEngineRead, VoiceError> {
        self.observed
            .commit(|apply| self.current?.with_current(apply))
    }
}

/// Complete fixed Root call-frame inventory, additional to the actual App envelope
/// and independently paid native publisher/output. Candidate backing has its own
/// original Exchange credit; its retained wrapper is included here, not charged twice.
pub const fn sdp_engine_admission_bytes() -> usize {
    std::mem::size_of::<SdpEngineAdmission<'static>>()
        + std::mem::size_of::<crate::voice_v2::crypto_actions::PreparedSdpEngine<'static>>()
        + std::mem::size_of::<InputCurrent<'static>>()
        + std::mem::size_of::<Result<crate::voice_v2::Context, VoiceError>>()
        + std::mem::size_of::<Result<crate::voice_v2::helper::RoomAuthority, VoiceError>>()
        + std::mem::size_of::<crate::voice_v2::resources::ByteBudget>()
        + std::mem::size_of::<Result<Option<crate::voice_v2::resources::Credit>, VoiceError>>()
        + std::mem::size_of::<SdpEngineRead>()
        + 5 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 256 // captured samples, selectors, callback and try-lock bookkeeping
}
