//! Affine capacity preparation for the later causal Ready/SDP transition. These owners
//! carry no protocol, engine, signature or RTC authority. They deliberately expose no
//! budget or lane; only a future native LocalJoin factory may adopt them after validating
//! its exact original plan/current pair and causal Ready state. UI activation stays off.
use super::*;

/// Inert SDP envelope, prepared before confirmation can be irreversibly frozen. Failure
/// leaves the old lane/plan usable. Drop returns unissued slots to their ORIGINAL reserve.
/// No Crypto position or readiness deadline is created by preparing this capacity.
pub struct PreparedSdpCapacity {
    // One paid indirection keeps retry refusals small and affine. This allocation follows
    // payment below; caller-owned Result storage does not conceal another retained graph.
    inner: Box<PreparedCapacity>,
}
struct PreparedCapacity {
    resources: Arc<Resources>,
    _reserve: Arc<Reservation>,
    _lowers: Arc<GroupForwardLanes>,
    _slots: Arc<slots::ReservedSlots>,
    binding: Arc<PlanBinding>,
    _preparation: Preparation,
    // Last: the prepared wrapper/subreservation graph stays paid through field destruction.
    _metadata: Credit,
}
// A weak reference cannot pin variable capacity or delay its retirement publisher. Its
// allocation/owner storage stays covered by the prepared metadata and fixed plan binding.
// No parent points back to this guard, and Drop enters only WorkflowState, never Control.
struct Preparation(Weak<Workflow>);
impl Drop for Preparation {
    fn drop(&mut self) {
        if let Some(workflow) = self.0.upgrade() {
            workflow
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .sdp_preparing = false;
        }
    }
}

/// One extracted original Crypto lane and prepaid SDP envelope. Not Clone and not a
/// capability to sign, compute, dispatch or report a pair ready. No Workflow is retained:
/// actual old input/Lower owners alone determine the separate retirement witness.
/// Dropping this ticket closes its lane; it never reopens confirmation or renews time.
pub struct TransferredSdpCapacity {
    lane: Option<Arc<WorkLane>>,
    _prepared: PreparedSdpCapacity,
    retirement: ConfirmationRetirement,
}
redacted_debug!(PreparedSdpCapacity, TransferredSdpCapacity);

impl TransferredSdpCapacity {
    /// Actual old Workflow retirement, not SDP activation or a current authority check.
    pub fn confirmation_drained(&self) -> bool {
        self.retirement.is_drained()
    }
}
impl Drop for TransferredSdpCapacity {
    fn drop(&mut self) {
        if let Some(lane) = self.lane.take() {
            lane.close();
        }
    }
}

impl ConfirmationControl {
    /// Prepay inert successor bytes/slots before extracting anything. This native-only
    /// capacity operation does not certify Finish/Ready acceptance or activate SDP.
    /// Keep the actual returned owner across any asynchronous work or uncertain reply.
    /// Another control cannot create a competing envelope while this one is retained.
    pub fn prepare_sdp_capacity(&self) -> Result<PreparedSdpCapacity, VoiceError> {
        let chain = self.control.upgrade().ok_or(VoiceError::Unauthorized)?;
        let (original_slots, preparation) = {
            let control = chain.control.lock().unwrap_or_else(|e| e.into_inner());
            if control.closed
                || !Arc::ptr_eq(&chain.binding, &self.binding)
                || !chain.binding.live()
            {
                return Err(VoiceError::Unauthorized);
            }
            let workflow = control.workflow.as_ref().ok_or(VoiceError::Unauthorized)?;
            let mut state = workflow.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed || state.sdp_preparing {
                return Err(VoiceError::Capacity);
            }
            state.sdp_preparing = true;
            (
                chain.history.slots.clone(),
                Preparation(Arc::downgrade(workflow)),
            )
        };
        let resources = self.binding.resources().clone();
        let group = self.binding.scope.group;
        let reserve = resources.reserve(group, super::super::negotiation::EXCHANGE_BYTES)?;
        // The successor's existing 4 KiB metadata allowance covers these prepared/ticket
        // wrappers and child reservation too; no allocation precedes its payment.
        let metadata = ByteBudget::Reserved(reserve.clone()).charge(
            &resources,
            group,
            super::super::negotiation::METADATA_BYTES,
            ResourceKind::Retained,
        )?;
        let lowers = resources.group_forward_lanes(group)?;
        let slots = original_slots.subreserve(2)?;
        let prepared = PreparedSdpCapacity {
            inner: Box::new(PreparedCapacity {
                resources,
                _reserve: reserve,
                _lowers: lowers,
                _slots: slots,
                binding: self.binding.clone(),
                _preparation: preparation,
                _metadata: metadata,
            }),
        };
        let open = {
            let control = chain.control.lock().unwrap_or_else(|e| e.into_inner());
            !control.closed && chain.binding.live()
        };
        if !open {
            return Err(VoiceError::Unauthorized);
        }
        Ok(prepared)
    }

    /// Freeze the exact old phase and move its IDLE lane without returning its scarce
    /// Crypto position to the ordinary pool. All fallible successor payment already
    /// exists. A pre-effect refusal returns that SAME prepared owner for retry.
    ///
    /// The caller must still establish causal Ready, original grant/pair/epoch and final
    /// native admission before later adoption. This operation is capacity closure only,
    /// like retire(); it starts no SDP deadline and creates no action/receipt/packet.
    pub fn transfer_sdp_capacity(
        &self,
        prepared: PreparedSdpCapacity,
    ) -> Result<TransferredSdpCapacity, (VoiceError, PreparedSdpCapacity)> {
        let Some(chain) = self.control.upgrade() else {
            return Err((VoiceError::Unauthorized, prepared));
        };
        if !Arc::ptr_eq(&prepared.inner.binding, &self.binding)
            || !Arc::ptr_eq(&prepared.inner.resources, self.binding.resources())
        {
            return Err((VoiceError::Scope, prepared));
        }
        let mut control = chain.control.lock().unwrap_or_else(|e| e.into_inner());
        if control.closed || !Arc::ptr_eq(&chain.binding, &self.binding) || !chain.binding.live() {
            drop(control);
            return Err((VoiceError::Unauthorized, prepared));
        }
        let workflow = control
            .workflow
            .as_ref()
            .expect("open original Workflow")
            .clone();
        let mut state = workflow.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed
            || state.handoff_busy
            || !state
                .crypto
                .as_ref()
                .is_some_and(|lane| lane.idle_for_transfer())
        {
            drop(state);
            drop(control);
            return Err((VoiceError::Capacity, prepared));
        }
        // All existing borrowers enter WorkflowState before the lane. Holding that fence
        // across idle observation, closure and take prevents an old engine preflight from
        // acquiring the lane in between. No callback/destructor/Engine entry is allowed here.
        // This manual close path does not call PreparedChain::close below. Fence an already
        // prepared Finish window too; phase extraction must never leave an old Open arm alive.
        chain.binding.transition.close();
        state.closed = true;
        let lane = state.crypto.take().expect("checked original idle lane");
        control.closed = true;
        chain.live.revoke();
        let original = control.workflow.take();
        let actions = std::mem::replace(&mut control.actions, std::array::from_fn(|_| Weak::new()));
        drop(state);
        drop(control);

        // Never call chain.close() after setting closed: it would skip this actual cleanup.
        // The child slot reserve retains exactly the successor's two original positions.
        chain.history.close();
        for action in actions.into_iter().filter_map(|action| action.upgrade()) {
            action.retire_if_unstarted();
        }
        workflow.close();
        let retirement = ConfirmationRetirement {
            binding: self.binding.clone(),
        };
        drop(original);
        drop(workflow);
        Ok(TransferredSdpCapacity {
            lane: Some(lane),
            _prepared: prepared,
            retirement,
        })
    }
}

#[cfg(test)]
mod tests;
