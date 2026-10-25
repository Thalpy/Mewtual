//! Small historical results, prepaid with the native action. A receipt describes one past
//! transition; it is never a permit to sign, deliver plaintext, send a packet or change RTC.
use super::*;
use sha2::{Digest, Sha256};

/// Closed native outcome. Signed bytes stay in the original packet ledger rather than a second
/// result cache. Derivation reports readiness only; no key material crosses this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoOutcome {
    Derived,
    Sealed { message: Nonce, hash: [u8; 32] },
    Opened,
    Rejected(VoiceError),
}

/// Metadata only. Even an unretired accepted receipt supplies no fresh protocol/RTC authority.
/// Native IPC must authorize its registered action owner separately before exposing this record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoResultReceipt {
    pub action: u64,
    pub input_tag: [u8; 32],
    pub output_tag: [u8; 32],
    pub outcome: CryptoOutcome,
    pub retired: bool,
}

impl CryptoResultReceipt {
    /// Receipt acknowledgement names the immutable past transition. Retirement is a present-time
    /// observation and may change between status delivery and acknowledgement; it is never part
    /// of the receipt's identity and cannot turn acknowledgement into fresh effect authority.
    pub fn same_transition(&self, other: &Self) -> bool {
        self.action == other.action
            && self.input_tag == other.input_tag
            && self.output_tag == other.output_tag
            && self.outcome == other.outcome
    }
}

/// One action-fence snapshot for native history retention. This contains no input or plaintext.
/// `settled` reports actual engine/RPC settlement, independently of receipt acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoResultStatus {
    pub settled: bool,
    pub receipt: Option<CryptoResultReceipt>,
}

/// Atomic native-custody observation of the original action fence. A receipt or retired
/// authority is not an acknowledgement: only successful `acknowledge_result` sets this bit.
/// This projection grants no fresh work and does not prove browser/SDK/physical drainage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoHandoffStatus {
    pub engine: CryptoResultStatus,
    pub acknowledged: bool,
}

pub(super) struct RecordedResult {
    output_tag: [u8; 32],
    result: TerminalResult,
}

/// Captured from actual POST-transition owners, especially Hello's newly installed attempt.
/// Original expiry/deadlines are immutable; neither cached lookup nor engine settlement renews
/// them. Pending-token/engine authority deliberately does not appear in this type.
pub(crate) struct ResultBinding {
    pub pair: CallLease,
    pub attempt: Option<CallLease>,
    pub deadline: u64,
    pub expires: u64,
}
pub(crate) struct TerminalResult {
    pub binding: ResultBinding,
    pub outcome: CryptoOutcome,
}

/// Exact bounded output and ORIGINAL clock observations, captured before Engine. New
/// SDP transactions use this instead of invoking callbacks while holding their result
/// fence. It grants no first effect until actual current authority is checked again.
pub(in crate::voice_v2) struct ObservedSdpResult<'a> {
    action: &'a CryptoAction,
    output_tag: [u8; 32],
    mono: u64,
    wall: u64,
}
impl ObservedSdpResult<'_> {
    pub(in crate::voice_v2) fn recover(&self) -> Result<Option<CryptoResultReceipt>, VoiceError> {
        let engine = self.try_engine()?;
        match &engine.terminal {
            Some(recorded) if recorded.output_tag == self.output_tag => {
                Ok(Some(recorded.receipt_at(self.action, self.mono, self.wall)))
            }
            Some(_) => Err(VoiceError::RetryMismatch),
            None => Ok(None),
        }
    }
    fn try_engine(&self) -> Result<std::sync::MutexGuard<'_, Engine>, VoiceError> {
        match self.action.engine.try_lock() {
            Ok(engine) => Ok(engine),
            Err(std::sync::TryLockError::Poisoned(error)) => Ok(error.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => Err(VoiceError::Capacity),
        }
    }
    /// Terminal lookup precedes pending token/settlement checks. The callback must only
    /// consume an already prepared native origin and perform a pure infallible apply tail.
    /// Every actual input/Phase/slot/result destructor stays in the caller's outer scope.
    pub(in crate::voice_v2) fn commit(
        self,
        apply: impl FnOnce(u64, u64) -> Result<TerminalResult, VoiceError>,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        self.commit_with_delivery(|mono, wall| Ok((apply(mono, wall)?, None)))
    }
    /// Publish an already prepared fixed delivery identity with the terminal. No delivery
    /// allocation after Opened is allowed: a lost reply must never strand a consumed token.
    pub(in crate::voice_v2) fn commit_with_delivery(
        self,
        apply: impl FnOnce(
            u64,
            u64,
        ) -> Result<
            (TerminalResult, Option<Arc<super::super::sdp::SdpDelivery>>),
            VoiceError,
        >,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        let mut engine = self.try_engine()?;
        if let Some(recorded) = &engine.terminal {
            if recorded.output_tag != self.output_tag {
                return Err(VoiceError::RetryMismatch);
            }
            return Ok(recorded.receipt_at(self.action, self.mono, self.wall));
        }
        if engine.settled || engine.permit.is_none() {
            return Err(VoiceError::Unauthorized);
        }
        self.action.check_pending_at(self.mono, self.wall)?;
        let (result, delivery) = apply(self.mono, self.wall)?;
        debug_assert!(delivery.is_none() || result.outcome == CryptoOutcome::Opened);
        engine.sdp_delivery = delivery;
        engine.terminal = Some(RecordedResult {
            output_tag: self.output_tag,
            result,
        });
        Ok(engine
            .terminal
            .as_ref()
            .expect("one exact SDP terminal")
            .receipt_at(self.action, self.mono, self.wall))
    }
}

impl RecordedResult {
    fn receipt(&self, action: &CryptoAction) -> CryptoResultReceipt {
        let (monotonic, wall) = action.authority.history_time();
        self.receipt_at(action, monotonic, wall)
    }

    fn receipt_at(&self, action: &CryptoAction, monotonic: u64, wall: u64) -> CryptoResultReceipt {
        let binding = &self.result.binding;
        CryptoResultReceipt {
            action: action.serial,
            input_tag: action.input_tag,
            output_tag: self.output_tag,
            outcome: self.result.outcome,
            retired: !action.call.is_live()
                || action.chain.as_ref().is_some_and(|chain| !chain.is_live())
                || !binding.pair.is_live()
                || binding
                    .attempt
                    .as_ref()
                    .is_some_and(|attempt| !attempt.is_live())
                || action
                    .authority
                    .sdp_binding()
                    .is_some_and(|sdp| !sdp.current_at(wall, monotonic))
                || monotonic >= binding.deadline
                || wall > binding.expires,
        }
    }
}

impl CryptoAction {
    /// Closed AEAD failure has its own output identity. It is never represented by invented
    /// expected-length plaintext, and cannot collide with the tag of a successful byte result.
    pub(in crate::voice_v2) fn observe_sdp_open_output(
        &self,
        output: super::super::sdp::SdpOpenOutput<'_>,
    ) -> Result<ObservedSdpResult<'_>, VoiceError> {
        if self.operation != CryptoOperation::OpenSdp {
            return Err(VoiceError::Unauthorized);
        }
        match output {
            super::super::sdp::SdpOpenOutput::Plaintext { bytes, claimed_tag } => {
                self.observe_sdp_result(bytes, claimed_tag)
            }
            super::super::sdp::SdpOpenOutput::AuthenticationFailed => {
                let mut hash = Sha256::new();
                hash.update(b"mewtual/voice/native-sdp-aead-failure/v1\0");
                hash.update(self.input_tag);
                let (mono, wall) = self.authority.history_time();
                Ok(ObservedSdpResult {
                    action: self,
                    output_tag: hash.finalize().into(),
                    mono,
                    wall,
                })
            }
        }
    }
    /// Exact native recovery after input compaction, token consumption, End or a lost reply.
    /// This reports one historical result and the SAME fixed observer, never fresh RTC use.
    pub fn recover_sdp_open_result(
        &self,
        output: super::super::sdp::SdpOpenOutput<'_>,
    ) -> Result<Option<super::super::sdp::SdpOpenResult>, VoiceError> {
        let observed = self.observe_sdp_open_output(output)?;
        let engine = observed.try_engine()?;
        match &engine.terminal {
            Some(recorded) if recorded.output_tag == observed.output_tag => {
                Ok(Some(super::super::sdp::SdpOpenResult {
                    receipt: recorded.receipt_at(self, observed.mono, observed.wall),
                    delivery: engine.sdp_delivery.clone(),
                }))
            }
            Some(_) => Err(VoiceError::RetryMismatch),
            None => Ok(None),
        }
    }
    /// Recover the terminal result of this SAME original Open action without asking the
    /// browser to retain or reconstruct plaintext merely to identify a lost result. The
    /// native adapter must authenticate its registered action owner before exposing it.
    ///
    /// This returns fixed historical metadata and the original delivery observer, including
    /// after token consumption, ACK, settlement or End. It never creates another observer,
    /// renews a deadline, reads input or grants fresh delivery/RTC permission. A busy Engine
    /// is a temporary refusal; it is never evidence that no terminal result exists.
    pub fn recover_sdp_open_history(
        &self,
    ) -> Result<Option<super::super::sdp::SdpOpenResult>, VoiceError> {
        if self.operation != CryptoOperation::OpenSdp {
            return Err(VoiceError::Unauthorized);
        }
        // The original Clock is injected code. Observe it BEFORE Engine, including for
        // closed history; callbacks may inspect this same Action without lock inversion.
        let (mono, wall) = self.authority.history_time();
        let engine = match self.engine.try_lock() {
            Ok(engine) => engine,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        let Some(recorded) = &engine.terminal else {
            return Ok(None);
        };
        // Match the actual terminal invariant rather than inventing a missing delivery or
        // letting a non-Open result be represented by this typed recovery interface.
        match (recorded.result.outcome, &engine.sdp_delivery) {
            (CryptoOutcome::Opened, Some(_)) | (CryptoOutcome::Rejected(_), None) => {}
            _ => return Err(VoiceError::Unauthorized),
        }
        Ok(Some(super::super::sdp::SdpOpenResult {
            receipt: recorded.receipt_at(self, mono, wall),
            delivery: engine.sdp_delivery.clone(),
        }))
    }
    pub(in crate::voice_v2) fn owns_sdp_delivery(
        &self,
        delivery: &Arc<super::super::sdp::SdpDelivery>,
    ) -> Result<bool, VoiceError> {
        // Sample no Clock while Engine is held. Original current-owner/time checks belong
        // to the separate native delivery transaction, not this historical identity check.
        let engine = match self.engine.try_lock() {
            Ok(engine) => engine,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        Ok(engine
            .terminal
            .as_ref()
            .is_some_and(|r| r.result.outcome == CryptoOutcome::Opened)
            && engine
                .sdp_delivery
                .as_ref()
                .is_some_and(|same| Arc::ptr_eq(same, delivery)))
    }
    pub(in crate::voice_v2) fn observe_sdp_result(
        &self,
        bytes: &[u8],
        claimed_tag: [u8; 32],
    ) -> Result<ObservedSdpResult<'_>, VoiceError> {
        if !matches!(
            self.operation,
            CryptoOperation::SealSdp | CryptoOperation::OpenSdp
        ) {
            return Err(VoiceError::Unauthorized);
        }
        let output_tag = self.output_tag(bytes)?;
        if output_tag != claimed_tag {
            return Err(VoiceError::RetryMismatch);
        }
        let (mono, wall) = self.authority.history_time();
        Ok(ObservedSdpResult {
            action: self,
            output_tag,
            mono,
            wall,
        })
    }
    /// Acknowledge the immutable terminal result independently of actual engine settlement.
    /// Only when BOTH have occurred may native discard its input reference. Already borrowed
    /// input leases retain their own bytes/credit; separately held IPC/result copies require
    /// their separate retirement contract. This never deletes history or restores eligibility.
    pub fn acknowledge_result(&self, receipt: &CryptoResultReceipt) -> Result<(), VoiceError> {
        let compacted = {
            let mut engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
            let recorded = engine.terminal.as_ref().ok_or(VoiceError::Unauthorized)?;
            if receipt.action != self.serial
                || receipt.input_tag != self.input_tag
                || receipt.output_tag != recorded.output_tag
                || receipt.outcome != recorded.result.outcome
            {
                return Err(VoiceError::RetryMismatch);
            }
            engine.acknowledged = true;
            self.take_settled_payload(&engine)
        };
        // Do not run buffer/phase/credit destructors while an action or payload lock is held.
        drop(compacted);
        Ok(())
    }

    /// Inspect an already registered action's terminal metadata after the original browser has
    /// destroyed its plaintext, including after End/LocalJoin pruning. The native adapter must
    /// authorize the caller and exact registered identity independently. Unlike recover_result,
    /// this is NOT an exact-output retry: it proves nothing about caller-supplied output and
    /// cannot perform a first transition, admit work, sign, consume, deliver or grant RTC access.
    /// Reading settlement and history under one fence lets that adapter collect only genuinely
    /// drained, acknowledged history without racing a first terminal publication.
    pub fn terminal_status(&self) -> CryptoResultStatus {
        self.handoff_status().engine
    }

    /// Observe actual ACK and terminal/settlement under ONE Engine fence. Sample callback-
    /// bearing clocks before that fence, so a trusted native adapter can observe outside its
    /// own locks without a reentrant Clock acquiring an already-held Engine. Retirement is
    /// merely a timestamped observation; neither it nor this method restores pending authority.
    pub fn handoff_status(&self) -> CryptoHandoffStatus {
        let (monotonic, wall) = self.authority.history_time();
        let engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        CryptoHandoffStatus {
            engine: CryptoResultStatus {
                settled: engine.settled && !engine.preparing,
                receipt: engine
                    .terminal
                    .as_ref()
                    .map(|recorded| recorded.receipt_at(self, monotonic, wall)),
            },
            acknowledged: engine.acknowledged,
        }
    }

    /// Recover metadata for an exact past result without a surviving LocalJoin/Root. Native
    /// must first recognize this registered action and authorize the calling UI independently.
    /// This method NEVER performs a first transition, admits an engine, signs, consumes a token
    /// or grants delivery/RTC authority. A miss may only be handled by the separate CURRENT
    /// actor's serialized first-effect path. Settlement and End do not acknowledge lost replies.
    pub fn recover_result(
        &self,
        bytes: &[u8],
        claimed_tag: [u8; 32],
    ) -> Result<Option<CryptoResultReceipt>, VoiceError> {
        if matches!(
            self.operation,
            CryptoOperation::SealSdp | CryptoOperation::OpenSdp
        ) {
            // SDP callers may recover without a surviving Root. Preserve the same
            // callback-free Engine rule on this public historical seam too.
            return self.observe_sdp_result(bytes, claimed_tag)?.recover();
        }
        // Hash the actual bounded output even for historical retries: trusting the claimed
        // digest alone would allow changed bytes to masquerade as an accepted past transition.
        let output_tag = self.output_tag(bytes)?;
        if output_tag != claimed_tag {
            return Err(VoiceError::RetryMismatch);
        }
        let engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        match &engine.terminal {
            Some(recorded) if recorded.output_tag == output_tag => Ok(Some(recorded.receipt(self))),
            Some(_) => Err(VoiceError::RetryMismatch),
            None => Ok(None),
        }
    }

    /// SHA-256 of the bounded, native-input-selected output. Input identity remains an independent
    /// check; a caller's claimed digest cannot substitute for hashing the actual returned bytes.
    pub fn output_tag(&self, bytes: &[u8]) -> Result<[u8; 32], VoiceError> {
        // Select the bound from the actual native operation; larger SDP must never widen
        // confirmation acceptance or let an invented output length choose another contract.
        let maximum = match self.operation {
            CryptoOperation::Derive => 0,
            CryptoOperation::EncryptConfirmation => CONFIRM_BYTES + 16,
            CryptoOperation::DecryptConfirmation => CONFIRM_BYTES,
            CryptoOperation::SealSdp => super::super::MAX_SDP_PLAINTEXT + 16,
            CryptoOperation::OpenSdp => super::super::MAX_SDP_PLAINTEXT,
        };
        if bytes.len() != usize::from(self.output_bytes) || bytes.len() > maximum {
            return Err(VoiceError::Bounds);
        }
        Ok(Sha256::digest(bytes).into())
    }

    /// Native-only first transition. The closure must prepare ALL fallible work before any
    /// irreversible effect, then apply/publish infallibly. Capacity is retryable only because
    /// that ordering proves no effect occurred. Malformed closed results return a prepared
    /// Rejected outcome, not a post-consumption error. No await or action reentry is permitted.
    pub(crate) fn with_result(
        &self,
        bytes: &[u8],
        claimed_tag: [u8; 32],
        commit: impl FnOnce() -> Result<TerminalResult, VoiceError>,
    ) -> Result<CryptoResultReceipt, VoiceError> {
        let output_tag = self.output_tag(bytes)?;
        if output_tag != claimed_tag {
            return Err(VoiceError::RetryMismatch);
        }
        let mut engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        // Lookup precedes consumed token, pending grant and settled-engine checks. A terminal
        // identity survives End and actual settlement; changed output never replaces it.
        if let Some(recorded) = &engine.terminal {
            if recorded.output_tag != output_tag {
                return Err(VoiceError::RetryMismatch);
            }
            return Ok(recorded.receipt(self));
        }
        if engine.settled || engine.permit.is_none() || !self.authority.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        let result = commit()?;
        engine.terminal = Some(RecordedResult { output_tag, result });
        // Committed is not Settled. The actual engine/result RPC retains its original permit
        // until settle_engine; historical success cannot free or re-admit that work.
        Ok(engine
            .terminal
            .as_ref()
            .expect("installed terminal result")
            .receipt(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_history_fits_the_prepaid_closed_record_allowance() {
        assert!(std::mem::size_of::<RecordedResult>() <= CONFIRM_BYTES);
        assert!(std::mem::size_of::<CryptoResultReceipt>() <= CONFIRM_BYTES);
    }
}
