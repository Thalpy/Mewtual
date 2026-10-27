//! Original SDP engine admission. Capacity preparation and Clock callbacks precede
//! native publication; the final actual-Actor check runs under Engine and Group/Policy.
//! Engine credit belongs to the original Root Exchange, not the native IPC Phase.
use super::*;

/// Metadata-only canonical history or the SAME original action's admitted engine serial.
/// A permit is not a signed packet, result, plaintext-delivery grant or RTC permission.
pub enum SdpEngineRead {
    History(CryptoHandoffStatus),
    Permit(u64),
}

/// Original candidate is prepared before the Actor captures its final origin time.
/// This outer owner never crosses the native callback, so credit Drop stays outside.
pub(crate) struct PreparedSdpEngine<'a> {
    action: &'a Arc<CryptoAction>,
    candidate: Result<Option<Credit>, VoiceError>,
}

pub(crate) struct ObservedSdpEngine<'a> {
    action: &'a Arc<CryptoAction>,
    mono: u64,
    wall: u64,
    // Borrow only: unused/failed credit preparation is destroyed by the outer factory
    // AFTER the native hook and all its final guards return, including unwind.
    candidate: &'a mut Result<Option<Credit>, VoiceError>,
}
redacted_debug!(SdpEngineRead, ObservedSdpEngine<'_>);

impl CryptoAction {
    pub(crate) fn prepare_sdp_engine_candidate(
        self: &Arc<Self>,
        intent: &Arc<super::super::sdp::InitialSdpIntent>,
        bytes: usize,
    ) -> Result<PreparedSdpEngine<'_>, VoiceError> {
        if !self.uses_sdp_intent(intent)
            || !matches!(
                self.operation,
                CryptoOperation::SealSdp | CryptoOperation::OpenSdp
            )
        {
            return Err(VoiceError::Unauthorized);
        }
        if bytes != self.engine_bytes {
            return Err(VoiceError::RetryMismatch);
        }
        // A terminal result/settlement may race either this capacity preparation or
        // the actual Actor's current-context preparation. Preserve errors until final
        // Engine lookup; they cannot hide canonical historical metadata.
        Ok(PreparedSdpEngine {
            action: self,
            candidate: self.prepare_engine_candidate(bytes),
        })
    }
}

impl PreparedSdpEngine<'_> {
    /// Sample the SAME action last, after current Actor origin preparation. Lending
    /// this borrowed tail cannot move or destroy the outer prepared credit in native.
    pub(crate) fn with_observation<T>(
        mut self,
        publish: impl FnOnce(ObservedSdpEngine<'_>) -> Result<T, VoiceError>,
    ) -> Result<T, VoiceError> {
        let (mono, wall) = self.action.authority.history_time();
        let result = publish(ObservedSdpEngine {
            action: self.action,
            mono,
            wall,
            candidate: &mut self.candidate,
        });
        drop(self);
        result
    }
}

impl ObservedSdpEngine<'_> {
    pub(in crate::voice_v2) fn belongs_to(&self, action: &Arc<CryptoAction>) -> bool {
        Arc::ptr_eq(self.action, action)
    }
    /// Only the actual Root actor supplies `current`. It must invoke its infallible
    /// apply tail under current Group/Policy, without awaiting, Clock or Engine reentry.
    /// All candidate capacity remains in the outer factory on refusal and unwind.
    pub(in crate::voice_v2) fn commit(
        self,
        current: impl FnOnce(&mut dyn FnMut()) -> Result<(), VoiceError>,
    ) -> Result<SdpEngineRead, VoiceError> {
        let mut engine = match self.action.engine.try_lock() {
            Ok(engine) => engine,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        if engine.terminal.is_some() || engine.settled {
            return Ok(SdpEngineRead::History(CryptoHandoffStatus {
                engine: CryptoResultStatus {
                    settled: engine.settled && !engine.preparing,
                    receipt: engine
                        .terminal
                        .as_ref()
                        .map(|recorded| recorded.receipt_at(self.action, self.mono, self.wall)),
                },
                acknowledged: engine.acknowledged,
            }));
        }
        self.action.check_pending_at(self.mono, self.wall)?;
        if engine.permit.is_none() {
            // Do not take a credit before the actual current fence succeeds: a local
            // failure would otherwise destroy that last credit under native guards.
            match self.candidate.as_ref() {
                Ok(Some(_)) => (),
                Ok(None) => return Err(VoiceError::Capacity),
                Err(error) => return Err(*error),
            }
        }
        let mut applied = false;
        current(&mut || {
            if engine.permit.is_none() {
                engine.permit = self
                    .candidate
                    .as_mut()
                    .expect("prepared original credit")
                    .take();
            }
            applied = true;
        })?;
        // The trusted current-origin closure must consume its apply tail. Callback
        // success alone is not an engine permit and cannot be serialized as one.
        if !applied {
            return Err(VoiceError::Unauthorized);
        }
        Ok(SdpEngineRead::Permit(self.action.serial))
    }
}
