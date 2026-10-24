//! One retained original-session key-custody obligation. No renderer identifiers construct
//! it, and old Workflow drainage does not substitute for authenticated browser Release.
use super::super::requests::Action;
use super::*;
use catcoms_sync::voice_v2::crypto_actions::ConfirmationCustody;
use sha2::{Digest, Sha256};

pub(super) struct Claim {
    pub(super) session: Arc<SessionIdentity>,
    custody: ConfirmationCustody,
    receipt: [u8; 32],
    pub(super) released: bool,
}
pub(in crate::voice_handoff) struct TransitionReceipt {
    pub row: RowId,
    pub action: Action,
    pub handshake: [u8; 32],
    pub epoch: u64,
    pub message: [u8; 32],
    pub hash: [u8; 32],
    pub until: u64,
    pub remaining: u32,
    pub receipt: [u8; 32],
    // 1=borrowed current custody,2=closing history with no current custody,3=released.
    pub state: u8,
}

fn action_matches(state: &State, expected: Action) -> bool {
    state.status.action == Some((expected.serial, expected.version, expected.input_tag))
}
impl Claim {
    fn snapshot(
        &self,
        row: RowId,
        action: Action,
        handshake: [u8; 32],
        mono: u64,
    ) -> TransitionReceipt {
        let remaining = self
            .custody
            .remaining_at(mono)
            .ok()
            .and_then(|value| u32::try_from(value).ok());
        TransitionReceipt {
            row,
            action,
            handshake,
            epoch: self.custody.epoch(),
            message: self.custody.message(),
            hash: self.custody.object_hash(),
            until: self.custody.transition_until(),
            remaining: if self.released {
                0
            } else {
                remaining.unwrap_or(0)
            },
            receipt: self.receipt,
            state: if self.released {
                3
            } else if remaining.is_some() {
                1
            } else {
                2
            },
        }
    }
}

impl ResolvedRow {
    #[cfg(test)]
    pub(in crate::voice_handoff) fn has_transition_claim(&self) -> bool {
        let _index = self.indexed().unwrap();
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.plan().is_some_and(|plan| {
            plan.transition
                .as_ref()
                .is_some_and(|claim| !claim.released)
        })
    }
    #[cfg(test)]
    pub(in crate::voice_handoff) fn transition_test_state(&self) -> (bool, bool, bool, bool) {
        let _index = self.indexed().unwrap();
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let plan = state.plan().unwrap();
        (
            state.session_collected,
            plan.root_drained(),
            plan.transition.as_ref().is_some_and(|claim| claim.released),
            plan.can_adopt_child(),
        )
    }
    /// All final acquisitions after native Clock sampling refuse contention. Blocking a
    /// sampled deadline behind another thread would let it issue a stale remaining bound.
    fn transition_index(&self) -> Result<std::sync::MutexGuard<'_, Index>, VoiceError> {
        let index = try_lock(&self.ledger.index)?;
        if !index
            .rows
            .get(self.row.id.slot)
            .and_then(Option::as_ref)
            .is_some_and(|row| row.id == self.row.id && Arc::ptr_eq(row, &self.row))
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(index)
    }
    /// Same-Session immutable history remains recoverable AFTER End/lock/expiry, allowing
    /// a lost initial reply to recover the receipt needed for Release. This creates nothing.
    pub(in crate::voice_handoff) fn recover_transition(
        &self,
        session: &Arc<SessionIdentity>,
        action: Action,
        handshake: [u8; 32],
        mono: u64,
    ) -> Result<Option<TransitionReceipt>, VoiceError> {
        let _index = self.transition_index()?;
        let state = try_lock(&self.row.state)?;
        if self.row.key.parent.is_some() || !action_matches(&state, action) {
            return Err(VoiceError::Unauthorized);
        }
        let plan = state.plan().ok_or(VoiceError::Unauthorized)?;
        if plan.identity.handshake() != handshake {
            return Err(VoiceError::Unauthorized);
        }
        let Some(claim) = &plan.transition else {
            return Ok(None);
        };
        if !Arc::ptr_eq(session, &claim.session) {
            return Err(VoiceError::Unauthorized);
        }
        let mut receipt = claim.snapshot(self.row.id, action, handshake, mono);
        if !self.row.key.lease.is_live() && receipt.state == 1 {
            receipt.state = 2;
            receipt.remaining = 0;
        }
        Ok(Some(receipt))
    }
    /// Latch only through the private current UI/selection admission, while that actual
    /// fence remains held. No Root/Clock/engine callback or owner destruction occurs here.
    pub(in crate::voice_handoff) fn claim_transition(
        &self,
        admission: &super::super::transition::Admission<'_>,
        action: Action,
        handshake: [u8; 32],
        mono: u64,
    ) -> Result<TransitionReceipt, VoiceError> {
        if !Arc::ptr_eq(&self.ledger, admission.ledger())
            || !self
                .row
                .key
                .matches(admission.selected(), self.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let index = self.transition_index()?;
        let mut state = try_lock(&self.row.state)?;
        if self.row.key.parent.is_some()
            || !action_matches(&state, action)
            || state.final_delivery().is_some()
            || state.final_phase.is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        if !matches!(state.body, Body::Drained(..))
            || !state.status.drained
            || !state.status.engine.settled
            || (state.status.engine.receipt.is_some() && !state.status.acknowledged)
        {
            return Err(VoiceError::Capacity);
        }
        for child in index.rows.iter().flatten().filter(|child| {
            child
                .key
                .parent
                .as_ref()
                .is_some_and(|parent| Arc::ptr_eq(parent, &self.row))
        }) {
            let child = try_lock(&child.state)?;
            if !(matches!(child.body, Body::ClosedPending)
                || (matches!(child.body, Body::Drained(..)) && child.status.drained))
            {
                return Err(VoiceError::Capacity);
            }
        }
        let plan = state.plan_mut().ok_or(VoiceError::Unauthorized)?;
        if plan.identity.handshake() != handshake {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(claim) = &plan.transition {
            if !Arc::ptr_eq(&claim.session, admission.session()) {
                return Err(VoiceError::Unauthorized);
            }
            return Ok(claim.snapshot(self.row.id, action, handshake, mono));
        }
        let control = plan.control.as_ref().ok_or(VoiceError::Unauthorized)?;
        let custody = plan.identity.confirmation_custody(control, mono)?;
        let mut hash = Sha256::new();
        hash.update(b"Mewtual original native voice key custody v1\0");
        hash.update(admission.session().0);
        hash.update(self.row.id.logical());
        hash.update(action.serial.to_be_bytes());
        hash.update(action.version.to_be_bytes());
        hash.update(action.input_tag);
        hash.update(handshake);
        hash.update(custody.epoch().to_be_bytes());
        hash.update(custody.message());
        hash.update(custody.object_hash());
        hash.update(custody.transition_until().to_be_bytes());
        admission.current()?;
        plan.transition = Some(Claim {
            session: admission.session().clone(),
            custody,
            receipt: hash.finalize().into(),
            released: false,
        });
        let receipt = plan
            .transition
            .as_ref()
            .expect("one original Claim")
            .snapshot(self.row.id, action, handshake, mono);
        state.session_collected = false; // a prior physical proof cannot cover this obligation
        Ok(receipt)
    }
    /// Exact original-session cleanup, available after End/lock. Released history remains
    /// immutable; a later Claim cannot reopen it or restore child-adoption authority.
    pub(in crate::voice_handoff) fn release_transition(
        &self,
        session: &Arc<SessionIdentity>,
        action: Action,
        handshake: [u8; 32],
        proof: [u8; 32],
        mono: u64,
    ) -> Result<TransitionReceipt, VoiceError> {
        let _index = self.transition_index()?;
        let mut state = try_lock(&self.row.state)?;
        if self.row.key.parent.is_some() || !action_matches(&state, action) {
            return Err(VoiceError::Unauthorized);
        }
        let plan = state.plan_mut().ok_or(VoiceError::Unauthorized)?;
        if plan.identity.handshake() != handshake {
            return Err(VoiceError::Unauthorized);
        }
        let claim = plan.transition.as_mut().ok_or(VoiceError::Unauthorized)?;
        if !Arc::ptr_eq(&claim.session, session) || claim.receipt != proof {
            return Err(VoiceError::Unauthorized);
        }
        claim.released = true;
        Ok(claim.snapshot(self.row.id, action, handshake, mono))
    }
}
fn try_lock<T>(lock: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, VoiceError> {
    match lock.try_lock() {
        Ok(guard) => Ok(guard),
        Err(std::sync::TryLockError::Poisoned(error)) => Ok(error.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => Err(VoiceError::Capacity),
    }
}
