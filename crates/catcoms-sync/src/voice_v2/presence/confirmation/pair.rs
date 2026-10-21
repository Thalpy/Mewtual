//! Production pair admission selects native watch evidence, then publishes both original pair
//! owners under the same short local origin fence. No Clock/RNG/signer runs beneath that fence.
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::ActorOwner;

/// Immutable native request identity. Device alone is insufficient: after a lost reply a peer
/// may rejoin with another key. Retrying this selector must never spend a second pair on it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PairSelector {
    pub peer: DeviceId,
    pub join: Nonce,
    pub key_ref: [u8; 32],
    pub epoch: u64,
}

/// Fixed metadata, not a pair/key/RTC capability. Only the original native CallGrant can use
/// its handshake with Root's protected factories. No transcript or browser key is cloned here.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PairReceipt {
    pub handshake: Nonce,
    pub peer: DeviceId,
    pub local_is_low: bool,
}
redacted_debug!(PairSelector, PairReceipt);

pub(crate) struct ProtectedPair<'a> {
    pub actor: &'a ActorOwner,
    pub room: &'a RoomAuthority,
    pub watch: &'a WatchLease,
    pub selector: PairSelector,
}

impl LocalJoin {
    /// Caller selects evidence only from the actual participant's implicit watcher. Discovery
    /// is needed for first admission, not exact recovery of an independently pinned pair.
    pub(crate) fn admit_protected_pair(
        &mut self,
        request: ProtectedPair<'_>,
        fresh: Option<&FreshPeer>,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
    ) -> Result<(PairReceipt, bool), VoiceError> {
        let ProtectedPair {
            actor,
            room,
            watch,
            selector,
        } = request;
        self.owned(authority)?;
        if selector.epoch != authority.context.epoch
            || room.scope != self.scope
            || !room.lease.is_live()
            || !watch.is_live()
            || !actor.uses_resources(&self.resources)
        {
            return Err(VoiceError::Unauthorized);
        }
        self.collect_confirmations(clock);
        // Retained failed/expired records are deliberately never removed. A matching selector
        // therefore cannot recreate its allocator, deadline, mode or confirmation allowance.
        if let Some((handshake, pair)) = self.confirmations.pairs.iter().find(|(_, pair)| {
            pair.binding.peer == selector.peer
                && pair.binding.join == selector.join
                && pair.binding.key_ref == selector.key_ref
        }) {
            let receipt = PairReceipt {
                handshake: *handshake,
                peer: selector.peer,
                local_is_low: pair.low,
            };
            actor.with_local_origin(selector.peer, authority.context, |wall, mono, origin| {
                final_owner(wall, mono)?;
                if !room.lease.is_live()
                    || !watch.is_live()
                    || !self.lease.is_live()
                    || pair.retired
                    || !pair.pin.is_live()
                    || pair.protected_epoch != Some(selector.epoch)
                {
                    return Err(VoiceError::Unauthorized);
                }
                if !pair.established && mono >= pair.deadline {
                    return Err(VoiceError::Freshness);
                }
                origin.publish(|| ())
            })?;
            return Ok((receipt, false));
        }
        let fresh = fresh.ok_or(VoiceError::Unauthorized)?;
        let Body::Announcement { key, .. } = fresh.announcement().body() else {
            return Err(VoiceError::Malformed);
        };
        if fresh.announcement().issuer() != selector.peer
            || key.join != selector.join
            || key.key_ref != selector.key_ref
            || !fresh.0.owner.is_same(watch)
        {
            return Err(VoiceError::Unauthorized);
        }
        fresh.authorize(authority.context, authority.member, clock)?;
        let own = self.announcement.as_ref().ok_or(VoiceError::Reservation)?;
        own.check_freshness(clock.now_ms())?;
        let transcript = pair_transcript(own, fresh.announcement())?;
        let handshake = transcript_hash(&transcript);
        let mono = clock.monotonic_ms();
        if self.confirmations.pairs.len() >= MAX_PAIRS
            || self
                .confirmations
                .pairs
                .values()
                .filter(|pair| !pair.established && !pair.retired && mono < pair.deadline)
                .count()
                >= MAX_CONFIRMING
        {
            return Err(VoiceError::Capacity);
        }
        let generation = self
            .confirmations
            .serial
            .checked_add(1)
            .ok_or(VoiceError::Capacity)?;
        let credit = self
            .resources
            .charge(self.scope.group, PAIR_BYTES, ResourceKind::Retained)?;
        let binding = PairBinding {
            peer: selector.peer,
            join: selector.join,
            key_ref: selector.key_ref,
            handshake,
        };
        let pin = CallLease::default();
        let mut reservation = actor.prepare_pair_pin(&binding, authority.context, &pin)?;
        let claim =
            self.ledger
                .prepare_confirmation_claim(binding.clone(), pin.clone(), authority)?;
        let low = self.local < binding.peer;
        let pair = Pair {
            binding,
            generation,
            transcript,
            peer_key: key.public_key,
            pin,
            deadline: mono.saturating_add(MAX_PACKET_LIFETIME_MS),
            attempt: RevocationOwner::default(),
            low,
            starts: 0,
            action_serial: 0,
            low_nonce: None,
            high_nonce: None,
            protected_hello_tags: [None; 3],
            hello: super::super::super::crypto_authority::SequenceGate::default(),
            accepted_hello: None,
            accepted_proof: None,
            accepted_finish: None,
            action: None,
            established: false,
            retired: false,
            chain: None,
            // Protection and its creation epoch are pinned BEFORE any later chain request.
            // An intervening MLS change cannot adopt this pair into a different epoch.
            protected_epoch: Some(selector.epoch),
            _credit: credit,
        };
        let receipt = PairReceipt {
            handshake,
            peer: selector.peer,
            local_is_low: low,
        };
        // These owners remain outside the FnOnce environment. Refusal drops them only after
        // Group/Policy release; no credit or revocation cleanup reenters a held native guard.
        let mut unpublished = Some((claim, pair));
        actor.with_local_origin(selector.peer, authority.context, |wall, mono, origin| {
            final_owner(wall, mono)?;
            let (_, pair) = unpublished.as_ref().expect("unpublished pair");
            if !room.lease.is_live()
                || !watch.is_live()
                || !self.lease.is_live()
                || !pair.pin.is_live()
                || !fresh.0.owner.is_same(watch)
            {
                return Err(VoiceError::Unauthorized);
            }
            if mono >= pair.deadline {
                return Err(VoiceError::Freshness);
            }
            own.check_freshness(wall)?;
            // Membership callbacks ran during preparation. The final original Group/Policy
            // loan checks the selected peer/local members directly, without opaque callbacks.
            fresh.authorize_at(authority.context, wall, mono)?;
            origin.publish(|| {
                let (claim, pair) = unpublished.take().expect("one pair publication");
                self.ledger.publish_confirmation_claim(claim);
                self.confirmations.pairs.insert(handshake, pair);
                self.confirmations.serial = generation;
                reservation.publish();
            })
        })?;
        Ok((receipt, true))
    }
}
