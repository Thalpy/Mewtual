//! Native-only admission preparation. No renderer peer/role can enter these APIs.
//! Calls only observe/reserve original capacity: signed agreement and native-row
//! admission are separate, unimplemented effect boundaries.
use super::*;
use voice_v2::admission::{AdmissionSessionObserver, RootAdmissionSession, RootHighPreclaim};

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Prepare one paid Actor invocation. Native must stage the returned registration
    /// canonically even if HIGH reservation failed or its generation was cancelled.
    /// No first native permission is implied; final attachment still orders against UI
    /// lock/End. A lost/ambiguous invocation cannot be replaced by another mint.
    pub fn prepare_voice_admission_step(
        &mut self,
        grant: &Arc<CallGrant>,
        original: Option<&AdmissionSessionObserver>,
        generation: u64,
        payment: &voice_v2::admission::AdmissionRpcPermit,
    ) -> Result<voice_v2::admission::AdmissionPrepared, VoiceError> {
        if generation == 0 {
            return Err(VoiceError::Bounds);
        }
        self.validate_current_voice_call_grant(grant)?;
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if !payment.matches(&voice.resources, grant.scope()) {
            return Err(VoiceError::Unauthorized);
        }
        let registration = if original.is_none() {
            Some(self.prepare_voice_admission_session(grant)?)
        } else {
            None
        };
        let fresh = registration.as_ref().map(RootAdmissionSession::observer);
        let observed = original
            .or(fresh.as_ref())
            .ok_or(VoiceError::Unauthorized)?;
        let capacity = self.reserve_voice_high_preclaim(grant, observed, generation);
        Ok(voice_v2::admission::AdmissionPrepared::new(
            registration,
            capacity,
        ))
    }

    /// Mint one original admission registration for the actual current participant.
    /// Native must install Pending before calling and recover this same returned owner
    /// across remount/lost replies. Calling again intentionally creates a NEW identity.
    pub fn prepare_voice_admission_session(
        &mut self,
        grant: &Arc<CallGrant>,
    ) -> Result<RootAdmissionSession, VoiceError> {
        self.prepare_voice_admission_session_at(grant, || {})
    }

    // Private common path. The genuine Root fixture uses only this allocation
    // boundary; shipping callers cannot supply a callback or replace the Root RNG.
    pub(super) fn prepare_voice_admission_session_at(
        &mut self,
        grant: &Arc<CallGrant>,
        before_final: impl FnOnce(),
    ) -> Result<RootAdmissionSession, VoiceError> {
        self.validate_current_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let participant = voice
            .endpoints
            .participant
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if !std::sync::Weak::ptr_eq(&participant.grant, &Arc::downgrade(grant))
            || participant.watch.scope != context.scope
            || !participant.watch.is_live()
            || !participant.lease.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        participant.join.prepare_admission_session(
            actor,
            &context,
            &participant.watch.lease,
            &mut self.rng,
            before_final,
        )
    }

    /// Reserve HIGH packet layout using the CURRENT original LocalJoin's pools.
    /// With no counterpart this proves no DeviceId orientation and emits nothing.
    /// Root's capacity does not reserve a native ledger row or start a deadline.
    pub fn reserve_voice_high_preclaim(
        &self,
        grant: &Arc<CallGrant>,
        original: &AdmissionSessionObserver,
        generation: u64,
    ) -> Result<RootHighPreclaim, VoiceError> {
        self.validate_current_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let participant = voice
            .endpoints
            .participant
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if !std::sync::Weak::ptr_eq(&participant.grant, &Arc::downgrade(grant))
            || participant.watch.scope != context.scope
            || !participant.watch.is_live()
            || !participant.lease.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        participant.join.reserve_high_admission_preclaim(
            actor,
            &context,
            &participant.watch.lease,
            original,
            generation,
        )
    }
}
