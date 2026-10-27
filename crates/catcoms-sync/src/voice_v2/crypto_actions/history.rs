//! Remove only the exact native-certified terminal Plan's records. No identity, replay,
//! issuance, Source allowance or namespace is reset along with this bounded lookup map.
use super::*;

impl CryptoAction {
    /// Test-only observer in the ACTUAL fixed Credit's destruction, without adding a
    /// production field or replacing the credit. Requires the sole real Action/funding.
    #[cfg(test)]
    pub(in crate::voice_v2) fn observe_record_credit_drop_for_test(
        action: &mut Arc<Self>,
        observer: catcoms_rt::SharedRequestKeepalive,
    ) {
        let action = Arc::get_mut(action).expect("sole actual Action after unlink");
        let temporary = action
            .resources
            .charge(action.scope.group, 1, ResourceKind::Retained)
            .unwrap();
        let record = Arc::get_mut(&mut action._record).expect("sole actual fixed history funding");
        let original = std::mem::replace(&mut record._credit, temporary);
        let temporary = std::mem::replace(&mut record._credit, original.with_custody(observer));
        drop(temporary);
    }
    pub(in crate::voice_v2) fn certified_history_removable(
        &self,
        binding: &Arc<super::super::chain::PlanBinding>,
    ) -> bool {
        binding.history_collection_requested()
            && self.history_belongs_to(binding)
            && self.history_removable()
    }
    fn history_belongs_to(&self, binding: &Arc<super::super::chain::PlanBinding>) -> bool {
        self.binding
            .as_ref()
            .is_some_and(|actual| Arc::ptr_eq(actual, binding))
            || self
                .sdp_binding()
                .is_some_and(|actual| Arc::ptr_eq(actual.original_plan_binding(), binding))
    }

    fn history_removable(&self) -> bool {
        // Never wait under sole-Root maintenance for an external SDK/result observer.
        // Engine -> input is the existing order; only inspect under these short guards.
        let engine = match self.engine.try_lock() {
            Ok(engine) => engine,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return false,
        };
        if !engine.settled
            || engine.preparing
            || engine.permit.is_some()
            || engine.lender.is_some()
            || (engine.terminal.is_some() && !engine.acknowledged)
        {
            return false;
        }
        match self.input.try_lock() {
            Ok(input) => input.is_none(),
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner().is_none(),
            Err(std::sync::TryLockError::WouldBlock) => false,
        }
    }
}

impl ActionQueue {
    pub(in crate::voice_v2) fn collect_plan_history(
        &mut self,
        binding: &Arc<super::super::chain::PlanBinding>,
    ) {
        if !binding.history_collection_requested() {
            return;
        }
        // One bounded Arc scratch, with no Engine/input/native fence held during Drop.
        // External owners may outlive unlinking; their slot/credit and last-field history
        // custody remain real and PlanActionHistory::is_collected stays false for them.
        while let Some(key) = self
            .actions
            .iter()
            .find_map(|(key, action)| action.certified_history_removable(binding).then_some(*key))
        {
            drop(self.actions.remove(&key));
        }
    }
}
