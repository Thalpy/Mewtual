//! Whole original-plan cleanup, independent from the old confirmation Workflow witness.
//! Preparing successor capacity is custody even before extraction/Exchange construction.
//! This tiny fixed-plan state grants no protocol, crypto, receive or RTC authority.
use super::*;

/// Observation of old Workflow AND successor capacity retirement. No Workflow/Exchange,
/// large weak allocation, credit lender or effect authority is retained by this witness.
pub struct PlanRetirement {
    binding: Arc<PlanBinding>,
}
redacted_debug!(PlanRetirement);
impl PlanRetirement {
    pub const OBSERVER_MEMORY_BYTES: usize = std::mem::size_of::<usize>();
    pub fn is_drained(&self) -> bool {
        plan_drained(&self.binding)
    }
}
fn plan_drained(binding: &PlanBinding) -> bool {
    binding.workflow_drained.load(Ordering::Acquire)
        && !binding.successor_busy.load(Ordering::Acquire)
}
impl ConfirmationControl {
    /// Whole-plan observation, distinct from `is_drained`'s old-Workflow-only meaning.
    /// Before explicit closure, callers must separately fence new admission (for example
    /// native dead-owner GC requires its irreversible original lease revocation).
    pub fn is_plan_drained(&self) -> bool {
        plan_drained(&self.binding)
    }
    /// Permanently fence successor birth/lending BEFORE closing the old Workflow.
    /// Call outside native/Engine/Group guards, as for ordinary confirmation retirement.
    /// Existing old-Workflow witnesses intentionally retain their original meaning: Ready
    /// needs to observe that phase while its new successor is already retained.
    pub fn retire_all(self) -> PlanRetirement {
        self.binding.successor_closed.store(true, Ordering::Release);
        let binding = self.binding.clone();
        drop(self.retire());
        PlanRetirement { binding }
    }
}

/// One affine guard moves with ALL prepared/transferred capacity. Put it LAST, after
/// actual reservation/lane/slot/metadata fields. Parent retains only original PlanBinding;
/// storing this guard there would keep its own drainage predicate permanently false.
pub(in crate::voice_v2) struct SuccessorCustody {
    binding: Arc<PlanBinding>,
}
impl SuccessorCustody {
    /// Caller MUST hold the original PreparedChain Control -> Workflow fence. A racing
    /// retire_all may set closed first, but cannot drop the old Workflow until the same
    /// fence releases. Therefore old-Workflow drainage cannot overtake guard publication.
    pub(super) fn begin(binding: &Arc<PlanBinding>) -> Result<Self, VoiceError> {
        if binding.successor_closed.load(Ordering::Acquire) {
            return Err(VoiceError::Unauthorized);
        }
        binding
            .successor_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| VoiceError::Capacity)?;
        Ok(Self {
            binding: binding.clone(),
        })
    }
}
impl Drop for SuccessorCustody {
    fn drop(&mut self) {
        // Actual fields/credits precede this publisher. An unpublished abandonment may
        // permit another preparation while open; retire_all's closed latch never resets.
        self.binding.successor_busy.store(false, Ordering::Release);
    }
}
