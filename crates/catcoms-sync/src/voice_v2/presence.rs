//! Fresh room observation and active-join proofs, independent of media/key confirmation.
//! Announcements advertise a candidate; only a response to this exact native watcher's live
//! challenge admits presence. Neither a helper nor a nonparticipant watcher owns a LocalJoin.
//! Every operation takes a fresh actor-turn context/membership view. None may be captured over
//! an await. The actor must keep OriginLimits across watcher, helper and controller replacement.

use super::limits::OriginLimits;
use super::packets::{Authority, PacketLedger};
use super::resources::{Credit, ResourceKind, Resources};
use super::*;
use catcoms_rt::{Clock, CryptoRngCore};
use std::collections::BTreeMap;
use std::sync::Arc;

mod confirmation;
pub use confirmation::{
    ConfirmationOutput, CryptoFlight, FlightSubmission, OpenedFlight, PairSetup, ReceiveProgress,
};
pub(crate) use confirmation::{
    ProtectedDerivation, ProtectedDerivationResult, ProtectedOpenedResult, ProtectedResponse,
    ProtectedSealedResult, ProtectedStart,
};

const RECORD_BYTES: usize = 1536;
const MAX_PENDING: usize = 32;
const RENEW_MS: u64 = 5000;

/// Current observer authority, deliberately distinct from participant/capture authority.
pub struct WatchAuthority<'a> {
    pub context: &'a Context,
    pub lease: &'a WatchLease,
    pub member: &'a dyn Fn(DeviceId) -> bool,
}

struct Challenge {
    announcement: SignedObject,
    object: SignedObject,
    ordinal: u64,
    deadline: u64,
    _credit: Credit,
}
struct Observation {
    announcement: SignedObject,
    proof: SignedObject,
    watcher: DeviceId,
    owner: WatchLease,
    live: CallLease,
    ordinal: u64,
    deadline: u64,
    _credit: Credit,
}

/// Opaque native evidence, revoked when superseded/removed or the watcher ends. Clones retain
/// the exact resource charge until actually released. This proves room liveness only, never
/// an established key pair, RTC connection, microphone consent or audible media.
#[derive(Clone)]
pub struct FreshPeer(Arc<Observation>);
impl FreshPeer {
    pub fn announcement(&self) -> &SignedObject {
        &self.0.announcement
    }
    pub fn ordinal(&self) -> u64 {
        self.0.ordinal
    }
    pub fn authorize(
        &self,
        context: &Context,
        member: &dyn Fn(DeviceId) -> bool,
        clock: &dyn Clock,
    ) -> Result<(), VoiceError> {
        let record = &self.0;
        if record.watcher != context.local || record.announcement.scope() != context.scope {
            return Err(VoiceError::Unauthorized);
        }
        if !member(record.announcement.issuer()) || !member(record.watcher) {
            // Membership loss is irreversible for issued evidence. A retained clone must not
            // become usable again merely because the same device is later re-added.
            record.live.revoke();
            return Err(VoiceError::Unauthorized);
        }
        if !record.owner.is_live()
            || !record.live.is_live()
            || clock.monotonic_ms() >= record.deadline
        {
            return Err(VoiceError::Unauthorized);
        }
        record.announcement.check_freshness(clock.now_ms())?;
        record.proof.check_freshness(clock.now_ms())?;
        if matches!(record.announcement.body(), Body::Announcement { epoch, .. } if *epoch > context.epoch)
        {
            return Err(VoiceError::Freshness);
        }
        Ok(())
    }
}

pub struct Watcher {
    scope: Scope,
    local: DeviceId,
    incarnation: Nonce,
    lease: WatchLease,
    resources: Arc<Resources>,
    _credit: Credit,
    serial: u64,
    pending: BTreeMap<MessageId, Challenge>,
    present: BTreeMap<DeviceId, FreshPeer>,
}

impl Watcher {
    /// Time/owner collection needs no current actor permission and remains safe while locked.
    /// Membership transitions independently retire every issued presence capability at source.
    pub fn collect_expired(&mut self, clock: &dyn Clock) -> bool {
        let before = (self.pending.len(), self.present.len());
        let now = clock.now_ms();
        let mono = clock.monotonic_ms();
        self.pending.retain(|_, challenge| {
            self.lease.is_live()
                && mono < challenge.deadline
                && challenge.announcement.check_freshness(now).is_ok()
        });
        self.present.retain(|_, peer| {
            let live = self.lease.is_live()
                && peer.0.owner.is_live()
                && peer.0.live.is_live()
                && mono < peer.0.deadline
                && peer.0.announcement.check_freshness(now).is_ok()
                && peer.0.proof.check_freshness(now).is_ok();
            if !live {
                peer.0.live.revoke();
            }
            live
        });
        before != (self.pending.len(), self.present.len())
    }
    /// Cheap exact native routing only. Full pending-challenge, signature/freshness and member
    /// checks remain in prove; matching this nonce never grants presence or pair authority.
    pub(crate) fn targets_proof(&self, object: &SignedObject) -> bool {
        object.scope() == self.scope
            && matches!(object.body(), Body::JoinProof { watcher, watcher_incarnation, .. }
            if *watcher == self.local && *watcher_incarnation == self.incarnation)
    }
    pub fn source_activity(
        &self,
        authority: &WatchAuthority<'_>,
    ) -> Result<super::source::SourceActivity, VoiceError> {
        self.current(authority)?;
        Ok(super::source::SourceActivity::watching(
            self.scope,
            self.local,
            self.incarnation,
            self.lease.clone(),
        ))
    }
    /// Native generates the watcher incarnation; a restarted/replaced watcher cannot replay
    /// its predecessor's challenges. Claiming a lease twice refuses even after the first drops.
    pub fn new(
        context: &Context,
        lease: WatchLease,
        resources: Arc<Resources>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, VoiceError> {
        let credit = resources.charge(context.scope.group, RECORD_BYTES, ResourceKind::Watcher)?;
        lease.claim()?;
        let mut incarnation = [0; 32];
        rng.fill_bytes(&mut incarnation);
        Ok(Self {
            scope: context.scope,
            local: context.local,
            incarnation,
            lease,
            resources,
            _credit: credit,
            serial: 0,
            pending: BTreeMap::new(),
            present: BTreeMap::new(),
        })
    }
    fn current(&self, authority: &WatchAuthority<'_>) -> Result<(), VoiceError> {
        if !self.lease.is_live()
            || !self.lease.is_same(authority.lease)
            || !authority.lease.is_live()
            || self.scope != authority.context.scope
            || self.local != authority.context.local
            || !(authority.member)(self.local)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    fn prune(&mut self, authority: &WatchAuthority<'_>, clock: &dyn Clock) {
        self.pending.retain(|_, challenge| {
            clock.monotonic_ms() < challenge.deadline
                && (authority.member)(challenge.announcement.issuer())
        });
        self.present.retain(|_, peer| {
            let live = peer
                .authorize(authority.context, authority.member, clock)
                .is_ok();
            if !live {
                peer.0.live.revoke();
            }
            live
        });
    }
    /// Actor membership transitions call this synchronously, before any re-add or queued
    /// delivery. Polling the room later is insufficient: retained FreshPeer clones also exist.
    /// Exact full identities come from authenticated membership, never UI roster fingerprints.
    pub fn retire_member(&mut self, member: DeviceId) {
        if member == self.local {
            self.lease.revoke();
        }
        self.pending
            .retain(|_, pending| pending.announcement.issuer() != member);
        if let Some(peer) = self.present.remove(&member) {
            peer.0.live.revoke();
        }
    }
    /// The lower authenticated ingress spends original/requester traffic budgets once per
    /// admitted packet. Here the shared actor owner pins the join/key and distinct-join budget.
    /// Exact retransmissions/renewals retain a pending challenge and its original deadline.
    pub fn challenge(
        &mut self,
        announcement: &SignedObject,
        signer: &dyn VoiceSigner,
        authority: &WatchAuthority<'_>,
        limits: &mut OriginLimits,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<SignedObject, VoiceError> {
        self.current(authority)?;
        let Body::Announcement {
            key,
            epoch,
            renewal,
        } = announcement.body()
        else {
            return Err(VoiceError::Malformed);
        };
        if announcement.scope() != self.scope || limits.group() != self.scope.group {
            return Err(VoiceError::Scope);
        }
        if announcement.issuer() == self.local
            || !(authority.member)(announcement.issuer())
            || DeviceId::from_public_key_bytes(&signer.voice_public_key()?) != self.local
        {
            return Err(VoiceError::Unauthorized);
        }
        announcement.check_freshness(clock.now_ms())?;
        if *epoch > authority.context.epoch {
            return Err(VoiceError::Freshness);
        }
        self.prune(authority, clock);
        let remaining = announcement
            .expires()
            .saturating_add(CLOCK_SKEW_MS)
            .saturating_sub(clock.now_ms());
        if remaining == 0 {
            return Err(VoiceError::Freshness);
        }
        limits.join(
            announcement.issuer(),
            key.join,
            key.key_ref,
            clock.monotonic_ms(),
            clock
                .monotonic_ms()
                .saturating_add(remaining.min(MAX_REMOTE_DELEGATION_REMAINING_MS)),
        )?;
        let cached = self.pending.values().find(|pending| pending.announcement.issuer() == announcement.issuer()
            && matches!(pending.announcement.body(), Body::Announcement { key: original, .. } if original == key));
        let renewal_check = limits.renewal(
            announcement.issuer(),
            key.join,
            key.key_ref,
            *renewal,
            clock.monotonic_ms(),
        );
        // A newer announcement updates the actor high-water mark even while an older challenge
        // remains pending. Retransmission may finish that exact request, never mint a new one.
        if let Some(pending) = cached {
            if renewal_check != Err(VoiceError::Freshness) {
                renewal_check?;
            }
            pending.object.check_freshness(clock.now_ms())?;
            return Ok(pending.object.clone());
        }
        renewal_check?;
        if self.pending.len() >= MAX_PENDING {
            return Err(VoiceError::Capacity);
        }
        let ordinal = self.serial.checked_add(1).ok_or(VoiceError::Capacity)?;
        let credit =
            self.resources
                .charge(self.scope.group, RECORD_BYTES, ResourceKind::Challenge)?;
        let issued = clock.now_ms();
        // No skew allowance extends the local 15-second challenge lifetime. A signed challenge
        // cannot outlive the original delegation; a stale delegation cannot create new work.
        let expires = issued
            .saturating_add(MAX_PACKET_LIFETIME_MS)
            .min(announcement.expires());
        if expires <= issued {
            return Err(VoiceError::Freshness);
        }
        let mut id = [0; 32];
        rng.fill_bytes(&mut id);
        if self.pending.contains_key(&id) {
            return Err(VoiceError::Capacity);
        }
        let mut nonce = [0; 32];
        rng.fill_bytes(&mut nonce);
        let object = SignedObject::issue(
            self.scope,
            signer,
            id,
            issued,
            expires,
            Body::WatchChallenge {
                watcher: self.incarnation,
                target: announcement.issuer(),
                join: key.join,
                key_ref: key.key_ref,
                challenge: nonce,
            },
        )?;
        self.serial = ordinal;
        self.pending.insert(
            id,
            Challenge {
                announcement: announcement.clone(),
                object: object.clone(),
                ordinal,
                deadline: clock.monotonic_ms().saturating_add(expires - issued),
                _credit: credit,
            },
        );
        Ok(object)
    }
    /// Admit a matching fresh proof once. A newer accepted challenge retires ALL older pending
    /// challenges for that issuer, so deleting an expired visible row cannot resurrect an older
    /// join via its delayed proof. Ordering protection therefore outlives display retention.
    pub fn prove(
        &mut self,
        proof: &SignedObject,
        authority: &WatchAuthority<'_>,
        clock: &dyn Clock,
    ) -> Result<FreshPeer, VoiceError> {
        self.current(authority)?;
        if proof.scope() != self.scope {
            return Err(VoiceError::Scope);
        }
        proof.check_freshness(clock.now_ms())?;
        self.prune(authority, clock);
        let Body::JoinProof {
            join,
            key_ref,
            watcher,
            watcher_incarnation,
            challenge,
            challenge_id,
        } = proof.body()
        else {
            return Err(VoiceError::Malformed);
        };
        if *watcher != self.local
            || *watcher_incarnation != self.incarnation
            || !(authority.member)(proof.issuer())
        {
            return Err(VoiceError::Unauthorized);
        }
        let pending = self.pending.get(challenge_id).ok_or(VoiceError::Replay)?;
        if pending.announcement.issuer() != proof.issuer()
            || !matches!(pending.announcement.body(), Body::Announcement { key, .. } if key.join == *join && key.key_ref == *key_ref)
            || !matches!(pending.object.body(), Body::WatchChallenge { challenge: nonce, .. } if nonce == challenge)
            || self
                .present
                .get(&proof.issuer())
                .is_some_and(|peer| peer.ordinal() >= pending.ordinal)
        {
            return Err(VoiceError::Unauthorized);
        }
        pending.announcement.check_freshness(clock.now_ms())?;
        pending.object.check_freshness(clock.now_ms())?;
        // Bound even malicious original issuers to the challenge/delegation window. Original
        // signatures cannot authorize a proof to create a longer observation interval.
        if proof.expires() > pending.object.expires() {
            return Err(VoiceError::Freshness);
        }
        // Reuse a uniquely held row's affine credit only after all fallible validation. Actual
        // external clones retain the old credit and require fresh capacity for replacement.
        // With strong_count==1 and no exposed Weak handle, try_unwrap cannot race a new clone.
        let unique = self
            .present
            .get(&proof.issuer())
            .is_some_and(|old| Arc::strong_count(&old.0) == 1);
        let credit = if unique {
            let old = self.present.remove(&proof.issuer()).expect("checked row");
            old.0.live.revoke();
            Arc::try_unwrap(old.0)
                .ok()
                .expect("uniquely held observation")
                ._credit
        } else {
            self.resources
                .charge(self.scope.group, RECORD_BYTES, ResourceKind::Presence)?
        };
        let peer = FreshPeer(Arc::new(Observation {
            announcement: pending.announcement.clone(),
            proof: proof.clone(),
            watcher: self.local,
            owner: self.lease.clone(),
            live: CallLease::default(),
            ordinal: pending.ordinal,
            deadline: pending.deadline,
            _credit: credit,
        }));
        if let Some(old) = self.present.insert(proof.issuer(), peer.clone()) {
            old.0.live.revoke();
        }
        self.pending.retain(|_, pending| {
            pending.announcement.issuer() != proof.issuer() || pending.ordinal > peer.ordinal()
        });
        Ok(peer)
    }
    pub fn peers(
        &mut self,
        authority: &WatchAuthority<'_>,
        clock: &dyn Clock,
    ) -> Result<Vec<FreshPeer>, VoiceError> {
        self.current(authority)?;
        self.prune(authority, clock);
        Ok(self.present.values().cloned().collect())
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.lease.revoke();
    }
}

struct CachedProof {
    challenge: SignedObject,
    proof: SignedObject,
    deadline: u64,
    _credit: Credit,
}

/// Active native participant with one immutable browser public key and one packet allocator.
/// Only the exact active join can answer watch challenges. The eventual Endpoint owns this
/// together with key confirmation/negotiation state; a public IPC must never expose its signer.
pub struct LocalJoin {
    scope: Scope,
    local: DeviceId,
    key: JoinKey,
    lease: CallLease,
    ledger: PacketLedger,
    resources: Arc<Resources>,
    renewal: u64,
    next_renewal: u64,
    announcement: Option<SignedObject>,
    proofs: BTreeMap<(DeviceId, MessageId), CachedProof>,
    confirmations: confirmation::Confirmations,
    actions: super::crypto_actions::ActionQueue,
}
impl LocalJoin {
    /// Prepare the supported one-pair SDP exchange only after real native confirmation. No
    /// outgoing SDP nonce or incoming positive admission may precede this separate readiness
    /// step. Capacity refusal leaves confirmation, retained history and counters intact.
    /// `limits` must be the actual actor's shared group owner; `helpers` selects at most two
    /// distinct current member identities for this immutable exchange. Prepay their normal
    /// requester/origin records before reserving the workflow; no rate bucket is reset/spent.
    pub fn prepare_sdp_exchange(
        &mut self,
        handshake: Nonce,
        authority: &Authority<'_>,
        limits: &mut OriginLimits,
        helpers: &[DeviceId],
        clock: &dyn Clock,
    ) -> Result<Arc<super::negotiation::Exchange>, VoiceError> {
        self.pair_current(handshake, authority, clock)?;
        if !self.confirmations.established(handshake) {
            return Err(VoiceError::Unauthorized);
        }
        self.ledger
            .prepare_sdp_exchange(handshake, authority, limits, helpers, clock)
    }
    /// Pending application seal authority, bound to the actual unsigned native reservation.
    /// Confirmation actions use their stronger action/attempt factories instead.
    pub fn packet_encryption_authority(
        &self,
        reservation: &super::packets::ReservedPacket,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
    ) -> Result<super::crypto_authority::CryptoAuthority, VoiceError> {
        self.owned(authority)?;
        self.ledger
            .outgoing_crypto_authority(reservation, false, authority, clock)
    }
    /// Pending decrypt authority consumes no token. Proof/Finish capture the admitting native
    /// attempt, so nonce restart closes an old queued proof. Hello candidates instead capture
    /// the stable accepted-sequence floor, preserving newer work across reordered completion.
    pub fn incoming_crypto_authority(
        &mut self,
        event: &super::packets::InboundPacket,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
    ) -> Result<super::crypto_authority::CryptoAuthority, VoiceError> {
        self.owned(authority)?;
        let confirmation = matches!(
            event.header.kind,
            EncryptedKind::ConfirmHello
                | EncryptedKind::ConfirmProof
                | EncryptedKind::ConfirmFinish
        );
        if confirmation {
            self.pair_current(event.header.handshake, authority, clock.as_ref())?;
        }
        let grant = self
            .ledger
            .incoming_crypto_authority(event, authority, clock)?;
        if confirmation {
            let pair = &self.confirmations.pairs[&event.header.handshake];
            grant.limit_deadline(pair.deadline)
        } else {
            Ok(grant)
        }
    }
    /// Expire caches without a live UI/actor grant. Pair/sequence/replay records survive until
    /// this join ends; collection must never make an old handshake or nonce allocatable again.
    pub fn collect_expired(&mut self, clock: &dyn Clock) {
        let mono = clock.monotonic_ms();
        self.proofs
            .retain(|_, cached| self.lease.is_live() && mono < cached.deadline);
        self.collect_confirmations(clock);
        self.ledger.collect_expired(clock);
    }
    /// Trusted native action transition. Endpoint OfferId/confirmation ownership must call this
    /// on supersession before starting its replacement; it is not a general WebView command.
    pub fn retire_outgoing(
        &self,
        id: MessageId,
        authority: &Authority<'_>,
    ) -> Result<(), VoiceError> {
        self.owned(authority)?;
        self.ledger.retire_outgoing(id, authority)
    }
    /// Only the actual cached outgoing ledger packet gets an encrypted source grant. Its pair
    /// lease closes on timeout/newer-pair promotion even when this local join remains active.
    pub fn source_packet_activity(
        &self,
        object: &SignedObject,
        authority: &Authority<'_>,
        clock: Arc<dyn Clock>,
    ) -> Result<super::source::SourceActivity, VoiceError> {
        self.owned(authority)?;
        let packet = self.ledger.source_authority(object, authority, clock)?;
        let packet = match object.body() {
            Body::Encrypted { header, .. }
                if matches!(
                    header.kind,
                    EncryptedKind::ConfirmHello
                        | EncryptedKind::ConfirmProof
                        | EncryptedKind::ConfirmFinish
                ) =>
            {
                // Even Finish, which promotes the local pair before delivery, belongs to the
                // original confirmation window. Later packet expiry does not extend that window.
                packet.limit_deadline(self.confirmations.source_deadline(header.handshake)?)?
            }
            _ => packet,
        };
        Ok(self.source_activity(authority)?.with_packet(packet))
    }
    /// Focused owner tests may set up a native ledger pair without claiming a live handshake.
    #[cfg(test)]
    pub(crate) fn packet_ledger_for_test(&mut self) -> &mut PacketLedger {
        &mut self.ledger
    }
    pub fn source_activity(
        &self,
        authority: &Authority<'_>,
    ) -> Result<super::source::SourceActivity, VoiceError> {
        self.owned(authority)?;
        Ok(super::source::SourceActivity::joined(
            self.scope,
            self.local,
            self.key.clone(),
            self.lease.clone(),
        ))
    }
    pub fn new(
        context: &Context,
        key: JoinKey,
        lease: CallLease,
        resources: Arc<Resources>,
    ) -> Result<Self, VoiceError> {
        key.validate()?;
        let ledger = PacketLedger::new(
            context,
            key.join,
            key.key_ref,
            lease.clone(),
            resources.clone(),
        )?;
        let actions = super::crypto_actions::ActionQueue::with_slots(ledger.action_slots());
        Ok(Self {
            scope: context.scope,
            local: context.local,
            key,
            lease,
            ledger,
            resources,
            renewal: 0,
            next_renewal: 0,
            announcement: None,
            proofs: BTreeMap::new(),
            confirmations: confirmation::Confirmations::default(),
            actions,
        })
    }
    pub fn key(&self) -> &JoinKey {
        &self.key
    }
    /// Membership removal ends this native join if it is local, or permanently retires the
    /// removed remote member's packet pairs/proof retries. Re-add needs fresh admission.
    pub fn retire_member(&mut self, member: DeviceId) {
        if member == self.local {
            self.lease.revoke();
        }
        self.proofs.retain(|(issuer, _), _| *issuer != member);
        self.ledger.retire_member(member);
        self.confirmations.retire_member(member);
    }
    fn current(
        &self,
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
    ) -> Result<(), VoiceError> {
        self.owned(authority)?;
        if DeviceId::from_public_key_bytes(&signer.voice_public_key()?) != self.local {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    fn owned(&self, authority: &Authority<'_>) -> Result<(), VoiceError> {
        if !self.lease.is_live()
            || !self.lease.is_same(authority.lease)
            || !authority.lease.is_live()
            || self.scope != authority.context.scope
            || self.local != authority.context.local
            || !(authority.member)(self.local)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    pub fn announce(
        &mut self,
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<SignedObject, VoiceError> {
        self.current(signer, authority)?;
        if let Some(object) = &self.announcement {
            if clock.monotonic_ms() < self.next_renewal
                && object.check_freshness(clock.now_ms()).is_ok()
                && matches!(object.body(), Body::Announcement { epoch, .. } if *epoch == authority.context.epoch)
            {
                return Ok(object.clone());
            }
        }
        let renewal = self.renewal.checked_add(1).ok_or(VoiceError::Capacity)?;
        let mut id = [0; 32];
        rng.fill_bytes(&mut id);
        // One observation defines the signed interval. Two samples can cross a millisecond and
        // exceed the constructor's exact lifetime cap; saturation could also hide overflow.
        let issued = clock.now_ms();
        let expires = issued
            .checked_add(MAX_DELEGATION_LIFETIME_MS)
            .ok_or(VoiceError::Freshness)?;
        let object = SignedObject::issue(
            self.scope,
            signer,
            id,
            issued,
            expires,
            Body::Announcement {
                key: self.key.clone(),
                epoch: authority.context.epoch,
                renewal,
            },
        )?;
        self.renewal = renewal;
        self.next_renewal = clock.monotonic_ms().saturating_add(RENEW_MS);
        self.announcement = Some(object.clone());
        Ok(object)
    }
    pub fn answer(
        &mut self,
        challenge: &SignedObject,
        signer: &dyn VoiceSigner,
        authority: &Authority<'_>,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<SignedObject, VoiceError> {
        self.current(signer, authority)?;
        if challenge.scope() != self.scope {
            return Err(VoiceError::Scope);
        }
        challenge.check_freshness(clock.now_ms())?;
        let Body::WatchChallenge {
            watcher,
            target,
            join,
            key_ref,
            challenge: nonce,
        } = challenge.body()
        else {
            return Err(VoiceError::Malformed);
        };
        if *target != self.local
            || *join != self.key.join
            || *key_ref != self.key.key_ref
            || !(authority.member)(challenge.issuer())
        {
            return Err(VoiceError::Unauthorized);
        }
        self.proofs.retain(|_, cached| {
            clock.monotonic_ms() < cached.deadline && (authority.member)(cached.challenge.issuer())
        });
        let cache_id = (challenge.issuer(), challenge.id());
        if let Some(cached) = self.proofs.get(&cache_id) {
            if cached.challenge != *challenge {
                return Err(VoiceError::RetryMismatch);
            }
            cached.proof.check_freshness(clock.now_ms())?;
            return Ok(cached.proof.clone());
        }
        if self.proofs.len() >= MAX_PENDING {
            return Err(VoiceError::Capacity);
        }
        let credit =
            self.resources
                .charge(self.scope.group, RECORD_BYTES, ResourceKind::Retained)?;
        let issued = clock.now_ms();
        let expires = challenge
            .expires()
            .min(issued.saturating_add(MAX_PACKET_LIFETIME_MS));
        let mut id = [0; 32];
        rng.fill_bytes(&mut id);
        let proof = SignedObject::issue(
            self.scope,
            signer,
            id,
            issued,
            expires,
            Body::JoinProof {
                join: self.key.join,
                key_ref: self.key.key_ref,
                watcher: challenge.issuer(),
                watcher_incarnation: *watcher,
                challenge: *nonce,
                challenge_id: challenge.id(),
            },
        )?;
        self.proofs.insert(
            cache_id,
            CachedProof {
                challenge: challenge.clone(),
                proof: proof.clone(),
                deadline: clock
                    .monotonic_ms()
                    .saturating_add(expires.saturating_sub(issued).saturating_add(CLOCK_SKEW_MS)),
                _credit: credit,
            },
        );
        Ok(proof)
    }
}
redacted_debug!(WatchAuthority<'_>, FreshPeer, Watcher, LocalJoin);

#[cfg(test)]
mod tests;
