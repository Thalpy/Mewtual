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

impl Workflow {
    /// Capacity-only table fixture: occupy every ORIGINAL claim without inventing a new
    /// global pool or reducing a limit. Native third-attempt tests use real packet rows.
    #[cfg(test)]
    pub(in crate::voice_v2) fn occupy_packet_slots_for_test(
        &self,
        tables: &super::super::packets::PacketSlots,
    ) -> (Vec<slots::Slot>, Vec<slots::Slot>) {
        let state = self.state.lock().unwrap();
        let packets = state.packets.as_ref().unwrap();
        (
            (0..if self.binding.low { 3 } else { 6 })
                .map(|_| packets.confirmation_incoming_slot(tables).unwrap())
                .collect(),
            (0..if self.binding.low { 6 } else { 3 })
                .map(|_| packets.outgoing_slot(tables).unwrap())
                .collect(),
        )
    }
    /// Only HIGH's actual native Finish-open makes its old outgoing Hello/Proof work
    /// obsolete. Closed old state is irreversible. LOW must keep Finish retries until
    /// its separately authenticated Ready admission exists; this predicate excludes LOW.
    pub(in crate::voice_v2) fn high_confirmation_sources_retired(&self) -> bool {
        !self.binding.low
            && self.binding.transition.original_until().is_some()
            && self.state.lock().unwrap_or_else(|e| e.into_inner()).closed
    }
    pub(in crate::voice_v2) fn belongs_to_plan(&self, chain: &PreparedChain) -> bool {
        Arc::ptr_eq(&self.binding, &chain.binding)
    }
}
impl PreparedChain {
    /// Cleanup-only original binding. This cannot reconstruct a Workflow or authorize
    /// work. Call outside Group/Policy: subsequent cache drops may retire real credits.
    pub(in crate::voice_v2) fn retired_high_confirmation_binding(&self) -> Option<&Binding> {
        (!self.binding.low
            && self.binding.transition.original_until().is_some()
            && self
                .control
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .closed)
            .then_some(&self.binding.value)
    }
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
        self.retire_all_after_close(|| {})
    }
    // Production supplies no callback; a deterministic test observes the real close store
    // before it waits for Control, while preparation is between its open check and busy CAS.
    pub(super) fn retire_all_after_close(self, after_close: impl FnOnce()) -> PlanRetirement {
        self.binding.successor_closed.store(true, Ordering::Release);
        after_close();
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
        Self::begin_after_open_check(binding, || {})
    }
    // Same unpublished decision, with a private deterministic scheduling seam. No public
    // caller can inject code under Control/Workflow; the production closure is empty.
    pub(super) fn begin_after_open_check(
        binding: &Arc<PlanBinding>,
        after_open: impl FnOnce(),
    ) -> Result<Self, VoiceError> {
        if binding.successor_closed.load(Ordering::Acquire) {
            return Err(VoiceError::Unauthorized);
        }
        after_open();
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
