//! Trusted native first-PC intent factory. No renderer labels, SDP body or PC constructor
//! crosses this API. Native must independently bind its original Session/SDK/phase first.
use super::*;
use voice_v2::chain::sdp_handoff::EffectObserver;
use voice_v2::chain::EstablishedKeyCustody;
use voice_v2::packets::Authority;
use voice_v2::sdp::InitialSdpIntent;

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Mint/recover the exact original first intent. Revalidate the current MLS/Actor and
    /// room issuer before entering LocalJoin; its own final origin transaction repeats
    /// original key/phase/Ready currency after all fallible preparation.
    pub fn prepare_voice_initial_sdp_intent(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        key: EstablishedKeyCustody,
        effect: EffectObserver,
        generation: u64,
    ) -> Result<Arc<InitialSdpIntent>, VoiceError> {
        self.validate_current_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        voice.endpoints.prune();
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let room = voice.directory.authority(context.scope)?;
        let participant = voice
            .endpoints
            .participant
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        if !std::sync::Weak::ptr_eq(&participant.grant, &Arc::downgrade(grant))
            || !participant.watch.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let peer = participant.join.protected_ready_peer(handshake)?;
        let members = super::ready::ready_member_snapshot(actor, &context, peer, &[])?;
        participant.join.prepare_initial_sdp_intent(
            actor,
            &room,
            handshake,
            &key,
            effect,
            generation,
            &Authority {
                context: &context,
                lease: &participant.lease,
                member: &|id| members.contains(&Some(id)),
            },
        )
    }
}
