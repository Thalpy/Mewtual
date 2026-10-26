//! Original protected-plan custody, independent of any one action's variable Phase. The
//! parent Row's existing fixed payment covers these inline handles through ordinary Return,
//! late child/history replies and final channel collection. No Action or Workflow is retained.
use super::*;
use catcoms_sync::voice_v2::crypto_actions::{
    ConfirmationControl, PlanRetirement, ProtectedCryptoPlan,
};

pub(super) struct Plan {
    pub(super) identity: ProtectedCryptoPlan,
    // Immutable scheduling metadata captured from the ACTUAL Derive action. Native Start
    // selection must not guess orientation from renderer fields or retained public IDs.
    pub(super) local_is_low: bool,
    pub(super) control: Option<ConfirmationControl>,
    retirement: Option<PlanRetirement>,
    pub(super) transition: Option<super::transition::Claim>,
}
impl Plan {
    pub(super) fn capture(action: &CryptoAction) -> Result<Self, VoiceError> {
        Ok(Self {
            identity: action.protected_plan()?,
            local_is_low: action.protected_binding()?.local_is_low,
            control: Some(action.confirmation_control()?),
            retirement: None,
            transition: None,
        })
    }
    pub(super) fn is_drained(&self) -> bool {
        self.transition
            .as_ref()
            .is_none_or(|claim| claim.all_released())
            && self.root_drained()
    }
    pub(super) fn can_adopt_child(&self) -> bool {
        self.control.is_some() && self.transition.is_none()
    }
    pub(super) fn root_drained(&self) -> bool {
        self.retirement.as_ref().map_or_else(
            || {
                self.control
                    .as_ref()
                    .is_some_and(ConfirmationControl::is_plan_drained)
            },
            PlanRetirement::is_drained,
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
        let witness = control.retire_all();
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
    /// Latch a DISTINCT original-Session Plan intent after action Returns. The opaque
    /// admission retains the SAME UI commit used by normal bootstrap closure, preventing
    /// preflight-then-close from overtaking newly required final delivery. No grant upgrade,
    /// Phase relabelling, reconstructed capability, allocation or detached work is involved.
    pub(in crate::voice_handoff) fn request_plan_final(
        &self,
        admission: &super::super::plan_final::Admission,
    ) -> Result<(), VoiceError> {
        if !Arc::ptr_eq(&self.ledger, admission.ledger()) {
            return Err(VoiceError::Unauthorized);
        }
        let ticket = {
            let index = self.indexed()?;
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if self.row.key.parent.is_some()
                || !matches!(self.row.key.purpose, Purpose::Derive(_))
                || !self
                    .row
                    .key
                    .matches(admission.selected(), self.row.key.purpose)
                || !matches!(state.body, Body::Drained(..))
                || state.final_phase.is_some()
            {
                return Err(VoiceError::Unauthorized);
            }
            if !state.status.drained
                || !state.status.engine.settled
                || (state.status.engine.receipt.is_some() && !state.status.acknowledged)
                || !Ledger::children_locally_closed(&index, &self.row)
            {
                return Err(VoiceError::Capacity);
            }
            if let Some(delivery) = state.final_delivery() {
                return if delivery.kind == FinalKind::Plan
                    && Arc::ptr_eq(&delivery.session, admission.session())
                {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                };
            }
            let plan = state.plan().ok_or(VoiceError::Unauthorized)?;
            // A released key-custody obligation remains bound to its actual original
            // Session allocation, even before the first final-delivery latch exists.
            if plan
                .transition
                .as_ref()
                .is_some_and(|claim| !Arc::ptr_eq(&claim.session, admission.session()))
            {
                return Err(VoiceError::Unauthorized);
            }
            // Intent closes first; issuance still waits for every original obligation.
            // A lost Claim/key/storage/SDK reply needs this negative fence BEFORE its
            // admission-shaped history recovery can safely run on a still-live Session.
            // Do not cancel a live Answer Source merely because the browser lost its
            // media-admission reply: native media/Source custody is authoritative here.
            if plan
                .transition
                .as_ref()
                .and_then(|claim| claim.key.as_ref())
                .and_then(|key| key.sdp.as_ref())
                .is_some_and(|storage| !super::media::negotiation_may_close(storage))
            {
                return Err(VoiceError::Capacity);
            }
            if plan.control.is_none() && plan.retirement.is_none() {
                // The existing Retirement ticket is between native publication and Root
                // witness installation. Its missing control is not retirement evidence.
                return Err(VoiceError::Capacity);
            }
            // A parked Ready/SDK/SDP worker can already own a clone of this Root binding.
            // Fence that SAME binding before publishing the irreversible native intent.
            // This atomic negative operation releases nothing and invokes no callback;
            // actual control retirement and affine destruction stay outside all guards.
            if let Some(key) = plan
                .transition
                .as_ref()
                .and_then(|claim| claim.key.as_ref())
            {
                key.custody.close_successor_effects();
            }
            state.session_collected = false; // prior physical proof cannot cover new intent
            let Body::Drained(delivery, ..) = &mut state.body else {
                unreachable!("validated original history")
            };
            *delivery = Some(FinalDelivery {
                session: admission.session().clone(),
                kind: FinalKind::Plan,
                issued: None,
                acknowledged: false,
            });
            state
                .plan_mut()
                .expect("same original retained plan")
                .control
                .take()
                .map(|control| Retirement {
                    row: self.row.clone(),
                    control: Some(control),
                })
        };
        drop(ticket); // Root retirement and affine destruction outside every native guard
        Ok(())
    }

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
            .is_some_and(|plan| plan.can_adopt_child() && plan.identity.contains(action)))
    }

    /// Trusted whole-plan cleanup AFTER the selected browser/provider/Phase has drained.
    /// This is deliberately separate from immutable Phase Return/Retire. In particular, a
    /// Derive Return cannot be upgraded, but it must not strand cleanup if End happens before
    /// any successor exists. Caller must resolve the original authenticated native owner.
    pub(crate) fn retire_plan(&self) -> Result<bool, VoiceError> {
        let ticket = self.prepare_retirement()?;
        drop(ticket); // Root may enter Engine and release credits; native guards are gone.
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Ok(state.plan().is_some_and(Plan::is_drained))
    }

    /// Take the actual original control under the native gate, before its destructor can
    /// enter Root. Keeping this transition separate permits deterministic cancellation/race
    /// tests to park the SAME production ticket rather than inventing a missing-control flag.
    fn prepare_retirement(&self) -> Result<Option<Retirement>, VoiceError> {
        let index = self.indexed()?;
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !state.status.drained
            || !state.status.engine.settled
            || (state.status.engine.receipt.is_some() && !state.status.acknowledged)
            || !Ledger::children_locally_closed(&index, &self.row)
        {
            return Err(VoiceError::Capacity);
        }
        let plan = state.plan_mut().ok_or(VoiceError::Unauthorized)?;
        if plan
            .transition
            .as_ref()
            .is_some_and(|claim| !claim.all_released())
        {
            return Err(VoiceError::Capacity);
        }
        if plan.retirement.is_some() {
            return Ok(None);
        }
        let control = plan.control.take().ok_or(VoiceError::Capacity)?;
        Ok(Some(Retirement {
            row: self.row.clone(),
            control: Some(control),
        }))
    }

    #[cfg(test)]
    pub(in crate::voice_handoff) fn park_retirement_for_test(
        &self,
    ) -> Result<impl Send, VoiceError> {
        self.prepare_retirement()
    }

    /// Park the actual retirement ticket after native capture, then retain a genuine
    /// trusted capacity preparation before its Root closure. All Root calls/Drop occur
    /// outside native guards. This certifies no Finish, readiness or SDP activation.
    #[cfg(test)]
    pub(in crate::voice_handoff) fn successor_before_retirement_for_test(
        &self,
    ) -> catcoms_sync::voice_v2::chain::PreparedSdpCapacity {
        let ticket = self.prepare_retirement().unwrap().unwrap();
        let prepared = ticket
            .control
            .as_ref()
            .unwrap()
            .prepare_sdp_capacity()
            .unwrap();
        drop(ticket);
        prepared
    }
}
