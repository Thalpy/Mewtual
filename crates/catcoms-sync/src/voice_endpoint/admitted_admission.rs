//! Exact admitted residence -> inert neutral capacity. No role/peer/round may be
//! supplied by a renderer, and this API publishes no availability or protocol work.
use super::*;
use voice_v2::admission::AdmissionRpcPermit;
use voice_v2::admitted_admission::{
    AdmittedAdmissionObserver, AdmittedAdmissionPrepared, AdmittedAdmissionSession,
};
use voice_v2::call_activation::ActivationObserver;

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Prepare from the exact selected admitted key, preserving a freshly made
    /// registration even when its later neutral reservation is locally refused.
    /// Native must stage that owner and retry with its observer; None is a new
    /// registration request, not lost-reply recovery or canonical-generation proof.
    pub fn prepare_admitted_voice_admission_step(
        &mut self,
        grant: &Arc<CallGrant>,
        activation: &ActivationObserver,
        original: Option<&AdmittedAdmissionObserver>,
        generation: u64,
        payment: &AdmissionRpcPermit,
    ) -> Result<AdmittedAdmissionPrepared, VoiceError> {
        if generation == 0 {
            return Err(VoiceError::Bounds);
        }
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        if !activation.matches_grant(grant) {
            return Err(VoiceError::Unauthorized);
        }
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        let context = admitted_presence::current(voice, &self.blob_fetch_instance, activation)?;
        if !payment.matches(&voice.resources, context.scope) {
            return Err(VoiceError::Unauthorized);
        }
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let presence = voice
            .endpoints
            .activation
            .as_mut()
            .and_then(|held| held.presence.as_mut())
            .ok_or(VoiceError::Unauthorized)?
            .get_mut();
        let registration = if original.is_none() {
            Some(presence.prepare_admission_session(
                actor,
                &context,
                activation,
                &mut self.rng,
                || {},
            )?)
        } else {
            None
        };
        let fresh = registration
            .as_ref()
            .map(AdmittedAdmissionSession::observer);
        let observed = original
            .or(fresh.as_ref())
            .ok_or(VoiceError::Unauthorized)?;
        let mut capacity =
            presence.reserve_round_preclaim(actor, &context, activation, observed, generation);
        // Return the SAME registration on a late retryable refusal too. Native
        // stages historical ownership, never infers currentness from that return.
        if let Err(error) = admitted_presence::current(voice, &self.blob_fetch_instance, activation)
        {
            capacity = Err(error);
        }
        Ok(AdmittedAdmissionPrepared::new(registration, capacity))
    }
}
