//! One borrowed publication tail on the actual Actor. Prepared packets, inactive actions,
//! reservations and affine issue owners stay in their enclosing factory. They are never
//! moved into this callback, so a native refusal cannot destroy them beneath its guards.
use super::{publication::Outcome, VoiceError};
use std::sync::Arc;

/// A single synchronous opportunity to publish an already prepared original SDP Action.
/// Every injected Clock callback and fallible preparation precedes this loan. A trusted
/// native publisher may enter its UI/Coordinator/route/row fences, then consume commit;
/// commit only tries the original Group/Policy and publishes the existing prepared tail.
/// It may not await, call Clock, reenter Root, or retain this lifetime-bound value.
///
/// The native publisher must retain its actual admitted UI/physical/payment custody after
/// a successful commit. This value itself grants neither a reusable permit nor RTC use.
pub struct SdpPublicationAdmission<'a> {
    commit: &'a mut dyn FnMut() -> Result<(), VoiceError>,
    outcome: Option<&'a Arc<Outcome>>,
    wall: u64,
    mono: u64,
}
impl<'a> SdpPublicationAdmission<'a> {
    pub(crate) fn new(
        wall: u64,
        mono: u64,
        outcome: Option<&'a Arc<Outcome>>,
        commit: &'a mut dyn FnMut() -> Result<(), VoiceError>,
    ) -> Self {
        Self {
            commit,
            outcome,
            wall,
            mono,
        }
    }
    /// Pure identity of the actual entered recorded Writer. Native must match its own
    /// original row Outcome before final admission; equal labels/intent are insufficient.
    /// This does not relax Writer::belongs_to, whose separate PreRoot contract remains.
    pub fn belongs_to_outcome(&self, original: &Arc<Outcome>) -> bool {
        self.outcome
            .is_some_and(|outcome| Arc::ptr_eq(outcome, original))
    }
    /// The original final Actor samples, captured before native fences. They are not a
    /// transferable deadline, independent current authority, or a request to read Clock.
    pub fn observed_time(&self) -> (u64, u64) {
        (self.wall, self.mono)
    }
    /// Consume this one tail. All owned preparation remains outside the native callback
    /// on failure/unwind. Payload allocation and fallible preparation are complete. The
    /// existing publication tail may insert its prepaid fixed map node; it invokes no
    /// Clock, SDK, token-result producer or RTC call.
    pub fn commit(self) -> Result<(), VoiceError> {
        (self.commit)()
    }
}
impl std::fmt::Debug for SdpPublicationAdmission<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SdpPublicationAdmission { .. }")
    }
}
