//! Exact terminal Action-history retirement. Native final-delivery certification and
//! real last-owner return are different observations; neither may be inferred from
//! an absent map entry, elapsed time, a revoked lease, or ordinary phase Return.
use super::*;

/// Small negative-only capability for a trusted native adapter. It retains the SAME
/// paid original PlanBinding, never an Action, Workflow, Exchange, Phase or native row.
/// Native must keep this in independently paid storage after removing its parent row.
pub struct PlanActionHistory {
    binding: Arc<PlanBinding>,
}
redacted_debug!(PlanActionHistory);

impl PlanActionHistory {
    pub const OBSERVER_MEMORY_BYTES: usize = std::mem::size_of::<usize>();

    pub(in crate::voice_v2) fn capture(
        binding: &Arc<PlanBinding>,
        retirement: &PlanRetirement,
    ) -> Result<Self, VoiceError> {
        if !retirement.belongs_to(binding)
            || !binding.whole_closed.load(Ordering::Acquire)
            || !retirement.is_drained()
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(Self {
            binding: binding.clone(),
        })
    }

    /// Trusted Rust-only certification. Call ONLY after the adapter checked its exact
    /// original issued-and-ACKed Plan proof AND its native collection scratch returned.
    /// This atomic request schedules existing Root maintenance; it does not mean that
    /// Root records, SDK work or credits have returned. Repeat after a lost reply safely.
    pub fn request_collection(&self) {
        self.binding
            .history_requested
            .store(true, Ordering::Release);
    }

    /// Actual fixed-record return plus old/successor drainage, independent of current
    /// call authority. Historical result observers retain the same fee and keep this
    /// false after map unlinking. This grants no admission, retry, renewal or new keys.
    pub fn is_collected(&self) -> bool {
        self.binding.history_requested.load(Ordering::Acquire)
            && self.binding.history_records.load(Ordering::Acquire) == 0
            && super::lifecycle::plan_drained(&self.binding)
    }
}

/// LAST field of the actual fixed history funding. Its Arc covers this inline handle
/// under the unchanged Action-record allowance. No Weak<Action> keeps unpaid allocation
/// storage alive. A racing observer/Weak upgrade still retains its real original fee.
pub(in crate::voice_v2) struct ActionHistoryCustody(Arc<PlanBinding>);
impl ActionHistoryCustody {
    pub(in crate::voice_v2) fn begin(binding: Arc<PlanBinding>) -> Self {
        binding.history_records.fetch_add(1, Ordering::AcqRel);
        Self(binding)
    }
}
impl Drop for ActionHistoryCustody {
    fn drop(&mut self) {
        let previous = self.0.history_records.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "actual original history record return");
    }
}

impl PreparedChain {
    pub(in crate::voice_v2) fn requested_action_history(&self) -> Option<&Arc<PlanBinding>> {
        self.binding
            .history_requested
            .load(Ordering::Acquire)
            .then_some(&self.binding)
    }
}

impl PlanBinding {
    pub(in crate::voice_v2) fn history_collection_requested(&self) -> bool {
        self.whole_closed.load(Ordering::Acquire) && self.history_requested.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests;
