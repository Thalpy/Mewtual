//! Bounded native control intake with one public/encrypted identity namespace. Announcements
//! remain candidates; only a current native watch challenge/proof promotes presence. Protected
//! Confirmation admission is custody only, and does not establish a pair or enable live dispatch.
use super::*;
use catcoms_rt::RequestAuthority;
use voice_v2::helper::RoomAuthority;
use voice_v2::packets::Authority;
use voice_v2::presence::{FreshPeer, WatchAuthority};
use voice_v2::runtime::{EffectAuthority, IngressTicket};
use voice_v2::source::SourceActivity;
use voice_v2::{Body, Context, Envelope, MessageId, SignedObject};

const PACKETS: usize = 256;
const WORK_PER_PASS: usize = 4;
type Key = (DeviceId, MessageId);

enum Targets {
    Watch(Vec<(u64, WatchLease)>),
    Join(u64, CallLease),
}

mod encrypted;
#[cfg(test)]
mod tests;
pub(crate) use encrypted::{ConfirmationReceipt, EndpointReceipt};
impl Targets {
    fn live(&self) -> bool {
        match self {
            Self::Watch(targets) => targets.iter().any(|(_, lease)| lease.is_live()),
            Self::Join(_, lease) => lease.is_live(),
        }
    }
    async fn revoked(&self) {
        match self {
            // Admission originally selected ANY live observer. Wait for ALL original targets;
            // a retired observer must neither cancel another observer nor create a ready loop.
            Self::Watch(targets) => {
                futures::future::join_all(targets.iter().map(|(_, lease)| lease.wait_revoked()))
                    .await;
            }
            Self::Join(_, lease) => lease.wait_revoked().await,
        }
    }
}
struct Packet {
    object: SignedObject,
    hash: [u8; 32],
    targets: Targets,
    room: RoomAuthority,
    effect: Arc<EffectAuthority>,
    deadline: u64,
    _credit: Credit,
}
impl Packet {
    fn candidate_live(&self) -> bool {
        self.room.lease.is_live() && self.effect.authorized()
    }
    fn admitted(&self) -> bool {
        self.candidate_live() && self.targets.live()
    }
}
/// Exact original queue and original native recipients, retained through response signing.
/// Replacing a watcher cannot turn an old cached ACK into admission by the new native owner.
pub(crate) struct PublicReceipt(Arc<Packet>);
impl PublicReceipt {
    pub(crate) fn authorized(&self) -> bool {
        self.0.admitted()
    }
    pub(crate) fn object_hash(&self) -> [u8; 32] {
        self.0.hash
    }
    pub(crate) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        self.0.effect.authorized_at(wall, mono)
            && self.0.room.lease.is_live()
            && self.0.targets.live()
    }
    pub(crate) async fn revoked(&self) {
        use futures::FutureExt;
        if !self.authorized() {
            return;
        }
        let waits = vec![
            self.0.effect.revoked().boxed(),
            self.0.room.lease.wait_revoked().boxed(),
            self.0.targets.revoked().boxed(),
        ];
        futures::future::select_all(waits).await;
    }
}
struct Work {
    packet: Arc<Packet>,
    processed: BTreeSet<u64>,
}
#[derive(Default)]
pub(super) struct PublicInbox {
    packets: BTreeMap<Key, Work>,
    // Two work representations share ONE classifier and combined capacity. Never insert in
    // either map before check_identity: public control cannot reuse an encrypted admission ID.
    encrypted: BTreeMap<Key, ConfirmationReceipt>,
    cursor: Option<Key>,
}
impl PublicInbox {
    fn check_identity(&self, object: &SignedObject) -> Result<(), VoiceError> {
        let key = (object.issuer(), object.id());
        if self
            .packets
            .get(&key)
            .is_some_and(|work| work.packet.object != *object)
            || self
                .encrypted
                .get(&key)
                .is_some_and(|receipt| receipt.object_hash() != object.hash())
        {
            return Err(VoiceError::RetryMismatch);
        }
        Ok(())
    }
    fn full(&self) -> bool {
        self.packets.len() + self.encrypted.len() >= PACKETS
    }
    pub(super) fn prune(&mut self, clock: &dyn Clock) {
        let now = clock.now_ms();
        let mono = clock.monotonic_ms();
        self.packets.retain(|_, work| {
            now <= work
                .packet
                .object
                .expires()
                .saturating_add(voice_v2::CLOCK_SKEW_MS)
                || mono < work.packet.deadline
                || Arc::strong_count(&work.packet) > 1
        });
        // Three identities per incoming role (at most six for the high endpoint). Keep them for
        // the original call, including closed-rejected results; actual ACK holders may outlive
        // End. This fixed metadata does not retain an action, engine or Workflow.
        self.encrypted.retain(|_, receipt| receipt.retain());
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub(crate) fn admit_voice_endpoint_public(
        &mut self,
        ticket: IngressTicket,
        envelope: Envelope,
        context: &Context,
        room: RoomAuthority,
    ) -> Result<PublicReceipt, VoiceError> {
        // Caller has already authenticated the exact frame and final-delivery role. Never let
        // public control intake bypass encrypted replay/confirmation/negotiation ownership.
        if matches!(envelope.object().body(), Body::Encrypted { .. }) {
            return Err(VoiceError::Unauthorized);
        }
        let clock = self.voice_v2_clock();
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        voice.endpoints.prune();
        voice.endpoints.public.prune(clock.as_ref());
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        if actor.authorize_verified_route(&ticket, &envelope, context)?
            != voice_v2::Disposition::Deliver
        {
            return Err(VoiceError::Unauthorized);
        }
        let key = (envelope.object().issuer(), envelope.object().id());
        voice.endpoints.public.check_identity(envelope.object())?;
        if let Some(work) = voice.endpoints.public.packets.get(&key) {
            if work.packet.object != *envelope.object() {
                return Err(VoiceError::RetryMismatch);
            }
            if !work.packet.admitted() {
                return Err(VoiceError::Unauthorized);
            }
            ticket.check(actor, &envelope.encode())?;
            return Ok(PublicReceipt(work.packet.clone()));
        }
        if voice.endpoints.public.full() {
            return Err(VoiceError::Capacity);
        }
        let targets = match envelope.object().body() {
            Body::Announcement { .. } => Targets::Watch(
                voice
                    .endpoints
                    .observers
                    .iter()
                    .filter(|(_, observer)| {
                        observer.scope == context.scope && observer.lease.is_live()
                    })
                    .map(|(id, observer)| (*id, observer.lease.clone()))
                    .collect(),
            ),
            Body::JoinProof { .. } => Targets::Watch(
                voice
                    .endpoints
                    .observers
                    .iter()
                    .filter(|(_, observer)| {
                        observer.scope == context.scope
                            && observer.lease.is_live()
                            && observer.watcher.targets_proof(envelope.object())
                    })
                    .map(|(id, observer)| (*id, observer.lease.clone()))
                    .collect(),
            ),
            Body::WatchChallenge {
                target,
                join,
                key_ref,
                ..
            } => {
                let participant = voice
                    .endpoints
                    .participant
                    .as_ref()
                    .ok_or(VoiceError::Unauthorized)?;
                if *target != context.local
                    || participant
                        .grant
                        .upgrade()
                        .is_none_or(|grant| grant.scope() != context.scope)
                    || participant.join.key().join != *join
                    || participant.join.key().key_ref != *key_ref
                {
                    return Err(VoiceError::Unauthorized);
                }
                Targets::Join(participant.watch.id, participant.lease.clone())
            }
            Body::Encrypted { .. } => unreachable!(),
        };
        if !targets.live() || room.scope != context.scope || !room.lease.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let bytes = envelope.object().encode().len();
        let credit =
            voice
                .resources
                .charge(context.scope.group, bytes + 1024, ResourceKind::Retained)?;
        let effect = actor.effect(
            envelope.object().issuer(),
            context.local,
            envelope.object().expires(),
        )?;
        let remaining = envelope
            .object()
            .expires()
            .saturating_add(voice_v2::CLOCK_SKEW_MS)
            .saturating_sub(clock.now_ms());
        if remaining == 0 || remaining > voice_v2::MAX_REMOTE_DELEGATION_REMAINING_MS {
            return Err(VoiceError::Freshness);
        }
        let deadline = clock.monotonic_ms().saturating_add(remaining);
        let packet = Arc::new(Packet {
            hash: envelope.object().hash(),
            object: envelope.object().clone(),
            targets,
            room,
            effect,
            deadline,
            _credit: credit,
        });
        // No fallible queue step follows this native origin/consent transaction. Cached retries
        // above spend requester work, but never the original issuer's allowance a second time.
        actor.commit_endpoint_origin(&ticket, &envelope, context, |wall, mono| {
            if packet.effect.authorized_at(wall, mono)
                && packet.room.lease.is_live()
                && packet.targets.live()
            {
                Ok(())
            } else {
                Err(VoiceError::Unauthorized)
            }
        })?;
        voice.endpoints.public.packets.insert(
            key,
            Work {
                packet: packet.clone(),
                processed: BTreeSet::new(),
            },
        );
        voice.endpoints.changed();
        Ok(PublicReceipt(packet))
    }

    /// One native pass signs/queues at most four public responses. Source capacity refusal keeps
    /// the exact Watcher/LocalJoin cache for retry; member/room/parent retirement closes old work.
    /// `voice_schedule` runs it each sync tick, after served kind28 requests and completions,
    /// and on its quiet-network wake while a join or watcher is live.
    pub fn pump_voice_endpoint(&mut self) -> Result<(), VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        // A published confirmation/SDP/ICE packet must not depend on a caller retaining a
        // copied payload or explicitly queueing it. Run before public response work, whose
        // four-item bound may return early. Source rechecks the exact original authority.
        self.pump_voice_packet_sources()?;
        let clock = self.voice_v2_clock();
        let members: HashSet<_> = self.group.member_device_ids().into_iter().collect();
        let announcement = {
            let voice = self
                .voice_runtime
                .as_mut()
                .ok_or(VoiceError::Unauthorized)?;
            voice.endpoints.prune();
            if let Some(participant) = &mut voice.endpoints.participant {
                let context = voice
                    .current
                    .as_ref()
                    .ok_or(VoiceError::Unauthorized)?
                    .source_context(participant.watch.scope)?;
                let authority = Authority {
                    context: &context,
                    lease: &participant.lease,
                    member: &|id| members.contains(&id),
                };
                Some((
                    participant.join.announce(
                        &self.device,
                        &authority,
                        clock.as_ref(),
                        &mut self.rng,
                    )?,
                    participant.join.source_activity(&authority)?,
                ))
            } else {
                None
            }
        };
        if let Some((object, activity)) = announcement {
            if let Err(error) = self.queue_voice_source(object, activity) {
                if error != VoiceError::Capacity && error != VoiceError::RateLimited {
                    return Err(error);
                }
            }
        }
        let keys = {
            let voice = self
                .voice_runtime
                .as_mut()
                .ok_or(VoiceError::Unauthorized)?;
            voice.endpoints.public.prune(clock.as_ref());
            let mut keys: Vec<_> = voice.endpoints.public.packets.keys().copied().collect();
            if let Some(cursor) = voice.endpoints.public.cursor {
                keys.sort_by_key(|key| (*key <= cursor, *key));
            }
            keys
        };
        let mut processed = 0;
        for key in keys {
            let targets = {
                let endpoints = &mut self
                    .voice_runtime
                    .as_mut()
                    .ok_or(VoiceError::Unauthorized)?
                    .endpoints;
                let work = &mut endpoints
                    .public
                    .packets
                    .get_mut(&key)
                    .expect("bounded key snapshot");
                if !work.packet.candidate_live() {
                    continue;
                }
                // A retained Announcement is only a candidate for a replacement watcher. It
                // starts a NEW native challenge, never adopts old proof/ACK authority or spends
                // original quota again. Old target progress may be discarded after native death.
                let mut current: Vec<_> = match work.packet.object.body() {
                    Body::Announcement { .. } => endpoints
                        .observers
                        .iter()
                        .filter(|(_, observer)| {
                            observer.scope == work.packet.object.scope() && observer.lease.is_live()
                        })
                        .map(|(id, _)| *id)
                        .collect(),
                    _ => match &work.packet.targets {
                        Targets::Watch(targets) => targets
                            .iter()
                            .filter(|(_, lease)| lease.is_live())
                            .map(|(id, _)| *id)
                            .collect(),
                        Targets::Join(id, lease) => {
                            if lease.is_live() {
                                vec![*id]
                            } else {
                                Vec::new()
                            }
                        }
                    },
                };
                work.processed.retain(|id| current.contains(id));
                current.retain(|id| !work.processed.contains(id));
                current
            };
            for id in targets {
                let result = self.process_voice_public(key, id);
                let success = match result {
                    Ok(Some((object, activity))) => {
                        self.queue_voice_source(object, activity).map(|_| ())
                    }
                    Ok(None) => Ok(()),
                    Err(error) => Err(error),
                };
                let endpoints = &mut self
                    .voice_runtime
                    .as_mut()
                    .ok_or(VoiceError::Unauthorized)?
                    .endpoints;
                if success.as_ref().err().is_none_or(|error| {
                    *error != VoiceError::Capacity && *error != VoiceError::RateLimited
                }) {
                    // A malformed proof may have entered the authenticated endpoint queue; it
                    // remains a terminal rejection and never becomes liveness/key evidence.
                    endpoints
                        .public
                        .packets
                        .get_mut(&key)
                        .expect("current work")
                        .processed
                        .insert(id);
                }
                endpoints.public.cursor = Some(key);
                processed += 1;
                if processed >= WORK_PER_PASS {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
    fn process_voice_public(
        &mut self,
        key: Key,
        id: u64,
    ) -> Result<Option<(SignedObject, SourceActivity)>, VoiceError> {
        let clock = self.voice_v2_clock();
        let members: HashSet<_> = self.group.member_device_ids().into_iter().collect();
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        let endpoints = &mut voice.endpoints;
        let packet = endpoints
            .public
            .packets
            .get(&key)
            .ok_or(VoiceError::Reservation)?
            .packet
            .clone();
        if !packet.candidate_live() {
            return Err(VoiceError::Unauthorized);
        }
        let actor = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let context = actor.source_context(packet.object.scope())?;
        match packet.object.body() {
            Body::Announcement { .. } | Body::JoinProof { .. } => {
                let observer = endpoints
                    .observers
                    .get_mut(&id)
                    .ok_or(VoiceError::Unauthorized)?;
                let authority = WatchAuthority {
                    context: &context,
                    lease: &observer.lease,
                    member: &|id| members.contains(&id),
                };
                if matches!(packet.object.body(), Body::Announcement { .. }) {
                    let object = actor.with_limits(|limits| {
                        observer.watcher.challenge(
                            &packet.object,
                            &self.device,
                            &authority,
                            limits,
                            clock.as_ref(),
                            &mut self.rng,
                        )
                    })?;
                    Ok(Some((
                        object,
                        observer.watcher.source_activity(&authority)?,
                    )))
                } else {
                    observer
                        .watcher
                        .prove(&packet.object, &authority, clock.as_ref())?;
                    endpoints.changed();
                    Ok(None)
                }
            }
            Body::WatchChallenge { .. } => {
                let participant = endpoints
                    .participant
                    .as_mut()
                    .filter(|participant| participant.watch.id == id)
                    .ok_or(VoiceError::Unauthorized)?;
                let authority = Authority {
                    context: &context,
                    lease: &participant.lease,
                    member: &|id| members.contains(&id),
                };
                let object = participant.join.answer(
                    &packet.object,
                    &self.device,
                    &authority,
                    clock.as_ref(),
                    &mut self.rng,
                )?;
                Ok(Some((
                    object,
                    participant.join.source_activity(&authority)?,
                )))
            }
            Body::Encrypted { .. } => Err(VoiceError::Unauthorized),
        }
    }
    pub fn voice_watch_peers(
        &mut self,
        grant: &VoiceWatchGrant,
    ) -> Result<Vec<FreshPeer>, VoiceError> {
        self.validate_voice_watch(grant)?;
        let context = self.voice_v2_context(grant.scope.room)?;
        let clock = self.voice_v2_clock();
        let members: HashSet<_> = self.group.member_device_ids().into_iter().collect();
        let observer = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?
            .endpoints
            .observers
            .get_mut(&grant.id)
            .ok_or(VoiceError::Unauthorized)?;
        observer.watcher.peers(
            &WatchAuthority {
                context: &context,
                lease: &observer.lease,
                member: &|id| members.contains(&id),
            },
            clock.as_ref(),
        )
    }

    /// Observe the participant's ORIGINAL implicit watcher. A separate explicit observer or
    /// equal-scope grant cannot establish readiness for this participant's protected pairs.
    /// Returned evidence owns its existing native charge; it grants no engine/RTC authority.
    pub fn voice_participant_peers(
        &mut self,
        grant: &Arc<CallGrant>,
    ) -> Result<Vec<FreshPeer>, VoiceError> {
        self.validate_voice_call_grant(grant)?;
        let endpoints = &mut self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?
            .endpoints;
        endpoints.prune();
        let participant = endpoints
            .participant
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if !std::sync::Weak::ptr_eq(&participant.grant, &Arc::downgrade(grant))
            || !participant.lease.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        let watch = participant.watch.clone();
        self.voice_watch_peers(&watch)
    }

    /// Paid version for a long-lived native producer. Its original metadata owner follows
    /// the command and completed output independently of the observing caller. A foreign
    /// pool/scope cannot debit this Actor or convert equal public labels into call authority.
    pub fn voice_participant_peer_snapshot(
        &mut self,
        grant: &Arc<CallGrant>,
        permit: voice_v2::presence::PeerSnapshotPermit,
    ) -> voice_v2::presence::PeerSnapshot {
        let result = (|| {
            let context = self.voice_v2_context(grant.scope().room)?;
            if permit.scope() != grant.scope()
                || context.scope != permit.scope()
                || !self
                    .voice_runtime
                    .as_ref()
                    .and_then(|voice| voice.current.as_ref())
                    .is_some_and(|actor| actor.uses_resources(permit.resources()))
            {
                return Err(VoiceError::Unauthorized);
            }
            self.voice_participant_peers(grant)
                .map(|peers| (context.epoch, peers))
        })();
        permit.complete(result)
    }
}
