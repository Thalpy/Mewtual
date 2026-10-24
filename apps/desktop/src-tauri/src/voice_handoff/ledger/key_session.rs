//! A separately released key-retention obligation nested in the original finite Claim.
//! Neither this native history nor its Root proof owns old progress or authorizes RTC.
use super::super::requests::Action;
use super::transition::{action_matches, try_lock, Claim};
use super::*;
use catcoms_sync::voice_v2::crypto_actions::EstablishedKeyCustody;
use sha2::{Digest, Sha256};

pub(super) struct KeyClaim {
    pub(super) custody: EstablishedKeyCustody,
    pub(super) receipt: [u8; 32],
    pub(super) released: bool,
    pub(super) sdp: Option<super::sdp::Claim>,
}
pub(in crate::voice_handoff) struct KeySessionReceipt {
    pub row: RowId,
    pub action: Action,
    pub handshake: [u8; 32],
    pub epoch: u64,
    pub message: [u8; 32],
    pub hash: [u8; 32],
    pub claim: [u8; 32],
    pub receipt: [u8; 32],
    // Current is ONLY a key-retention snapshot, never a standing effect permission.
    pub state: u8,
}
impl Claim {
    pub(super) fn all_released(&self) -> bool {
        self.released
            && self.key.as_ref().is_none_or(|key| {
                key.released
                    && key
                        .sdp
                        .as_ref()
                        .is_none_or(super::sdp::Claim::whole_drained)
            })
    }
    fn key_snapshot(
        &self,
        row: RowId,
        action: Action,
        handshake: [u8; 32],
    ) -> Option<KeySessionReceipt> {
        self.key.as_ref().map(|key| KeySessionReceipt {
            row,
            action,
            handshake,
            epoch: self.custody.epoch(),
            message: self.custody.message(),
            hash: self.custody.object_hash(),
            claim: self.receipt,
            receipt: key.receipt,
            state: if key.released {
                3
            } else if key.custody.is_current() {
                1
            } else {
                2
            },
        })
    }
}
impl ResolvedRow {
    /// Fixture-only independent oracle for actual opcode11 native publication. Metadata,
    /// queued work or a browser boolean cannot satisfy the real TCP acceptance scenario.
    #[cfg(test)]
    pub(in crate::voice_handoff) fn has_established_key_claim(&self) -> bool {
        let _index = self.indexed().unwrap();
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state
            .plan()
            .and_then(|plan| plan.transition.as_ref())
            .and_then(|claim| claim.key.as_ref())
            .is_some_and(|key| !key.released && key.custody.is_current())
    }

    /// Same-session immutable recovery precedes old deadline and UI checks. An uncertain
    /// successful adoption can be recovered after End without reconstructing browser keys.
    pub(in crate::voice_handoff) fn recover_key_session(
        &self,
        session: &Arc<SessionIdentity>,
        action: Action,
        handshake: [u8; 32],
        proof: [u8; 32],
    ) -> Result<Option<KeySessionReceipt>, VoiceError> {
        let _index = self.transition_index()?;
        let state = try_lock(&self.row.state)?;
        if self.row.key.parent.is_some() || !action_matches(&state, action) {
            return Err(VoiceError::Unauthorized);
        }
        let plan = state.plan().ok_or(VoiceError::Unauthorized)?;
        let claim = plan.transition.as_ref().ok_or(VoiceError::Unauthorized)?;
        if plan.identity.handshake() != handshake
            || !Arc::ptr_eq(session, &claim.session)
            || claim.receipt != proof
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(claim.key_snapshot(self.row.id, action, handshake))
    }
    /// Final synchronous UI→Coordinator→Route→Index→parent admission. The original Root
    /// Finish proof is already in Claim; converting it requires no asynchronous Root lookup.
    /// Check LOCAL drainage, not whole-plan drainage, which includes this very obligation.
    pub(in crate::voice_handoff) fn adopt_key_session(
        &self,
        admission: &super::super::transition::Admission<'_>,
        action: Action,
        handshake: [u8; 32],
        proof: [u8; 32],
        mono: u64,
    ) -> Result<KeySessionReceipt, VoiceError> {
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
        let claim = plan.transition.as_mut().ok_or(VoiceError::Unauthorized)?;
        if !Arc::ptr_eq(&claim.session, admission.session()) || claim.receipt != proof {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(receipt) = claim.key_snapshot(self.row.id, action, handshake) {
            return Ok(receipt);
        }
        if claim.released {
            return Err(VoiceError::Unauthorized);
        }
        let custody = plan
            .identity
            .established_key_custody(&claim.custody, mono)?;
        let mut hash = Sha256::new();
        hash.update(b"Mewtual established native voice key custody v1\0");
        hash.update(admission.session().0);
        hash.update(self.row.id.logical());
        hash.update(action.serial.to_be_bytes());
        hash.update(action.version.to_be_bytes());
        hash.update(action.input_tag);
        hash.update(handshake);
        hash.update(proof);
        hash.update(claim.custody.epoch().to_be_bytes());
        hash.update(claim.custody.message());
        hash.update(claim.custody.object_hash());
        admission.current()?;
        if !custody.is_current() {
            return Err(VoiceError::Unauthorized);
        }
        // Inline fixed ownership is covered by the parent's measured existing row payment.
        // No progress owner/destructor or new allocation enters this infallible tail.
        claim.key = Some(KeyClaim {
            custody,
            receipt: hash.finalize().into(),
            released: false,
            sdp: None,
        });
        let receipt = claim
            .key_snapshot(self.row.id, action, handshake)
            .expect("one native key adoption");
        state.session_collected = false;
        Ok(receipt)
    }
    /// Cleanup remains available on the original channel after lock/End/old expiry. A late
    /// Adopt recovers Released; it cannot reopen this same obligation or its old Claim.
    pub(in crate::voice_handoff) fn release_key_session(
        &self,
        session: &Arc<SessionIdentity>,
        action: Action,
        handshake: [u8; 32],
        claim_proof: [u8; 32],
        proof: [u8; 32],
    ) -> Result<KeySessionReceipt, VoiceError> {
        let (receipt, close) = {
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
            if !Arc::ptr_eq(&claim.session, session) || claim.receipt != claim_proof {
                return Err(VoiceError::Unauthorized);
            }
            let key = claim.key.as_mut().ok_or(VoiceError::Unauthorized)?;
            if key.receipt != proof {
                return Err(VoiceError::Unauthorized);
            }
            key.released = true;
            let receipt = claim
                .key_snapshot(self.row.id, action, handshake)
                .expect("original released key claim");
            // Construct the affine cleanup ticket only in this final infallible tail. It
            // moves out before native guards drop; no destructor/notification runs inside.
            (
                receipt,
                claim
                    .key
                    .as_mut()
                    .unwrap()
                    .sdp
                    .as_mut()
                    .and_then(super::sdp::Claim::request_close),
            )
        };
        drop(close);
        Ok(receipt)
    }
}
