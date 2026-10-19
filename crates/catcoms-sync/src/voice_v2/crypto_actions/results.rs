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

impl RecordedResult {
    fn receipt(&self, action: &CryptoAction) -> CryptoResultReceipt {
        let binding = &self.result.binding;
        let (monotonic, wall) = action.authority.history_time();
        CryptoResultReceipt {
            action: action.serial,
            input_tag: action.input_tag,
            output_tag: self.output_tag,
            outcome: self.result.outcome,
            retired: !action.call.is_live()
                || !binding.pair.is_live()
                || binding
                    .attempt
                    .as_ref()
                    .is_some_and(|attempt| !attempt.is_live())
                || monotonic >= binding.deadline
                || wall > binding.expires,
        }
    }
}

impl CryptoAction {
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
            self.take_acknowledged_payload(&engine)
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
        let engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        CryptoResultStatus {
            settled: engine.settled,
            receipt: engine
                .terminal
                .as_ref()
                .map(|recorded| recorded.receipt(self)),
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
        if bytes.len() != self.output_bytes || bytes.len() > CONFIRM_BYTES + 16 {
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
