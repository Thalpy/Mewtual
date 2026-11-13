//! Actual ChannelSync nodes, real MLS devices and the production general verifier/plan issuer.
//! The founder of a fresh group is a member of it, so these exercise the joined-member seams
//! on a genuine live node. Removal of a joined member needs the separate joined-member source
//! and governance replay; it is covered at Core level and remains an App integration gap.
use super::governance_fixture::*;
use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};

#[path = "native_tests.rs"]
mod native;

fn member_node(
    hub: &Arc<Hub>,
    seed: u64,
    peer: u64,
) -> (
    ChannelSync<MemNetwork, ChaCha20Rng>,
    ManualClock,
    Governance,
) {
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let key: [u8; 32] = device.public_key_bytes().try_into().unwrap();
    let (profile, admission, header, image) =
        signed_genesis(&group.group_id(), key, |bytes| device.sign(bytes).unwrap());
    let g = governance(&profile, &admission, &header, &image);
    let clock = ManualClock::new(1_000);
    let node = ChannelSync::new(
        hub.join(PeerId::from_u64(peer)),
        group,
        device,
        ChaCha20Rng::seed_from_u64(seed),
        Box::new(clock.clone()),
    );
    (node, clock, g)
}

#[test]
fn joined_draw_uses_the_nodes_own_clock_and_rng_exactly_once() {
    let hub = Hub::new();
    let (mut node, clock, _g) = member_node(&hub, 901, 1);
    let (mut other, _, _) = member_node(&hub, 905, 2);
    let instance = node.registry_instance();
    clock.set_wall_ms(5_000);
    // Another node's incarnation cannot drive this node's seams; nothing is drawn or bound.
    let mut slot = V2MemberDraftInputs::new();
    assert_eq!(
        node.draw_ordinary_draft_inputs(&other.registry_instance(), &mut slot)
            .unwrap_err(),
        V2MemberSignRefusal::Source
    );
    assert!(slot.drawn_data().is_none());
    // Fork the node's actual RNG state so the exact consumed stream can be checked.
    let mut fork = node.with_registry_context(|_, _, _, rng| rng.clone());
    let first = node
        .draw_ordinary_draft_inputs(&instance, &mut slot)
        .unwrap();
    assert_eq!(first.0, 5_000);
    let mut nonce = [0; 16];
    fork.fill_bytes(&mut nonce);
    assert_eq!(first.1, nonce);
    // A retry after the clock moved returns the SAME cached values and consumes nothing.
    clock.set_wall_ms(9_000);
    assert_eq!(
        node.draw_ordinary_draft_inputs(&instance, &mut slot)
            .unwrap(),
        first
    );
    assert_eq!(slot.drawn_data(), Some(first));
    let mut expected_next = [0; 16];
    fork.fill_bytes(&mut expected_next);
    let actual_next = node.with_registry_context(|_, _, _, rng| {
        let mut next = [0; 16];
        rng.fill_bytes(&mut next);
        next
    });
    assert_eq!(actual_next, expected_next);
    // The slot is bound to this node: another live node cannot adopt it, even unchanged.
    let other_instance = other.registry_instance();
    assert_eq!(
        other
            .draw_ordinary_draft_inputs(&other_instance, &mut slot)
            .unwrap_err(),
        V2MemberSignRefusal::Source
    );
    assert_eq!(slot.drawn_data(), Some(first));
}

#[test]
fn joined_seam_signs_once_and_a_retry_reuses_the_cached_raw() {
    let hub = Hub::new();
    let (mut node, _clock, g) = member_node(&hub, 902, 1);
    let instance = node.registry_instance();
    let signer = node.ordinary_member_signer_data(&instance).unwrap();
    let mut slot = V2MemberDraftInputs::new();
    let drawn = node
        .draw_ordinary_draft_inputs(&instance, &mut slot)
        .unwrap();
    let draft = create_draft(&g, &g.founder, signer, drawn, "joined ordinary create");
    let mut plan = g
        .verifier
        .prepare_ordinary_signing_plan(&draft, &g.founder, &[], &g.view)
        .unwrap();
    let original = plan.observe_original();
    let calls = key_calls_for_test();
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    drop(node.sign_ordinary_admitted(&slot, guard).unwrap());
    assert_eq!(key_calls_for_test(), calls + 1);
    // Lost reply: the same request admits again and gets the SAME raw, no key call.
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    let guard = node.sign_ordinary_admitted(&slot, guard).unwrap();
    assert!(guard.cached_raw64_data().is_some());
    drop(guard);
    assert_eq!(key_calls_for_test(), calls + 1);
    let signed = plan.complete_cached_raw64_data(&original, &draft).unwrap();
    assert!(signed.verify());
    assert_eq!(signed.signer(), &signer);
    assert_eq!(signed.semantic_bytes(), draft.signing_bytes_data());
}

#[test]
fn another_incarnation_or_device_refuses_before_the_key_entry() {
    let hub = Hub::new();
    let (mut node, _clock, g) = member_node(&hub, 903, 1);
    let (mut other, _clock2, _g2) = member_node(&hub, 904, 2);
    let instance = node.registry_instance();
    let other_instance = other.registry_instance();
    let signer = node.ordinary_member_signer_data(&instance).unwrap();
    let mut slot = V2MemberDraftInputs::new();
    let drawn = node
        .draw_ordinary_draft_inputs(&instance, &mut slot)
        .unwrap();
    let mut foreign_slot = V2MemberDraftInputs::new();
    other
        .draw_ordinary_draft_inputs(&other_instance, &mut foreign_slot)
        .unwrap();
    let draft = create_draft(&g, &g.founder, signer, drawn, "only the original key");
    let mut plan = g
        .verifier
        .prepare_ordinary_signing_plan(&draft, &g.founder, &[], &g.view)
        .unwrap();
    let original = plan.observe_original();
    let calls = key_calls_for_test();
    // The right node with inputs drawn on another incarnation.
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    assert_eq!(
        node.sign_ordinary_admitted(&foreign_slot, guard)
            .unwrap_err(),
        V2MemberSignRefusal::Source
    );
    // Another live node presented with this node's original inputs.
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    assert_eq!(
        other.sign_ordinary_admitted(&slot, guard).unwrap_err(),
        V2MemberSignRefusal::Source
    );
    // A bare device with another key reaches the shared tail and refuses as Key.
    let stranger = MlsDevice::generate().unwrap();
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    assert_eq!(
        sign_admitted_with_device(&stranger, &slot, guard).unwrap_err(),
        V2MemberSignRefusal::Key
    );
    assert_eq!(key_calls_for_test(), calls);
    assert!(plan
        .borrow_cached_raw64_data(&original, &draft)
        .unwrap()
        .is_none());
    // None of those consumed the one key entry: the original node still signs it.
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    drop(node.sign_ordinary_admitted(&slot, guard).unwrap());
    assert_eq!(key_calls_for_test(), calls + 1);
    assert!(plan
        .complete_cached_raw64_data(&original, &draft)
        .unwrap()
        .verify());
}

#[test]
fn closed_origin_refuses_admission_but_cached_raw_completes_as_historical_data() {
    let hub = Hub::new();
    let (mut node, _clock, g) = member_node(&hub, 906, 1);
    let instance = node.registry_instance();
    let signer = node.ordinary_member_signer_data(&instance).unwrap();
    let mut slot = V2MemberDraftInputs::new();
    let drawn = node
        .draw_ordinary_draft_inputs(&instance, &mut slot)
        .unwrap();
    let draft = create_draft(&g, &g.founder, signer, drawn, "before stop");
    let mut plan = g
        .verifier
        .prepare_ordinary_signing_plan(&draft, &g.founder, &[], &g.view)
        .unwrap();
    let original = plan.observe_original();
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    drop(node.sign_ordinary_admitted(&slot, guard).unwrap());
    // Live: a RawCached Plan is still admitted (that is how cached-raw retries work), so the
    // refusal below is caused by Stop and nothing else.
    drop(plan.admit_signing(&draft, &signer).unwrap());
    // Source Stop: no fresh admission, but the original raw is still exact DATA.
    g.live.0.store(false, std::sync::atomic::Ordering::Release);
    assert!(plan.admit_signing(&draft, &signer).is_err());
    let signed = plan.complete_cached_raw64_data(&original, &draft).unwrap();
    assert!(signed.verify());
    assert!(signed.id() == draft.action_id_data());
}

#[test]
fn a_draft_not_built_from_this_requests_draw_refuses_before_the_key_entry() {
    let hub = Hub::new();
    let (mut node, _clock, g) = member_node(&hub, 907, 1);
    let instance = node.registry_instance();
    let signer = node.ordinary_member_signer_data(&instance).unwrap();
    let mut slot = V2MemberDraftInputs::new();
    let (time, mut nonce) = node
        .draw_ordinary_draft_inputs(&instance, &mut slot)
        .unwrap();
    // Same node, same bound cell, but the Draft carries another nonce (a second Draft
    // reusing one draw). The cut must refuse and leave the Plan unconsumed.
    nonce[0] ^= 1;
    let other = create_draft(
        &g,
        &g.founder,
        signer,
        (time, nonce),
        "second draft, one draw",
    );
    let mut plan = g
        .verifier
        .prepare_ordinary_signing_plan(&other, &g.founder, &[], &g.view)
        .unwrap();
    let original = plan.observe_original();
    let calls = key_calls_for_test();
    let guard = plan.admit_signing(&other, &signer).unwrap();
    assert_eq!(
        node.sign_ordinary_admitted(&slot, guard).unwrap_err(),
        V2MemberSignRefusal::Source
    );
    assert_eq!(key_calls_for_test(), calls);
    assert!(plan
        .borrow_cached_raw64_data(&original, &other)
        .unwrap()
        .is_none());
}

#[test]
fn entered_draw_without_a_result_is_terminal_and_never_redraws() {
    let mut slot = V2MemberDraftInputs::new();
    assert_eq!(slot.enter_clock(), Ok(true));
    // A lost return between the mark and the cached time: never a second Clock call.
    assert_eq!(slot.enter_clock(), Err(V2MemberSignRefusal::Terminal));
    assert_eq!(slot.enter_nonce(), Err(V2MemberSignRefusal::Terminal));
    assert!(slot.drawn_data().is_none());

    let mut slot = V2MemberDraftInputs::new();
    assert_eq!(slot.enter_nonce(), Err(V2MemberSignRefusal::Terminal));

    let mut slot = V2MemberDraftInputs::new();
    assert_eq!(slot.enter_clock(), Ok(true));
    slot.cache_time(7);
    assert_eq!(slot.enter_clock(), Ok(false));
    assert_eq!(slot.enter_nonce(), Ok(true));
    // A lost return after the RNG mark: never a second RNG fill.
    assert_eq!(slot.enter_nonce(), Err(V2MemberSignRefusal::Terminal));
    assert!(slot.drawn_data().is_none());

    let mut slot = V2MemberDraftInputs::new();
    assert_eq!(slot.enter_clock(), Ok(true));
    slot.cache_time(8);
    assert_eq!(slot.enter_nonce(), Ok(true));
    slot.cache_nonce([9; 16]);
    assert_eq!(slot.drawn_data(), Some((8, [9; 16])));
    assert_eq!(slot.enter_clock(), Ok(false));
    assert_eq!(slot.enter_nonce(), Ok(false));
}
