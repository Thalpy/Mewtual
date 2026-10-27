//! First native PC intent on the actual protected Pair. This performs no SDP crypto,
//! packet allocation, PC construction or activation. Those operations require this same
//! intent plus their own native final admission; an RPC receipt cannot reconstruct it.
use super::*;
use crate::voice_v2::chain::sdp_handoff::EffectObserver;
use crate::voice_v2::chain::EstablishedKeyCustody;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;
use crate::voice_v2::sdp::InitialSdpIntent;

impl LocalJoin {
    /// Only the native original-KeyClaim adapter may call this with its actual funded
    /// phase and admitted SDK obligation. Root independently derives the original Ready,
    /// role and finite time from its Pair. Different generation/phase retries never replace
    /// a possibly admitted intent. Exact recovery may return CLOSED history, not effects.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_initial_sdp_intent(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        key: &EstablishedKeyCustody,
        effect: EffectObserver,
        generation: u64,
        authority: &Authority<'_>,
    ) -> Result<Arc<InitialSdpIntent>, VoiceError> {
        // No new native work is created by historical exact recovery. The adapter still
        // authenticates the original Session/key/storage/SDK identities independently.
        let pair = self
            .confirmations
            .pairs
            .get(&handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if let Some(intent) = &pair.sdp_intent {
            return if intent.same_preparation(key, &effect, generation) {
                Ok(intent.clone())
            } else {
                Err(VoiceError::RetryMismatch)
            };
        }
        self.owned(authority)?;
        if pair.retired
            || !pair.established
            || !pair.pin.is_live()
            || pair.protected_epoch != Some(authority.context.epoch)
            || room.scope != self.scope
            || !room.lease.is_live()
            || !actor.uses_resources(&self.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        let Some(successor::Preparation::Transferred { owner, .. }) = &pair.sdp_successor else {
            return Err(VoiceError::Capacity);
        };
        let (wall, mono) = actor.preparation_time()?;
        let metadata = owner.initial_sdp_metadata(wall, mono)?;
        let binding = metadata.original.binding();
        if binding.scope != self.scope
            || binding.epoch != authority.context.epoch
            || binding.local != authority.context.local
            || !binding.call.is_same(authority.lease)
            || binding.handshake != handshake
        {
            return Err(VoiceError::Unauthorized);
        }
        let peer = binding.peer;
        // Allocation and every fallible preparation precede Group/Policy. The outer
        // candidate remains alive on refusal/unwind; no last owner drops beneath guards.
        let candidate = InitialSdpIntent::prepare(metadata, effect, generation, key, wall, mono)?;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("same exclusive Pair");
        actor.with_local_origin(peer, authority.context, |wall, mono, origin| {
            if !room.lease.is_live() || !candidate.current_at(wall, mono) {
                return Err(VoiceError::Unauthorized);
            }
            origin.publish(|| {
                debug_assert!(pair.sdp_intent.is_none());
                pair.sdp_intent = Some(candidate.clone());
            })
        })?;
        Ok(candidate)
    }
}

impl LocalJoin {
    pub(crate) fn has_initial_sdp_seal(&self, handshake: Nonce) -> bool {
        self.confirmations
            .pairs
            .get(&handshake)
            .is_some_and(|pair| {
                // A certified forgotten first round is permanently closed, not unused.
                // In particular the Root wrapper must not reserve another Answer
                // namespace merely because the old heavyweight Action was collected.
                pair.sdp_seal.is_some()
                    || pair
                        .chain
                        .as_ref()
                        .is_some_and(|chain| chain.requested_action_history().is_some())
            })
    }

    /// One native LOW Offer on the original inactive successor. All input/packet/history
    /// capacity and the future Answer namespace are owned before the final origin fence.
    /// A lost reply resolves the SAME action, including after its input has compacted.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_low_sdp_offer(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        intent: &Arc<InitialSdpIntent>,
        plaintext: &[u8],
        position: Option<crate::voice_v2::negotiation::namespace::ReceiptPosition>,
        namespace: &crate::voice_v2::negotiation::namespace::ReceiptNamespace,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        rng: &mut impl CryptoRngCore,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
        outcome: Option<&mut crate::voice_v2::sdp::publication::Writer>,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        self.prepare_low_sdp_offer_scoped(
            actor,
            room,
            handshake,
            intent,
            plaintext,
            position,
            namespace,
            authority,
            clock,
            rng,
            final_owner,
            outcome,
            |step| step.commit(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_low_sdp_offer_scoped(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        intent: &Arc<InitialSdpIntent>,
        plaintext: &[u8],
        position: Option<crate::voice_v2::negotiation::namespace::ReceiptPosition>,
        namespace: &crate::voice_v2::negotiation::namespace::ReceiptNamespace,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        rng: &mut impl CryptoRngCore,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
        mut outcome: Option<&mut crate::voice_v2::sdp::publication::Writer>,
        publish: impl FnOnce(
            crate::voice_v2::sdp::SdpPublicationAdmission<'_>,
        ) -> Result<(), VoiceError>,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        use crate::voice_v2::negotiation::answer::AnswerPosition;
        use crate::voice_v2::sdp::action::{plaintext_tag, Direction};
        self.owned(authority)?;
        intent.check_publication_profile(outcome.is_some())?;
        if plaintext.len() > crate::voice_v2::MAX_SDP_PLAINTEXT
            || std::str::from_utf8(plaintext).is_err()
        {
            return Err(VoiceError::Bounds);
        }
        if let Some(outcome) = &outcome {
            outcome.check_entered(intent, Some(plaintext_tag(plaintext)))?;
        }
        let pair = self
            .confirmations
            .pairs
            .get(&handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if !pair.low
            || !pair
                .sdp_intent
                .as_ref()
                .is_some_and(|original| Arc::ptr_eq(original, intent))
            || pair.retired
            || !pair.established
            || !pair.pin.is_live()
            || pair.protected_epoch != Some(authority.context.epoch)
            || room.scope != self.scope
            || !room.lease.is_live()
            || !actor.uses_resources(&self.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(action) = &pair.sdp_seal {
            let binding = action.sdp_binding().ok_or(VoiceError::Unauthorized)?;
            return if binding.belongs_to_intent(intent) && binding.matches_plaintext(plaintext) {
                if let Some(outcome) = outcome.as_deref_mut() {
                    outcome.published(action);
                }
                Ok(action.clone())
            } else {
                Err(VoiceError::RetryMismatch)
            };
        }
        let position = position
            .filter(|position| namespace.owns(position))
            .ok_or(VoiceError::Capacity)?;
        let Some(successor::Preparation::Transferred { owner, .. }) = &pair.sdp_successor else {
            return Err(VoiceError::Unauthorized);
        };
        let owner = owner.clone();
        let peer = pair.binding.peer;
        let (wall, mono) = actor.preparation_time()?;
        let funding = owner.prepare_answer_funding(intent, wall, mono)?;
        let (action_capacity, packet_capacity) = owner.prepare_initial_offer(
            intent,
            &self.ledger.action_slots(),
            self.ledger.packet_slots(),
            plaintext.len(),
            wall,
            mono,
        )?;
        let public_key = *self
            .announcement
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?
            .public_key();
        let packet = self.ledger.prepare_initial_offer_outgoing(
            public_key,
            plaintext.len(),
            packet_capacity,
            authority,
            clock.as_ref(),
            rng,
        )?;
        let message = packet.id();
        let header = packet.header().clone();
        let mut prepared = self.actions.prepare_initial_sdp_seal(
            Key::SdpSeal(handshake, message),
            self.resources.clone(),
            self.scope,
            self.lease.clone(),
            owner,
            action_capacity,
        )?;
        let (wall, mono) = actor.preparation_time()?;
        let binding = prepared.sdp_binding(
            intent.clone(),
            &header,
            message,
            Direction::Seal,
            plaintext_tag(plaintext),
            wall,
            mono,
        )?;
        let mut answer = Some(Box::new(AnswerPosition {
            position,
            funding,
            offer: header.offer.expect("native LOW Offer"),
        }));
        let (mut publication, reservation) =
            self.ledger
                .validate_initial_offer_outgoing(packet, authority, clock.as_ref())?;
        publication.bind_sdp(binding.clone());
        let pending = publication.crypto_authority(clock.clone())?;
        let flight = CryptoFlight {
            handshake,
            action: prepared.serial(),
            reservation,
            // Original action payment precedes this only owned plaintext copy.
            plaintext: plaintext.to_vec(),
        };
        let (mut issue, inactive) = crypto_actions::ActionQueue::build_sdp_seal(
            prepared,
            flight,
            binding.clone(),
            pending,
        )?;
        let mut inactive = Some(inactive);
        let mut publication = Some(publication);
        let mut published = None;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("exclusive original Pair");
        // All Clock callbacks finish before the native publisher receives this loan.
        // Actual prepared owners remain in this outer frame on refusal and unwind.
        {
            let mut origin = Some(actor.prepare_local_origin(peer, authority.context)?);
            let (wall, mono) = origin
                .as_ref()
                .expect("original Actor samples")
                .observed_time();
            // Actor preparation itself calls injected Clock. Refresh the original
            // Action/packet domain afterward, before lending native publication.
            let own_mono = clock.monotonic_ms();
            let own_wall = clock.now_ms();
            let recorded = outcome
                .as_deref()
                .map(crate::voice_v2::sdp::publication::Writer::original_outcome);
            let mut final_owner = Some(final_owner);
            let mut commit = || {
                origin
                    .take()
                    .ok_or(VoiceError::Unauthorized)?
                    .with_origin(|wall, mono, origin| {
                        final_owner.take().expect("one original owner check")(wall, mono)?;
                        if !room.lease.is_live()
                            || !binding.current_at(wall, mono)
                            || !namespace.owns(
                                &answer
                                    .as_ref()
                                    .expect("unpublished Answer position")
                                    .position,
                            )
                        {
                            return Err(VoiceError::Unauthorized);
                        }
                        publication
                            .as_ref()
                            .expect("one draft")
                            .admission()
                            .check_at(origin.epoch(), wall, mono)?;
                        inactive
                            .as_ref()
                            .expect("one action")
                            .check_pending_at(own_mono, own_wall)?;
                        intent.admit_publication_profile(outcome.is_some())?;
                        origin.publish(|| {
                            issue.publish();
                            let action = self
                                .actions
                                .publish_inactive(inactive.take().expect("one action"));
                            if let Some(outcome) = outcome.as_deref_mut() {
                                outcome.published(&action);
                            }
                            pair.sdp_seal = Some(action.clone());
                            pair.answer_position = answer.take();
                            published = Some(action);
                            // Actual ledger sequence and OfferId precede activation; no recoverable tail.
                            publication
                                .take()
                                .expect("one draft")
                                .publish_initial_offer();
                        })
                    })
            };
            publish(crate::voice_v2::sdp::SdpPublicationAdmission::new(
                wall,
                mono,
                recorded.as_ref(),
                &mut commit,
            ))?;
        }
        // Actual Phase/slot/input/issue destruction must remain outside Group/Policy.
        drop((inactive, publication, issue, answer));
        published.ok_or(VoiceError::Unauthorized)
    }

    /// Native HIGH Open action for the SAME already admitted first Offer. Admission has
    /// already spent its replay/token position; preparing an action must neither repeat it
    /// nor remove its only paid input on a final capacity refusal. LOW Answer uses its own
    /// reserved admission path before sharing this action/result machinery.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_high_sdp_open(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        intent: &Arc<InitialSdpIntent>,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
        outcome: Option<&mut crate::voice_v2::sdp::publication::Writer>,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        if intent.local_is_low() {
            return Err(VoiceError::Unauthorized);
        }
        self.prepare_sdp_open(
            actor,
            room,
            handshake,
            intent,
            authority,
            clock,
            final_owner,
            outcome,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_low_sdp_open(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        intent: &Arc<InitialSdpIntent>,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
        outcome: Option<&mut crate::voice_v2::sdp::publication::Writer>,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        if !intent.local_is_low() {
            return Err(VoiceError::Unauthorized);
        }
        self.prepare_sdp_open(
            actor,
            room,
            handshake,
            intent,
            authority,
            clock,
            final_owner,
            outcome,
        )
    }

    /// Both native directions open only the original positively admitted packet. No
    /// supplied token or header may select a new input or recreate a consumed one.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn prepare_sdp_open(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        intent: &Arc<InitialSdpIntent>,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
        outcome: Option<&mut crate::voice_v2::sdp::publication::Writer>,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        self.prepare_sdp_open_scoped(
            actor,
            room,
            handshake,
            intent,
            authority,
            clock,
            final_owner,
            outcome,
            |step| step.commit(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_sdp_open_scoped(
        &mut self,
        actor: &ActorOwner,
        room: &RoomAuthority,
        handshake: Nonce,
        intent: &Arc<InitialSdpIntent>,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
        mut outcome: Option<&mut crate::voice_v2::sdp::publication::Writer>,
        publish: impl FnOnce(
            crate::voice_v2::sdp::SdpPublicationAdmission<'_>,
        ) -> Result<(), VoiceError>,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        use crate::voice_v2::negotiation::Stage;
        use crate::voice_v2::sdp::action::Direction;
        self.owned(authority)?;
        intent.check_publication_profile(outcome.is_some())?;
        if let Some(outcome) = &outcome {
            outcome.check_entered(intent, None)?;
        }
        let pair = self
            .confirmations
            .pairs
            .get(&handshake)
            .ok_or(VoiceError::Unauthorized)?;
        if !pair
            .sdp_intent
            .as_ref()
            .is_some_and(|original| Arc::ptr_eq(original, intent))
        {
            return Err(VoiceError::Unauthorized);
        }
        let receipt = if pair.low {
            &pair.answer_receipt
        } else {
            &pair.first_offer
        };
        let Some(receipt) = receipt.as_ref() else {
            // SDP can arrive after this native request on an ordinary delayed P2P route.
            // Absence is transient ONLY for the exact still-current original PC/pair.
            // Do not permanently poison its canonical Open row or renew its deadline.
            // Known receipt + absent input below is intentionally NOT this case: consumed
            // or retired input must never be reconstructed from a missing buffer.
            let current = || {
                !pair.retired
                    && pair.established
                    && pair.pin.is_live()
                    && room.lease.is_live()
                    && room.scope == self.scope
                    && actor.uses_resources(&self.resources)
                    && pair.protected_epoch == Some(authority.context.epoch)
            };
            if !current() {
                return Err(VoiceError::Unauthorized);
            }
            let (wall, mono) = actor.preparation_time()?;
            // Clock is an injected callback outside locks. Recheck independent room
            // revocation after it; a stale initial snapshot must not authorize waiting.
            if !current() {
                return Err(VoiceError::Unauthorized);
            }
            intent.remaining_at(wall, mono)?;
            return Err(VoiceError::Capacity);
        };
        let receipt = receipt.clone();
        let key = Key::SdpOpen(receipt.token);
        // Exact history lookup does not recreate an input after Open consumed its token.
        if let Some(action) = self.actions.get(key) {
            if let Some(outcome) = outcome.as_deref_mut() {
                outcome.published(&action);
            }
            return Ok(action);
        }
        if pair.retired
            || !pair.established
            || !pair.pin.is_live()
            || !room.lease.is_live()
            || room.scope != self.scope
            || !actor.uses_resources(&self.resources)
            || pair.protected_epoch != Some(authority.context.epoch)
        {
            return Err(VoiceError::Unauthorized);
        }
        let packet = pair
            .sdp_input
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?
            .clone();
        if packet.token != receipt.token
            || packet.object_hash() != receipt.hash
            || packet.header.kind.round() != Some(intent.round())
        {
            return Err(VoiceError::Unauthorized);
        }
        let Some(successor::Preparation::Transferred { owner, .. }) = &pair.sdp_successor else {
            return Err(VoiceError::Unauthorized);
        };
        let (payload_bytes, engine_bytes) =
            crypto_actions::sdp_bytes(packet.aad().len(), packet.ciphertext().len())?;
        let mut prepared = self.actions.prepare(
            key,
            Allocation {
                resources: self.resources.clone(),
                scope: self.scope,
                call: self.lease.clone(),
                budget: ByteBudget::Exchange {
                    exchange: owner.clone(),
                    stage: Stage::DecryptAction,
                },
                history: None,
                payload_bytes,
                engine_bytes,
            },
        )?;
        let (wall, mono) = actor.preparation_time()?;
        let binding = prepared.sdp_binding(
            intent.clone(),
            &packet.header,
            receipt.message,
            Direction::Open,
            packet.object_hash(),
            wall,
            mono,
        )?;
        let pending = self
            .ledger
            .incoming_crypto_authority(&packet, authority, clock.clone())?;
        let (mut issue, inactive) = crypto_actions::ActionQueue::build_sdp_open(
            prepared,
            packet.clone(),
            binding.clone(),
            pending,
        )?;
        let mut inactive = Some(inactive);
        let mut published = None;
        let mut retired_input = None;
        let pair = self
            .confirmations
            .pairs
            .get_mut(&handshake)
            .expect("same exclusive pair");
        // All Clock callbacks finish before the native publisher receives this loan.
        // Actual prepared owners remain in this outer frame on refusal and unwind.
        {
            let mut origin = Some(actor.prepare_local_origin(receipt.issuer, authority.context)?);
            let (wall, mono) = origin
                .as_ref()
                .expect("original Actor samples")
                .observed_time();
            // Actor preparation itself calls injected Clock. Refresh the original
            // Action/packet domain afterward, before lending native publication.
            let own_mono = clock.monotonic_ms();
            let own_wall = clock.now_ms();
            let recorded = outcome
                .as_deref()
                .map(crate::voice_v2::sdp::publication::Writer::original_outcome);
            let mut final_owner = Some(final_owner);
            let mut commit = || {
                origin
                    .take()
                    .ok_or(VoiceError::Unauthorized)?
                    .with_origin(|wall, mono, origin| {
                        final_owner.take().expect("one original owner check")(wall, mono)?;
                        if !room.lease.is_live()
                            || !binding.current_at(wall, mono)
                            || !receipt.authorized_at(wall, mono)
                            || !pair
                                .sdp_input
                                .as_ref()
                                .is_some_and(|original| Arc::ptr_eq(original, &packet))
                        {
                            return Err(VoiceError::Unauthorized);
                        }
                        inactive
                            .as_ref()
                            .expect("unpublished original action")
                            .check_pending_at(own_mono, own_wall)?;
                        intent.admit_publication_profile(outcome.is_some())?;
                        origin.publish(|| {
                            issue.publish();
                            let action = self
                                .actions
                                .publish_inactive(inactive.take().expect("one original open"));
                            if let Some(outcome) = outcome.as_deref_mut() {
                                outcome.published(&action);
                            }
                            published = Some(action);
                            retired_input = pair.sdp_input.take();
                        })
                    })
            };
            publish(crate::voice_v2::sdp::SdpPublicationAdmission::new(
                wall,
                mono,
                recorded.as_ref(),
                &mut commit,
            ))?;
        }
        // These actual owners and affine issue outlive Group/Policy, including every refusal.
        drop(retired_input);
        drop(inactive);
        drop(issue);
        published.ok_or(VoiceError::Unauthorized)
    }
}
