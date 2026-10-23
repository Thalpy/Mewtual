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

/// Opaque original protected-plan identity for a native capacity custodian. Its fixed
/// PlanBinding allocation remains paid by the SAME Root metadata Credit. Unlike retaining
/// a Weak<CryptoAction>, this does not keep an entire dropped action allocation alive after
/// that action's record payment has gone. No Workflow, input, engine or current authority is
/// retained, and this owner cannot close or renew a chain.
pub struct ProtectedCryptoPlan {
    // Drop the independent lease/watch allocation BEFORE the possibly last PlanBinding
    // owner releases the SAME fixed metadata payment which covers this token's graph.
    chain: CallLease,
    binding: Arc<super::super::chain::PlanBinding>,
}
redacted_debug!(ProtectedCryptoPlan);

impl ProtectedCryptoPlan {
    /// Capacity provenance only. Native still has to recognize the exact Action allocation
    /// in its bounded original row before exposing input, admitting an engine or publishing.
    pub fn contains(&self, action: &CryptoAction) -> bool {
        action
            .binding
            .as_ref()
            .is_some_and(|binding| Arc::ptr_eq(binding, &self.binding))
            && action
                .chain
                .as_ref()
                .is_some_and(|chain| chain.is_same(&self.chain))
            && action.call.is_same(&self.binding.call)
            && Arc::ptr_eq(&action.resources, self.binding.resources())
    }
}

impl CryptoAction {
    /// Capture the exact native plan's cleanup authority before ordinary phase Return.
    /// This is a trusted Rust ownership operation, never a renderer-supplied identifier.
    /// Pending ownership is checked under the original Engine fence, with Clock callbacks
    /// sampled beforehand. The capability neither owns the Workflow nor permits an effect.
    pub fn confirmation_control(&self) -> Result<ConfirmationControl, VoiceError> {
        let (mono, wall) = self.pending_observation();
        let engine = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        if engine.settled || engine.terminal.is_some() {
            return Err(VoiceError::Unauthorized);
        }
        self.check_pending_at(mono, wall)?;
        let Some(ByteBudget::Workflow(workflow)) = engine.lender.as_ref() else {
            return Err(VoiceError::Unauthorized);
        };
        Ok(workflow.confirmation_control())
    }
    /// Preserve the actual independently paid fixed plan identity without an Action/Input/
    /// Workflow owner. Compatibility and SDP actions have no protected confirmation plan.
    pub fn protected_plan(&self) -> Result<ProtectedCryptoPlan, VoiceError> {
        Ok(ProtectedCryptoPlan {
            binding: self
                .binding
                .as_ref()
                .ok_or(VoiceError::Unauthorized)?
                .clone(),
            chain: self.chain.as_ref().ok_or(VoiceError::Unauthorized)?.clone(),
        })
    }

    /// Original role position published by the protected confirmation HistoryPlan. This is
    /// stable historical metadata, not first-effect authority or a native row lookup token.
    /// Native adapters must also check `same_protected_plan` and retain the actual Action Arc;
    /// equal indexes from separate plans must never share input, result or ACK ownership.
    pub fn protected_row(&self) -> Result<u8, VoiceError> {
        self.protected_row.ok_or(VoiceError::Unauthorized)
    }

    /// Compare original protected-plan allocations without observing time or acquiring an
    /// engine/Workflow lock. Historical compaction and End do not change this identity.
    /// Copied scope/key/handshake metadata cannot stand in for either allocation. This allows
    /// native custody to attach a genuine child to its original chain, never to authorize a
    /// new protocol effect or revive the chain after closure.
    pub fn same_protected_plan(&self, original: &CryptoAction) -> bool {
        matches!(
            (&self.binding, &original.binding, &self.chain, &original.chain),
            (Some(binding), Some(other_binding), Some(chain), Some(other_chain))
                if Arc::ptr_eq(binding, other_binding) && chain.is_same(other_chain)
                    && self.call.is_same(&original.call)
                    && Arc::ptr_eq(&self.resources, &original.resources)
        )
    }

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
