//! Read-only original binding and prepaid copy custody for a trusted native adapter.
//! Neither method registers a renderer identity or enables normal V2 dispatch.
use super::*;

/// Immutable native-selected chain metadata. Equality of this projection never establishes
/// authority: the bridge must retain and resolve the original CallGrant/CryptoAction again.
/// Any serialized or otherwise owned copy needs its own prepaid handoff allowance.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProtectedCryptoBinding {
    pub scope: Scope,
    pub epoch: u64,
    pub local: super::super::DeviceId,
    pub local_join: Nonce,
    pub local_key: Nonce,
    pub peer: super::super::DeviceId,
    pub peer_join: Nonce,
    pub peer_key: Nonce,
    pub handshake: Nonce,
    pub generation: u64,
    pub deadline: u64,
    pub local_is_low: bool,
}
redacted_debug!(ProtectedCryptoBinding);

impl CryptoAction {
    /// Historical metadata only, still readable after input compaction/End. The shared fixed
    /// PlanBinding has no progress reserve, input or engine ownership and cannot renew a join.
    /// Compatibility/SDP actions without a protected confirmation chain return Unauthorized.
    pub fn protected_binding(&self) -> Result<ProtectedCryptoBinding, VoiceError> {
        let binding = self.binding.as_ref().ok_or(VoiceError::Unauthorized)?;
        Ok(ProtectedCryptoBinding {
            scope: binding.scope,
            epoch: binding.epoch,
            local: binding.local,
            local_join: binding.local_join,
            local_key: binding.local_key,
            peer: binding.peer,
            peer_join: binding.peer_join,
            peer_key: binding.peer_key,
            handshake: binding.handshake,
            generation: binding.generation,
            deadline: binding.deadline,
            local_is_low: binding.low,
        })
    }

    /// Borrow a single chain-wide copy/work position before the trusted adapter exposes this
    /// action. Caller supplies its measured peak, bounded by the existing confirmation reserve;
    /// bytes are not an untrusted IPC argument. Capacity is retryable without spending identity.
    ///
    /// Retain the original loan across uncertain pull/result/settlement replies. A terminal or
    /// settled action cannot borrow again; fixed historical retries must use the original paid
    /// delivery owner or a separately prepaid fixed terminal-envelope allowance. Returning this
    /// loan asserts observable drainage only, not physical SDK/GC deallocation or key zeroization.
    pub fn borrow_handoff(&self, bytes: usize) -> Result<HandoffLoan, VoiceError> {
        let (mono, wall) = self.pending_observation();
        let engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        if engine.settled || engine.terminal.is_some() {
            return Err(VoiceError::Unauthorized);
        }
        self.check_pending_at(mono, wall)?;
        let Some(ByteBudget::Workflow(workflow)) = engine.lender.as_ref() else {
            return Err(VoiceError::Unauthorized);
        };
        // No Clock or effect callback runs under Engine/Workflow. This consumes Retained
        // bytes only; the original protected Crypto lane remains available for this action.
        workflow.borrow_handoff(bytes)
    }
}
