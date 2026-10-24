//! Trusted native HIGH readiness preparation. Both the packet inbox AND Root's combined
//! receipt position precede any Source permission. No renderer-supplied Ready is accepted.
use super::*;
use voice_v2::packets::Authority;

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Prepare an existing confirmed HIGH pair. Application work scheduling/browser phase
    /// consumers are separate; this native seam exposes no signature or unrestricted signer.
    pub fn prepare_voice_high_ready(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        helpers: &[DeviceId],
    ) -> Result<bool, VoiceError> {
        self.validate_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let clock = self.voice_v2_clock();
        let members: HashSet<_> = self.group.member_device_ids().into_iter().collect();
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        voice.endpoints.prune();
        voice.endpoints.public.prune(clock.as_ref());
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let room = voice.directory.authority(context.scope)?;
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
        if participant.join.has_first_offer(handshake) {
            return Ok(false);
        }
        let position = if participant
            .join
            .has_ready_position(handshake, voice.endpoints.public.namespace())
        {
            None
        } else {
            Some(voice.endpoints.public.reserve_position()?)
        };
        let call = participant.lease.clone();
        let watch = participant.watch.clone();
        participant.join.install_high_ready_receive(
            actor,
            &room,
            handshake,
            helpers,
            &self.device,
            &Authority {
                context: &context,
                lease: &call,
                member: &|id| members.contains(&id),
            },
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
        // Root's exclusive borrow prevents first-Offer/ordinary ingress until attachment.
        // Source refuses caches lacking this exact position, including all failed attempts.
        if let Some(position) = position {
            participant
                .join
                .attach_ready_position(handshake, position)?;
        }
        Ok(true)
    }
}
