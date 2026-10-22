//! Native low-side start under the actual current Root call and directory. This seam deliberately
//! exposes no serialized payload or engine permit; native/browser custody is a separate boundary.
use super::*;
use voice_v2::crypto_actions::{CryptoAction, CryptoResultReceipt};
use voice_v2::packets::Authority;
use voice_v2::presence::{
    ProtectedOpenedResult, ProtectedResponse, ProtectedSealedResult, ProtectedStart,
};

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Publish the original registered seal result once. Signing produces an inert private
    /// artifact before admission; local contention retains it for an exact retry without a
    /// second signature. The receipt grants no source send, browser handle or RTC authority.
    pub fn submit_voice_confirmation_result(
        &mut self,
        grant: &Arc<CallGrant>,
        action: &Arc<CryptoAction>,
        ciphertext: &[u8],
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
        let result = participant.join.submit_protected_confirmation_result(
            ProtectedSealedResult {
                actor,
                room: &room,
                action,
                ciphertext,
                claimed_tag,
            },
            &self.device,
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
        Ok(result)
    }
    /// Commit an exact native-registered decrypt result through the current Root origin. Caller
    /// supplied serials/receipts are not authority: both the call and action allocations must be
    /// recognized. This private integration seam creates neither browser IPC nor RTC delivery.
    pub fn commit_voice_confirmation_result(
        &mut self,
        grant: &Arc<CallGrant>,
        action: &Arc<CryptoAction>,
        plaintext: &[u8],
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
        let result = participant.join.commit_protected_confirmation_result(
            ProtectedOpenedResult {
                actor,
                room: &room,
                action,
                plaintext,
                claimed_tag,
            },
            &authority,
            clock.as_ref(),
            &mut self.rng,
            |_, _| {
                if call.is_live() && watch.is_live() {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            },
        )?;
        voice.endpoints.changed();
        Ok(result)
    }
    /// Start or recover one protected Hello for an already proven native pair. None always
    /// means the original request; Some(id) names a previous packet's exact successor. Only the
    /// recognized affine call owner may issue this command, including retries with equal joins.
    /// This reserves no IPC/browser registry and does not enable normal V2 network dispatch.
    pub fn start_voice_confirmation(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        predecessor: Option<voice_v2::Nonce>,
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
        let authority = Authority {
            context: &context,
            lease: &participant.lease,
            member: &|id| members.contains(&id),
        };
        // Selection is permanent even when capacity refuses. An existing ordinary-active pair
        // cannot be adopted here; no hidden fallback or replacement pair is created on refusal.
        participant
            .join
            .prepare_crypto_chain(handshake, &authority, clock.as_ref())?;
        let parent = participant.grant.clone();
        let call = participant.lease.clone();
        let watch = participant.watch.clone();
        let action = participant.join.start_protected_hello(
            ProtectedStart {
                actor,
                room: &room,
                handshake,
                predecessor,
            },
            &authority,
            clock,
            &mut self.rng,
            |_, _| {
                if parent.strong_count() > 0
                    && call.is_live()
                    && watch.is_live()
                    && room.lease.is_live()
                {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            },
        )?;
        voice.endpoints.changed();
        Ok(action)
    }
    /// Produce or recover Proof/Finish for an exact retained accepted native decrypt action.
    /// This shares the actual call/room boundary with start; serialized result fields cannot
    /// manufacture a trigger, and a superseded trigger cannot snapshot newer response state.
    pub fn respond_voice_confirmation(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        trigger: &Arc<CryptoAction>,
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
        let authority = Authority {
            context: &context,
            lease: &participant.lease,
            member: &|id| members.contains(&id),
        };
        let parent = participant.grant.clone();
        let call = participant.lease.clone();
        let watch = participant.watch.clone();
        let action = participant.join.respond_protected_confirmation(
            ProtectedResponse {
                actor,
                room: &room,
                handshake,
                trigger,
            },
            &authority,
            clock,
            &mut self.rng,
            |_, _| {
                if parent.strong_count() > 0
                    && call.is_live()
                    && watch.is_live()
                    && room.lease.is_live()
                {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            },
        )?;
        voice.endpoints.changed();
        Ok(action)
    }
}
