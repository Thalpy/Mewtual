//! Retention of an already confirmed browser key domain, independent of old progress.
//! This proof contains no key bytes, lane, packet/input owner, timer or effect permission.
use super::*;

/// Original Root-established key provenance for a trusted native custodian. Converting the
/// finite Finish custody does NOT renew it, authorize SDP, or keep confirmation work alive.
/// The native adapter must attach one exact original-session release obligation before
/// exposing a receipt, and must gate every later protocol/SDK effect independently.
pub struct EstablishedKeyCustody {
    // The existing fixed PlanBinding payment covers its own allocation through last Drop.
    // No strong or weak reference here retains an Action, Workflow, Exchange or input.
    binding: Arc<PlanBinding>,
}
redacted_debug!(EstablishedKeyCustody);

impl EstablishedKeyCustody {
    /// Retention snapshot only. Normal phase extraction closes the old Finish window; that
    /// must not erase already adopted keys. Whole-plan retirement, original call/pair or
    /// actor/session/policy/epoch revocation does end custody irreversibly. A caller must
    /// never use this snapshot as an authorization for a later SDK or network operation.
    pub fn is_current(&self) -> bool {
        self.binding.value.live()
            && self.binding.transition.epoch_is_live()
            && !self.binding.successor_closed.load(Ordering::Acquire)
    }
    pub(in crate::voice_v2) fn belongs_to(&self, binding: &Arc<PlanBinding>) -> bool {
        Arc::ptr_eq(&self.binding, binding)
    }
}

impl transition::ConfirmationCustody {
    /// Pure conversion of the SAME already captured native Finish proof. The adapter samples
    /// its original Runtime Clock before its final try-lock chain. No allocation, callback,
    /// Root lookup or progress acquisition may occur under that native admission fence.
    pub(in crate::voice_v2) fn retain_established(
        &self,
        binding: &Arc<PlanBinding>,
        mono: u64,
    ) -> Result<EstablishedKeyCustody, VoiceError> {
        if !self.belongs_to(binding) || binding.successor_closed.load(Ordering::Acquire) {
            return Err(VoiceError::Unauthorized);
        }
        self.remaining_at(mono)?;
        Ok(EstablishedKeyCustody {
            binding: binding.clone(),
        })
    }
}
