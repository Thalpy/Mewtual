//! Final input publication serializes with the original result/settlement fence. An
//! immutable input loan alone is not authority: only the actual Root actor may supply
//! the current group/room/participant check consumed by this scoped observation.
use super::*;

/// Historical metadata or one affine publication of the original input. Neither variant
/// admits WebCrypto, consumes an inbound token, signs a packet or authorizes RTC.
pub enum SdpInputRead {
    History(CryptoHandoffStatus),
    Data(SdpAdmittedInput),
}

/// One admitted read, not a reusable current-authority snapshot. Native must retain this
/// actual loan, its separately paid response and UI/physical custody through collection.
/// A subsequent End/epoch change does not erase these already admitted bytes; another
/// read and every engine/result/delivery effect require their own current admission.
pub struct SdpAdmittedInput {
    input: CryptoInputLease,
    // Last: the original action record remains paid through input destruction. There is
    // no action -> admitted-input edge and this owner cannot replace the action's input.
    action: Arc<CryptoAction>,
}
impl SdpAdmittedInput {
    pub fn input(&self) -> CryptoInput<'_> {
        self.input.as_input()
    }
    /// Allocation identity only; a serial/tag comparison does not establish provenance.
    pub fn belongs_to(&self, action: &Arc<CryptoAction>) -> bool {
        Arc::ptr_eq(&self.action, action)
    }
}

pub(crate) struct ObservedSdpInput<'a> {
    action: &'a Arc<CryptoAction>,
    mono: u64,
    wall: u64,
}
redacted_debug!(SdpInputRead, SdpAdmittedInput, ObservedSdpInput<'_>);

impl CryptoAction {
    #[cfg(test)]
    pub(crate) fn with_input_fence_held_for_test<T>(&self, check: impl FnOnce() -> T) -> T {
        let _payload = self.input.lock().unwrap();
        check()
    }
    pub(crate) fn observe_sdp_input<'a>(
        self: &'a Arc<Self>,
        intent: &Arc<super::super::sdp::InitialSdpIntent>,
    ) -> Result<ObservedSdpInput<'a>, VoiceError> {
        if !self.uses_sdp_intent(intent)
            || !matches!(
                self.operation,
                CryptoOperation::SealSdp | CryptoOperation::OpenSdp
            )
        {
            return Err(VoiceError::Unauthorized);
        }
        // Original injected Clock callbacks precede native UI, Engine and Group/Policy.
        let (mono, wall) = self.authority.history_time();
        Ok(ObservedSdpInput {
            action: self,
            mono,
            wall,
        })
    }
}
impl ObservedSdpInput<'_> {
    fn history(&self, engine: &Engine) -> Option<SdpInputRead> {
        if engine.terminal.is_none() && !engine.settled {
            return None;
        }
        Some(SdpInputRead::History(CryptoHandoffStatus {
            engine: CryptoResultStatus {
                settled: engine.settled && !engine.preparing,
                receipt: engine
                    .terminal
                    .as_ref()
                    .map(|recorded| recorded.receipt_at(self.action, self.mono, self.wall)),
            },
            acknowledged: engine.acknowledged,
        }))
    }
    /// Avoid preparing current state or lending input when terminal history is already
    /// known. The final commit repeats this lookup, including a result racing preparation.
    pub(crate) fn known_history(&self) -> Result<Option<SdpInputRead>, VoiceError> {
        let engine = try_engine(self.action)?;
        Ok(self.history(&engine))
    }
    /// Called only inside the actual Root's scoped native callback. `current` may enter
    /// try-only Group/Policy guards; it must not await, sample a Clock or enter Engine.
    pub(in crate::voice_v2) fn commit(
        self,
        current: impl FnOnce() -> Result<(), VoiceError>,
    ) -> Result<SdpInputRead, VoiceError> {
        let engine = try_engine(self.action)?;
        if let Some(history) = self.history(&engine) {
            return Ok(history);
        }
        self.action.check_pending_at(self.mono, self.wall)?;
        let payload = match self.action.input.try_lock() {
            Ok(payload) => payload,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        let original = payload.as_ref().ok_or(VoiceError::Unauthorized)?;
        // No new loan exists before ALL fallible gates pass. Dropping a refused loan
        // after Engine unlock would still be inside native's outer admission guards;
        // concurrent settlement could make it the last payload/Credit destructor there.
        // Retain Engine -> input -> try Group/Policy, then clone only in the success tail.
        current()?;
        Ok(SdpInputRead::Data(SdpAdmittedInput {
            input: CryptoInputLease(original.clone()),
            action: self.action.clone(),
        }))
    }
}
fn try_engine(action: &CryptoAction) -> Result<std::sync::MutexGuard<'_, Engine>, VoiceError> {
    match action.engine.try_lock() {
        Ok(engine) => Ok(engine),
        Err(std::sync::TryLockError::Poisoned(error)) => Ok(error.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => Err(VoiceError::Capacity),
    }
}
