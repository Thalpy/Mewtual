//! Native Ready cache without wire publication. Signatures, Finish, original actor and
//! both native clocks are real. Only the earlier confirmation AEAD fixture is mocked.
use super::*;
use rand_core::{CryptoRng, Error, RngCore};
use std::cell::Cell;
mod admission;
mod low;
mod receive;
mod source;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    PublicKey,
    Signing,
    InvalidSignature,
}
struct Signer {
    key: DeviceKeypair,
    fault: Cell<Fault>,
    calls: Cell<usize>,
    public_calls: Cell<usize>,
    callback: Option<Box<dyn Fn()>>,
}
impl Signer {
    fn original(p: &ProtectedFixture) -> Self {
        Self {
            key: [1, 2]
                .into_iter()
                .map(|seed| DeviceKeypair::from_seed(&[seed; 32]))
                .find(|key| key.device_id() == p.f.high.context.local)
                .unwrap(),
            fault: Cell::new(Fault::None),
            calls: Cell::new(0),
            public_calls: Cell::new(0),
            callback: None,
        }
    }
}
impl VoiceSigner for Signer {
    fn voice_public_key(&self) -> Result<[u8; 32], VoiceError> {
        self.public_calls.set(self.public_calls.get() + 1);
        if self.fault.get() == Fault::PublicKey {
            return Err(VoiceError::Signing);
        }
        self.key.voice_public_key()
    }
    fn sign_voice(&self, bytes: &[u8]) -> Result<[u8; 64], VoiceError> {
        self.calls.set(self.calls.get() + 1);
        if let Some(callback) = &self.callback {
            callback();
        }
        match self.fault.get() {
            Fault::Signing => Err(VoiceError::Signing),
            Fault::InvalidSignature => Ok([0; 64]),
            _ => self.key.sign_voice(bytes),
        }
    }
}
struct Rng {
    inner: ChaCha20Rng,
    fills: usize,
}
impl Rng {
    fn new() -> Self {
        Self {
            inner: ChaCha20Rng::from_seed([199; 32]),
            fills: 0,
        }
    }
}
impl CryptoRng for Rng {}
impl RngCore for Rng {
    fn next_u32(&mut self) -> u32 {
        self.inner.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.fills += 1;
        self.inner.fill_bytes(dest);
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Error> {
        self.fills += 1;
        self.inner.try_fill_bytes(dest)
    }
}
fn cache(p: &ProtectedFixture) -> &crate::voice_v2::ready::producer::CacheCapacity {
    let Some(Preparation::Transferred {
        ready: Some(ready), ..
    }) = &p.f.high.join.confirmations.pairs[&p.f.id].sdp_successor
    else {
        panic!("retained native Ready cache")
    };
    ready
}
fn prepare_artifact(
    p: &mut ProtectedFixture,
    helpers: &[DeviceId],
    signer: &dyn VoiceSigner,
    rng: &mut Rng,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<(), VoiceError> {
    let room = room(&p.f.high);
    p.f.high.join.prepare_high_ready_artifact(
        &p.high,
        &room,
        p.f.id,
        helpers,
        signer,
        &ca(&p.f.high.context, &p.f.high.lease),
        &p.f.clock,
        rng,
        final_owner,
    )
}

#[test]
fn native_ready_stamp_survives_signer_failures_and_final_refusal_without_new_identity_or_signature()
{
    let mut p = confirmed_high();
    let signer = Signer::original(&p);
    let mut rng = Rng::new();
    let mut original = None;
    for (fault, error) in [
        (Fault::PublicKey, VoiceError::Signing),
        (Fault::Signing, VoiceError::Signing),
        (Fault::InvalidSignature, VoiceError::Signature),
    ] {
        signer.fault.set(fault);
        assert_eq!(
            prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())),
            Err(error)
        );
        let stamp = cache(&p).stamp_for_test().unwrap();
        if let Some(original) = original {
            assert_eq!(stamp, original);
        } else {
            original = Some(stamp);
        }
        assert_eq!(rng.fills, 1);
        assert!(cache(&p).artifact_for_test().is_none());
    }
    signer.fault.set(Fault::None);
    assert_eq!(
        prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Err(
            VoiceError::Capacity
        )),
        Err(VoiceError::Capacity)
    );
    let bytes = cache(&p).artifact_for_test().unwrap().0.encode();
    let signs = signer.calls.get();
    let public_calls = signer.public_calls.get();
    for _ in 0..4 {
        prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
        assert_eq!(cache(&p).artifact_for_test().unwrap().0.encode(), bytes);
        assert_eq!(cache(&p).stamp_for_test(), original);
        assert_eq!(rng.fills, 1);
        assert_eq!(signer.calls.get(), signs);
        assert_eq!(signer.public_calls.get(), public_calls);
    }
    assert!(
        p.f.high.join.published_source_packets().next().is_none(),
        "inert Ready never enters published Source packets"
    );
    assert!(!p.f.high.join.ledger.has_sdp_for_test());
    assert!(!pending(&p).is_live());
}

#[test]
fn native_ready_rejects_a_valid_foreign_signer_without_resetting_the_stamp() {
    let mut p = confirmed_high();
    let mut rng = Rng::new();
    let foreign = (3..=255)
        .map(|seed| DeviceKeypair::from_seed(&[seed; 32]))
        .find(|key| {
            key.device_id() > p.f.low.context.local && key.device_id() != p.f.high.context.local
        })
        .unwrap();
    assert_eq!(
        prepare_artifact(&mut p, &[], &foreign, &mut rng, |_, _| Ok(())),
        Err(VoiceError::Signature)
    );
    let original = cache(&p).stamp_for_test();
    assert!(cache(&p).artifact_for_test().is_none());
    let signer = Signer::original(&p);
    prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
    assert_eq!(cache(&p).stamp_for_test(), original);
    assert_eq!(rng.fills, 1);
    assert_eq!(
        cache(&p).artifact_for_test().unwrap().0.issuer(),
        p.f.high.context.local
    );
}

#[test]
fn native_ready_waits_for_actual_old_lower_without_rng_or_signer_work() {
    let mut p = confirmed_high();
    let chain = p.f.high.join.confirmations.pairs[&p.f.id]
        .chain
        .clone()
        .unwrap();
    let workflow = chain.workflow().unwrap();
    let actual = workflow
        .charge(
            &p._runtimes[1].resources(),
            p.f.high.context.scope.group,
            128,
            ResourceKind::Forward,
        )
        .unwrap();
    drop(workflow);
    let signer = Signer::original(&p);
    let mut rng = Rng::new();
    assert_eq!(
        prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())),
        Err(VoiceError::Capacity)
    );
    assert!(cache(&p).stamp_for_test().is_none());
    assert_eq!(
        (rng.fills, signer.public_calls.get(), signer.calls.get()),
        (0, 0, 0)
    );
    drop(actual);
    prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
    assert_eq!((rng.fills, signer.calls.get()), (1, 1));
}

#[test]
fn native_ready_maximum_two_helper_cache_prepares_under_full_ordinary_pressure_and_wrong_retry_pays_no_rows(
) {
    let mut helpers = [
        DeviceKeypair::from_seed(&[88; 32]).device_id(),
        DeviceKeypair::from_seed(&[89; 32]).device_id(),
    ];
    helpers.sort();
    let mut p = confirmed_high_in(fixture_with_members(&helpers));
    let room = room(&p.f.high);
    p.f.high
        .join
        .prepare_high_sdp_successor(
            &p.high,
            &room,
            p.f.id,
            &helpers,
            &ca(&p.f.high.context, &p.f.high.lease),
            &p.f.clock,
            |_, _| Ok(()),
        )
        .unwrap();
    let pool = p._runtimes[1].resources();
    let pressure = pool
        .charge(
            p.f.high.context.scope.group,
            GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let full = pool.counts();
    let signer = Signer::original(&p);
    let mut rng = Rng::new();
    prepare_artifact(&mut p, &helpers, &signer, &mut rng, |_, _| Ok(())).unwrap();
    let (object, hash, first, until) = cache(&p).artifact_for_test().unwrap();
    assert_eq!(object.encode().len(), 692);
    assert_eq!(hash, object.hash());
    assert_eq!(until - first, 15000);
    assert!(
        matches!(object.body(), Body::PairReadyRound { header, .. } if header.helpers == helpers)
    );
    assert_eq!(pool.counts(), full);
    assert_eq!(
        prepare_artifact(&mut p, &helpers[..1], &signer, &mut rng, |_, _| Ok(())),
        Err(VoiceError::Scope)
    );
    assert_eq!(pool.counts(), full);
    assert_eq!(rng.fills, 1);
    drop(pressure);
}

#[test]
fn native_ready_monotonic_deadline_cannot_be_extended_by_signed_skew_wall_rollback_or_retry() {
    let mut p = confirmed_high();
    let signer = Signer::original(&p);
    let mut rng = Rng::new();
    prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
    let original = cache(&p).stamp_for_test();
    p.f.clock.advance_ms(15000);
    p.f.clock.set_wall_ms(1);
    assert!(prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).is_err());
    assert_eq!(cache(&p).stamp_for_test(), original);
    assert_eq!((rng.fills, signer.calls.get()), (1, 1));
    assert!(
        pending(&p).successor_current_at(
            &p.f.high
                .join
                .chain_binding(p.f.id, &ca(&p.f.high.context, &p.f.high.lease)),
            &p._runtimes[1].resources(),
            p.f.clock.monotonic_ms()
        ),
        "original Finish window remains open; Ready's shorter deadline is independent"
    );
}

#[test]
fn native_ready_signer_delay_and_session_close_cannot_report_current_preparation() {
    for close in [false, true] {
        let mut p = confirmed_high();
        let mut signer = Signer::original(&p);
        let clock = p.f.clock.clone();
        let runtime = p._runtimes[1].clone();
        signer.callback = Some(Box::new(move || {
            if close {
                runtime.revoke_session();
            } else {
                clock.advance_ms(15000);
            }
        }));
        let mut rng = Rng::new();
        let entered_final = Cell::new(false);
        assert!(prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| {
            entered_final.set(true);
            Ok(())
        })
        .is_err());
        assert!(!entered_final.get());
        assert!(
            cache(&p).artifact_for_test().is_some(),
            "already signed artifact remains inert history until quiet collection"
        );
        assert_eq!((rng.fills, signer.calls.get()), (1, 1));
        assert!(!pending(&p).is_live());
        assert!(p.f.high.join.published_source_packets().next().is_none());
    }
}

#[test]
fn native_ready_changed_helper_retry_refuses_before_creating_any_new_origin_row() {
    let helpers = [
        DeviceKeypair::from_seed(&[88; 32]).device_id(),
        DeviceKeypair::from_seed(&[89; 32]).device_id(),
    ];
    let mut p = confirmed_high_in(fixture_with_members(&helpers));
    let signer = Signer::original(&p);
    let mut rng = Rng::new();
    prepare_artifact(&mut p, &helpers[..1], &signer, &mut rng, |_, _| Ok(())).unwrap();
    let before = p._runtimes[1].resources().counts();
    assert_eq!(
        prepare_artifact(&mut p, &helpers[1..], &signer, &mut rng, |_, _| Ok(())),
        Err(VoiceError::Scope)
    );
    assert_eq!(
        p._runtimes[1].resources().counts(),
        before,
        "changed valid helper must not acquire its previously absent2KiB origin row"
    );
    assert_eq!((rng.fills, signer.calls.get()), (1, 1));
}

#[test]
fn native_ready_cache_payment_failure_preserves_unissued_latch_and_same_original_capacity() {
    let p = confirmed_high();
    let chain = p.f.high.join.confirmations.pairs[&p.f.id]
        .chain
        .clone()
        .unwrap();
    let workflow = chain.workflow().unwrap();
    let control = workflow.confirmation_control();
    let ticket = control
        .transfer_sdp_capacity(control.prepare_sdp_capacity().unwrap())
        .unwrap();
    let owner = Exchange::from_transferred(ticket, Vec::new());
    // Capacity-only test parks real bytes from this original reserve BEFORE its first
    // constructor. No replacement pool, mocked error or production raw lender is used.
    let held = owner.occupy_unissued_ready_bytes_for_test();
    assert_eq!(
        owner.prepare_ready_cache().err(),
        Some(VoiceError::Capacity)
    );
    drop(held);
    let cache = owner.prepare_ready_cache().unwrap();
    assert!(cache.current_for(&owner));
    drop(cache);
    assert_eq!(
        owner.prepare_ready_cache().err(),
        Some(VoiceError::Capacity)
    );
}

#[test]
fn native_ready_quiet_expiry_removes_native_cache_but_waits_for_actual_credit_return() {
    let mut p = confirmed_high();
    let chain = p.f.high.join.confirmations.pairs[&p.f.id]
        .chain
        .clone()
        .unwrap();
    let control = chain.workflow().unwrap().confirmation_control();
    let signer = Signer::original(&p);
    let mut rng = Rng::new();
    prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
    assert!(cache(&p).artifact_for_test().is_some());
    let output = prepare(&mut p, |_, _| Ok(())).unwrap();
    let producer = ready_producer(&p);
    let pool = p._runtimes[1].resources();
    let group = p.f.high.context.scope.group;
    let actual = producer.charge(&pool, group, Stage::Lower, 128).unwrap();
    drop((producer, output));
    p.f.clock.advance_ms(30000);
    p.f.high.join.collect_expired(&p.f.clock);
    assert!(p.f.high.join.confirmations.pairs[&p.f.id]
        .sdp_successor
        .is_none());
    let whole = control.retire_all();
    assert!(!whole.is_drained());
    assert!(
        pool.exchange(group).is_some(),
        "real last credit still retains registry capacity"
    );
    drop(actual);
    assert!(whole.is_drained());
    assert!(pool.exchange(group).is_none());
}

#[test]
fn native_ready_first_preparation_near_original_finish_expiry_clamps_both_deadlines() {
    let mut p = confirmed_high();
    let signer = Signer::original(&p);
    let mut rng = Rng::new();
    p.f.clock.advance_ms(29000);
    prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
    let stamp = cache(&p).stamp_for_test().unwrap();
    assert_eq!(
        (stamp.1, stamp.2, stamp.3, stamp.4),
        (30000, 31000, 30000, 31000)
    );
    p.f.clock.advance_ms(999);
    prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
    assert_eq!(cache(&p).stamp_for_test(), Some(stamp));
    p.f.clock.advance_ms(1);
    assert!(prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).is_err());
    assert_eq!((rng.fills, signer.calls.get()), (1, 1));
}

#[test]
fn native_ready_actual_final_group_and_policy_contention_keeps_identical_signed_artifact() {
    for policy in [false, true] {
        let mut p = confirmed_high();
        let mut signer = Signer::original(&p);
        let (held_tx, held_rx) = std::sync::mpsc::sync_channel(0);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
        let (start_tx, start_rx) = std::sync::mpsc::sync_channel(0);
        let hold = p
            .high
            .native_view_holder_for_test(policy, held_tx, release_rx);
        let thread = std::thread::spawn(move || {
            start_rx.recv().unwrap();
            hold();
        });
        signer.callback = Some(Box::new(move || {
            start_tx.send(()).unwrap();
            held_rx.recv().unwrap();
        }));
        let mut rng = Rng::new();
        assert_eq!(
            prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())),
            Err(VoiceError::Capacity)
        );
        let (object, hash, first, deadline) = cache(&p).artifact_for_test().unwrap();
        let bytes = object.encode();
        release_tx.send(()).unwrap();
        thread.join().unwrap();
        prepare_artifact(&mut p, &[], &signer, &mut rng, |_, _| Ok(())).unwrap();
        let (object, recovered, recovered_first, recovered_deadline) =
            cache(&p).artifact_for_test().unwrap();
        assert_eq!(object.encode(), bytes);
        assert_eq!(
            (recovered, recovered_first, recovered_deadline),
            (hash, first, deadline)
        );
        assert_eq!((rng.fills, signer.calls.get()), (1, 1));
    }
}
