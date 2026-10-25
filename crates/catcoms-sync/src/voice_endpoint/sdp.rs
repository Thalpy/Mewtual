//! Trusted native first-PC intent factory. No renderer labels, SDP body or PC constructor
//! crosses this API. Native must independently bind its original Session/SDK/phase first.
use super::*;
use voice_v2::chain::sdp_handoff::EffectObserver;
use voice_v2::chain::EstablishedKeyCustody;
use voice_v2::packets::Authority;
use voice_v2::sdp::InitialSdpIntent;

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Lend one final SDP Input admission while this actual actor state remains
    /// exclusively borrowed. The trusted native callback is synchronous: it must acquire
    /// its final UI/Session/SDK/PC/row fence, commit, and retain any admitted input with
    /// separately paid output custody. It may not await or reenter ChannelSync.
    ///
    /// Original terminal/settlement history precedes current grant/MLS/room validation.
    /// An equal serialized action/intent cannot enter; native must authenticate its own
    /// registered allocation before calling. No reusable permission snapshot is returned.
    ///
    /// ```compile_fail
    /// use catcoms_rt::{CryptoRngCore, MeshTransport};
    /// use catcoms_sync::ChannelSync;
    /// use catcoms_sync::voice_v2::{CallGrant, crypto_actions::CryptoAction, sdp::InitialSdpIntent};
    /// use std::sync::Arc;
    /// fn escape<T: MeshTransport, R: CryptoRngCore>(
    ///     sync: &mut ChannelSync<T, R>, grant: &Arc<CallGrant>,
    ///     intent: &Arc<InitialSdpIntent>, action: &Arc<CryptoAction>,
    /// ) {
    ///     let _escaped = sync.with_voice_sdp_input_admission(grant, intent, action, |step| Ok(step));
    /// }
    /// ```
    pub fn with_voice_sdp_input_admission<U>(
        &mut self,
        grant: &Arc<CallGrant>,
        intent: &Arc<InitialSdpIntent>,
        action: &Arc<voice_v2::crypto_actions::CryptoAction>,
        publish: impl FnOnce(voice_v2::sdp::SdpInputAdmission<'_>) -> Result<U, VoiceError>,
    ) -> Result<U, VoiceError> {
        use voice_v2::crypto_actions::SdpInputRead;
        use voice_v2::sdp::SdpInputAdmission;
        let observed = action.observe_sdp_input(intent)?;
        if let Some(SdpInputRead::History(history)) = observed.known_history()? {
            return publish(SdpInputAdmission::history(history));
        }
        // These are fallible preparations, not final authority. Keep their failures until
        // commit so terminal publication racing this preparation can still recover history.
        // Actual MLS context is compared with the paid actor view even if its retained
        // EpochObservation has not yet been advanced by reconciliation.
        let context = self
            .validate_current_voice_call_grant(grant)
            .and_then(|()| self.voice_v2_context(grant.scope().room));
        let room = context
            .as_ref()
            .map_err(|error| *error)
            .and_then(|context| {
                self.voice_runtime
                    .as_ref()
                    .ok_or(VoiceError::Unauthorized)?
                    .directory
                    .authority(context.scope)
            });
        let current = (|| {
            let context = context.as_ref().map_err(|error| *error)?;
            let room = room.as_ref().map_err(|error| *error)?;
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
            {
                return Err(VoiceError::Unauthorized);
            }
            participant.join.prepare_sdp_input_current(
                actor,
                room,
                action,
                intent,
                context,
                &participant.lease,
                &participant.watch.lease,
            )
        })();
        // Actor origin preparation invokes its own injected Clock after the initial
        // history lookup. Refresh the SAME Action's samples last: an Open packet may
        // expire before the broader Ready/PC intent. No Clock runs after publish starts.
        let observed = action.observe_sdp_input(intent)?;
        publish(SdpInputAdmission::current(observed, current))
    }

    /// Prepare only HIGH's original applied Offer response. Root resolves the actual
    /// accepted Open/delivery and derives its OfferId internally; no renderer label or
    /// result receipt is accepted as a substitute for that fixed native allocation.
    pub fn prepare_voice_high_sdp_answer(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        intent: &Arc<InitialSdpIntent>,
        delivery: &Arc<voice_v2::sdp::SdpDelivery>,
        plaintext: &[u8],
    ) -> Result<Arc<voice_v2::crypto_actions::CryptoAction>, VoiceError> {
        self.prepare_voice_high_sdp_answer_inner(
            grant, handshake, intent, delivery, plaintext, None,
        )
    }
    /// Record the original invocation independently of Pair teardown. Current grant,
    /// membership, PC/Ready and final origin checks remain in the same underlying factory.
    pub fn prepare_voice_high_sdp_answer_recorded(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        intent: &Arc<InitialSdpIntent>,
        delivery: &Arc<voice_v2::sdp::SdpDelivery>,
        plaintext: &[u8],
        outcome: &mut voice_v2::sdp::publication::Writer,
    ) -> Result<Arc<voice_v2::crypto_actions::CryptoAction>, VoiceError> {
        outcome.enter(
            intent,
            Some(voice_v2::sdp::plaintext_commitment(plaintext)?),
        )?;
        let result = self.prepare_voice_high_sdp_answer_inner(
            grant,
            handshake,
            intent,
            delivery,
            plaintext,
            Some(&mut *outcome),
        );
        if let Err(reason) = &result {
            outcome.refused(*reason);
        }
        result
    }

    fn prepare_voice_high_sdp_answer_inner(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        intent: &Arc<InitialSdpIntent>,
        delivery: &Arc<voice_v2::sdp::SdpDelivery>,
        plaintext: &[u8],
        outcome: Option<&mut voice_v2::sdp::publication::Writer>,
    ) -> Result<Arc<voice_v2::crypto_actions::CryptoAction>, VoiceError> {
        self.validate_current_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let clock = self.voice_v2_clock();
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
            || participant.watch.scope != context.scope
            || !participant.watch.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let peer = participant.join.protected_ready_peer(handshake)?;
        let members = super::ready::ready_member_snapshot(actor, &context, peer, &[])?;
        let call = participant.lease.clone();
        let watch = participant.watch.lease.clone();
        participant.join.prepare_high_sdp_answer(
            actor,
            &room,
            handshake,
            intent,
            delivery,
            plaintext,
            &Authority {
                context: &context,
                lease: &call,
                member: &|id| members.contains(&Some(id)),
            },
            clock,
            &mut self.rng,
            |_, _| {
                if call.is_live() && watch.is_live() {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            },
            outcome,
        )
    }

    /// Commit the actual private SDK Open output. Root keeps only terminal metadata and an
    /// opaque delivery identity; plaintext remains in its separately paid browser inbox.
    pub fn submit_voice_sdp_open_result(
        &mut self,
        grant: &Arc<CallGrant>,
        action: &Arc<voice_v2::crypto_actions::CryptoAction>,
        output: voice_v2::sdp::SdpOpenOutput<'_>,
    ) -> Result<voice_v2::sdp::SdpOpenResult, VoiceError> {
        self.validate_current_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let clock = self.voice_v2_clock();
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
            || participant.watch.scope != context.scope
            || !participant.watch.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let peer = action.sdp_peer().ok_or(VoiceError::Unauthorized)?;
        let members = super::ready::ready_member_snapshot(actor, &context, peer, &[])?;
        let call = participant.lease.clone();
        let watch = participant.watch.lease.clone();
        let result = participant.join.submit_sdp_open_result(
            actor,
            &room,
            action,
            output,
            &Authority {
                context: &context,
                lease: &call,
                member: &|id| members.contains(&Some(id)),
            },
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

    /// Separate current admission before the browser applies the exact accepted SDP to its
    /// original PC. Repeated replies name one logical effect; SDK-started is latched there.
    pub fn admit_voice_sdp_delivery(
        &mut self,
        grant: &Arc<CallGrant>,
        action: &Arc<voice_v2::crypto_actions::CryptoAction>,
        delivery: &Arc<voice_v2::sdp::SdpDelivery>,
    ) -> Result<(), VoiceError> {
        self.transition_voice_sdp_delivery(grant, action, delivery, false)
    }

    /// Called only by the original private native/browser SDK completion path. This is
    /// causal history of a successful remote-description application, never drainage or
    /// proof that a renderer-supplied boolean/Offer/header may authorize an Answer.
    pub fn complete_voice_sdp_delivery(
        &mut self,
        grant: &Arc<CallGrant>,
        action: &Arc<voice_v2::crypto_actions::CryptoAction>,
        delivery: &Arc<voice_v2::sdp::SdpDelivery>,
    ) -> Result<(), VoiceError> {
        self.transition_voice_sdp_delivery(grant, action, delivery, true)
    }

    fn transition_voice_sdp_delivery(
        &mut self,
        grant: &Arc<CallGrant>,
        action: &Arc<voice_v2::crypto_actions::CryptoAction>,
        delivery: &Arc<voice_v2::sdp::SdpDelivery>,
        applied: bool,
    ) -> Result<(), VoiceError> {
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
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if !std::sync::Weak::ptr_eq(&participant.grant, &Arc::downgrade(grant))
            || participant.watch.scope != context.scope
            || !participant.watch.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let peer = action.sdp_peer().ok_or(VoiceError::Unauthorized)?;
        let members = super::ready::ready_member_snapshot(actor, &context, peer, &[])?;
        let transition = if applied {
            voice_v2::presence::LocalJoin::complete_sdp_delivery
        } else {
            voice_v2::presence::LocalJoin::admit_sdp_delivery
        };
        transition(
            &participant.join,
            actor,
            &room,
            action,
            delivery,
            &Authority {
                context: &context,
                lease: &participant.lease,
                member: &|id| members.contains(&Some(id)),
            },
            |_, _| {
                if participant.lease.is_live() && participant.watch.is_live() {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            },
        )
    }

    /// Submit only the original native SDP seal result. The packet stays in its original
    /// cache; Source queues it after real crypto settlement, using the same PC authority.
    pub fn submit_voice_sdp_result(
        &mut self,
        grant: &Arc<CallGrant>,
        action: &Arc<voice_v2::crypto_actions::CryptoAction>,
        ciphertext: &[u8],
        claimed_tag: [u8; 32],
    ) -> Result<voice_v2::crypto_actions::CryptoResultReceipt, VoiceError> {
        self.validate_current_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let clock = self.voice_v2_clock();
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
            || participant.watch.scope != context.scope
            || !participant.watch.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let peer = action.sdp_peer().ok_or(VoiceError::Unauthorized)?;
        let members = super::ready::ready_member_snapshot(actor, &context, peer, &[])?;
        let call = participant.lease.clone();
        let watch = participant.watch.lease.clone();
        let receipt = participant.join.submit_sdp_sealed_result(
            voice_v2::presence::ProtectedSealedResult {
                actor,
                room: &room,
                action,
                ciphertext,
                claimed_tag,
            },
            &self.device,
            &Authority {
                context: &context,
                lease: &call,
                // Preparation only needs membership of the exact original endpoints.
                // Never copy the whole MLS roster onto this bounded SDP worker's payment.
                member: &|id| members.contains(&Some(id)),
            },
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

    /// Native adapter's original LOW PC is the authority for the first complete Offer.
    /// Reserve the future Answer's shared Root namespace position before issuing any
    /// nonce/action. Exact action recovery remains possible when that namespace is full.
    pub fn prepare_voice_low_sdp_offer(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        intent: &Arc<InitialSdpIntent>,
        plaintext: &[u8],
    ) -> Result<Arc<voice_v2::crypto_actions::CryptoAction>, VoiceError> {
        self.prepare_voice_low_sdp_offer_inner(grant, handshake, intent, plaintext, None)
    }
    /// Record the original invocation independently of Pair teardown. Current grant,
    /// membership, PC/Ready and final origin checks remain in the same underlying factory.
    pub fn prepare_voice_low_sdp_offer_recorded(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        intent: &Arc<InitialSdpIntent>,
        plaintext: &[u8],
        outcome: &mut voice_v2::sdp::publication::Writer,
    ) -> Result<Arc<voice_v2::crypto_actions::CryptoAction>, VoiceError> {
        outcome.enter(
            intent,
            Some(voice_v2::sdp::plaintext_commitment(plaintext)?),
        )?;
        let result = self.prepare_voice_low_sdp_offer_inner(
            grant,
            handshake,
            intent,
            plaintext,
            Some(&mut *outcome),
        );
        if let Err(reason) = &result {
            outcome.refused(*reason);
        }
        result
    }

    fn prepare_voice_low_sdp_offer_inner(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        intent: &Arc<InitialSdpIntent>,
        plaintext: &[u8],
        outcome: Option<&mut voice_v2::sdp::publication::Writer>,
    ) -> Result<Arc<voice_v2::crypto_actions::CryptoAction>, VoiceError> {
        self.validate_current_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let clock = self.voice_v2_clock();
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
        let position = if participant.join.has_initial_sdp_seal(handshake) {
            None
        } else {
            Some(voice.endpoints.public.reserve_position()?)
        };
        participant.join.prepare_low_sdp_offer(
            actor,
            &room,
            handshake,
            intent,
            plaintext,
            position,
            voice.endpoints.public.namespace(),
            &Authority {
                context: &context,
                lease: &participant.lease,
                member: &|id| members.contains(&Some(id)),
            },
            clock,
            &mut self.rng,
            |_, _| Ok(()),
            outcome,
        )
    }

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

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Trusted native adapter only: resolve the current original grant and opaque PC intent.
    /// No token/header/SDP bytes from a renderer may choose the already admitted HIGH input.
    pub fn prepare_voice_high_sdp_open(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        intent: &Arc<InitialSdpIntent>,
    ) -> Result<Arc<voice_v2::crypto_actions::CryptoAction>, VoiceError> {
        if intent.local_is_low() {
            return Err(VoiceError::Unauthorized);
        }
        self.prepare_voice_sdp_open(grant, handshake, intent)
    }

    /// Resolve either original admitted Offer or Answer using the PC's native role.
    pub fn prepare_voice_sdp_open(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        intent: &Arc<InitialSdpIntent>,
    ) -> Result<Arc<voice_v2::crypto_actions::CryptoAction>, VoiceError> {
        self.prepare_voice_sdp_open_inner(grant, handshake, intent, None)
    }
    /// Record the original invocation independently of Pair teardown. Current grant,
    /// membership, PC/Ready and final origin checks remain in the same underlying factory.
    pub fn prepare_voice_sdp_open_recorded(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        intent: &Arc<InitialSdpIntent>,
        outcome: &mut voice_v2::sdp::publication::Writer,
    ) -> Result<Arc<voice_v2::crypto_actions::CryptoAction>, VoiceError> {
        outcome.enter(intent, None)?;
        let result =
            self.prepare_voice_sdp_open_inner(grant, handshake, intent, Some(&mut *outcome));
        if let Err(reason) = &result {
            outcome.refused(*reason);
        }
        result
    }

    fn prepare_voice_sdp_open_inner(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        intent: &Arc<InitialSdpIntent>,
        outcome: Option<&mut voice_v2::sdp::publication::Writer>,
    ) -> Result<Arc<voice_v2::crypto_actions::CryptoAction>, VoiceError> {
        self.validate_current_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let clock = self.voice_v2_clock();
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
        let prepare = if intent.local_is_low() {
            voice_v2::presence::LocalJoin::prepare_low_sdp_open
        } else {
            voice_v2::presence::LocalJoin::prepare_high_sdp_open
        };
        prepare(
            &mut participant.join,
            actor,
            &room,
            handshake,
            intent,
            &Authority {
                context: &context,
                lease: &participant.lease,
                member: &|id| members.contains(&Some(id)),
            },
            clock,
            |_, _| Ok(()),
            outcome,
        )
    }
}
