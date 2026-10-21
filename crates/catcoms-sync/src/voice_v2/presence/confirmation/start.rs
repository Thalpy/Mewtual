//! Protected low Hello publication. Every fallible allocation precedes one native local-origin
//! and History fence; unsigned preparation never consumes an incoming peer's origin allowance.
use super::*;
use crate::voice_v2::chain::StartCandidate;
use crate::voice_v2::crypto_actions::ActionQueue;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;

/// Native-only request identity. None retries the original start; Some(id) names the exact
/// predecessor of an explicit next start. It cannot mint a call, room, pair or new deadline.
pub(crate) struct ProtectedStart<'a> {
    pub actor: &'a ActorOwner,
    pub room: &'a RoomAuthority,
    pub handshake: Nonce,
    pub predecessor: Option<Nonce>,
}

impl LocalJoin {
    /// Return the exact retained action, or publish one new protected Hello. This is still an
    /// inactive native seam: registry/browser custody and actual source admission are separate.
    /// The Root caller must supply its pure final current-call/room observer, as for intake.
    pub(crate) fn start_protected_hello(
        &mut self,
        request: ProtectedStart<'_>,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
        rng: &mut impl CryptoRngCore,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<Arc<CryptoAction>, VoiceError> {
        let ProtectedStart {
            actor,
            room,
            handshake,
            predecessor,
        } = request;
        if room.scope != self.scope
            || !room.lease.is_live()
            || !actor.uses_resources(&self.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        self.pair_current(handshake, authority, clock.as_ref())?;
        let pair = &self.confirmations.pairs[&handshake];
        let chain = pair.chain.clone().ok_or(VoiceError::Unauthorized)?;
        if !pair.low
            || pair.protected_epoch != Some(authority.context.epoch)
            || !chain.matches(
                &self.chain_binding(handshake, authority),
                &self.resources,
                &self.ledger.action_slots(),
                clock.monotonic_ms(),
            )
        {
            return Err(VoiceError::Unauthorized);
        }
        let peer = pair.binding.peer;
        if let StartCandidate::Cached(receipt) = chain.classify_start(predecessor)? {
            if !receipt.same_room(&room.lease) {
                return Err(VoiceError::Unauthorized);
            }
            let action = self
                .actions
                .get(Key::Encrypt(handshake, receipt.action, receipt.message))
                .filter(|action| action.uses_chain(&chain.lease()))
                .ok_or(VoiceError::Unauthorized)?;
            actor.with_local_origin(peer, authority.context, |wall, mono, origin| {
                final_owner(wall, mono)?;
                if !receipt.authorized_at(origin.epoch(), wall, mono) {
                    return Err(VoiceError::Unauthorized);
                }
                // Recheck the atomic actor/session close even for a historical lookup. This
                // consumes no candidate, packet position, engine work, nonce or origin quota.
                origin.publish(|| ())
            })?;
            return Ok(action);
        }
        if pair.established {
            return Err(VoiceError::Unauthorized);
        }
        let previous_crypto = pair.action.as_ref().and_then(|action| {
            action.draft.as_ref().and_then(|draft| {
                self.actions
                    .get(Key::Encrypt(handshake, action.serial, draft.id))
            })
        });
        let mut low = [0; 32];
        rng.fill_bytes(&mut low);
        let mut change = prepare_action(
            pair,
            EncryptedKind::ConfirmHello,
            low,
            None,
            &self.ledger,
            authority,
        )?;
        let low_tag = confirmation_tag(&change.action.plaintext);
        if pair.protected_hello_tags.contains(&Some(low_tag)) {
            // Entropy failure must not make an old Proof(A) answer a distinct start(C).
            // No packet/candidate/nonce was published; an exact request may retry fresh RNG.
            return Err(VoiceError::Reservation);
        }
        let public_key = *self
            .announcement
            .as_ref()
            .ok_or(VoiceError::Reservation)?
            .public_key();
        let workflow = chain.workflow()?;
        let prepared_packet = self.ledger.prepare_protected_outgoing(
            public_key,
            ReserveRequest {
                handshake,
                kind: EncryptedKind::ConfirmHello,
                offer: None,
                ciphertext_bytes: change.action.plaintext.len() + 16,
            },
            authority,
            clock.as_ref(),
            rng,
            workflow,
        )?;
        let (publication, reservation) =
            self.ledger
                .validate_prepared_outgoing(prepared_packet, authority, clock.as_ref())?;
        change.action.created = publication.started();
        change.action.draft = Some(reservation.clone());
        let effect = actor.effect(authority.context.local, peer, reservation.expires)?;
        let (history, receipt, workflow) = chain.prepare_start(
            predecessor,
            reservation.id,
            change.action.serial,
            reservation.expires,
            crate::voice_v2::chain::HelloConsent {
                room: room.lease.clone(),
                _room_custody: room.custody.clone(),
                effect,
                clock: clock.clone(),
            },
        )?;
        let (payload_bytes, engine_bytes) = crypto_actions::confirmation_bytes(
            reservation.aad.len(),
            change.action.plaintext.len() + 16,
        )?;
        let pending = publication
            .crypto_authority(clock)?
            .with_pending(change.action.live.0.clone())?;
        let flight = CryptoFlight {
            handshake,
            action: change.action.serial,
            reservation,
            plaintext: change.action.plaintext.clone(),
        };
        let prepared_action = self.actions.prepare(
            Key::Encrypt(handshake, flight.action, flight.reservation.id),
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
        let (mut issue, inactive) =
            ActionQueue::build_chain_encrypt(prepared_action, flight, pending)?;
        // Do not move these affine owners into the FnOnce environment. A refused final fence
        // must destroy them after Group/Policy/History, and never refund uncertain engines.
        let mut publication = Some(publication);
        let mut inactive = Some(inactive);
        let mut change = Some(change);
        let mut published = None;
        let mut retired_action = None;
        let mut retired_attempt = None;
        actor.with_local_origin(peer, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            publication
                .as_ref()
                .expect("unpublished packet")
                .admission()
                .check_at(authority.context.epoch, wall, mono)?;
            issue.publish_start(&receipt, origin, wall, mono, || {
                let change = change.take().expect("one native action publication");
                publication
                    .take()
                    .expect("one packet publication")
                    .publish_replacing(change.retire);
                let pair = self
                    .confirmations
                    .pairs
                    .get_mut(&handshake)
                    .expect("exclusive pair");
                // Actual irreversible attempt/action gates close before replacement becomes
                // visible. Destruction and engine retirement happen outside every final guard.
                pair.attempt.0.revoke();
                if let Some(old) = &pair.action {
                    old.live.0.revoke();
                }
                retired_attempt = Some(std::mem::take(&mut pair.attempt));
                pair.action_serial = change.action.serial;
                retired_action = pair.action.replace(change.action);
                pair.protected_hello_tags[usize::from(pair.starts)] = Some(low_tag);
                pair.starts += 1;
                pair.low_nonce = Some(low);
                pair.high_nonce = None;
                pair.accepted_proof = None;
                published = Some(
                    self.actions
                        .publish_inactive(inactive.take().expect("one crypto publication")),
                );
            })
        })?;
        drop((retired_action, retired_attempt));
        if let Some(previous) = previous_crypto {
            // Engine/payload mutexes and actual progress-credit destruction stay outside the
            // native final guards. A running old engine keeps its charge until real settlement.
            previous.retire_if_unstarted();
        }
        let action = published.expect("successful local publication");
        chain.register_action(&action);
        Ok(action)
    }
}
