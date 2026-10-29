//! Inert progression prepayment. Waiting for a counterpart must not start a transcript
//! deadline or consume a call/pair action. This carrier reserves the SAME bytes and
//! positions consumed by the eventual chain; it grants no native/browser permission.
use super::*;

/// Affine, opaque capacity for one group/role and its exact original native pools.
/// No Clone, serialization, budget extraction or effect API exists. Trusted consumers
/// must supply their CURRENT pools again; equal group/role labels do not prove custody.
pub struct ChainCapacity {
    inner: Option<Box<InertCapacity>>,
    // OUTSIDE and after the Box. Ready's original4KiB fee pays the complete inert graph
    // through body destruction/deallocation and any blocking final claim publication.
    _metadata: Arc<ProgressFunding>,
}
struct InertCapacity {
    resources: Arc<Resources>,
    group: [u8; 32],
    low: bool,
    slots: Arc<slots::Slots>,
    packets: super::super::packets::ConfirmationCapacity,
    history_bytes: Arc<Reservation>,
    history_slots: Arc<slots::ReservedSlots>,
    progress: ProgressCapacityOwner,
    crypto: Arc<WorkLane>,
    lowers: Arc<GroupForwardLanes>,
    #[cfg(test)]
    before_root_metadata: Option<RootReturnBoundary>,
    // Every separately allocated Root-funded capacity object precedes its fee. Their
    // real destructors may block; the Ready body fee cannot substitute for this2KiB.
    root_metadata: Credit,
    // Last variable-capacity owner. The outer fee additionally covers Box deallocation.
    claim: GroupProgressClaim,
}
redacted_debug!(ChainCapacity);

#[cfg(test)]
struct RootReturnBoundary {
    entered: Arc<std::sync::Barrier>,
    resume: Arc<std::sync::Barrier>,
}
#[cfg(test)]
impl Drop for RootReturnBoundary {
    fn drop(&mut self) {
        self.entered.wait();
        self.resume.wait();
    }
}

impl ChainCapacity {
    /// Reserve capacity only: no lease, binding, protocol state, clock or native row is
    /// created. LOW/HIGH determines only the original packet-position allocation.
    pub(in crate::voice_v2) fn reserve(
        resources: Arc<Resources>,
        group: [u8; 32],
        low: bool,
        slots: Arc<slots::Slots>,
        packet_slots: &super::super::packets::PacketSlots,
    ) -> Result<Self, VoiceError> {
        Self::reserve_after_payment(resources, group, low, slots, packet_slots, || {})
    }

    // Production's hook is empty. A deterministic refusal test observes the actual fee
    // before either packet wrapper can allocate; this runs outside every owner/pool lock.
    fn reserve_after_payment(
        resources: Arc<Resources>,
        group: [u8; 32],
        low: bool,
        slots: Arc<slots::Slots>,
        packet_slots: &super::super::packets::PacketSlots,
        after_payment: impl FnOnce(),
    ) -> Result<Self, VoiceError> {
        Self::reserve_with_row_bytes(
            resources,
            group,
            low,
            slots,
            packet_slots,
            ROW_BYTES,
            after_payment,
        )
    }

    /// The production caller fixes row_bytes to ROW_BYTES. Keeping the same
    /// early fence testable with the old allowance proves a growth refusal has
    /// not already charged metadata, allocated packet wrappers or spent slots.
    fn reserve_with_row_bytes(
        resources: Arc<Resources>,
        group: [u8; 32],
        low: bool,
        slots: Arc<slots::Slots>,
        packet_slots: &super::super::packets::PacketSlots,
        row_bytes: usize,
        after_payment: impl FnOnce(),
    ) -> Result<Self, VoiceError> {
        let action = super::super::crypto_actions::fixed_record_layout_bytes();
        let admission = super::admission::ADMISSION_RECORD_BYTES;
        let start = super::start::START_RECORD_BYTES;
        if row_bytes
            .checked_sub(admission)
            .is_none_or(|remaining| action > remaining)
            || row_bytes
                .checked_sub(start)
                .is_none_or(|remaining| action > remaining)
        {
            return Err(VoiceError::Capacity);
        }
        let history_total = ROWS.checked_mul(row_bytes).ok_or(VoiceError::Capacity)?;
        let root_metadata = resources.charge(group, METADATA_BYTES, ResourceKind::Retained)?;
        after_payment();
        let packets = packet_slots.reserve_confirmation(low)?;
        let history_bytes = resources.reserve(group, history_total)?;
        let history_slots = slots.reserve(ROWS)?;
        let core = resources.reserve(group, super::super::negotiation::EXCHANGE_BYTES)?;
        let ready = Arc::new(super::super::ready::capacity::ReadyCapacity::new(
            &resources, group,
        )?);
        let metadata = ready.metadata.clone();
        let claim = GroupProgressClaim::reserve(&resources, group, metadata.clone())?;
        let escrow = ByteBudget::Reserved(core.clone()).charge(
            &resources,
            group,
            super::super::negotiation::EXCHANGE_BYTES - VARIABLE_BYTES,
            ResourceKind::Retained,
        )?;
        let progress = ProgressCapacityOwner::new(SdpCapacityClosing { core, ready }, escrow);
        let crypto = resources.reserve_crypto_lane(group)?;
        let lowers = resources.group_forward_lanes(group)?;
        Ok(Self {
            inner: Some(Box::new(InertCapacity {
                resources,
                group,
                low,
                slots,
                packets,
                root_metadata,
                history_bytes,
                history_slots,
                progress,
                crypto,
                lowers,
                #[cfg(test)]
                before_root_metadata: None,
                claim,
            })),
            _metadata: metadata,
        })
    }

    /// Consume the exact prepayment under the trusted caller's current native identity.
    /// Validation is pure: never borrow/refund a slot just to compare pool ownership.
    /// The original binding deadline moves unchanged; this starts no replacement window.
    pub(super) fn into_chain(
        self,
        resources: &Arc<Resources>,
        binding: Binding,
        slots: &Arc<slots::Slots>,
        packet_slots: &super::super::packets::PacketSlots,
    ) -> Result<Arc<PreparedChain>, VoiceError> {
        self.into_chain_after_decomposition(resources, binding, slots, packet_slots, || {})
    }

    // Empty in production; a deterministic test unwinds after original owners move out
    // of the inert Box, before any bound graph is allocated. No lock or effect is entered.
    fn into_chain_after_decomposition(
        mut self,
        resources: &Arc<Resources>,
        binding: Binding,
        slots: &Arc<slots::Slots>,
        packet_slots: &super::super::packets::PacketSlots,
        after_decomposition: impl FnOnce(),
    ) -> Result<Arc<PreparedChain>, VoiceError> {
        if !binding.live() {
            return Err(VoiceError::Unauthorized);
        }
        let inner = self.inner.as_ref().expect("one original inert capacity");
        if !Arc::ptr_eq(resources, &inner.resources)
            || !Arc::ptr_eq(slots, &inner.slots)
            || binding.scope.group != inner.group
            || binding.low != inner.low
            || !inner.packets.uses_pools(packet_slots)
        {
            return Err(VoiceError::Scope);
        }
        let InertCapacity {
            // Reverse local destruction keeps this claim through all later capacity
            // locals. Self's OUTER metadata anchor remains through this whole consume.
            claim,
            root_metadata,
            resources: _,
            group: _,
            low: _,
            slots,
            packets,
            history_bytes,
            history_slots,
            progress,
            crypto,
            lowers,
            #[cfg(test)]
                before_root_metadata: _,
        } = *self
            .inner
            .take()
            .expect("consume one original inert capacity");
        after_decomposition();
        let reserve = progress.capacity().closing.core.clone();
        let live = CallLease::default();
        let binding = Arc::new(PlanBinding {
            _metadata: root_metadata,
            value: binding,
            transition: transition::FinishWindow::default(),
            workflow_drained: AtomicBool::new(false),
            successor_closed: AtomicBool::new(false),
            whole_closed: AtomicBool::new(false),
            successor_busy: AtomicBool::new(false),
            low_ready: AtomicBool::new(false),
            history_requested: AtomicBool::new(false),
            history_records: AtomicUsize::new(0),
        });
        let history = Arc::new(HistoryPlan {
            binding: binding.clone(),
            live: live.clone(),
            bytes: history_bytes,
            pool: slots,
            slots: history_slots,
            state: Mutex::new(HistoryState {
                closed: false,
                serial: 0,
                active_row: NO_ROW,
                issued: 0,
                candidates: if binding.low {
                    Candidates::Low(std::array::from_fn(|_| None))
                } else {
                    Candidates::High(std::array::from_fn(|_| None))
                },
                followups: None,
            }),
        });
        let chain = Arc::new_cyclic(|control| PreparedChain {
            binding: binding.clone(),
            live: live.clone(),
            history,
            control: Mutex::new(Control {
                closed: false,
                actions: std::array::from_fn(|_| Weak::new()),
                workflow: Some(Arc::new(Workflow {
                    binding: binding.clone(),
                    live,
                    control: control.clone(),
                    reserve,
                    state: Mutex::new(WorkflowState {
                        closed: false,
                        progress: Some(progress),
                        packets: Some(packets),
                        crypto: Some(crypto),
                        lowers: Some(lowers),
                        handoff_busy: false,
                        sdp_preparing: false,
                    }),
                    _retirement: Some(RetirementPublisher {
                        progress: claim,
                        binding,
                    }),
                })),
            }),
        });
        // Losing construction/destruction happens after Pool unlock, as before the split.
        {
            let control = chain
                .control
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            resources.install_workflow(
                chain.binding.scope.group,
                control.workflow.as_ref().expect("prepared workflow"),
            )?;
        }
        Ok(chain)
    }

    #[cfg(test)]
    pub(in crate::voice_v2) fn metadata_bytes() -> usize {
        std::mem::size_of::<InertCapacity>() + std::mem::size_of::<Self>()
    }
}

#[cfg(test)]
mod tests;
