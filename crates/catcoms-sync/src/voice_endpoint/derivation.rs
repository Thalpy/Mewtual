//! Native derivation joins the proven pair to its original Root actor and affine call. This
//! returns a charged action, never browser key material, and does not enable live dispatch.
use super::*;
use voice_v2::crypto_actions::{CryptoAction, CryptoResultReceipt};
use voice_v2::packets::Authority;
use voice_v2::presence::{ProtectedDerivation, ProtectedDerivationResult};

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Produce or recover row zero for a proven pair under the actual current native call.
    /// The browser must retain its original private join key; recovering a historical action
    /// or Derived receipt cannot recreate that key or confer fresh RTC/signalling authority.
    pub fn prepare_voice_derivation(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        self.validate_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let room = self.voice_room_authority(context.scope.room)?;
        let clock = self.voice_v2_clock();
        let members: HashSet<_> = self.group.member_device_ids().into_iter().collect();
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        voice.endpoints.prune();
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let participant = voice
            .endpoints
            .participant
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        if !std::sync::Weak::ptr_eq(&participant.grant, &Arc::downgrade(grant))
            || participant.watch.scope != context.scope
            || !participant.watch.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let call = participant.lease.clone();
        let watch = participant.watch.lease.clone();
        let authority = Authority {
            context: &context,
            lease: &call,
            member: &|id| members.contains(&id),
        };
        let action = participant.join.prepare_protected_derivation(
            ProtectedDerivation {
                actor,
                room: &room,
                handshake,
            },
            &authority,
            clock,
            |_, _| {
                if call.is_live() && watch.is_live() {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            },
        )?;
        voice.endpoints.changed();
        Ok(action)
    }

    /// Admit the empty Derive completion marker for the exact registered action. Native result
    /// history is metadata, not proof that a replacement browser owns the original derived keys.
    /// First readiness repeats original actor/session/room/member/epoch checks at publication.
    pub fn commit_voice_derivation_result(
        &mut self,
        grant: &Arc<CallGrant>,
        action: &Arc<CryptoAction>,
        claimed_tag: [u8; 32],
    ) -> Result<CryptoResultReceipt, VoiceError> {
        self.validate_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let room = self.voice_room_authority(context.scope.room)?;
        let clock = self.voice_v2_clock();
        let members: HashSet<_> = self.group.member_device_ids().into_iter().collect();
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        voice.endpoints.prune();
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let participant = voice
            .endpoints
            .participant
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        if !std::sync::Weak::ptr_eq(&participant.grant, &Arc::downgrade(grant))
            || participant.watch.scope != context.scope
            || !participant.watch.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let call = participant.lease.clone();
        let watch = participant.watch.lease.clone();
        let authority = Authority {
            context: &context,
            lease: &call,
            member: &|id| members.contains(&id),
        };
        let receipt = participant.join.commit_protected_derivation_result(
            ProtectedDerivationResult {
                actor,
                room: &room,
                action,
                claimed_tag,
            },
            &authority,
            clock.as_ref(),
            |_, _| {
                if call.is_live() && watch.is_live() {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            },
        )?;
        voice.endpoints.changed();
        Ok(receipt)
    }
}
