//! Native state-machine limits, with real signed watch proofs and actual Actor origin pins.
//! Actual registered CallGrant/Directory provenance is covered separately by the Root tests.
use super::*;
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::{ActorOwner, Runtime, View};

struct PairFixture {
    _runtime: Arc<Runtime>,
    actor: ActorOwner,
    clock: ManualClock,
    rng: ChaCha20Rng,
    local: Participant,
    observer: Observer,
    room: RoomAuthority,
    pool: Arc<Resources>,
}
impl PairFixture {
    fn new() -> Self {
        let clock = ManualClock::new(1000);
        let runtime = Runtime::new(Arc::new(clock.clone()));
        let pool = runtime.resources();
        let mut rng = rng();
        let local = Participant::new(1, pool.clone());
        let observer = Observer::new(1, pool.clone(), &mut rng);
        let ticket = runtime.session_ticket();
        runtime.activate_session(&ticket).unwrap();
        let (mut actor, registration) = runtime
            .prepare_actor(local.context.scope.group, local.context.local, &ticket)
            .unwrap();
        registration.register().unwrap();
        actor
            .sync_view(View {
                epoch: local.context.epoch,
                members: (1..=20).map(|seed| context(seed).1.local).collect(),
                policy: local.context.scope.policy,
                eligible: true,
                indirect: true,
            })
            .unwrap();
        let room = RoomAuthority {
            scope: local.context.scope,
            lease: CallLease::default(),
            custody: None,
        };
        let mut f = Self {
            _runtime: runtime,
            actor,
            clock,
            rng,
            local,
            observer,
            room,
            pool,
        };
        f.local.announce(&f.clock, &mut f.rng);
        f
    }
    fn discover(&mut self, seed: u8) -> (PairSelector, FreshPeer) {
        let mut peer = Participant::new(seed, self.pool.clone());
        let announcement = peer.announce(&self.clock, &mut self.rng);
        let challenge = self
            .actor
            .with_limits(|limits| {
                self.observer
                    .challenge(&announcement, limits, &self.clock, &mut self.rng)
            })
            .unwrap();
        let proof = peer.answer(&challenge, &self.clock, &mut self.rng).unwrap();
        let fresh = self.observer.prove(&proof, &self.clock).unwrap();
        (
            PairSelector {
                peer: peer.context.local,
                join: key(seed).join,
                key_ref: key(seed).key_ref,
                epoch: self.local.context.epoch,
            },
            fresh,
        )
    }
    fn admit(
        &mut self,
        selector: PairSelector,
        fresh: &FreshPeer,
    ) -> Result<PairReceipt, VoiceError> {
        self.local
            .join
            .admit_protected_pair(
                ProtectedPair {
                    actor: &self.actor,
                    room: &self.room,
                    watch: &self.observer.lease,
                    selector,
                },
                Some(fresh),
                &ca(&self.local.context, &self.local.lease),
                &self.clock,
                |_, _| Ok(()),
            )
            .map(|(receipt, _)| receipt)
    }
}

#[test]
fn protected_pair_eight_confirming_limit_precedes_pin_table_and_serial_effects() {
    let mut f = PairFixture::new();
    for seed in 2..=9 {
        let (selector, fresh) = f.discover(seed);
        f.admit(selector, &fresh).unwrap();
    }
    let (selector, fresh) = f.discover(10);
    let before = f.pool.counts();
    let pins = f
        .actor
        .pair_pin_observer_for_test(selector.peer, selector.join, selector.key_ref);
    for _ in 0..3 {
        assert_eq!(f.admit(selector, &fresh).err(), Some(VoiceError::Capacity));
        assert_eq!(f.local.join.confirmation_counts_for_test(), (8, 8, 8));
        assert_eq!(pins(), 0);
        assert_eq!(f.pool.counts(), before);
    }
}

#[test]
fn protected_pair_sixteen_total_limit_retains_retired_records_without_refill() {
    let mut f = PairFixture::new();
    let mut original = None;
    for seed in 2..=17 {
        let (selector, fresh) = f.discover(seed);
        let receipt = f.admit(selector, &fresh).unwrap();
        if original.is_none() {
            original = Some((selector, fresh, receipt));
        }
        f.local.join.retire_member(selector.peer);
    }
    assert_eq!(f.local.join.confirmation_counts_for_test(), (16, 16, 16));
    let (selector, fresh) = f.discover(18);
    let before = f.pool.counts();
    assert_eq!(f.admit(selector, &fresh).err(), Some(VoiceError::Capacity));
    assert_eq!(f.pool.counts(), before);
    let (selector, fresh, _) = original.unwrap();
    assert_eq!(
        f.admit(selector, &fresh).err(),
        Some(VoiceError::Unauthorized),
        "retired original cannot recreate a seventeenth allocator"
    );
    assert_eq!(f.local.join.confirmation_counts_for_test(), (16, 16, 16));
}

#[test]
fn protected_pair_full_actual_origin_pin_table_refuses_then_reuses_only_released_capacity() {
    let mut f = PairFixture::new();
    let (selector, fresh) = f.discover(2);
    let binding = crate::voice_v2::packets::PairBinding {
        peer: selector.peer,
        join: selector.join,
        key_ref: selector.key_ref,
        handshake: [99; 32],
    };
    let pins = f
        .actor
        .pair_pin_observer_for_test(selector.peer, selector.join, selector.key_ref);
    let mut reservations = Vec::new();
    for _ in 0..16 {
        reservations.push(
            f.actor
                .prepare_pair_pin(&binding, &f.local.context, &CallLease::default())
                .unwrap(),
        );
    }
    assert_eq!(pins(), 16);
    let before = f.pool.counts();
    assert_eq!(f.admit(selector, &fresh).err(), Some(VoiceError::Capacity));
    assert_eq!(pins(), 16);
    assert_eq!(f.pool.counts(), before);
    assert_eq!(f.local.join.confirmation_counts_for_test(), (0, 0, 0));
    drop(reservations);
    assert_eq!(pins(), 0);
    f.admit(selector, &fresh).unwrap();
    assert_eq!(pins(), 1);
    assert_eq!(f.local.join.confirmation_counts_for_test(), (1, 1, 1));
}
