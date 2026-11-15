//! Actual warm genesis node and native counter tests. These do NOT establish
//! Root currentness, a durable invite, route authorization or a working join.
use super::super::tests::ordinary_data::prepared_node_with_clock;
use super::*;
use catcoms_rt::{
    ManualClock, NativeChatSourceOwner, NativeChatUiBinding, NativeChatUiSessionOwner,
};
use rand_core::RngCore;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering::SeqCst},
    Arc,
};

const NOW: u64 = 1_800_000_000_000;
struct Native {
    binding: NativeChatUiBinding,
    ui: NativeChatUiSessionOwner,
    source: NativeChatSourceOwner,
    generation: Arc<AtomicU64>,
    authorized: Arc<AtomicU64>,
}
impl Native {
    fn new() -> Self {
        let generation = Arc::new(AtomicU64::new(7));
        let authorized = Arc::new(AtomicU64::new(7));
        let binding = NativeChatUiBinding::from_trusted_host(
            generation.clone(),
            authorized.clone(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(()),
        );
        let ui = binding.open_session(7, Arc::new(())).unwrap();
        let source =
            NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap();
        Self {
            binding,
            ui,
            source,
            generation,
            authorized,
        }
    }
    fn cut(&self, purpose: NativeChatKeyPurpose) -> NativeChatPreparedKeyCut {
        self.source
            .prepare_cut(&self.ui.observe(), purpose)
            .unwrap()
    }
    fn reopen(&mut self) {
        let generation = self.generation.load(SeqCst);
        self.authorized.store(generation, SeqCst);
        self.ui = self.binding.open_session(generation, Arc::new(())).unwrap();
    }
}
fn request() -> V2FounderInviteRequestData {
    V2FounderInviteRequestData {
        token: [31; 32],
        lifetime_ms: 60_000,
        // Explicit DATA fixture, never a claim of a usable or admitted route.
        public_routing_plan: JoinPayloadKindData::RoutingPlan
            .commitment_data(b"test plan")
            .unwrap(),
    }
}
fn hook(point: TestCut, run: impl FnOnce() + 'static) {
    TEST_CUT.with(|s| *s.borrow_mut() = Some((point, Box::new(run))));
}
fn panic_at(point: TestCut) {
    hook(point, || panic!("injected lost invite return"));
}

#[test]
fn native_invite_uses_original_anchor_policy_clock_and_exactly_one_nonce_draw() {
    let clock = ManualClock::new(NOW);
    let (mut node, _reservation, _opened, _fee) =
        prepared_node_with_clock(0xed01, Box::new(clock.clone()));
    let native = Native::new();
    let mut slot = None;
    let before =
        catcoms_mls::persist::snapshot_server(&node.seed.body.device, &node.seed.body.group)
            .unwrap();
    let mut expected = node._parts.body._rng.clone();
    let mut nonce = [0; 16];
    expected.fill_bytes(&mut nonce);
    node.prepare_invite_data(request(), &native.source.observe(), &mut slot)
        .unwrap();
    let p = slot.as_mut().unwrap();
    let (spec, bytes) = p.unsigned_data().unwrap();
    let bytes = bytes.to_vec();
    assert_eq!(spec.nonce, nonce);
    assert_ne!(spec.nonce, request().token[..16]);
    assert_eq!(spec.expires_at_ms, NOW + 60_000);
    assert_eq!(spec.anchor.group, node.data().profile.spec().group);
    assert_eq!(spec.anchor.founder, node.data().profile.spec().founder);
    assert_eq!(spec.anchor.profile, *node.data().profile.id().as_bytes());
    assert_eq!(
        spec.anchor.admission,
        *node.data().admission.admission_id().as_bytes()
    );
    assert_eq!(spec.anchor.tenure, *node.data().profile.tenure().as_bytes());
    assert_eq!(spec.anchor.header, *node.data().header.id().as_bytes());
    assert_eq!(
        spec.anchor.state,
        *node.data().header.spec().state.as_bytes()
    );
    assert_eq!(
        spec.policy,
        JoinPayloadKindData::Policy
            .commitment_data(&node.seed.body.metadata.policy.as_ref().unwrap().encode())
            .unwrap()
    );
    clock.advance_ms(500_000);
    node.prepare_invite_data(request(), &native.source.observe(), &mut slot)
        .unwrap();
    let p = slot.as_mut().unwrap();
    assert_eq!(p.unsigned_data().unwrap().1, bytes);
    assert_eq!(
        node._parts.body._rng.clone().next_u64(),
        expected.next_u64()
    );
    let mut cut = native.cut(NativeChatKeyPurpose::JoinInvite);
    let mut ends = NativeChatKeyEnds::new();
    node.sign_invite_native_data(p, &mut cut, &mut ends)
        .unwrap();
    p.materialize_data().unwrap();
    let signed = p.signed_data().unwrap();
    assert!(signed.verify());
    assert_eq!(signed.spec_data().expires_at_ms, NOW + 60_000);
    assert_eq!(p.attempts, 1);
    assert_eq!(
        catcoms_mls::persist::snapshot_server(&node.seed.body.device, &node.seed.body.group)
            .unwrap(),
        before
    );
}

#[test]
fn native_invite_invalid_requests_do_not_create_a_slot_or_draw() {
    let (mut node, _reservation, _opened, _fee) =
        prepared_node_with_clock(0xed02, Box::new(ManualClock::new(NOW)));
    let native = Native::new();
    let mut slot = None;
    let mut before = node._parts.body._rng.clone();
    for bad in [
        V2FounderInviteRequestData {
            token: [0; 32],
            ..request()
        },
        V2FounderInviteRequestData {
            public_routing_plan: [0; 32],
            ..request()
        },
        V2FounderInviteRequestData {
            lifetime_ms: 0,
            ..request()
        },
        V2FounderInviteRequestData {
            lifetime_ms: MAX_FOUNDER_INVITE_LIFETIME_MS + 1,
            ..request()
        },
    ] {
        assert!(node
            .prepare_invite_data(bad, &native.source.observe(), &mut slot)
            .is_err());
        assert!(slot.is_none());
    }
    assert_eq!(node._parts.body._rng.clone().next_u64(), before.next_u64());
}

#[test]
fn native_invite_changed_token_lifetime_plan_seed_or_source_cannot_replace_pending() {
    let (mut node, _reservation, _opened, _fee) =
        prepared_node_with_clock(0xed03, Box::new(ManualClock::new(NOW)));
    let (mut other, _r2, _o2, _f2) =
        prepared_node_with_clock(0xed04, Box::new(ManualClock::new(NOW)));
    let native = Native::new();
    let foreign = Native::new();
    let mut slot = None;
    node.prepare_invite_data(request(), &native.source.observe(), &mut slot)
        .unwrap();
    let bytes = slot.as_ref().unwrap().unsigned_data().unwrap().1.to_vec();
    for bad in [
        V2FounderInviteRequestData {
            token: [32; 32],
            ..request()
        },
        V2FounderInviteRequestData {
            lifetime_ms: 60_001,
            ..request()
        },
        V2FounderInviteRequestData {
            public_routing_plan: [33; 32],
            ..request()
        },
    ] {
        assert!(node
            .prepare_invite_data(bad, &native.source.observe(), &mut slot)
            .is_err());
    }
    assert!(node
        .prepare_invite_data(request(), &foreign.source.observe(), &mut slot)
        .is_err());
    assert!(other
        .prepare_invite_data(request(), &native.source.observe(), &mut slot)
        .is_err());
    assert_eq!(slot.as_ref().unwrap().unsigned_data().unwrap().1, bytes);
}

#[test]
fn native_invite_clock_or_nonce_entry_unwind_cannot_redraw() {
    for (i, point) in [TestCut::ClockEntered, TestCut::NonceEntered]
        .into_iter()
        .enumerate()
    {
        let clock = ManualClock::new(NOW);
        let (mut node, _reservation, _opened, _fee) =
            prepared_node_with_clock(0xed10 + i as u64, Box::new(clock.clone()));
        let native = Native::new();
        let mut slot = None;
        panic_at(point);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            node.prepare_invite_data(request(), &native.source.observe(), &mut slot)
                .unwrap();
        }))
        .is_err());
        let state = slot.as_ref().unwrap().draw;
        assert!(matches!(state, Draw::ClockEntered | Draw::NonceEntered));
        clock.advance_ms(1);
        assert!(node
            .prepare_invite_data(request(), &native.source.observe(), &mut slot)
            .is_err());
        assert!(slot.as_ref().unwrap().draw == state);
        assert!(slot.as_ref().unwrap().unsigned_data().is_err());
    }
}

#[test]
fn native_invite_lost_prepare_return_preserves_complete_original_after_revocation() {
    let (mut node, reservation, _opened, _fee) =
        prepared_node_with_clock(0xed20, Box::new(ManualClock::new(NOW)));
    let native = Native::new();
    let mut slot = None;
    panic_at(TestCut::Prepared);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.prepare_invite_data(request(), &native.source.observe(), &mut slot)
            .unwrap();
    }))
    .is_err());
    let bytes = slot.as_ref().unwrap().unsigned_data().unwrap().1.to_vec();
    reservation.revocation().revoke();
    node.prepare_invite_data(request(), &native.source.observe(), &mut slot)
        .unwrap();
    assert_eq!(slot.as_ref().unwrap().unsigned_data().unwrap().1, bytes);
    let mut cut = native.cut(NativeChatKeyPurpose::JoinInvite);
    assert!(node
        .sign_invite_native_data(
            slot.as_mut().unwrap(),
            &mut cut,
            &mut NativeChatKeyEnds::new()
        )
        .is_err());
    assert!(!cut.is_cancelled());
    assert!(slot.as_ref().unwrap().key == Key::Unstarted);
}

#[test]
fn native_invite_wrong_source_purpose_or_spent_cut_never_enters_key() {
    let (mut node, _reservation, _opened, _fee) =
        prepared_node_with_clock(0xed21, Box::new(ManualClock::new(NOW)));
    let native = Native::new();
    let foreign = Native::new();
    let mut slot = None;
    node.prepare_invite_data(request(), &native.source.observe(), &mut slot)
        .unwrap();
    let p = slot.as_mut().unwrap();
    let mut ends = NativeChatKeyEnds::new();
    for mut wrong in [
        foreign.cut(NativeChatKeyPurpose::JoinInvite),
        native.cut(NativeChatKeyPurpose::JoinWelcome),
    ] {
        assert!(node
            .sign_invite_native_data(p, &mut wrong, &mut ends)
            .is_err());
        assert_eq!(p.attempts, 0);
        assert!(p.key == Key::Unstarted && p.raw.is_none());
    }
    let mut spent = native.cut(NativeChatKeyPurpose::JoinInvite);
    spent
        .enter(
            &native.source.observe(),
            NativeChatKeyPurpose::JoinInvite,
            &mut ends,
        )
        .unwrap();
    assert!(node
        .sign_invite_native_data(p, &mut spent, &mut ends)
        .is_err());
    assert_eq!(p.attempts, 0);
    drop(ends);
    node.sign_invite_native_data(
        p,
        &mut native.cut(NativeChatKeyPurpose::JoinInvite),
        &mut NativeChatKeyEnds::new(),
    )
    .unwrap();
    assert_eq!(p.attempts, 1);
}

#[test]
fn native_invite_ui_refusal_can_use_fresh_same_source_without_redrawing() {
    let (mut node, _reservation, _opened, _fee) =
        prepared_node_with_clock(0xed22, Box::new(ManualClock::new(NOW)));
    let mut native = Native::new();
    let mut slot = None;
    node.prepare_invite_data(request(), &native.source.observe(), &mut slot)
        .unwrap();
    let p = slot.as_mut().unwrap();
    let bytes = p.unsigned_data().unwrap().1.to_vec();
    let mut old = native.cut(NativeChatKeyPurpose::JoinInvite);
    native.generation.store(8, SeqCst);
    assert!(node
        .sign_invite_native_data(p, &mut old, &mut NativeChatKeyEnds::new())
        .is_err());
    assert!(p.key == Key::KnownLocalRefusal);
    assert_eq!(p.attempts, 1);
    native.reopen();
    node.sign_invite_native_data(
        p,
        &mut native.cut(NativeChatKeyPurpose::JoinInvite),
        &mut NativeChatKeyEnds::new(),
    )
    .unwrap();
    assert_eq!(p.attempts, 2);
    assert_eq!(p.unsigned_data().unwrap().1, bytes);
}

#[test]
fn native_invite_unknown_native_or_key_entry_is_sticky_across_unlock() {
    for (i, point) in [TestCut::NativeAttempting, TestCut::KeyEntered]
        .into_iter()
        .enumerate()
    {
        let (mut node, _reservation, _opened, _fee) =
            prepared_node_with_clock(0xed30 + i as u64, Box::new(ManualClock::new(NOW)));
        let mut native = Native::new();
        let mut slot = None;
        node.prepare_invite_data(request(), &native.source.observe(), &mut slot)
            .unwrap();
        let p = slot.as_mut().unwrap();
        let mut cut = native.cut(NativeChatKeyPurpose::JoinInvite);
        let mut ends = NativeChatKeyEnds::new();
        panic_at(point);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            node.sign_invite_native_data(p, &mut cut, &mut ends)
                .unwrap();
        }))
        .is_err());
        drop((cut, ends)); // actual outside-writer finite custody
        native.generation.store(8, SeqCst);
        native.reopen();
        assert!(node
            .sign_invite_native_data(
                p,
                &mut native.cut(NativeChatKeyPurpose::JoinInvite),
                &mut NativeChatKeyEnds::new()
            )
            .is_err());
        assert_eq!(p.attempts, 1);
        assert!(p.raw_signature_data().is_err());
    }
}

#[test]
fn native_invite_lost_raw_return_materializes_after_lock_without_key_or_redraw() {
    let (mut node, reservation, _opened, _fee) =
        prepared_node_with_clock(0xed40, Box::new(ManualClock::new(NOW)));
    let native = Native::new();
    let foreign = Native::new();
    let mut slot = None;
    node.prepare_invite_data(request(), &native.source.observe(), &mut slot)
        .unwrap();
    let p = slot.as_mut().unwrap();
    panic_at(TestCut::RawCached);
    let mut ends = NativeChatKeyEnds::new();
    let mut cut = native.cut(NativeChatKeyPurpose::JoinInvite);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        node.sign_invite_native_data(p, &mut cut, &mut ends)
            .unwrap();
    }))
    .is_err());
    let raw = *p.raw_signature_data().unwrap();
    native.generation.store(8, SeqCst);
    reservation.revocation().revoke();
    // No validation/entry of this wrong cut: this returns cached DATA only.
    node.sign_invite_native_data(
        p,
        &mut foreign.cut(NativeChatKeyPurpose::Device),
        &mut NativeChatKeyEnds::new(),
    )
    .unwrap();
    p.materialize_data().unwrap();
    let encoded = p.signed_data().unwrap().encode();
    p.materialize_data().unwrap();
    assert_eq!(p.signed_data().unwrap().encode(), encoded);
    assert_eq!(p.raw_signature_data().unwrap(), &raw);
    assert_eq!(p.attempts, 1);
    assert!(p.signed_data().unwrap().verify());
}

#[test]
fn native_invite_expiry_overflow_keeps_spent_clock_without_rng_work() {
    let (mut node, _reservation, _opened, _fee) =
        prepared_node_with_clock(0xed41, Box::new(ManualClock::new(u64::MAX - 1)));
    let native = Native::new();
    let mut slot = None;
    let mut expected = node._parts.body._rng.clone();
    assert!(node
        .prepare_invite_data(request(), &native.source.observe(), &mut slot)
        .is_err());
    assert!(slot.as_ref().unwrap().draw == Draw::ClockEntered);
    assert!(node
        .prepare_invite_data(request(), &native.source.observe(), &mut slot)
        .is_err());
    assert_eq!(
        node._parts.body._rng.clone().next_u64(),
        expected.next_u64()
    );
}

#[test]
fn native_invite_partial_source_busy_retires_ui_only_end_before_exact_fresh_retry() {
    let clock = ManualClock::new(NOW);
    let (mut node, _reservation, _opened, _fee) =
        prepared_node_with_clock(0xed42, Box::new(clock.clone()));
    let mut native = Native::new();
    let mut slot = None;
    node.prepare_invite_data(request(), &native.source.observe(), &mut slot)
        .unwrap();
    let p = slot.as_mut().unwrap();
    node.preflight_invite_signing_data(p).unwrap();
    let bytes = p.unsigned_data().unwrap().1.to_vec();
    let rng = node._parts.body._rng.get_word_pos();
    let mut pressure = Vec::new();
    for _ in 0..32 {
        let mut ends = NativeChatKeyEnds::new();
        native
            .cut(NativeChatKeyPurpose::MemberAction)
            .enter(
                &native.source.observe(),
                NativeChatKeyPurpose::MemberAction,
                &mut ends,
            )
            .unwrap();
        pressure.push(ends);
    }
    native.generation.store(8, SeqCst);
    native.reopen();
    let mut cut = native.cut(NativeChatKeyPurpose::JoinInvite);
    let mut partial = NativeChatKeyEnds::new();
    assert!(node
        .sign_invite_native_data(p, &mut cut, &mut partial)
        .is_err());
    assert!(p.key == Key::KnownLocalRefusal && p.raw.is_none());
    assert_eq!(p.attempts, 1);
    assert!(cut
        .require_never_entered_data(&native.source.observe(), NativeChatKeyPurpose::JoinInvite)
        .is_err());
    assert!(!partial
        .holds_admitted_purpose_data(&native.source.observe(), NativeChatKeyPurpose::JoinInvite));
    drop((cut, partial)); // old UI-only custody leaves outside every writer
    drop(pressure);
    clock.advance_ms(1000);
    node.preflight_invite_signing_data(p).unwrap();
    node.sign_invite_native_data(
        p,
        &mut native.cut(NativeChatKeyPurpose::JoinInvite),
        &mut NativeChatKeyEnds::new(),
    )
    .unwrap();
    assert_eq!(p.attempts, 2);
    assert_eq!(p.unsigned_data().unwrap().1, bytes);
    assert_eq!(node._parts.body._rng.get_word_pos(), rng);
    p.materialize_data().unwrap();
    assert!(p.signed_data().unwrap().verify());
}
