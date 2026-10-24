//! Resolve original native SDP storage before any large body allocation or nonce debit.
//! This leaf admits capacity only. The eventual adapter must validate all encrypted PC,
//! SDK, action and purpose fields again after AEAD and before native/Root publication.
use super::transition::try_lock;
use super::*;
use crate::voice_handoff::channel::sdp::Permit;
impl Ledger {
    pub(in crate::voice_handoff) fn sdp_begin_intake(
        &self,
        selected: &SelectedHandoffOwner,
        session: &Arc<SessionIdentity>,
        authenticated: &AuthenticatedRequest,
        clock: Arc<dyn catcoms_rt::Clock>,
    ) -> Result<Permit, VoiceError> {
        // Bind the ACTUAL MAC-owning Session first, not its seed, slot or transcript. A
        // foreign authenticated Session must not even visit this owner's action index.
        if !authenticated.belongs_to(session)
            || authenticated.operation() != Operation::SdpSealBegin
            || authenticated.kind() != BodyKind::Sdp
        {
            return Err(VoiceError::Unauthorized);
        }
        let (phase, intent) = {
            let index = try_lock(&self.index)?;
            let id = RowId::from_logical(authenticated.logical())?;
            let row = index.rows[id.slot]
                .as_ref()
                .filter(|row| row.id == id)
                .ok_or(VoiceError::Unauthorized)?;
            if row.key.parent.is_some()
                || !matches!(row.key.purpose, Purpose::Derive(_))
                || !row.key.matches(selected, row.key.purpose)
                || !row.key.lease.is_live()
            {
                return Err(VoiceError::Unauthorized);
            }
            let state = try_lock(&row.state)?;
            let plan = state.plan().ok_or(VoiceError::Unauthorized)?;
            let transition = plan.transition.as_ref().ok_or(VoiceError::Unauthorized)?;
            let key = transition.key.as_ref().ok_or(VoiceError::Unauthorized)?;
            let storage = key.sdp.as_ref().ok_or(VoiceError::Unauthorized)?;
            let pc = storage.pc.as_ref().ok_or(VoiceError::Unauthorized)?;
            let intent = pc.intent().ok_or(VoiceError::Unauthorized)?;
            let phase = storage.phase.as_ref().ok_or(VoiceError::Unauthorized)?;
            if !Arc::ptr_eq(&transition.session, session)
                || transition.released
                || key.released
                || storage.closed
                || storage.worker()
                || storage.sdk_preparing
                || storage.sdk.as_ref().is_none_or(|sdk| sdk.released())
                || !key.custody.is_current()
                || !key.custody.contains_sdp_handoff(phase)
                || !intent.uses_phase(phase)
                || intent.generation() != 1
                || state.final_phase.is_some()
                || state.final_delivery().is_some()
            {
                return Err(VoiceError::Unauthorized);
            }
            (phase.clone(), intent.clone())
        };
        // Fixed funding may charge/allocate and later destroy credit owners. No native
        // Index/Row/UI/Coordinator guard is held here. Phase admission still fences End.
        Permit::original_begin(
            authenticated,
            session.clone(),
            phase,
            intent,
            selected.binding().scope,
            clock,
        )
    }
}
