//! Affine original input target. Labels select a row; they cannot replace its actual
//! Session, parent, Action, PC/SDK or funding allocation at final scoped publication.
use super::super::requests::Action;
use super::super::transition::Admission;
use super::sdp_rows;
use super::transition::try_lock;
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::{NativeRowFunding, NativeRowPurpose, Phase};
use catcoms_sync::voice_v2::sdp::InitialSdpIntent;

/// Fixed descriptor provenance, derived exclusively from original native owners. The
/// variable AAD/body/sequence must come from the admitted Action input, never a renderer.
#[derive(Clone, Copy)]
pub(in crate::voice_handoff) struct Descriptor {
    pub row: RowId,
    pub parent: RowId,
    pub action: Action,
    pub generation: u64,
    pub ready: [u8; 32],
    pub key: [u8; 32],
    pub sdk_generation: u64,
    pub sdk: [u8; 32],
    pub storage: [u8; 32],
    pub pc: [u8; 32],
}
/// Separately paid by the concrete native task, not another permanent row or factory.
/// Keeping it affine prevents a caller from manufacturing parallel unpaid snapshots.
pub(in crate::voice_handoff) struct Target {
    row: ResolvedRow,
    session: Arc<SessionIdentity>,
    selected: Arc<SelectedHandoffOwner>,
    pub action: Arc<CryptoAction>,
    pub intent: Arc<InitialSdpIntent>,
    pub phase: Arc<Phase>,
    pub funding: Arc<NativeRowFunding>,
    pub descriptor: Descriptor,
    pub handshake: [u8; 32],
}
impl ResolvedRow {
    /// Reuse the already inventoried fixed SubmissionOwners snapshot before TaskCopy
    /// reservation. No descriptor/Target, input loan or new history fee exists yet.
    pub(in crate::voice_handoff) fn sdp_input_owners(
        &self,
        session: &Arc<SessionIdentity>,
        expected: Action,
    ) -> Result<sdp_rows::SubmissionOwners, VoiceError> {
        self.recover_sdp_publication()?;
        let _index = self.transition_index()?;
        let parent = self
            .row
            .key
            .parent
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let parent_state = try_lock(&parent.state)?;
        let plan = parent_state.plan().ok_or(VoiceError::Unauthorized)?;
        let state = try_lock(&self.row.state)?;
        let owners = sdp_rows::input_owners(&state)?;
        if !Arc::ptr_eq(owners.session, session)
            || plan
                .transition
                .as_ref()
                .is_none_or(|claim| !Arc::ptr_eq(&claim.session, session))
            || expected.serial != owners.action.serial()
            || expected.version != owners.action.version()
            || expected.input_tag != owners.action.input_tag()
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(sdp_rows::SubmissionOwners {
            phase: owners.phase.clone(),
            intent: owners.intent.clone(),
            funding: owners.funding.clone(),
            handshake: plan.identity.handshake(),
        })
    }
    /// Run historical publication recovery outside native guards first. This snapshot
    /// creates no input loan and gives no permission; final publication repeats identity
    /// and currentness while Root still exclusively borrows the actual ChannelSync.
    pub(in crate::voice_handoff) fn sdp_input_target(
        self,
        session: &Arc<SessionIdentity>,
        expected: Action,
        prepaid: &sdp_rows::SubmissionOwners,
        task: &catcoms_sync::voice_v2::chain::sdp_handoff::TaskCopy,
    ) -> Result<Target, VoiceError> {
        self.recover_sdp_publication()?;
        let target = {
            let index = self.transition_index()?;
            let Purpose::Sdp {
                parent: id,
                purpose,
            } = self.row.key.purpose
            else {
                return Err(VoiceError::Unauthorized);
            };
            let parent = self
                .row
                .key
                .parent
                .as_ref()
                .ok_or(VoiceError::Unauthorized)?;
            if parent.id != id
                || !index.rows[id.slot]
                    .as_ref()
                    .is_some_and(|value| Arc::ptr_eq(value, parent))
            {
                return Err(VoiceError::Unauthorized);
            }
            let parent_state = try_lock(&parent.state)?;
            let plan = parent_state.plan().ok_or(VoiceError::Unauthorized)?;
            let claim = plan.transition.as_ref().ok_or(VoiceError::Unauthorized)?;
            let key = claim.key.as_ref().ok_or(VoiceError::Unauthorized)?;
            let storage = key.sdp.as_ref().ok_or(VoiceError::Unauthorized)?;
            let pc = storage.pc.as_ref().ok_or(VoiceError::Unauthorized)?;
            let sdk = storage.sdk.as_ref().ok_or(VoiceError::Unauthorized)?;
            let intent = pc.intent().ok_or(VoiceError::Unauthorized)?;
            let phase = storage.phase.as_ref().ok_or(VoiceError::Unauthorized)?;
            let state = try_lock(&self.row.state)?;
            let owners = sdp_rows::input_owners(&state)?;
            let actual = Action {
                serial: owners.action.serial(),
                version: owners.action.version(),
                input_tag: owners.action.input_tag(),
            };
            if !Arc::ptr_eq(&claim.session, session)
                || !Arc::ptr_eq(owners.session, session)
                || actual != expected
                || !Arc::ptr_eq(owners.intent, intent)
                || !Arc::ptr_eq(owners.phase, phase)
                || !owners.action.uses_sdp_intent(intent)
                || !owners.funding.uses_phase(phase)
                || owners.funding.purpose() != purpose
                || !Arc::ptr_eq(&prepaid.phase, phase)
                || !Arc::ptr_eq(&prepaid.intent, intent)
                || !Arc::ptr_eq(&prepaid.funding, owners.funding)
                || !task.uses_intent(intent)
            {
                return Err(VoiceError::Unauthorized);
            }
            if !task.covers_work(super::super::sdp_publication::TASK_BYTES) {
                return Err(VoiceError::Capacity);
            }
            let (sdk_generation, sdk) = sdk.identity();
            // Clone only fixed original owners after every fallible snapshot check. The
            // caller's prepayment must cover this entire temporary Target graph too.
            Target {
                row: ResolvedRow {
                    ledger: self.ledger.clone(),
                    row: self.row.clone(),
                },
                session: session.clone(),
                selected: owners.selected.clone(),
                action: owners.action.clone(),
                intent: intent.clone(),
                phase: phase.clone(),
                funding: owners.funding.clone(),
                handshake: prepaid.handshake,
                descriptor: Descriptor {
                    row: self.row.id,
                    parent: id,
                    action: actual,
                    generation: intent.generation(),
                    ready: intent.round().ready,
                    key: key.receipt,
                    sdk_generation,
                    sdk,
                    storage: storage.proof,
                    pc: pc.proof(),
                },
            }
        };
        Ok(target)
    }
}
impl Target {
    pub(in crate::voice_handoff) fn row(&self) -> &ResolvedRow {
        &self.row
    }
    /// All native guards remain borrowed through the final Root commit. No allocation,
    /// Clock, owner destruction or await belongs in this tail. Once it returns, the caller
    /// must drop every short guard BEFORE inspecting/destroying the admitted output.
    /// Step/target Action identity is additionally checked on the resulting Data outside
    /// guards: App's generic hook accepts a separately supplied same-intent Action.
    pub(in crate::voice_handoff) fn admit<T>(
        &self,
        admission: &Admission<'_>,
        commit: impl FnOnce() -> Result<T, VoiceError>,
    ) -> Result<T, VoiceError> {
        if !Arc::ptr_eq(&self.row.ledger, admission.ledger())
            || !Arc::ptr_eq(&self.session, admission.session())
            || !std::ptr::eq(self.selected.as_ref(), admission.selected())
            || !self
                .row
                .row
                .key
                .matches(admission.selected(), self.row.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let index = self.row.transition_index()?;
        let parent = self
            .row
            .row
            .key
            .parent
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if !index.rows[parent.id.slot]
            .as_ref()
            .is_some_and(|value| Arc::ptr_eq(value, parent))
        {
            return Err(VoiceError::Unauthorized);
        }
        let parent_state = try_lock(&parent.state)?;
        let claim = parent_state
            .plan()
            .and_then(|value| value.transition.as_ref())
            .ok_or(VoiceError::Unauthorized)?;
        let key = claim.key.as_ref().ok_or(VoiceError::Unauthorized)?;
        let storage = key.sdp.as_ref().ok_or(VoiceError::Unauthorized)?;
        let state = try_lock(&self.row.row.state)?;
        let owners = sdp_rows::input_owners(&state)?;
        let position = match self.funding.purpose() {
            NativeRowPurpose::Seal => 0,
            NativeRowPurpose::Open => 1,
        };
        if !Arc::ptr_eq(&claim.session, &self.session)
            || claim.released
            || key.released
            || !key.custody.is_current()
            || storage.closed
            || storage.actions_closed[position]
            || storage.worker()
            || storage.sdk_preparing
            || storage.sdk.as_ref().is_none_or(|sdk| {
                sdk.released() || !sdk.matches(self.descriptor.sdk_generation, self.descriptor.sdk)
            })
            || storage.pc.as_ref().is_none_or(|pc| {
                !pc.matches(self.descriptor.pc)
                    || pc
                        .intent()
                        .is_none_or(|intent| !Arc::ptr_eq(intent, &self.intent))
            })
            || storage
                .phase
                .as_ref()
                .is_none_or(|phase| !Arc::ptr_eq(phase, &self.phase))
            || !key.custody.contains_sdp_handoff(&self.phase)
            || !self.intent.uses_phase(&self.phase)
            || !self.phase.effect().is_live()
            || !self.row.row.key.lease.is_live()
            || owners.closed
            || self.funding.is_closed()
            || !Arc::ptr_eq(owners.session, &self.session)
            || !Arc::ptr_eq(owners.selected, &self.selected)
            || !Arc::ptr_eq(owners.action, &self.action)
            || !Arc::ptr_eq(owners.intent, &self.intent)
            || !Arc::ptr_eq(owners.phase, &self.phase)
            || !Arc::ptr_eq(owners.funding, &self.funding)
            || parent_state.final_phase.is_some()
            || parent_state.final_delivery().is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        admission.current()?;
        commit()
    }
}

/// The pre-Task borrowed lookup is paid by its original physical Control caller. No
/// Target/descriptor or variable allocation exists yet. Keep this separate from the
/// later native Task graph instead of charging a new stack frame to the tight row fee.
#[cfg(test)]
pub(in crate::voice_handoff) fn prepayment_layout_bytes() -> usize {
    std::mem::size_of::<sdp_rows::InputOwners<'static>>()
        + std::mem::size_of::<sdp_rows::SubmissionOwners>()
        + 3 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 128 // original immutable selector/session/action borrows and error results
}
