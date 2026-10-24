//! Original-KeyClaim SDP storage intent. Publish before prepayment so End/final
//! cleanup sees every actual preparation, including refused or abandoned workers.
use super::super::requests::sdp::Request;
use super::transition::{action_matches, try_lock};
use super::*;
use catcoms_sync::voice_v2::chain::{
    sdp_handoff::{Phase, Status as PhaseStatus},
    EstablishedKeyCustody,
};
use sha2::{Digest, Sha256};

pub(super) struct Claim {
    sequence: u64,
    proof: [u8; 32],
    closed: bool,
    worker: bool,
    phase: Option<Arc<Phase>>,
}
/// Move only OUT of native guards. Actual notifications/reserve retirement run on Drop,
/// including abandonment/unwind immediately after the irreversible close publication.
pub(super) struct CloseTicket(Arc<Phase>);
impl Drop for CloseTicket {
    fn drop(&mut self) {
        self.0.close();
    }
}
impl Claim {
    pub(super) fn is_drained(&self) -> bool {
        !self.worker
            && self.phase.as_ref().is_none_or(|phase| {
                phase
                    .try_status()
                    .is_ok_and(|status| status == PhaseStatus::Drained)
            })
    }
    pub(super) fn request_close(&mut self) -> Option<CloseTicket> {
        self.closed = true;
        if let Some(phase) = &self.phase {
            phase.request_close();
        }
        self.phase.as_ref().map(|phase| CloseTicket(phase.clone()))
    }
    fn state(&self, original_current: bool) -> u8 {
        if self.closed || !original_current {
            if self.is_drained() {
                3
            } else {
                2
            }
        } else if self
            .phase
            .as_ref()
            .is_some_and(|phase| phase.effect().is_live())
        {
            1
        } else {
            0
        }
    }
}
/// Fixed history; not permission to create a PC or compute/sign SDP.
pub(in crate::voice_handoff) struct Receipt {
    pub row: RowId,
    pub action: super::super::requests::Action,
    pub handshake: [u8; 32],
    pub key: [u8; 32],
    pub sequence: u64,
    pub proof: [u8; 32],
    pub state: u8,
}
fn claim<'a>(
    row: &ResolvedRow,
    state: &'a State,
    session: &Arc<SessionIdentity>,
    request: &Request,
) -> Result<&'a super::key_session::KeyClaim, VoiceError> {
    if row.row.key.parent.is_some()
        || !action_matches(state, request.action)
        || request.sequence != 1
    {
        return Err(VoiceError::Unauthorized);
    }
    let plan = state.plan().ok_or(VoiceError::Unauthorized)?;
    let claim = plan.transition.as_ref().ok_or(VoiceError::Unauthorized)?;
    let key = claim.key.as_ref().ok_or(VoiceError::Unauthorized)?;
    if plan.identity.handshake() != request.handshake
        || !Arc::ptr_eq(&claim.session, session)
        || key.receipt != request.key
    {
        return Err(VoiceError::Unauthorized);
    }
    Ok(key)
}
fn snapshot(row: RowId, request: &Request, claim: &Claim, original_current: bool) -> Receipt {
    Receipt {
        row,
        action: request.action,
        handshake: request.handshake,
        key: request.key,
        sequence: claim.sequence,
        proof: claim.proof,
        state: claim.state(original_current),
    }
}
/// The real worker owns the paid parent row through every allocation and destructor.
/// It never sits inside Phase; Root observers cannot retain a Row/Phase custody cycle.
pub(in crate::voice_handoff) struct Preparation {
    row: Arc<Row>,
    custody: EstablishedKeyCustody,
    phase: Option<Arc<Phase>>,
}
impl Preparation {
    pub(in crate::voice_handoff) fn prepare(&mut self) -> Result<(), VoiceError> {
        self.phase = Some(self.custody.prepare_sdp_handoff()?);
        Ok(())
    }
}
impl Drop for Preparation {
    fn drop(&mut self) {
        // Destroy unattached storage BEFORE publishing actual worker drainage.
        if let Some(phase) = self.phase.take() {
            phase.close();
            drop(phase);
        }
        let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        let sdp = state
            .plan_mut()
            .and_then(|plan| plan.transition.as_mut())
            .and_then(|claim| claim.key.as_mut())
            .and_then(|key| key.sdp.as_mut())
            .expect("same original preparing SDP intent");
        sdp.worker = false;
        drop(state);
        self.row.changed.notify_waiters();
    }
}
impl ResolvedRow {
    /// Exact uncertain-reply recovery creates neither payment nor new authority. Failed
    /// local preparation with no worker may retry under the original final admission.
    pub(in crate::voice_handoff) fn recover_sdp(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Request,
    ) -> Result<Option<Receipt>, VoiceError> {
        let _index = self.transition_index()?;
        let state = try_lock(&self.row.state)?;
        let key = claim(self, &state, session, request)?;
        Ok(key
            .sdp
            .as_ref()
            .filter(|sdp| sdp.worker || sdp.closed || sdp.phase.is_some())
            .map(|sdp| snapshot(self.row.id, request, sdp, key.custody.is_current())))
    }
    /// Latch the small fixed intent BEFORE any64KiB reserve. The existing row pays for
    /// this inline state and the returned ticket keeps that exact row allocation alive.
    pub(in crate::voice_handoff) fn begin_sdp(
        &self,
        admission: &super::super::transition::Admission<'_>,
        request: &Request,
    ) -> Result<Option<Preparation>, VoiceError> {
        if !Arc::ptr_eq(&self.ledger, admission.ledger())
            || !self
                .row
                .key
                .matches(admission.selected(), self.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let _index = self.transition_index()?;
        let mut state = try_lock(&self.row.state)?;
        let key = claim(self, &state, admission.session(), request)?;
        if key.released
            || !key.custody.is_current()
            || state.final_delivery().is_some()
            || state.final_phase.is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        if key
            .sdp
            .as_ref()
            .is_some_and(|sdp| sdp.worker || sdp.closed || sdp.phase.is_some())
        {
            return Ok(None);
        }
        admission.current()?;
        let key = state
            .plan_mut()
            .unwrap()
            .transition
            .as_mut()
            .unwrap()
            .key
            .as_mut()
            .unwrap();
        if key.sdp.is_none() {
            let mut hash = Sha256::new();
            hash.update(b"Mewtual native SDP handoff storage v1\0");
            hash.update(admission.session().0);
            hash.update(self.row.id.logical());
            hash.update(request.action.serial.to_be_bytes());
            hash.update(request.action.version.to_be_bytes());
            hash.update(request.action.input_tag);
            hash.update(request.handshake);
            hash.update(request.key);
            hash.update(request.sequence.to_be_bytes());
            key.sdp = Some(Claim {
                sequence: request.sequence,
                proof: hash.finalize().into(),
                closed: false,
                worker: false,
                phase: None,
            });
        }
        key.sdp.as_mut().unwrap().worker = true;
        Ok(Some(Preparation {
            row: self.row.clone(),
            custody: key.custody.clone(),
            phase: None,
        }))
    }
    /// Arm only under the SAME final UI/selection/session fence. Refused storage remains
    /// with the worker until its actual Drop, so it cannot disappear from drainage.
    pub(in crate::voice_handoff) fn attach_sdp(
        &self,
        admission: &super::super::transition::Admission<'_>,
        request: &Request,
        preparation: &mut Preparation,
    ) -> Result<Receipt, VoiceError> {
        if !Arc::ptr_eq(&preparation.row, &self.row)
            || !Arc::ptr_eq(&self.ledger, admission.ledger())
            || !self
                .row
                .key
                .matches(admission.selected(), self.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let _index = self.transition_index()?;
        let mut state = try_lock(&self.row.state)?;
        let key = claim(self, &state, admission.session(), request)?;
        let sdp = key.sdp.as_ref().ok_or(VoiceError::Unauthorized)?;
        if key.released
            || sdp.closed
            || !sdp.worker
            || !key.custody.is_current()
            || state.final_delivery().is_some()
            || state.final_phase.is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        let phase = preparation.phase.as_ref().ok_or(VoiceError::Capacity)?;
        if !key.custody.contains_sdp_handoff(phase) {
            return Err(VoiceError::Unauthorized);
        }
        admission.current()?;
        phase.arm()?;
        // Infallible move after the actual gate; no duplicate storage/payment.
        let sdp = state
            .plan_mut()
            .unwrap()
            .transition
            .as_mut()
            .unwrap()
            .key
            .as_mut()
            .unwrap()
            .sdp
            .as_mut()
            .unwrap();
        sdp.phase = preparation.phase.take();
        Ok(snapshot(self.row.id, request, sdp, true))
    }
    pub(in crate::voice_handoff) fn release_sdp(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Request,
        proof: [u8; 32],
    ) -> Result<Receipt, VoiceError> {
        self.release_sdp_after_close(session, request, proof, || {})
    }
    // The production callback is empty. Tests unwind at the actual closed-bit publication
    // after every native guard has returned, with the original affine ticket still alive.
    pub(super) fn release_sdp_after_close(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Request,
        proof: [u8; 32],
        after_close: impl FnOnce(),
    ) -> Result<Receipt, VoiceError> {
        let close = {
            let _index = self.transition_index()?;
            let mut state = try_lock(&self.row.state)?;
            let key = claim(self, &state, session, request)?;
            if key.sdp.as_ref().is_none_or(|sdp| sdp.proof != proof) {
                return Err(VoiceError::Unauthorized);
            }
            state
                .plan_mut()
                .unwrap()
                .transition
                .as_mut()
                .unwrap()
                .key
                .as_mut()
                .unwrap()
                .sdp
                .as_mut()
                .unwrap()
                .request_close()
        };
        // Notifications/reserve return never run under native final admission/row guards.
        after_close();
        drop(close);
        self.recover_sdp(session, request)?
            .ok_or(VoiceError::Unauthorized)
    }
}
impl Ledger {
    /// End/revocation need not receive browser Release. Close under the row, retire actual
    /// capacity outside all guards. Active preparation retains its independent obligation.
    pub(super) fn close_retired_sdp(&self) {
        let mut close: [Option<CloseTicket>; ROWS] = std::array::from_fn(|_| None);
        {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            for (row, slot) in index.rows.iter().flatten().zip(close.iter_mut()) {
                let mut state = row.state.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(key) = state
                    .plan_mut()
                    .and_then(|plan| plan.transition.as_mut())
                    .and_then(|claim| claim.key.as_mut())
                {
                    if key.released
                        || !row.key.lease.is_live()
                        || !key.custody.is_current()
                        || key.sdp.as_ref().is_some_and(|sdp| sdp.closed)
                    {
                        *slot = key.sdp.as_mut().and_then(Claim::request_close);
                    }
                }
            }
        }
        drop(close);
    }
}
#[cfg(test)]
impl ResolvedRow {
    pub(in crate::voice_handoff) fn sdp_test_release_unwind(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Request,
        proof: [u8; 32],
    ) {
        let _ = self.release_sdp_after_close(session, request, proof, || {
            panic!("parked original SDP close unwind")
        });
    }
    pub(in crate::voice_handoff) fn sdp_test_phase(&self) -> Option<Arc<Phase>> {
        let _index = self.transition_index().unwrap();
        let state = self.row.state.lock().unwrap();
        state
            .plan()
            .and_then(|plan| plan.transition.as_ref())
            .and_then(|claim| claim.key.as_ref())
            .and_then(|key| key.sdp.as_ref())
            .and_then(|sdp| sdp.phase.clone())
    }
    pub(in crate::voice_handoff) fn sdp_test_drained(&self) -> bool {
        let _index = self.transition_index().unwrap();
        let state = self.row.state.lock().unwrap();
        state
            .plan()
            .and_then(|plan| plan.transition.as_ref())
            .and_then(|claim| claim.key.as_ref())
            .and_then(|key| key.sdp.as_ref())
            .is_none_or(Claim::is_drained)
    }
}
