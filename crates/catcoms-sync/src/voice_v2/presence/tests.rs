use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_rt::ManualClock;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn member(_: DeviceId) -> bool {
    true
}
fn wa<'a>(context: &'a Context, lease: &'a WatchLease) -> WatchAuthority<'a> {
    WatchAuthority {
        context,
        lease,
        member: &member,
    }
}
fn ca<'a>(context: &'a Context, lease: &'a CallLease) -> Authority<'a> {
    Authority {
        context,
        lease,
        member: &member,
    }
}
fn context(seed: u8) -> (DeviceKeypair, Context) {
    let signer = DeviceKeypair::from_seed(&[seed; 32]);
    let context = Context {
        scope: Scope::for_group(b"fresh exact room", Some([8; 32]), 123),
        local: signer.device_id(),
        now: 1000,
        epoch: 7,
        indirect: true,
    };
    (signer, context)
}
fn key(seed: u8) -> JoinKey {
    let point = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap();
    JoinKey::new([seed; 32], point.try_into().unwrap()).unwrap()
}
struct Participant {
    signer: DeviceKeypair,
    context: Context,
    lease: CallLease,
    join: LocalJoin,
}
impl Participant {
    fn new(seed: u8, pool: Arc<Resources>) -> Self {
        let (signer, context) = context(seed);
        let lease = CallLease::default();
        let join = LocalJoin::new(&context, key(seed), lease.clone(), pool).unwrap();
        Self {
            signer,
            context,
            lease,
            join,
        }
    }
    fn announce(&mut self, clock: &ManualClock, rng: &mut ChaCha20Rng) -> SignedObject {
        self.join
            .announce(&self.signer, &ca(&self.context, &self.lease), clock, rng)
            .unwrap()
    }
    fn answer(
        &mut self,
        challenge: &SignedObject,
        clock: &ManualClock,
        rng: &mut ChaCha20Rng,
    ) -> Result<SignedObject, VoiceError> {
        self.join.answer(
            challenge,
            &self.signer,
            &ca(&self.context, &self.lease),
            clock,
            rng,
        )
    }
}
struct Observer {
    signer: DeviceKeypair,
    context: Context,
    lease: WatchLease,
    watcher: Watcher,
}
impl Observer {
    fn new(seed: u8, pool: Arc<Resources>, rng: &mut ChaCha20Rng) -> Self {
        let (signer, context) = context(seed);
        let lease = WatchLease::default();
        let watcher = Watcher::new(&context, lease.clone(), pool, rng).unwrap();
        Self {
            signer,
            context,
            lease,
            watcher,
        }
    }
    fn challenge(
        &mut self,
        announcement: &SignedObject,
        limits: &mut OriginLimits,
        clock: &ManualClock,
        rng: &mut ChaCha20Rng,
    ) -> Result<SignedObject, VoiceError> {
        self.watcher.challenge(
            announcement,
            &self.signer,
            &wa(&self.context, &self.lease),
            limits,
            clock,
            rng,
        )
    }
    fn prove(
        &mut self,
        proof: &SignedObject,
        clock: &ManualClock,
    ) -> Result<FreshPeer, VoiceError> {
        self.watcher
            .prove(proof, &wa(&self.context, &self.lease), clock)
    }
    fn peers(&mut self, clock: &ManualClock) -> Vec<FreshPeer> {
        self.watcher
            .peers(&wa(&self.context, &self.lease), clock)
            .unwrap()
    }
}
fn limits(observer: &Observer, pool: Arc<Resources>) -> OriginLimits {
    OriginLimits::new(observer.context.scope.group, pool)
}
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::from_seed([99; 32])
}

#[test]
fn announcement_is_not_presence_and_exact_challenge_proof_expires_from_creation() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut participant = Participant::new(2, pool);
    let ann = participant.announce(&clock, &mut rng);
    let challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    assert!(observer.peers(&clock).is_empty());
    clock.advance_ms(10_000);
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    let fresh = observer.prove(&proof, &clock).unwrap();
    assert_eq!(fresh.announcement(), &ann);
    assert_eq!(observer.peers(&clock).len(), 1);
    assert!(matches!(
        observer.prove(&proof, &clock),
        Err(VoiceError::Replay)
    ));
    clock.advance_ms(5000);
    // Receipt at t=11s did not buy another 15s interval, nor does wall skew extend it.
    assert!(observer.peers(&clock).is_empty());
    assert!(fresh.authorize(&observer.context, &member, &clock).is_err());
}

#[test]
fn collection_retires_idle_observation_without_refunding_retained_fresh_peer_bytes() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut participant = Participant::new(2, pool.clone());
    let announcement = participant.announce(&clock, &mut rng);
    let challenge = observer
        .challenge(&announcement, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    let held = observer.prove(&proof, &clock).unwrap();
    let charged = pool.counts();
    clock.advance_ms(15_000);
    assert!(observer.watcher.collect_expired(&clock));
    assert!(observer.watcher.present.is_empty());
    assert!(!held.0.live.is_live());
    assert_eq!(pool.counts(), charged);
    drop(held);
    assert!(pool.counts().0 < charged.0);
}

#[test]
fn cached_challenges_and_proofs_do_not_move_deadlines_on_renewal_or_retry() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut participant = Participant::new(2, pool);
    let ann = participant.announce(&clock, &mut rng);
    assert_eq!(ann, participant.announce(&clock, &mut rng));
    let challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    clock.advance_ms(5000);
    let renewal = participant.announce(&clock, &mut rng);
    assert!(
        matches!(renewal.body(), Body::Announcement { key, renewal: 2, .. } if key == participant.join.key())
    );
    assert_eq!(
        observer
            .challenge(&renewal, &mut limits, &clock, &mut rng)
            .unwrap(),
        challenge
    );
    assert_eq!(
        participant.answer(&challenge, &clock, &mut rng).unwrap(),
        proof
    );
    observer.prove(&proof, &clock).unwrap();
    clock.advance_ms(10_000);
    assert!(observer.peers(&clock).is_empty());
}

#[test]
fn proof_requires_exact_watcher_incarnation_identity_nonce_and_challenge() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut participant = Participant::new(2, pool.clone());
    let ann = participant.announce(&clock, &mut rng);
    let challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    for change in 0..6 {
        let mut body = proof.body().clone();
        let Body::JoinProof {
            join,
            key_ref,
            watcher,
            watcher_incarnation,
            challenge,
            challenge_id,
        } = &mut body
        else {
            unreachable!()
        };
        match change {
            0 => join[0] ^= 1,
            1 => key_ref[0] ^= 1,
            2 => *watcher = context(3).1.local,
            3 => watcher_incarnation[0] ^= 1,
            4 => challenge[0] ^= 1,
            _ => challenge_id[0] ^= 1,
        }
        let altered = SignedObject::issue(
            proof.scope(),
            &participant.signer,
            proof.id(),
            proof.issued(),
            proof.expires(),
            body,
        )
        .unwrap();
        assert!(observer.prove(&altered, &clock).is_err());
    }
    let mut replacement = Observer::new(1, pool, &mut rng);
    assert!(replacement.prove(&proof, &clock).is_err());
    observer.prove(&proof, &clock).unwrap(); // Failed tuples did not consume the valid request.
}

#[test]
fn newer_short_presence_cannot_be_replaced_by_delayed_older_proof_after_expiry() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut older = Participant::new(2, pool.clone());
    let ann = older.announce(&clock, &mut rng);
    let old_challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let old_proof = older.answer(&old_challenge, &clock, &mut rng).unwrap();
    // Same original device's newer native join. Retained old signed responses may still arrive.
    let lease = CallLease::default();
    let mut newer = LocalJoin::new(&older.context, key(9), lease.clone(), pool).unwrap();
    let short = SignedObject::issue(
        ann.scope(),
        &older.signer,
        [55; 32],
        1000,
        4000,
        Body::Announcement {
            key: key(9),
            epoch: 7,
            renewal: 1,
        },
    )
    .unwrap();
    let challenge = observer
        .challenge(&short, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = newer
        .answer(
            &challenge,
            &older.signer,
            &ca(&older.context, &lease),
            &clock,
            &mut rng,
        )
        .unwrap();
    let fresh = observer.prove(&proof, &clock).unwrap();
    assert_eq!(observer.watcher.pending.len(), 0);
    clock.advance_ms(3000);
    assert!(observer.peers(&clock).is_empty());
    assert!(fresh.authorize(&observer.context, &member, &clock).is_err());
    assert!(matches!(
        observer.prove(&old_proof, &clock),
        Err(VoiceError::Replay)
    ));
    assert!(observer.peers(&clock).is_empty());
}

#[test]
fn replacement_revokes_cloned_evidence_and_ending_watch_keeps_actual_clones_charged() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut participant = Participant::new(2, pool.clone());
    let ann = participant.announce(&clock, &mut rng);
    let challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    let old = observer.prove(&proof, &clock).unwrap();
    let challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    let new = observer.prove(&proof, &clock).unwrap();
    assert!(old.authorize(&observer.context, &member, &clock).is_err());
    new.authorize(&observer.context, &member, &clock).unwrap();
    observer.lease.revoke();
    assert!(new.authorize(&observer.context, &member, &clock).is_err());
    drop(observer);
    drop(participant);
    drop(limits);
    assert_eq!(pool.counts().0, 2 * RECORD_BYTES);
    drop(old);
    drop(new);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn current_membership_scope_join_and_lock_are_checked_on_each_native_transition() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut participant = Participant::new(2, pool);
    let ann = participant.announce(&clock, &mut rng);
    let challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    let absent = |id| id != participant.context.local;
    assert!(observer
        .watcher
        .prove(
            &proof,
            &WatchAuthority {
                context: &observer.context,
                lease: &observer.lease,
                member: &absent
            },
            &clock
        )
        .is_err());
    observer.context.scope.room += 1;
    assert!(observer.prove(&proof, &clock).is_err());
    observer.context.scope.room -= 1;
    // Removal retired the pending proof. Re-adding the device cannot restore that request;
    // it must answer a newly created challenge even though its old signature is still valid.
    assert!(matches!(
        observer.prove(&proof, &clock),
        Err(VoiceError::Replay)
    ));
    let fresh_challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let fresh_proof = participant
        .answer(&fresh_challenge, &clock, &mut rng)
        .unwrap();
    observer.prove(&fresh_proof, &clock).unwrap();
    participant.lease.revoke();
    assert!(participant.answer(&challenge, &clock, &mut rng).is_err());
    observer.lease.revoke();
    assert!(observer
        .watcher
        .peers(&wa(&observer.context, &observer.lease), &clock)
        .is_err());
}

#[test]
fn pending_capacity_is_shared_and_duplicate_challenges_do_not_refund_it() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut observer2 = Observer::new(3, pool.clone(), &mut rng);
    let mut anns = Vec::new();
    for seed in 10..42 {
        let mut participant = Participant::new(seed, pool.clone());
        anns.push(participant.announce(&clock, &mut rng));
    }
    for ann in &anns {
        observer
            .challenge(ann, &mut limits, &clock, &mut rng)
            .unwrap();
    }
    let cached = observer
        .challenge(&anns[0], &mut limits, &clock, &mut rng)
        .unwrap();
    assert_eq!(
        observer
            .challenge(&anns[0], &mut limits, &clock, &mut rng)
            .unwrap(),
        cached
    );
    assert!(matches!(
        observer2.challenge(&anns[0], &mut limits, &clock, &mut rng),
        Err(VoiceError::Capacity)
    ));
    clock.advance_ms(15_000);
    observer.peers(&clock); // Expired original challenge credits retire; no arrival-time renewal.
    observer2
        .challenge(&anns[0], &mut limits, &clock, &mut rng)
        .unwrap();
}

#[test]
fn watcher_and_participant_state_cannot_be_reconstructed_under_the_same_capability() {
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let (_, context) = context(1);
    let watch = WatchLease::default();
    let watcher = Watcher::new(&context, watch.clone(), pool.clone(), &mut rng).unwrap();
    assert!(Watcher::new(&context, watch.clone(), pool.clone(), &mut rng).is_err());
    drop(watcher);
    assert!(!watch.is_live());
    let lease = CallLease::default();
    let join = LocalJoin::new(&context, key(1), lease.clone(), pool.clone()).unwrap();
    assert!(LocalJoin::new(&context, key(1), lease.clone(), pool).is_err());
    drop(join);
    assert!(!lease.is_live());
}

#[test]
fn membership_loss_permanently_revokes_retained_evidence_even_after_readd() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut participant = Participant::new(2, pool);
    let ann = participant.announce(&clock, &mut rng);
    let challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    let fresh = observer.prove(&proof, &clock).unwrap();
    let absent = |id| id != participant.context.local;
    assert!(fresh.authorize(&observer.context, &absent, &clock).is_err());
    assert!(fresh.authorize(&observer.context, &member, &clock).is_err());
    assert!(observer.peers(&clock).is_empty());

    let pending = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&pending, &clock, &mut rng).unwrap();
    observer.watcher.retire_member(participant.context.local);
    assert!(matches!(
        observer.prove(&proof, &clock),
        Err(VoiceError::Replay)
    ));
    participant.join.retire_member(participant.context.local);
    assert!(participant.answer(&pending, &clock, &mut rng).is_err());
}

#[test]
fn explicit_member_retirement_revokes_clones_without_waiting_for_room_polling() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut participant = Participant::new(2, pool);
    let ann = participant.announce(&clock, &mut rng);
    let challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    let fresh = observer.prove(&proof, &clock).unwrap();
    observer.watcher.retire_member(participant.context.local);
    assert!(fresh.authorize(&observer.context, &member, &clock).is_err());
    assert!(observer.peers(&clock).is_empty());
}

#[test]
fn another_groups_membership_view_cannot_revoke_this_groups_evidence() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut participant = Participant::new(2, pool);
    let ann = participant.announce(&clock, &mut rng);
    let challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    let fresh = observer.prove(&proof, &clock).unwrap();
    observer.context.scope.group[0] ^= 1;
    assert!(fresh
        .authorize(&observer.context, &|_| false, &clock)
        .is_err());
    observer.context.scope.group[0] ^= 1;
    fresh.authorize(&observer.context, &member, &clock).unwrap();
}

#[test]
fn older_renewal_cannot_mint_a_new_challenge_or_replace_a_newer_observation() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut participant = Participant::new(2, pool.clone());
    let old = participant.announce(&clock, &mut rng);
    clock.advance_ms(5000);
    let newer = participant.announce(&clock, &mut rng);
    let challenge = observer
        .challenge(&newer, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    observer.prove(&proof, &clock).unwrap();
    assert!(matches!(
        observer.challenge(&old, &mut limits, &clock, &mut rng),
        Err(VoiceError::Freshness)
    ));
    assert_eq!(observer.peers(&clock)[0].announcement(), &newer);
    // A replacement watcher must consult the same actor high-water mark.
    let mut replacement = Observer::new(1, pool, &mut rng);
    assert!(matches!(
        replacement.challenge(&old, &mut limits, &clock, &mut rng),
        Err(VoiceError::Freshness)
    ));
}

#[test]
fn full_group_and_process_presence_tables_can_refresh_unique_rows_without_refunding_clones() {
    for process_full in [false, true] {
        let clock = ManualClock::new(1000);
        let mut rng = rng();
        let pool = Arc::new(Resources::default());
        let mut observer = Observer::new(1, pool.clone(), &mut rng);
        let mut limits = limits(&observer, pool.clone());
        let mut participant = Participant::new(2, pool.clone());
        let ann = participant.announce(&clock, &mut rng);
        let challenge = observer
            .challenge(&ann, &mut limits, &clock, &mut rng)
            .unwrap();
        let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
        drop(observer.prove(&proof, &clock).unwrap());
        let group = observer.context.scope.group;
        let mut held = (0..63)
            .map(|_| pool.charge(group, 1, ResourceKind::Presence).unwrap())
            .collect::<Vec<_>>();
        if process_full {
            for seed in 50..57 {
                for _ in 0..64 {
                    held.push(pool.charge([seed; 32], 1, ResourceKind::Presence).unwrap());
                }
            }
        }
        let challenge = observer
            .challenge(&ann, &mut limits, &clock, &mut rng)
            .unwrap();
        let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
        let retained = observer.prove(&proof, &clock).unwrap(); // Unique row's credit transfers.
        let challenge = observer
            .challenge(&ann, &mut limits, &clock, &mut rng)
            .unwrap();
        let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
        assert!(matches!(
            observer.prove(&proof, &clock),
            Err(VoiceError::Capacity)
        ));
        retained
            .authorize(&observer.context, &member, &clock)
            .unwrap();
        drop(retained); // Actual clone release makes unique replacement possible again.
        observer.prove(&proof, &clock).unwrap();
        drop(held);
        drop(observer);
        drop(participant);
        drop(limits);
        assert_eq!(pool.counts(), (0, 0));
    }
}

#[test]
fn wall_clock_rollback_does_not_extend_challenge_lifetime() {
    let clock = ManualClock::new(10_000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool.clone());
    let mut participant = Participant::new(2, pool);
    let ann = participant.announce(&clock, &mut rng);
    let challenge = observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    let proof = participant.answer(&challenge, &clock, &mut rng).unwrap();
    clock.advance_ms(15_000);
    clock.set_wall_ms(10_000);
    assert!(matches!(
        observer.prove(&proof, &clock),
        Err(VoiceError::Replay)
    ));
    assert!(observer.peers(&clock).is_empty());
}

#[test]
fn future_issued_delegation_protection_includes_both_skew_windows() {
    let clock = ManualClock::new(1000);
    let mut rng = rng();
    let pool = Arc::new(Resources::default());
    let mut observer = Observer::new(1, pool.clone(), &mut rng);
    let mut limits = limits(&observer, pool);
    let (signer, context) = context(2);
    let ann = SignedObject::issue(
        context.scope,
        &signer,
        [88; 32],
        6000,
        36_000,
        Body::Announcement {
            key: key(2),
            epoch: 7,
            renewal: 1,
        },
    )
    .unwrap();
    observer
        .challenge(&ann, &mut limits, &clock, &mut rng)
        .unwrap();
    // Forty seconds of native binding protection, but still only fifteen for this challenge.
    let pending = observer.watcher.pending.values().next().unwrap();
    assert_eq!(pending.deadline, 16_000);
    assert_eq!(
        limits.delegation_deadline(context.local, key(2).join),
        Some(41_000)
    );
    assert_eq!(
        limits.join(context.local, key(2).join, [3; 32], 40_999, 41_000),
        Err(VoiceError::Unauthorized)
    );
}
