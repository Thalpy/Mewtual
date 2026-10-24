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
    ready: super::super::ready::capacity::ReadyCapacity,
    binding: Arc<PlanBinding>,
    _preparation: Preparation,
    // All capacity fields precede their metadata credit; only the fixed-plan publisher
    // follows it, so actual successor credits return before whole-plan drainage is visible.
    _metadata: Credit,
    // Last publisher covers partial preparation, extraction and eventual Exchange fields.
    custody: SuccessorCustody,
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

/// One extracted original Crypto lane, packet-reservation closing rights and prepaid SDP
/// envelope. Not Clone and not a
/// capability to sign, compute, dispatch or report a pair ready. No Workflow is retained:
/// actual old input/Lower owners alone determine the separate retirement witness.
/// Dropping this ticket closes its lane; it never reopens confirmation or renews time.
pub struct TransferredSdpCapacity {
    lane: Option<Arc<WorkLane>>,
    packets: Option<super::super::packets::ConfirmationCapacity>,
    _prepared: Option<PreparedSdpCapacity>,
    retirement: ConfirmationRetirement,
}
redacted_debug!(PreparedSdpCapacity, TransferredSdpCapacity);

impl TransferredSdpCapacity {
    /// Actual old Workflow retirement, not SDP activation or a current authority check.
    pub fn confirmation_drained(&self) -> bool {
        self.retirement.is_drained()
    }

    /// Private affine decomposition for the native successor constructor. The original lane
    /// moves before this ticket's destructor; no reserve/slot/lane is closed or recharged.
    /// Call outside Group/Policy because dropping Preparation may enter old WorkflowState.
    pub(in crate::voice_v2) fn into_parts(mut self) -> TransferredParts {
        let prepared = self._prepared.take().expect("one original preparation");
        let PreparedCapacity {
            // Bind FIRST so this publisher drops LAST if decomposition ever unwinds.
            custody,
            resources,
            _reserve: reserve,
            _lowers: lowers,
            _slots: slots,
            ready,
            binding,
            _preparation: preparation,
            _metadata: metadata,
        } = *prepared.inner;
        drop(preparation);
        TransferredParts {
            resources,
            reserve,
            lowers,
            slots,
            ready,
            packets: self
                .packets
                .take()
                .expect("one original packet reservation owner"),
            proof: SuccessorProof(binding),
            lane: self.lane.take().expect("one original idle lane"),
            metadata,
            custody,
        }
    }
}

/// Only an actual affine extraction constructs this proof. It owns fixed paid provenance,
/// never Workflow or browser keys. Old transition Closed is expected after extraction, so
/// successor checks deliberately use the ORIGINAL call/pair and epoch observation instead.
pub(in crate::voice_v2) struct SuccessorProof(Arc<PlanBinding>);

/// Fixed historical provenance, independently of the old closed confirmation transition.
/// This owns no Workflow, Exchange, byte lender or crypto lane. Receipt lookup is not effect
/// authority: its caller must also check the actual room/epoch and immutable receipt times.
pub(in crate::voice_v2) struct OriginalPlanObservation(Arc<PlanBinding>);
impl OriginalPlanObservation {
    pub(in crate::voice_v2) fn binding(&self) -> &Binding {
        &self.0.value
    }
    pub(in crate::voice_v2) fn current(&self) -> bool {
        self.0.value.live()
            && self.0.transition.epoch_is_live()
            && !self.0.successor_closed.load(Ordering::Acquire)
    }
}
impl SuccessorProof {
    pub(in crate::voice_v2) fn original_observation(&self) -> OriginalPlanObservation {
        OriginalPlanObservation(self.0.clone())
    }
    pub(in crate::voice_v2) fn finish_identity(&self) -> Option<FinishIdentity> {
        self.0.transition.original_identity()
    }
    pub(in crate::voice_v2) fn binding(&self) -> &Binding {
        &self.0.value
    }
    pub(in crate::voice_v2) fn matches(
        &self,
        binding: &Binding,
        resources: &Arc<Resources>,
    ) -> bool {
        self.0.value.same(binding) && Arc::ptr_eq(self.0.resources(), resources)
    }
    pub(in crate::voice_v2) fn live(&self) -> bool {
        self.0.value.live()
            && self.0.transition.epoch_is_live()
            && !self.0.successor_closed.load(Ordering::Acquire)
    }
    pub(in crate::voice_v2) fn current_at(&self, mono: u64) -> bool {
        self.live() && self.until().is_some_and(|until| mono < until)
    }
    pub(in crate::voice_v2) fn until(&self) -> Option<u64> {
        self.0.transition.original_until()
    }
    pub(in crate::voice_v2) fn drained(&self) -> bool {
        self.0.workflow_drained.load(Ordering::Acquire)
    }
}
pub(in crate::voice_v2) struct TransferredParts {
    pub resources: Arc<Resources>,
    pub reserve: Arc<Reservation>,
    pub lowers: Arc<GroupForwardLanes>,
    pub slots: Arc<slots::ReservedSlots>,
    pub ready: super::super::ready::capacity::ReadyCapacity,
    pub packets: super::super::packets::ConfirmationCapacity,
    pub proof: SuccessorProof,
    pub lane: Arc<WorkLane>,
    pub metadata: Credit,
    pub custody: SuccessorCustody,
}

impl PreparedSdpCapacity {
    pub(in crate::voice_v2) fn current_at(&self, mono: u64) -> bool {
        self.inner.binding.live()
            && !self.inner.binding.successor_closed.load(Ordering::Acquire)
            && self
                .inner
                .binding
                .transition
                .original_until()
                .is_some_and(|until| mono < until)
    }
}

impl PreparedChain {
    pub(in crate::voice_v2) fn original_observation(&self) -> OriginalPlanObservation {
        OriginalPlanObservation(self.binding.clone())
    }
    pub(in crate::voice_v2) fn successor_actor_matches(
        &self,
        actor: &super::super::runtime::ActorOwner,
        context: &super::super::Context,
    ) -> Result<bool, VoiceError> {
        Ok(self
            .binding
            .transition
            .same_epoch(&actor.observe_epoch(context)?))
    }
    /// Actual high-side Finish-open history, not a public marker or low-side local seal.
    pub(in crate::voice_v2) fn high_successor_current(
        &self,
        binding: &Binding,
        resources: &Arc<Resources>,
        mono: u64,
    ) -> bool {
        !self.binding.low
            && !self.binding.successor_closed.load(Ordering::Acquire)
            && self.binding.value.same(binding)
            && Arc::ptr_eq(self.binding.resources(), resources)
            && self.binding.value.live()
            && self.binding.transition.epoch_is_live()
            && self
                .binding
                .transition
                .original_until()
                .is_some_and(|until| mono < until)
    }
}
impl Drop for TransferredSdpCapacity {
    fn drop(&mut self) {
        if let Some(lane) = self.lane.take() {
            lane.close();
        }
        if let Some(packets) = self.packets.take() {
            packets.close();
        }
    }
}

impl ConfirmationControl {
    /// Prepay inert successor bytes/slots before extracting anything. This native-only
    /// capacity operation does not certify Finish/Ready acceptance or activate SDP.
    /// Keep the actual returned owner across any asynchronous work or uncertain reply.
    /// Another control cannot create a competing envelope while this one is retained.
    pub fn prepare_sdp_capacity(&self) -> Result<PreparedSdpCapacity, VoiceError> {
        self.prepare_sdp_capacity_before_slots(|| {})
    }
    // Production supplies no callback. Tests close the real original plan after payment,
    // before its original row subreserve, to pin closure-versus-capacity classification.
    fn prepare_sdp_capacity_before_slots(
        &self,
        before_slots: impl FnOnce(),
    ) -> Result<PreparedSdpCapacity, VoiceError> {
        let chain = self.control.upgrade().ok_or(VoiceError::Unauthorized)?;
        let (original_slots, preparation, custody) = {
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
            let custody = SuccessorCustody::begin(&chain.binding)?;
            state.sdp_preparing = true;
            (
                chain.history.slots.clone(),
                Preparation(Arc::downgrade(workflow)),
                custody,
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
        // Ready cache/Source/actual Lower/ACK work must survive ordinary-pool pressure.
        // Pay before irreversible extraction; never borrow SDP's sole Source quota for it.
        let ready = super::super::ready::capacity::ReadyCapacity::new(&resources, group)?;
        let lowers = resources.group_forward_lanes(group)?;
        before_slots();
        let slots = original_slots.subreserve(2).map_err(|error| {
            // Whole retirement closes the original row reserve too. A refusal caused by
            // that irreversible authority loss is terminal, not transient slot pressure.
            if self.binding.successor_closed.load(Ordering::Acquire) {
                VoiceError::Unauthorized
            } else {
                error
            }
        })?;
        let prepared = PreparedSdpCapacity {
            inner: Box::new(PreparedCapacity {
                resources,
                _reserve: reserve,
                _lowers: lowers,
                _slots: slots,
                ready,
                binding: self.binding.clone(),
                _preparation: preparation,
                _metadata: metadata,
                custody,
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
        let packets = state
            .packets
            .take()
            .expect("original open packet reservations");
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
            packets: Some(packets),
            _prepared: Some(prepared),
            retirement,
        })
    }
}

#[cfg(test)]
mod tests;
