//! One protected output per actual accepted input lane. A historical result is evidence of a
//! transition, never fresh authority: first response preparation still observes current owners.
use super::*;
use crate::voice_v2::crypto_actions::ActionQueue;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;

pub(crate) struct ProtectedResponse<'a> {
    pub actor: &'a ActorOwner,
    pub room: &'a RoomAuthority,
    pub handshake: Nonce,
    pub trigger: &'a Arc<CryptoAction>,
}

impl LocalJoin {
    /// Recover the exact original Proof/Finish action, or publish it once for a still-current
    /// accepted decrypt. The caller cannot supply plaintext, nonce, output kind or result fields.
    /// Superseded unproduced inputs refuse; produced identities survive without renewing work.
    pub(crate) fn respond_protected_confirmation(
        &mut self,
        request: ProtectedResponse<'_>,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        rng: &mut impl CryptoRngCore,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        let ProtectedResponse {
            actor,
            room,
            handshake,
            trigger,
        } = request;
        if room.scope != self.scope
            || !room.lease.is_live()
            || !actor.uses_resources(&self.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        let Some(Key::Decrypt(token)) = self.actions.key_of(trigger) else {
            return Err(VoiceError::Unauthorized);
        };
        self.pair_current(handshake, authority, clock.as_ref())?;
        let pair = &self.confirmations.pairs[&handshake];
        let chain = pair.chain.clone().ok_or(VoiceError::Unauthorized)?;
        if pair.protected_epoch != Some(authority.context.epoch)
            || !chain.matches(
                &self.chain_binding(handshake, authority),
                &self.resources,
                &self.ledger.action_slots(),
                clock.monotonic_ms(),
            )
        {
            return Err(VoiceError::Unauthorized);
        }
        let input = chain.confirmation_token(token)?;
        if !input.same_room(&room.lease) {
            return Err(VoiceError::Unauthorized);
        }
        let peer = pair.binding.peer;
        if let Some(receipt) = chain.cached_response(&input)? {
            let action = self
                .actions
                .get(Key::Encrypt(handshake, receipt.action, receipt.message))
                .filter(|action| action.uses_chain(&chain.lease()))
                .ok_or(VoiceError::Unauthorized)?;
            actor.with_local_origin(peer, authority.context, |wall, mono, origin| {
                final_owner(wall, mono)?;
                if !receipt.same_room(&room.lease)
                    || !receipt.authorized_at(origin.epoch(), wall, mono)
                {
                    return Err(VoiceError::Unauthorized);
                }
                origin.publish(|| ())
            })?;
            return Ok(action);
        }
        let accepted = trigger
            .terminal_status()
            .receipt
            .ok_or(VoiceError::Unauthorized)?;
        if accepted.retired || accepted.outcome != CryptoOutcome::Opened {
            return Err(VoiceError::Unauthorized);
        }
        let pending = pair.action.as_ref().ok_or(VoiceError::Unauthorized)?;
        let current_trigger = match (pair.low, input.kind, pending.kind) {
            (false, EncryptedKind::ConfirmHello, EncryptedKind::ConfirmProof) => {
                pair.accepted_hello
            }
            (true, EncryptedKind::ConfirmProof, EncryptedKind::ConfirmFinish) => {
                pair.accepted_proof
            }
            _ => return Err(VoiceError::Unauthorized),
        };
        if current_trigger != Some(input.hash)
            || pair.established
            || pending.draft.is_some()
            || pending.signed.is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        let action_serial = pending.serial;
        let action_live = pending.live.0.clone();
        let attempt = pair.attempt.0.clone();
        let plaintext = pending.plaintext.clone();
        let kind = pending.kind;
        let public_key = *self
            .announcement
            .as_ref()
            .ok_or(VoiceError::Reservation)?
            .public_key();
        let prepared = self.ledger.prepare_protected_outgoing(
            public_key,
            ReserveRequest {
                handshake,
                kind,
                offer: None,
                ciphertext_bytes: plaintext.len() + 16,
            },
            authority,
            clock.as_ref(),
            rng,
            chain.workflow()?,
        )?;
        let (publication, reservation) =
            self.ledger
                .validate_prepared_outgoing(prepared, authority, clock.as_ref())?;
        let effect = actor.effect(authority.context.local, peer, reservation.expires)?;
        let (history, receipt, workflow) = chain.prepare_response(
            &input,
            reservation.id,
            action_serial,
            reservation.expires,
            crate::voice_v2::chain::ConfirmationConsent {
                room: room.lease.clone(),
                _room_custody: room.custody.clone(),
                effect,
                clock: clock.clone(),
            },
        )?;
        let (payload_bytes, engine_bytes) =
            crypto_actions::confirmation_bytes(reservation.aad.len(), plaintext.len() + 16)?;
        let pending = publication
            .crypto_authority(clock)?
            .with_pending(action_live.clone())?;
        let flight = CryptoFlight {
            handshake,
            action: action_serial,
            reservation: reservation.clone(),
            plaintext,
        };
        let prepared = self.actions.prepare(
            Key::Encrypt(handshake, action_serial, reservation.id),
            Allocation {
                resources: self.resources.clone(),
                scope: self.scope,
                call: self.lease.clone(),
                budget: ByteBudget::Workflow(workflow),
                history: Some(history),
                payload_bytes,
                engine_bytes,
            },
        )?;
        let (mut issue, inactive) = ActionQueue::build_chain_encrypt(prepared, flight, pending)?;
        let mut publication = Some(publication);
        let mut inactive = Some(inactive);
        let mut published = None;
        // All outer affine owners survive refused closures until Group/Policy/History return.
        actor.with_local_origin(peer, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            if !action_live.is_live() || !attempt.is_live() {
                return Err(VoiceError::Unauthorized);
            }
            publication
                .as_ref()
                .expect("unpublished response")
                .admission()
                .check_at(authority.context.epoch, wall, mono)?;
            issue.publish_response(&receipt, &input, origin, (wall, mono), || {
                let publication = publication.take().expect("one packet publication");
                let started = publication.started();
                publication.publish_replacing(None);
                let pending = self
                    .confirmations
                    .pairs
                    .get_mut(&handshake)
                    .expect("exclusive pair")
                    .action
                    .as_mut()
                    .expect("current response");
                pending.draft = Some(reservation);
                pending.created = started;
                published = Some(
                    self.actions
                        .publish_inactive(inactive.take().expect("one response publication")),
                );
            })
        })?;
        let action = published.expect("successful response publication");
        chain.register_action(&action);
        Ok(action)
    }
}
