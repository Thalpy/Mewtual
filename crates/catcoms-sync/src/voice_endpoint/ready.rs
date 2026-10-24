//! Trusted native HIGH readiness preparation. Both the packet inbox AND Root's combined
//! receipt position precede any Source permission. No renderer-supplied Ready is accepted.
use super::*;
use voice_v2::packets::Authority;

/// Snapshot only identities this factory can query, from the already-paid exact Actor
/// roster. Never allocate a full MLS Vec/HashSet on the small native producer payment.
/// This is not an epoch lease; original/final Actor gates still validate the current view.
fn ready_member_snapshot(
    actor: &voice_v2::runtime::ActorOwner,
    context: &voice_v2::Context,
    peer: DeviceId,
    helpers: &[DeviceId],
) -> Result<[Option<DeviceId>; 4], VoiceError> {
    if helpers.len() > 2 {
        return Err(VoiceError::Bounds);
    }
    let candidates = [
        Some(context.local),
        Some(peer),
        helpers.first().copied(),
        helpers.get(1).copied(),
    ];
    actor.try_with_current_members(context, |member| {
        Ok(candidates.map(|candidate| candidate.filter(|id| member(*id))))
    })
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Native application preparation with a bounded, authenticated helper choice. A member
    /// need not join this call to bridge it. This advertises no protocol-version capability;
    /// actual V2 endpoint/helper dispatch still authenticates and admits every packet.
    pub fn prepare_voice_high_ready_connected(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
    ) -> Result<bool, VoiceError> {
        self.validate_current_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let participant = voice
            .endpoints
            .participant
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if !std::sync::Weak::ptr_eq(&participant.grant, &Arc::downgrade(grant))
            || !participant.watch.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let (peer, frozen) = actor.try_with_current_members(&context, |member| {
            participant.join.high_ready_helpers(
                handshake,
                &Authority {
                    context: &context,
                    lease: &participant.lease,
                    member,
                },
            )
        })?;
        let helpers = match frozen {
            Some(helpers) => helpers,
            None => self.voice_ready_helpers(&context, peer)?,
        };
        match helpers {
            [None, None] => self.prepare_voice_high_ready(grant, handshake, &[]),
            [Some(a), None] => self.prepare_voice_high_ready(grant, handshake, &[a]),
            [Some(a), Some(b)] => self.prepare_voice_high_ready(grant, handshake, &[a, b]),
            [None, Some(_)] => Err(VoiceError::Malformed),
        }
    }

    /// Do not call voice_bound_routes here: its full connection/path snapshot and roster
    /// maps can exceed the native producer's small payment. Preserve its authentication
    /// checks with fixed output, one captured lease, and the already-paid Actor roster.
    fn voice_ready_helpers(
        &self,
        context: &voice_v2::Context,
        endpoint: DeviceId,
    ) -> Result<[Option<DeviceId>; 2], VoiceError> {
        let mut helpers = [None, None];
        if !context.indirect || context.scope.policy.is_none() {
            return Ok(helpers);
        }
        if self.member_peers.len() > MAX_PEER_RECORDS || self.peer_records.len() > MAX_PEER_RECORDS
        {
            return Err(VoiceError::Capacity);
        }
        let actor = self.voice_transport_actor()?;
        for proof in &self.member_peers {
            if !proof.bound
                || proof.device == context.local
                || proof.device == endpoint
                || helpers.contains(&Some(proof.device))
                || self
                    .peer_records
                    .get(&proof.device)
                    .is_none_or(|record| record.peer_id != *proof.peer.as_bytes())
            {
                continue;
            }
            // Transport can run its own locks/callbacks; capture it BEFORE Group/Policy.
            let Some(connection) = self.transport.connection_lease(proof.peer) else {
                continue;
            };
            let eligible = actor.try_with_current_members(context, |member| {
                Ok(connection.is_live()
                    && member(proof.device)
                    && !self.peer_records.iter().any(|(other, record)| {
                        *other != proof.device
                            && record.peer_id == *proof.peer.as_bytes()
                            && member(*other)
                    }))
            })?;
            if !eligible {
                continue;
            }
            // Deterministic smallest-two selection independent of VecDeque/HashMap order.
            if helpers[0].is_none_or(|first| proof.device < first) {
                helpers[1] = helpers[0];
                helpers[0] = Some(proof.device);
            } else if helpers[1].is_none_or(|second| proof.device < second) {
                helpers[1] = Some(proof.device);
            }
        }
        Ok(helpers)
    }
    #[cfg(test)]
    pub(crate) fn occupy_voice_receipt_namespace_for_test(
        &mut self,
    ) -> Vec<voice_v2::negotiation::namespace::ReceiptPosition> {
        self.voice_runtime
            .as_mut()
            .expect("native fixture actor")
            .endpoints
            .public
            .occupy_namespace_for_test()
    }
    /// Prepay native LOW successor capacity after its original Finish, without accepting
    /// Ready or stopping Finish. Application SDP scheduling remains separate.
    pub fn prepay_voice_low_sdp(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
    ) -> Result<(), VoiceError> {
        self.advance_voice_low_sdp(grant, handshake, true)
            .map(|_| ())
    }
    /// Retry affine LOW extraction after exact native Ready. `true` observes actual old
    /// confirmation drainage only; it is not browser adoption, SDP activation or RTC success.
    pub fn prepare_voice_low_sdp(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
    ) -> Result<bool, VoiceError> {
        self.advance_voice_low_sdp(grant, handshake, false)
    }
    fn advance_voice_low_sdp(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        prepay_only: bool,
    ) -> Result<bool, VoiceError> {
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
        let call = participant.lease.clone();
        let watch = participant.watch.clone();
        let peer = participant.join.protected_ready_peer(handshake)?;
        // LOW's Authority queries only local/peer; accepted helper membership is checked
        // by its actual Ready receipt and final origin.contains_members transaction.
        let members = ready_member_snapshot(actor, &context, peer, &[])?;
        let authority = Authority {
            context: &context,
            lease: &call,
            member: &|id| members.contains(&Some(id)),
        };
        if prepay_only {
            participant
                .join
                .prepay_low_sdp_successor(actor, handshake, &authority)?;
            return Ok(false);
        }
        let parent = participant.grant.clone();
        let successor = participant.join.prepare_low_sdp_successor(
            actor,
            &room,
            handshake,
            &authority,
            |_, _| {
                if parent.strong_count() > 0 && call.is_live() && watch.is_live() {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            },
        )?;
        Ok(successor.confirmation_drained())
    }
    /// Queue only the original prepared HIGH Ready. Native KeyClaim maintenance uses this
    /// seam after actual core extraction; it exposes neither signing nor renderer input.
    /// `true` means the exact Source record already existed, never another retry allowance.
    pub fn queue_voice_high_ready(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
    ) -> Result<bool, VoiceError> {
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
            || !participant.watch.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let authority = Authority {
            context: &context,
            lease: &participant.lease,
            member: &|id| {
                actor
                    .with_current_members(&context, |member| Ok(member(id)))
                    .unwrap_or(false)
            },
        };
        let (object, activity) = participant.join.ready_source_activity(
            handshake,
            voice.endpoints.public.namespace(),
            &authority,
        )?;
        actor
            .source()?
            .admit_borrowed(actor, object, activity, room)
    }
    /// Prepare an existing confirmed HIGH pair. The native KeyClaim worker calls this
    /// through the connected-helper selector; no signature or unrestricted signer escapes.
    pub fn prepare_voice_high_ready(
        &mut self,
        grant: &Arc<CallGrant>,
        handshake: voice_v2::Nonce,
        helpers: &[DeviceId],
    ) -> Result<bool, VoiceError> {
        self.validate_current_voice_call_grant(grant)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let clock = self.voice_v2_clock();
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
        let needs_position = !participant
            .join
            .has_ready_position(handshake, voice.endpoints.public.namespace());
        // Check the shared count before preparation, but allocate its observer only AFTER
        // the original Ready payment exists. Root's exclusive borrow prevents another
        // reservation/insertion meanwhile; external position destruction only lowers it.
        if needs_position && voice.endpoints.public.full_for_ready() {
            return Err(VoiceError::Capacity);
        }
        let call = participant.lease.clone();
        let watch = participant.watch.clone();
        let peer = participant.join.protected_ready_peer(handshake)?;
        let members = ready_member_snapshot(actor, &context, peer, helpers)?;
        participant.join.install_high_ready_receive(
            actor,
            &room,
            handshake,
            helpers,
            &self.device,
            &Authority {
                context: &context,
                lease: &call,
                member: &|id| members.contains(&Some(id)),
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
        if needs_position {
            let position = voice
                .endpoints
                .public
                .reserve_position()
                .expect("exclusive prechecked receipt position");
            participant
                .join
                .attach_ready_position(handshake, position)?;
        }
        Ok(true)
    }
}
