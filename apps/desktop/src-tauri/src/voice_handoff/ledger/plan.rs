//! Original protected-plan custody, independent of any one action's variable Phase. The
//! parent Row's existing fixed payment covers these inline handles through ordinary Return,
//! late child/history replies and final channel collection. No Action or Workflow is retained.
use super::*;
use catcoms_sync::voice_v2::crypto_actions::{
    ConfirmationControl, ConfirmationRetirement, ProtectedCryptoPlan,
};

pub(super) struct Plan {
    identity: ProtectedCryptoPlan,
    control: Option<ConfirmationControl>,
    retirement: Option<ConfirmationRetirement>,
}
impl Plan {
    pub(super) fn capture(action: &CryptoAction) -> Result<Self, VoiceError> {
        Ok(Self {
            identity: action.protected_plan()?,
            control: Some(action.confirmation_control()?),
            retirement: None,
        })
    }
    pub(super) fn is_drained(&self) -> bool {
        self.retirement.as_ref().map_or_else(
            || {
                self.control
                    .as_ref()
                    .is_some_and(ConfirmationControl::is_drained)
            },
            ConfirmationRetirement::is_drained,
        )
    }
}

/// Close outside every native guard, then publish the SAME original retirement witness.
/// This synchronous ticket also completes closure on unwind/abandoned local callers. No
/// timer, current grant or replacement lookup may manufacture completion in the interim.
struct Retirement {
    row: Arc<Row>,
    control: Option<ConfirmationControl>,
}
#[cfg(test)]
pub(super) const RETIREMENT_MEMORY_BYTES: usize = std::mem::size_of::<Retirement>();
impl Drop for Retirement {
    fn drop(&mut self) {
        let Some(control) = self.control.take() else {
            return;
        };
        let witness = control.retire();
        {
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let plan = state.plan_mut().expect("same original captured plan");
            debug_assert!(plan.control.is_none() && plan.retirement.is_none());
            plan.retirement = Some(witness);
        }
        self.row.changed.notify_waiters();
    }
}

impl ResolvedRow {
    /// Capacity provenance for a trusted child factory, never an admission or effect grant.
    /// Frozen retirement excludes NEW child adoption even if its public metadata matches.
    pub(crate) fn contains_plan_action(&self, action: &CryptoAction) -> Result<bool, VoiceError> {
        let _index = self.indexed()?;
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Ok(state
            .plan()
            .is_some_and(|plan| plan.control.is_some() && plan.identity.contains(action)))
    }

    /// Trusted whole-plan cleanup AFTER the selected browser/provider/Phase has drained.
    /// This is deliberately separate from immutable Phase Return/Retire. In particular, a
    /// Derive Return cannot be upgraded, but it must not strand cleanup if End happens before
    /// any successor exists. Caller must resolve the original authenticated native owner.
    pub(crate) fn retire_plan(&self) -> Result<bool, VoiceError> {
        let ticket = {
            let _index = self.indexed()?;
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !state.status.drained
                || !state.status.engine.settled
                || (state.status.engine.receipt.is_some() && !state.status.acknowledged)
            {
                return Err(VoiceError::Capacity);
            }
            let plan = state.plan_mut().ok_or(VoiceError::Unauthorized)?;
            if plan.retirement.is_some() {
                return Ok(plan.is_drained());
            }
            let control = plan.control.take().ok_or(VoiceError::Capacity)?;
            Retirement {
                row: self.row.clone(),
                control: Some(control),
            }
        };
        drop(ticket); // Root may enter Engine and release credits; native guards are gone.
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Ok(state.plan().is_some_and(Plan::is_drained))
    }
}
