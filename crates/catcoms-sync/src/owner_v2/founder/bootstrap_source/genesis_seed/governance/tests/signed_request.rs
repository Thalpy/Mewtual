//! Actual original Clock/SDK and signed representations, not production Root
//! invitation authorization, durable response coverage or joined-source proof.
use super::*;
use catcoms_wire::Decoder;

#[test]
fn expiry_is_checked_at_first_real_admission_not_at_preparation() {
    let clock = ManualClock::new(NOW);
    let (original, _reservation, _opened, _old_fee) =
        prepared_node_with_clock(0xea01, Box::new(clock.clone()));
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 61);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    clock.advance_ms(60_000); // exact signed expiry, after plan was frozen
    assert!(node.execute_add_data_native(request.data()).is_err());
    let p = node.pending.as_ref().unwrap();
    assert!(p.phase == Phase::Prepared && p.cell.is_none());
    assert_eq!(p.admitted_at_ms, None);
    assert_eq!(node.node.seed.body.group.epoch(), 0);
    assert_eq!(count.load(SeqCst), 1);
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn first_admission_samples_original_clock_and_exact_retry_preserves_it_after_expiry() {
    let clock = ManualClock::new(NOW);
    let (original, _reservation, _opened, _old_fee) =
        prepared_node_with_clock(0xea02, Box::new(clock.clone()));
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 62);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    clock.advance_ms(321);
    node.execute_add_data_native(request.data()).unwrap();
    assert_eq!(
        node.pending.as_ref().unwrap().admitted_at_ms,
        Some(NOW + 321)
    );
    let result = node.completed_add_data().unwrap();
    let sealed = result.sealed_routing_transfer_data().unwrap().to_vec();
    let snapshot = Zeroizing::new(result.secret_snapshot_data().to_vec());
    let rng_position = node.node._parts.body._rng.get_word_pos();
    clock.advance_ms(120_000);
    node.execute_add_data_native(request.data()).unwrap();
    assert_eq!(
        node.pending.as_ref().unwrap().admitted_at_ms,
        Some(NOW + 321)
    );
    assert_eq!(node.node._parts.body._rng.get_word_pos(), rng_position);
    let result = node.completed_add_data().unwrap();
    assert!(result.secret_snapshot_data() == snapshot.as_slice());
    assert_eq!(result.sealed_routing_transfer_data().unwrap(), sealed);
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn expired_actual_package_refuses_with_a_still_valid_signed_invite() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xea03);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::with_package_times(&node, 63, NOW / 1000 - 3600, NOW / 1000 - 60);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    assert!(!request.invite.expired_at_data(NOW));
    assert!(node.execute_add_data_native(request.data()).is_err());
    let p = node.pending.as_ref().unwrap();
    assert!(p.phase == Phase::SdkEntered && p.cell.is_none());
    assert_eq!(p.admitted_at_ms, Some(NOW));
    assert_eq!(node.node.seed.body.group.epoch(), 0);
    assert!(node.execute_add_data_native(request.data()).is_err());
    drop(node);
    assert_eq!(count.load(SeqCst), 0);
}

#[test]
fn invalid_signature_sharing_an_id_cannot_prepare_or_retry_the_original_plan() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xea04);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 64);
    let count = Arc::new(AtomicUsize::new(0));
    let mut bytes = request.request.encode();
    *bytes.last_mut().unwrap() ^= 1;
    let invalid = SignedJoinRequestBindingData::decode(&bytes).unwrap();
    assert_eq!(invalid.id_data(), request.request.id_data());
    assert!(!invalid.verify());
    let bad = V2FounderAddRequestData {
        request: &invalid,
        ..request.data()
    };
    let refused = node
        .prepare_add_data(
            bad,
            native.source.observe(),
            admission(&count),
            native.cut(NativeChatKeyPurpose::AddSdk),
            native.cut(NativeChatKeyPurpose::GoverningHeader),
            native.cut(NativeChatKeyPurpose::JoinWelcome),
        )
        .unwrap_err();
    assert!(node.pending.is_none());
    let (original, fee, sdk, header, welcome) = *refused;
    node.prepare_add_data(request.data(), original, fee, sdk, header, welcome)
        .unwrap();
    assert!(node.execute_add_data_native(bad).is_err());
    assert!(node.pending.as_ref().unwrap().phase == Phase::Prepared);
    node.execute_add_data_native(request.data()).unwrap();
    let original = node.completed_add_data().unwrap().commit_data().to_vec();
    assert!(node.execute_add_data_native(bad).is_err());
    assert_eq!(node.completed_add_data().unwrap().commit_data(), original);
    node.execute_add_data_native(request.data()).unwrap();
}

#[test]
fn complete_original_anchor_and_pinned_policy_are_checked_before_native_entry() {
    type Change = fn(&mut InviteBindingSpecData);
    let changes: [Change; 8] = [
        |s| s.anchor.group[0] ^= 1,
        |s| s.anchor.founder[0] ^= 1,
        |s| s.anchor.profile[0] ^= 1,
        |s| s.anchor.admission[0] ^= 1,
        |s| s.anchor.tenure[0] ^= 1,
        |s| s.anchor.header[0] ^= 1,
        |s| s.anchor.state[0] ^= 1,
        |s| s.policy[0] ^= 1,
    ];
    for change in changes {
        let (original, _reservation, _opened, _old_fee) = prepared_node(0xea05);
        let mut node = original.prepare_governance_data().unwrap();
        let native = Native::new();
        let mut request = Request::new(&node, 65);
        request.change_invite(&node, change);
        assert!(request.invite.verify() && request.request.verify());
        let count = Arc::new(AtomicUsize::new(0));
        let refused = node
            .prepare_add_data(
                request.data(),
                native.source.observe(),
                admission(&count),
                native.cut(NativeChatKeyPurpose::AddSdk),
                native.cut(NativeChatKeyPurpose::GoverningHeader),
                native.cut(NativeChatKeyPurpose::JoinWelcome),
            )
            .unwrap_err();
        assert!(node.pending.is_none());
        assert_eq!(node.node.seed.body.group.epoch(), 0);
        drop(refused);
        assert_eq!(count.load(SeqCst), 0);
        let request = Request::new(&node, 65);
        begin(&mut node, &native, &count, request.data());
        node.execute_add_data_native(request.data()).unwrap();
    }
}

#[test]
fn unsupported_original_routing_or_absent_policy_refuses_before_cut_and_valid_retry_works() {
    for case in 0..4 {
        let (original, _reservation, _opened, _old_fee) = prepared_node(0xea06);
        let mut node = original.prepare_governance_data().unwrap();
        let native = Native::new();
        let request = Request::new(&node, 66);
        let count = Arc::new(AtomicUsize::new(0));
        let m = &mut node.node.seed.body.metadata;
        let old_policy = if case == 3 { m.policy.take() } else { None };
        match case {
            0 => m.routing_label = 1,
            1 => m.routing[0].0 = 1,
            2 => m.routing.push((1, Zeroizing::new([99; 32]))),
            _ => {}
        }
        let refused = node
            .prepare_add_data(
                request.data(),
                native.source.observe(),
                admission(&count),
                native.cut(NativeChatKeyPurpose::AddSdk),
                native.cut(NativeChatKeyPurpose::GoverningHeader),
                native.cut(NativeChatKeyPurpose::JoinWelcome),
            )
            .unwrap_err();
        assert!(node.pending.is_none());
        let m = &mut node.node.seed.body.metadata;
        m.routing_label = 0;
        m.routing[0].0 = 0;
        m.routing.truncate(1);
        if case == 3 {
            m.policy = old_policy;
        }
        let (original, fee, sdk, header, welcome) = *refused;
        node.prepare_add_data(request.data(), original, fee, sdk, header, welcome)
            .unwrap();
        node.execute_add_data_native(request.data()).unwrap();
        require_joiner_routing(&node, &request);
        drop(node);
        assert_eq!(count.load(SeqCst), 0);
    }
}

#[test]
fn wrong_wire_bytes_and_oversize_cannot_replace_the_frozen_original() {
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xea07);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 67);
    let count = Arc::new(AtomicUsize::new(0));
    begin(&mut node, &native, &count, request.data());
    let mut changed = request.frame.clone();
    changed[0] ^= 1;
    let oversized = vec![9; MAX_FOUNDER_ADD_KEY_PACKAGE_BYTES + 1];
    for bytes in [changed.as_slice(), oversized.as_slice()] {
        let bad = V2FounderAddRequestData {
            key_package: bytes,
            ..request.data()
        };
        assert!(node.execute_add_data_native(bad).is_err());
        assert!(node.pending.as_ref().unwrap().phase == Phase::Prepared);
        assert_eq!(node.pending.as_ref().unwrap().admitted_at_ms, None);
    }
    node.execute_add_data_native(request.data()).unwrap();
    assert_eq!(node.completed_add_data().unwrap().after_epoch_data(), 1);
}

#[test]
fn no_routing_reservation_is_returned_before_any_native_or_request_plan_entry() {
    struct OldMode(Fee);
    impl TextCurrentAddReservation for OldMode {
        fn limits(&self) -> TextCurrentAddLimits {
            let mut limits = self.0.limits();
            limits.routing_typed_bytes = 0;
            limits.routing_work = 0;
            limits
        }
    }
    let (original, _reservation, _opened, _old_fee) = prepared_node(0xea08);
    let mut node = original.prepare_governance_data().unwrap();
    let native = Native::new();
    let request = Request::new(&node, 68);
    let count = Arc::new(AtomicUsize::new(1));
    let old = TextCurrentAddAdmission::new(OldMode(Fee(count.clone()))).unwrap();
    assert!(!old.supports_routing_data());
    let refused = node
        .prepare_add_data(
            request.data(),
            native.source.observe(),
            old,
            native.cut(NativeChatKeyPurpose::AddSdk),
            native.cut(NativeChatKeyPurpose::GoverningHeader),
            native.cut(NativeChatKeyPurpose::JoinWelcome),
        )
        .unwrap_err();
    assert!(node.pending.is_none());
    let (original, old, sdk, header, welcome) = *refused;
    drop(old);
    assert_eq!(count.load(SeqCst), 0);
    let paid = admission(&count);
    assert!(paid.supports_routing_data());
    node.prepare_add_data(request.data(), original, paid, sdk, header, welcome)
        .unwrap();
    node.execute_add_data_native(request.data()).unwrap();
}

/// Test-only strict decoder. Future production join custody must independently
/// authenticate these labels against its trusted pin/applied MLS Add; merely
/// opening AEAD does not create that provenance or a Source.
pub(super) fn require_joiner_routing(node: &Node, request: &Request) -> Vec<u8> {
    let result = node.completed_add_data().unwrap();
    let joined = ServerGroup::join(&request._peer, result.welcome_data()).unwrap();
    let key = Zeroizing::new(joined.routing_transfer_key(&request._peer).unwrap());
    let mut frame = Decoder::new(result.sealed_routing_transfer_data().unwrap());
    let nonce = frame.get_bytes().unwrap().try_into().unwrap();
    let ciphertext = frame.get_bytes().unwrap().to_vec();
    frame.finish().unwrap();
    let plain = Zeroizing::new(
        catcoms_crypto::unseal(&key, &catcoms_crypto::SealedBlob { nonce, ciphertext }).unwrap(),
    );
    let mut d = Decoder::new(&plain);
    assert_eq!(
        d.get_bytes().unwrap(),
        b"mewtual/chat/join-routing-plain/v2\0"
    );
    assert_eq!(d.get_u16().unwrap(), 2);
    let plan = &node.pending.as_ref().unwrap().request;
    let spec = request.request.spec_data();
    assert_eq!(
        d.get_bytes().unwrap(),
        request.invite.spec_data().anchor.group
    );
    assert_eq!(d.get_bytes().unwrap(), spec.anchor);
    assert_eq!(d.get_bytes().unwrap(), request.request.id_data());
    assert_eq!(d.get_bytes().unwrap(), spec.key_package_wire);
    assert_eq!(d.get_bytes().unwrap(), spec.joiner);
    assert_eq!(d.get_bytes().unwrap(), spec.key_package_reference);
    assert_eq!(d.get_bytes().unwrap(), spec.credential);
    assert_eq!(d.get_u64().unwrap(), plan.before_epoch);
    assert_eq!(d.get_u64().unwrap(), plan.after_epoch);
    assert_eq!(d.get_bytes().unwrap(), plan.parent.as_bytes());
    assert_eq!(d.get_bytes().unwrap(), request.invite.spec_data().policy);
    assert_eq!(
        d.get_bytes().unwrap(),
        request.invite.spec_data().public_routing_plan
    );
    assert_eq!(
        d.get_bytes().unwrap(),
        node.node
            .seed
            .body
            .metadata
            .policy
            .as_ref()
            .unwrap()
            .encode()
    );
    assert_eq!(d.get_u64().unwrap(), 0);
    assert_eq!(d.get_u32().unwrap(), 1);
    assert_eq!(d.get_u64().unwrap(), 0);
    assert!(d.get_bytes().unwrap() == node.node.seed.body.metadata.routing[0].1.as_ref());
    assert!(d.get_bytes().unwrap() == node.node.seed.body.metadata._file_wrap_key.as_ref());
    d.finish().unwrap();
    // Return only the ciphertext, never the opened secret frame.
    result.sealed_routing_transfer_data().unwrap().to_vec()
}
