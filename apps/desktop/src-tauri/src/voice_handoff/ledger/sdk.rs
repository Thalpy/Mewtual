//! Separately released browser SDK custody. Closing native storage or collecting its HTTP
//! Session says nothing about a browser promise. This record owns fixed provenance only.
use super::super::channel::{PhysicalHold, Session};
use super::super::requests::sdk::Request;
use super::transition::try_lock;
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::EffectObserver;
use sha2::{Digest, Sha256};

pub(super) struct Claim {
    generation: u64,
    proof: [u8; 32],
    released: AtomicBool,
    // Last: pays this fixed wrapper through actual destruction, independently of Phase.
    // It owns no Row, Session, browser promise, Workflow or variable reservation.
    effect: EffectObserver,
}
impl Claim {
    pub(super) fn released(&self) -> bool {
        self.released.load(Ordering::Acquire)
    }
    fn state(&self) -> u8 {
        if self.released() {
            3
        } else if self.effect.is_live() {
            1
        } else {
            2
        }
    }
}
pub(in crate::voice_handoff) struct Receipt {
    pub row: RowId,
    pub action: super::super::requests::Action,
    pub handshake: [u8; 32],
    pub key: [u8; 32],
    pub sequence: u64,
    pub storage: [u8; 32],
    pub generation: u64,
    pub proof: [u8; 32],
    pub state: u8,
}
fn snapshot(row: RowId, request: &Request, claim: &Claim) -> Receipt {
    Receipt {
        row,
        action: request.base.action,
        handshake: request.base.handshake,
        key: request.base.key,
        sequence: request.base.sequence,
        storage: request.storage,
        generation: claim.generation,
        proof: claim.proof,
        state: claim.state(),
    }
}
fn storage<'a>(
    row: &ResolvedRow,
    state: &'a State,
    session: &Arc<SessionIdentity>,
    request: &Request,
) -> Result<&'a super::sdp::Claim, VoiceError> {
    let key = super::sdp::claim(row, state, session, &request.base)?;
    let storage = key.sdp.as_ref().ok_or(VoiceError::Unauthorized)?;
    if storage.proof != request.storage {
        return Err(VoiceError::Unauthorized);
    }
    Ok(storage)
}
/// One unpublished candidate at a time, paid by Phase's explicit fixed SDK allowance.
/// The actual original Row survives this worker; it cannot disappear on physical GC.
pub(in crate::voice_handoff) struct Preparation {
    row: Arc<Row>,
    observer: Option<EffectObserver>,
    proof: [u8; 32],
    candidate: Option<Arc<Claim>>,
}
impl Preparation {
    pub(in crate::voice_handoff) fn allocate(&mut self) -> Result<(), VoiceError> {
        if self.candidate.is_some() {
            return Err(VoiceError::Unauthorized);
        }
        let observer = self.observer.take().ok_or(VoiceError::Unauthorized)?;
        if !observer.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        self.candidate = Some(Arc::new(Claim {
            generation: 1,
            proof: self.proof,
            released: AtomicBool::new(false),
            effect: observer,
        }));
        Ok(())
    }
}
impl Drop for Preparation {
    fn drop(&mut self) {
        drop(self.candidate.take());
        drop(self.observer.take());
        let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        let storage = state
            .plan_mut()
            .and_then(|plan| plan.transition.as_mut())
            .and_then(|claim| claim.key.as_mut())
            .and_then(|key| key.sdp.as_mut())
            .expect("same SDK preparation storage intent");
        storage.sdk_preparing = false;
        drop(state);
        self.row.changed.notify_waiters();
    }
}
impl ResolvedRow {
    /// Test-only observation of the actual installed obligation; this neither constructs
    /// custody nor mutates release state. Used after real HTTP/browser admission.
    #[cfg(test)]
    pub(in crate::voice_handoff) fn sdk_test_released(&self) -> Option<bool> {
        let state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        Some(
            state
                .plan()?
                .transition
                .as_ref()?
                .key
                .as_ref()?
                .sdp
                .as_ref()?
                .sdk
                .as_ref()?
                .released(),
        )
    }
    /// Recover one immutable original obligation before current-authority checks. An End,
    /// expired old Claim, or collected physical position cannot manufacture its Release.
    pub(in crate::voice_handoff) fn recover_sdk(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Request,
    ) -> Result<Option<Receipt>, VoiceError> {
        let _index = self.transition_index()?;
        let state = try_lock(&self.row.state)?;
        Ok(storage(self, &state, session, request)?
            .sdk
            .as_ref()
            .map(|sdk| snapshot(self.row.id, request, sdk)))
    }
    /// The exact physical position and sdk_preparing bit bound candidate allocation to one.
    /// Construction is inert and paid; no browser SDK work is authorized by this method.
    pub(in crate::voice_handoff) fn begin_sdk(
        &self,
        admission: &super::super::transition::Admission<'_>,
        physical: &PhysicalHold,
        session: &Arc<Session>,
        request: &Request,
    ) -> Result<Option<Preparation>, VoiceError> {
        if !physical.belongs_to(session)
            || !Arc::ptr_eq(session.identity(), admission.session())
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
        let key = super::sdp::claim(self, &state, admission.session(), &request.base)?;
        let storage = storage(self, &state, admission.session(), request)?;
        if storage.sdk.is_some() {
            return Ok(None);
        }
        if key.released
            || !key.custody.is_current()
            || storage.closed
            || state.final_delivery().is_some()
            || state.final_phase.is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        if storage.sdk_preparing {
            return Err(VoiceError::Capacity);
        }
        let observer = storage.phase.as_ref().ok_or(VoiceError::Capacity)?.effect();
        if !observer.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        // Capture only native-validated identities. No caller can relabel this candidate
        // by supplying another Session or request when allocating its fixed wrapper.
        let mut hash = Sha256::new();
        hash.update(b"Mewtual native voice SDK custody v1\0");
        hash.update(admission.session().0);
        hash.update(self.row.id.logical());
        hash.update(request.base.action.serial.to_be_bytes());
        hash.update(request.base.action.version.to_be_bytes());
        hash.update(request.base.action.input_tag);
        hash.update(request.base.handshake);
        hash.update(request.base.key);
        hash.update(request.base.sequence.to_be_bytes());
        hash.update(request.storage);
        hash.update(1u64.to_be_bytes());
        let proof = hash.finalize().into();
        admission.current()?;
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
            .sdk_preparing = true;
        Ok(Some(Preparation {
            row: self.row.clone(),
            observer: Some(observer),
            proof,
            candidate: None,
        }))
    }
    /// Final pure native admission, then one infallible move. A fixed record, native boolean
    /// or historical SDP receipt alone never grants permission to create a PC or enter RTC.
    pub(in crate::voice_handoff) fn attach_sdk(
        &self,
        admission: &super::super::transition::Admission<'_>,
        request: &Request,
        preparation: &mut Preparation,
    ) -> Result<Receipt, VoiceError> {
        if !Arc::ptr_eq(&self.row, &preparation.row)
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
        let key = super::sdp::claim(self, &state, admission.session(), &request.base)?;
        let storage = storage(self, &state, admission.session(), request)?;
        if key.released
            || !key.custody.is_current()
            || storage.closed
            || !storage.sdk_preparing
            || storage.sdk.is_some()
            || state.final_delivery().is_some()
            || state.final_phase.is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        let candidate = preparation
            .candidate
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if !candidate.effect.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        admission.current()?;
        let storage = state
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
        storage.sdk = preparation.candidate.take();
        let receipt = snapshot(self.row.id, request, storage.sdk.as_ref().unwrap());
        state.session_collected = false;
        Ok(receipt)
    }
    /// Original-session cleanup remains valid after End/lock/storage closure. Only this
    /// authenticated actual Release sets released; physical collection and expiry cannot.
    pub(in crate::voice_handoff) fn release_sdk(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Request,
        generation: u64,
        proof: [u8; 32],
    ) -> Result<Receipt, VoiceError> {
        let _index = self.transition_index()?;
        let state = try_lock(&self.row.state)?;
        let sdk = storage(self, &state, session, request)?
            .sdk
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if sdk.generation != generation || sdk.proof != proof {
            return Err(VoiceError::Unauthorized);
        }
        sdk.released.store(true, Ordering::Release);
        Ok(snapshot(self.row.id, request, sdk))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_and_sdk_claim_fit_the_original_fixed_metadata_allowance() {
        let bytes = std::mem::size_of::<Claim>()
            + 2 * std::mem::size_of::<usize>()
            + std::mem::size_of::<Preparation>()
            + std::mem::size_of::<Sha256>();
        assert!(
            bytes <= catcoms_sync::voice_v2::chain::sdp_handoff::NATIVE_SDK_METADATA_BYTES,
            "native SDK fixed graph {bytes}"
        );
    }
}
