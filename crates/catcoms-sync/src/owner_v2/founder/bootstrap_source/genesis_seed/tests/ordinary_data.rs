//! Genuine original source -> closed SDK -> exact unbound node DATA tests. These reuse
//! the existing receiver-only reservation/resource fixtures; they are NOT App Current or
//! actual Root operation-payment proofs. Real paid Current/TxnA/B tests are cohort gates.
//! Full production Root resources, storage selection, transport and native ingress remain
//! separate gates; HOST selection inside these fixtures is an explicit test trust decision.
use super::super::ordinary_append::{self as append, *};
use super::super::ordinary_create::{self as create, *};
use super::*;
mod materialized;
mod member_signing;
use catcoms_crypto::{unseal, SealedBlob};
use catcoms_replication::chat::owner_state::transition::{ReplayBody, ReplayMath};
use catcoms_replication::chat::{Body, SignedAction};
use catcoms_storage::pad::{unpad, OP_PAD_CEILING, OP_PAD_FLOOR};
use catcoms_wire::{Decoder, DocType};

fn prepared_node(
    id: u64,
) -> (
    UnboundV2GenesisNode<MemNetwork, ChaCha20Rng>,
    Reservation,
    OpenedBootstrapSourceData,
    Arc<AtomicUsize>,
) {
    let f = fixture(id);
    let count = Arc::new(AtomicUsize::new(0));
    let mut restored = seed(&f.opened, &count);
    restored.prepare_scope(&f.reservation).unwrap();
    let parts = f
        .source
        .into_genesis_runtime_parts(f.node, f.opened.source_id_data())
        .unwrap();
    let node = restored.prepare_runtime(parts).unwrap().into_node();
    (node, f.reservation, f.opened, count)
}
fn request<'a>(token: u8, channel: u128, text: &'a str) -> V2CreateRequest<'a> {
    V2CreateRequest {
        token: [token; 32],
        channel,
        text,
    }
}
fn snapshot<T, R>(node: &UnboundV2GenesisNode<T, R>) -> Zeroizing<Vec<u8>> {
    catcoms_mls::persist::snapshot_server(&node.seed.body.device, &node.seed.body.group).unwrap()
}
// Exercise TWO DATA phases only: no fake Saved intent or Root signing permission.
// Actual Root durable-intent -> private-sign fault tests remain a declaration gate.
fn append_data(
    node: &UnboundV2GenesisNode<MemNetwork, ChaCha20Rng>,
    model: &ReplayMath,
    action: &SignedAction,
    slot: &mut Option<PreparedV2AppendData>,
) -> Result<(), V2CreateRefusal> {
    let signs = append::signs_for_test();
    node.prepare_create_append_data(model, action, slot)?;
    assert_eq!(append::signs_for_test(), signs);
    let unsigned = slot.as_ref().unwrap().borrow_unsigned_data(node, model)?;
    let bytes = unsigned.signing_bytes.to_vec();
    let image = unsigned.image.to_vec();
    let intent = unsigned.intent;
    let original = slot.as_ref().unwrap().observe_plan_data(node, model)?;
    node.sign_reserved_append_bytes_data(&original, slot.as_mut().unwrap())?;
    node.materialize_signed_append_data(model, slot.as_mut().unwrap())?;
    let signed = slot.as_ref().unwrap().borrow_data(node, model)?;
    assert!(signed.header.semantic_bytes() == bytes);
    assert!(signed.image == image.as_slice());
    assert_eq!(
        slot.as_ref()
            .unwrap()
            .borrow_unsigned_data(node, model)?
            .intent,
        intent
    );
    Ok(())
}
// Independent implementation of the frozen KDF for these specification regressions.
fn key<T, R>(
    node: &UnboundV2GenesisNode<T, R>,
    channel: u128,
    profile: [u8; 32],
    epoch: u64,
) -> Zeroizing<[u8; 32]> {
    let base = Zeroizing::new(
        node.seed
            .body
            .group
            .channel_secret(&node.seed.body.device, DocType::Channel, channel)
            .unwrap(),
    );
    let realm = catcoms_replication::chat::Realm::new(node.seed.body.group.group_id_ref(), channel)
        .unwrap();
    let mut h = blake3::Hasher::new_keyed(&base);
    h.update(b"mewtual/chat/ordinary-key/v2\0");
    h.update(&profile);
    h.update(realm.id().as_bytes());
    h.update(&epoch.to_be_bytes());
    Zeroizing::new(*h.finalize().as_bytes())
}
fn check_encrypted<T, R>(node: &UnboundV2GenesisNode<T, R>, data: V2CreateDataView<'_>) {
    let profile = *node.data().profile.id().as_bytes();
    let blob = SealedBlob {
        nonce: *data.nonce,
        ciphertext: data.ciphertext.to_vec(),
    };
    let secret = key(node, data.channel, profile, 0);
    let plain = Zeroizing::new(unseal(&secret, &blob).unwrap());
    let raw = unpad(&plain, OP_PAD_FLOOR, OP_PAD_CEILING).unwrap();
    let mut d = Decoder::new(raw);
    assert!(d.get_bytes().unwrap() == b"mewtual/chat/ordinary-envelope/v2\0");
    assert_eq!(d.get_u16().unwrap(), 2);
    assert!(d.get_bytes().unwrap() == node.seed.body.group.group_id_ref());
    assert!(d.get_bytes().unwrap() == profile.as_slice());
    assert!(d.get_bytes().unwrap() == node.data().admission.admission_id().as_bytes());
    assert_eq!(d.get_u64().unwrap(), data.epoch);
    assert_eq!(d.get_u128().unwrap(), data.channel);
    let encoded = d.get_bytes().unwrap();
    assert!(encoded == data.action_bytes);
    assert!(SignedAction::decode(encoded).unwrap() == *data.action);
    assert!(data.action.verify());
    d.finish().unwrap();
    let legacy = Zeroizing::new(
        node.seed
            .body
            .group
            .channel_secret(&node.seed.body.device, DocType::Channel, data.channel)
            .unwrap(),
    );
    assert!(unseal(&legacy, &blob).is_err());
    let mut wrong_profile = profile;
    wrong_profile[0] ^= 1;
    for wrong in [
        key(node, data.channel, wrong_profile, 0),
        key(node, data.channel + 1, profile, 0),
        key(node, data.channel, profile, 1),
    ] {
        assert!(unseal(&wrong, &blob).is_err());
    }
}

#[test]
fn actual_create_max_padding_signature_and_source_unchanged_complete_retry_skips_all_calls() {
    let (mut node, _reservation, _opened, count) = prepared_node(0xc100);
    let before = snapshot(&node);
    let text = "x".repeat(64 * 1024);
    let mut slot = None;
    create::reset_for_test();
    node.prepare_create_data(request(1, 9, &text), &mut slot)
        .unwrap();
    assert_eq!(create::entries_for_test(), [1; 5]);
    let data = slot.as_ref().unwrap().borrow_data(&node).unwrap();
    assert_eq!(data.ciphertext.len(), MAX_CREATE_CIPHERTEXT_BYTES);
    assert!(data.action_bytes.len() <= MAX_CREATE_ACTION_BYTES);
    assert!(
        matches!(&data.action.spec().body, Body::Create { text: t, reply_to: None } if t == &text)
    );
    check_encrypted(&node, data);
    let encoded = slot
        .as_ref()
        .unwrap()
        .borrow_data(&node)
        .unwrap()
        .action_bytes
        .to_vec();
    let cipher = slot
        .as_ref()
        .unwrap()
        .borrow_data(&node)
        .unwrap()
        .ciphertext
        .to_vec();
    let mut rng = node._parts.body._rng.clone();
    let expected = rng.next_u64();
    node.prepare_create_data(request(1, 9, &text), &mut slot)
        .unwrap();
    assert_eq!(create::entries_for_test(), [1; 5]);
    assert_eq!(node._parts.body._rng.clone().next_u64(), expected);
    let data = slot.as_ref().unwrap().borrow_data(&node).unwrap();
    assert!(data.action_bytes == encoded.as_slice() && data.ciphertext == cipher.as_slice());
    assert!(snapshot(&node).as_slice() == before.as_slice());
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn bounds_and_changed_request_or_equal_new_graph_never_replace_the_original_pending() {
    let (mut node, reservation, opened, count) = prepared_node(0xc101);
    create::reset_for_test();
    let mut slot = None;
    for text in [String::new(), "x".repeat(64 * 1024 + 1)] {
        assert_eq!(
            node.prepare_create_data(request(2, 9, &text), &mut slot),
            Err(V2CreateRefusal::Bound)
        );
        assert!(slot.is_none());
    }
    assert_eq!(create::entries_for_test(), [0; 5]);
    node.prepare_create_data(request(2, 9, "original"), &mut slot)
        .unwrap();
    let original = slot
        .as_ref()
        .unwrap()
        .borrow_data(&node)
        .unwrap()
        .action_bytes
        .to_vec();
    for wrong in [
        request(3, 9, "original"),
        request(2, 10, "original"),
        request(2, 9, "changed"),
    ] {
        assert_eq!(
            node.prepare_create_data(wrong, &mut slot),
            Err(V2CreateRefusal::RequestMismatch)
        );
    }
    let mut restored = seed(&opened, &count);
    restored.prepare_scope(&reservation).unwrap();
    let mut equal = restored
        .prepare_runtime(V2GenesisRuntimeParts::cold_data(
            Arc::new(Hub::new().join(PeerId::from_u64(0xc199))),
            ChaCha20Rng::seed_from_u64(1),
            Arc::new(ManualClock::new(1000)),
        ))
        .unwrap()
        .into_node();
    assert!(equal
        .prepare_create_data(request(2, 9, "original"), &mut slot)
        .is_err());
    assert!(slot.as_ref().unwrap().borrow_data(&equal).is_err());
    assert!(
        slot.as_ref()
            .unwrap()
            .borrow_data(&node)
            .unwrap()
            .action_bytes
            == original.as_slice()
    );
    assert_eq!(create::entries_for_test(), [1; 5]);
    node.seed
        .body
        .metadata
        .scope
        .corrupt_genesis_planned_revision_for_test(Some(38));
    assert!(slot.as_ref().unwrap().borrow_data(&node).is_err());
    assert_eq!(create::entries_for_test(), [1; 5]);
}

#[test]
fn each_real_boundary_cancellation_retains_original_stage_without_repeating_any_operation() {
    for point in 0..5 {
        let (mut node, reservation, _opened, _count) = prepared_node(0xc110 + point as u64);
        let before = snapshot(&node);
        let owner = reservation.owner.clone();
        create::reset_for_test();
        create::after_for_test(
            point,
            Box::new(move || {
                owner.send_replace(true);
            }),
        );
        let mut slot = None;
        assert!(node
            .prepare_create_data(request(4, 9, "real late cancellation"), &mut slot)
            .is_err());
        let counts = create::entries_for_test();
        for (index, count) in counts.iter().enumerate() {
            assert_eq!(*count, usize::from(index <= point));
        }
        let original = slot.as_ref().unwrap().payload_sizes_for_test();
        if point == 4 {
            assert!(original.0 && original.1 > 0 && original.2.is_some());
        }
        assert!(node
            .prepare_create_data(request(4, 9, "real late cancellation"), &mut slot)
            .is_err());
        assert_eq!(create::entries_for_test(), counts);
        assert_eq!(slot.as_ref().unwrap().payload_sizes_for_test(), original);
        assert!(slot.as_ref().unwrap().borrow_data(&node).is_err());
        assert!(snapshot(&node).as_slice() == before.as_slice());
    }
}

#[test]
fn actual_detach_once_wrong_id_missing_cipher_and_no_node_keepalive() {
    let (mut node, _reservation, _opened, count) = prepared_node(0xc120);
    let mut slot = None;
    let mut other = None;
    create::reset_for_test();
    node.prepare_create_data(request(5, 9, "original detached"), &mut slot)
        .unwrap();
    node.prepare_create_data(request(6, 9, "other detached"), &mut other)
        .unwrap();
    let foreign = other.as_mut().unwrap().take_for_handoff(&node).unwrap();
    assert!(slot
        .as_ref()
        .unwrap()
        .validate_transferred_data(&node, foreign.id_data())
        .is_err());
    let detached = slot.as_mut().unwrap().take_for_handoff(&node).unwrap();
    let original_id = detached.id_data();
    slot.as_ref()
        .unwrap()
        .validate_transferred_data(&node, original_id)
        .unwrap();
    assert!(slot
        .as_ref()
        .unwrap()
        .validate_transferred_data(&node, foreign.id_data())
        .is_err());
    let calls = create::entries_for_test();
    assert!(node
        .prepare_create_data(request(5, 9, "original detached"), &mut slot)
        .is_err());
    assert!(slot.as_ref().unwrap().borrow_data(&node).is_err());
    assert!(slot.as_mut().unwrap().take_for_handoff(&node).is_err());
    assert_eq!(create::entries_for_test(), calls);
    let cipher_hash = blake3::hash(detached.view_data().ciphertext);
    assert!(detached.view_data().action.verify());
    drop(node);
    assert_eq!(count.load(Ordering::SeqCst), 0); // packet/pending cannot keep either SDK half
    assert_eq!(detached.id_data(), original_id);
    assert!(blake3::hash(detached.view_data().ciphertext) == cipher_hash);

    let (mut node, _reservation, _opened, _count) = prepared_node(0xc121);
    let mut missing = None;
    node.prepare_create_data(request(7, 9, "missing bytes"), &mut missing)
        .unwrap();
    missing.as_mut().unwrap().drop_ciphertext_for_test();
    assert!(node
        .prepare_create_data(request(7, 9, "missing bytes"), &mut missing)
        .is_err());
    assert!(missing.as_ref().unwrap().borrow_data(&node).is_err());
    assert!(missing.as_mut().unwrap().take_for_handoff(&node).is_err());
    assert!(missing
        .as_ref()
        .unwrap()
        .validate_transferred_data(&node, original_id)
        .is_err());
}

#[test]
fn actual_two_creates_append_on_own_tips_without_sdk_mutation_or_resigning() {
    let (mut node, _reservation, _opened, _count) = prepared_node(0xc130);
    let before = snapshot(&node);
    let genesis = node.data();
    let model = ReplayMath::from_genesis(
        genesis.profile,
        genesis.admission,
        genesis.header,
        genesis.state,
    )
    .unwrap();
    let mut first = None;
    node.prepare_create_data(request(8, 9, "first"), &mut first)
        .unwrap();
    let first_action = first
        .as_ref()
        .unwrap()
        .borrow_data(&node)
        .unwrap()
        .action
        .clone();
    let mut appended = None;
    append::reset_for_test();
    append_data(&node, &model, &first_action, &mut appended).unwrap();
    assert_eq!(append::signs_for_test(), 1);
    let mut invalid_bytes = first_action.encode();
    *invalid_bytes.last_mut().unwrap() ^= 1; // same semantic ActionId, invalid actual signature
    let invalid = SignedAction::decode(&invalid_bytes).unwrap();
    assert_eq!(invalid.id(), first_action.id());
    assert!(!invalid.verify());
    let mut refused = None;
    assert!(node
        .prepare_create_append_data(&model, &invalid, &mut refused)
        .is_err());
    assert!(refused.is_none());
    assert!(node
        .prepare_create_append_data(&model, &invalid, &mut appended)
        .is_err());
    assert_eq!(append::signs_for_test(), 1);
    let data = appended
        .as_ref()
        .unwrap()
        .borrow_data(&node, &model)
        .unwrap();
    data.record.matches_action(&first_action).unwrap();
    assert_eq!(data.header.spec().sequence, 2);
    assert_eq!(data.body.spec().old_tip.count, 0);
    assert_eq!(data.body.spec().new_tip.count, 1);
    let model2 = model
        .derive_successor(data.header, ReplayBody::Append(data.body), data.image)
        .unwrap()
        .next;
    assert_eq!(
        model2.slot(node.data().admission.id()).unwrap().tip_header,
        data.header.id()
    );
    append_data(&node, &model, &first_action, &mut appended).unwrap();
    assert_eq!(append::signs_for_test(), 1);
    assert!(appended
        .as_ref()
        .unwrap()
        .borrow_data(&node, &model2)
        .is_err());
    let mut second = None;
    node.prepare_create_data(request(9, 9, "second"), &mut second)
        .unwrap();
    let second_action = second
        .as_ref()
        .unwrap()
        .borrow_data(&node)
        .unwrap()
        .action
        .clone();
    assert_ne!(first_action.id(), second_action.id());
    let mut appended2 = None;
    append_data(&node, &model2, &second_action, &mut appended2).unwrap();
    let data2 = appended2
        .as_ref()
        .unwrap()
        .borrow_data(&node, &model2)
        .unwrap();
    assert_eq!(data2.header.spec().sequence, 3);
    assert_eq!(data2.body.spec().old_tip.count, 1);
    assert_eq!(data2.body.spec().new_tip.count, 2);
    assert_eq!(
        model2
            .derive_successor(data2.header, ReplayBody::Append(data2.body), data2.image)
            .unwrap()
            .next
            .slot(node.data().admission.id())
            .unwrap()
            .tip_header,
        data2.header.id()
    );
    assert!(snapshot(&node).as_slice() == before.as_slice());
    assert_eq!(append::signs_for_test(), 2);
}

#[test]
fn append_postsign_unwind_missing_header_max_text_and_foreign_parent_preserve_original() {
    let (mut node, _reservation, _opened, _count) = prepared_node(0xc140);
    let before = snapshot(&node);
    let d = node.data();
    let model = ReplayMath::from_genesis(d.profile, d.admission, d.header, d.state).unwrap();
    let mut create_slot = None;
    node.prepare_create_data(request(10, 9, &"z".repeat(64 * 1024)), &mut create_slot)
        .unwrap();
    let action = create_slot
        .as_ref()
        .unwrap()
        .borrow_data(&node)
        .unwrap()
        .action
        .clone();
    append::reset_for_test();
    append::after_sign_for_test(Box::new(|| {
        panic!("simulate lost return after actual signature cache")
    }));
    let mut slot = None;
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        append_data(&node, &model, &action, &mut slot)
    }))
    .is_err());
    assert_eq!(append::signs_for_test(), 1);
    assert_eq!(
        slot.as_ref().unwrap().payload_presence_for_test(),
        (true, false)
    );
    assert!(slot.as_ref().unwrap().borrow_data(&node, &model).is_err());
    append_data(&node, &model, &action, &mut slot).unwrap();
    assert_eq!(append::signs_for_test(), 1); // same cached signature, pure math only
    let original_header = slot
        .as_ref()
        .unwrap()
        .borrow_data(&node, &model)
        .unwrap()
        .header
        .id();
    let (foreign, _foreign_reservation, _foreign_opened, _foreign_count) = prepared_node(0xc141);
    let d = foreign.data();
    let wrong_parent = ReplayMath::from_genesis(d.profile, d.admission, d.header, d.state).unwrap();
    let mut other = None;
    assert!(node
        .prepare_create_append_data(&wrong_parent, &action, &mut other)
        .is_err());
    assert!(other.is_none());
    assert!(node
        .prepare_create_append_data(&wrong_parent, &action, &mut slot)
        .is_err());
    assert_eq!(append::signs_for_test(), 1);
    assert_eq!(
        slot.as_ref()
            .unwrap()
            .borrow_data(&node, &model)
            .unwrap()
            .header
            .id(),
        original_header
    );
    // A true, valid signature over a reply does not widen this initial no-reply profile.
    // Independently pin Core's existing no-reply/empty-parent relation BEFORE any signer.
    let mut nonreply_parents = action.spec().clone();
    nonreply_parents.parents = vec![action.id()];
    let sign_calls = std::cell::Cell::new(0);
    let invalid = SignedAction::sign_using(nonreply_parents, *action.signer(), |bytes| {
        sign_calls.set(sign_calls.get() + 1);
        node.seed
            .body
            .device
            .sign(bytes)
            .map_err(|_| catcoms_replication::chat::ChatError::Signature)
    });
    assert!(matches!(
        invalid,
        Err(catcoms_replication::chat::ChatError::Dependency)
    ));
    assert_eq!(sign_calls.get(), 0);
    let mut reply_spec = action.spec().clone();
    reply_spec.parents = vec![action.id()];
    reply_spec.body = Body::Create {
        text: "valid signed reply".into(),
        reply_to: Some(action.id()),
    };
    let reply = SignedAction::sign_using(reply_spec, *action.signer(), |bytes| {
        node.seed
            .body
            .device
            .sign(bytes)
            .map_err(|_| catcoms_replication::chat::ChatError::Signature)
    })
    .unwrap();
    assert!(reply.verify());
    assert!(node
        .prepare_create_append_data(&model, &reply, &mut other)
        .is_err());
    assert!(other.is_none());
    assert_eq!(append::signs_for_test(), 1);
    slot.as_mut().unwrap().drop_header_for_test();
    assert!(append_data(&node, &model, &action, &mut slot).is_err());
    assert!(slot.as_ref().unwrap().borrow_data(&node, &model).is_err());
    assert_eq!(append::signs_for_test(), 1);
    assert!(snapshot(&node).as_slice() == before.as_slice());
}

#[test]
fn append_actual_postsign_cancellation_and_foreign_scope_never_renew_a_signature() {
    let (mut node, reservation, _opened, _count) = prepared_node(0xc150);
    let before = snapshot(&node);
    let d = node.data();
    let model = ReplayMath::from_genesis(d.profile, d.admission, d.header, d.state).unwrap();
    let mut created = None;
    node.prepare_create_data(
        request(11, 9, "cancel after append signature"),
        &mut created,
    )
    .unwrap();
    let action = created
        .as_ref()
        .unwrap()
        .borrow_data(&node)
        .unwrap()
        .action
        .clone();
    append::reset_for_test();
    let owner = reservation.owner.clone();
    append::after_sign_for_test(Box::new(move || {
        owner.send_replace(true);
    }));
    let mut slot = None;
    assert!(append_data(&node, &model, &action, &mut slot).is_err());
    assert_eq!(
        slot.as_ref().unwrap().payload_presence_for_test(),
        (true, false)
    );
    assert_eq!(append::signs_for_test(), 1);
    assert!(append_data(&node, &model, &action, &mut slot).is_err());
    assert_eq!(append::signs_for_test(), 1);
    assert!(slot.as_ref().unwrap().borrow_data(&node, &model).is_err());
    assert!(snapshot(&node).as_slice() == before.as_slice());

    let (mut node, _reservation, _opened, _count) = prepared_node(0xc151);
    let d = node.data();
    let model = ReplayMath::from_genesis(d.profile, d.admission, d.header, d.state).unwrap();
    let mut created = None;
    node.prepare_create_data(request(12, 9, "scope changes after complete"), &mut created)
        .unwrap();
    let action = created
        .as_ref()
        .unwrap()
        .borrow_data(&node)
        .unwrap()
        .action
        .clone();
    let mut slot = None;
    append::reset_for_test();
    append_data(&node, &model, &action, &mut slot).unwrap();
    node.seed
        .body
        .metadata
        .scope
        .corrupt_genesis_planned_revision_for_test(Some(38));
    assert!(append_data(&node, &model, &action, &mut slot).is_err());
    assert!(slot.as_ref().unwrap().borrow_data(&node, &model).is_err());
    assert_eq!(append::signs_for_test(), 1);
    assert_eq!(
        slot.as_ref().unwrap().payload_presence_for_test(),
        (true, true)
    );
}

#[test]
fn complete_create_survives_late_source_cancellation_as_same_inert_packet() {
    let (mut node, reservation, _opened, count) = prepared_node(0xc160);
    let mut slot = None;
    create::reset_for_test();
    let owner = reservation.owner.clone();
    create::after_for_test(
        4,
        Box::new(move || {
            owner.send_replace(true);
        }),
    );
    assert!(node
        .prepare_create_data(
            request(13, 9, "complete bytes before cancellation"),
            &mut slot
        )
        .is_err());
    assert!(slot.as_ref().unwrap().borrow_data(&node).is_err());
    let before = create::entries_for_test();
    let packet = slot.take().unwrap().into_handoff_data().unwrap();
    assert_eq!(
        create::entries_for_test(),
        before,
        "negative extraction repeats no external call"
    );
    let bytes = packet.view_data().action_bytes.to_vec();
    let id = packet.id_data();
    drop(node);
    assert_eq!(
        count.load(Ordering::Acquire),
        0,
        "inert packet must not retain the SDK"
    );
    assert_eq!(packet.id_data(), id);
    assert_eq!(packet.view_data().action_bytes, bytes.as_slice());
    assert!(packet.view_data().action.verify());
}

#[test]
fn unsigned_append_refuses_negative_handoff_but_raw_signature_survives_failed_source() {
    let (mut node, reservation, _opened, count) = prepared_node(0xc161);
    let d = node.data();
    let model = ReplayMath::from_genesis(d.profile, d.admission, d.header, d.state).unwrap();
    let mut created = None;
    node.prepare_create_data(request(14, 9, "raw signature custody"), &mut created)
        .unwrap();
    let action = created
        .as_ref()
        .unwrap()
        .borrow_data(&node)
        .unwrap()
        .action
        .clone();
    let mut append = None;
    node.prepare_create_append_data(&model, &action, &mut append)
        .unwrap();
    let (original, why) = append.take().unwrap().into_handoff_data().unwrap_err();
    assert_eq!(why, V2CreateRefusal::Terminal);
    append = Some(original);
    let observation = append
        .as_ref()
        .unwrap()
        .observe_plan_data(&node, &model)
        .unwrap();
    assert_eq!(
        append
            .as_ref()
            .unwrap()
            .borrow_existing_raw_signature_data(&observation),
        Err(V2CreateRefusal::Terminal)
    );
    let expected = append
        .as_ref()
        .unwrap()
        .borrow_unsigned_data(&node, &model)
        .unwrap()
        .signing_bytes
        .to_vec();
    append::reset_for_test();
    let owner = reservation.owner.clone();
    append::after_sign_for_test(Box::new(move || {
        owner.send_replace(true);
    }));
    assert!(node
        .sign_reserved_append_bytes_data(&observation, append.as_mut().unwrap())
        .is_err());
    assert_eq!(append::signs_for_test(), 1);
    assert_eq!(
        append.as_ref().unwrap().payload_presence_for_test(),
        (true, false)
    );
    assert!(node
        .materialize_signed_append_data(&model, append.as_mut().unwrap())
        .is_err());
    let cached_raw = *append
        .as_ref()
        .unwrap()
        .borrow_existing_raw_signature_data(&observation)
        .unwrap();
    for _ in 0..3 {
        assert_eq!(
            *append
                .as_ref()
                .unwrap()
                .borrow_existing_raw_signature_data(&observation)
                .unwrap(),
            cached_raw
        );
    }
    assert!(catcoms_crypto::verify_with_public_bytes(
        action.signer(),
        &expected,
        &cached_raw
    ));
    assert_eq!(
        append::signs_for_test(),
        1,
        "negative raw borrows cannot repeat a key call"
    );
    let detached = append.take().unwrap().into_handoff_data().unwrap();
    assert!(
        detached.header_data().is_none(),
        "negative handoff must not fabricate a Header"
    );
    assert_eq!(detached.unsigned_data().signing_bytes, expected.as_slice());
    assert!(catcoms_crypto::verify_with_public_bytes(
        action.signer(),
        &expected,
        detached.raw_signature_data()
    ));
    drop(node);
    assert_eq!(count.load(Ordering::Acquire), 0);
    assert!(detached.unsigned_data().action == &action);
    assert_eq!(append::signs_for_test(), 1);
}

#[test]
fn exact_plan_observation_cannot_sign_another_prepared_child() {
    let (mut node, _reservation, _opened, _count) = prepared_node(0xc162);
    let d = node.data();
    let model = ReplayMath::from_genesis(d.profile, d.admission, d.header, d.state).unwrap();
    let mut created = None;
    node.prepare_create_data(request(15, 9, "exact plan identity"), &mut created)
        .unwrap();
    let action = created
        .as_ref()
        .unwrap()
        .borrow_data(&node)
        .unwrap()
        .action
        .clone();
    let mut first = None;
    let mut second = None;
    node.prepare_create_append_data(&model, &action, &mut first)
        .unwrap();
    node.prepare_create_append_data(&model, &action, &mut second)
        .unwrap();
    let observation = first
        .as_ref()
        .unwrap()
        .observe_plan_data(&node, &model)
        .unwrap();
    append::reset_for_test();
    assert_eq!(
        node.sign_reserved_append_bytes_data(&observation, second.as_mut().unwrap()),
        Err(V2CreateRefusal::RequestMismatch)
    );
    assert_eq!(append::signs_for_test(), 0);
    node.sign_reserved_append_bytes_data(&observation, first.as_mut().unwrap())
        .unwrap();
    assert_eq!(append::signs_for_test(), 1);
    let raw = *first
        .as_ref()
        .unwrap()
        .borrow_existing_raw_signature_data(&observation)
        .unwrap();
    assert_eq!(
        *first
            .as_ref()
            .unwrap()
            .borrow_existing_raw_signature_data(&observation)
            .unwrap(),
        raw
    );
    assert_eq!(
        second
            .as_ref()
            .unwrap()
            .borrow_existing_raw_signature_data(&observation),
        Err(V2CreateRefusal::RequestMismatch)
    );
    assert_eq!(append::signs_for_test(), 1);
    node.sign_reserved_append_bytes_data(&observation, first.as_mut().unwrap())
        .unwrap();
    assert_eq!(append::signs_for_test(), 1);
    let detached = first.take().unwrap().into_handoff_data().unwrap();
    assert!(detached.header_data().is_none());
    assert!(!observation.matches_original_data(second.as_ref().unwrap()));
    assert_eq!(
        second
            .as_ref()
            .unwrap()
            .borrow_existing_raw_signature_data(&observation),
        Err(V2CreateRefusal::RequestMismatch)
    );
}

#[test]
fn actual_node_raw_signature_materializes_for_one_core_candidate_and_exact_create_issuer() {
    use catcoms_replication::chat::{
        ChatLedger, FounderGenesisAnchorData, GuardedProjection, LedgerLimits, ProjectionGroup,
        Realm, V2FounderExpectation, V2FounderPrefixVerifier, V2HostSelectionExpectation,
        V2OriginNegative, V2ProjectionOrigin,
    };
    #[derive(Debug)]
    struct TestHostNegative;
    impl V2OriginNegative for TestHostNegative {
        fn is_live(&self) -> bool {
            true
        }
    }
    let (mut node, _reservation, _opened, _count) = prepared_node(0xc163);
    let d = node.data();
    let group_bytes = d.profile.spec().group.clone();
    let mut domain = ProjectionGroup::new(&group_bytes).unwrap();
    let group = domain.begin_view();
    let expected = V2FounderExpectation::from_trusted_host(FounderGenesisAnchorData {
        group: group_bytes.clone(),
        founder: d.profile.spec().founder,
        profile: d.profile.id(),
        admission: d.admission.admission_id(),
        tenure: d.profile.tenure(),
        header: d.header.id(),
        state: d.header.spec().state,
    })
    .unwrap();
    let mut verifier = V2FounderPrefixVerifier::from_genesis(
        expected,
        d.profile,
        d.admission,
        d.header,
        d.state,
        &group,
        V2ProjectionOrigin::new(Arc::new(TestHostNegative)),
    )
    .unwrap();
    let mut created = None;
    node.prepare_create_data(request(16, 9, "actual node to Core issuer"), &mut created)
        .unwrap();
    let action = created
        .as_ref()
        .unwrap()
        .borrow_data(&node)
        .unwrap()
        .action
        .clone();
    let mut append = None;
    node.prepare_create_append_data(verifier.replay_data(), &action, &mut append)
        .unwrap();
    let observation = append
        .as_ref()
        .unwrap()
        .observe_plan_data(&node, verifier.replay_data())
        .unwrap();
    node.sign_reserved_append_bytes_data(&observation, append.as_mut().unwrap())
        .unwrap();
    node.materialize_signed_append_data(verifier.replay_data(), append.as_mut().unwrap())
        .unwrap();
    let data = append
        .as_ref()
        .unwrap()
        .borrow_data(&node, verifier.replay_data())
        .unwrap();
    let mut candidate = Some(
        verifier
            .derive_verified_append(data.header, data.body, data.image)
            .unwrap(),
    );
    let record = data.record.clone();
    let selected =
        V2HostSelectionExpectation::from_trusted_host(data.header.id(), data.header.spec().state);
    verifier
        .advance_host_selected(&selected, &mut candidate)
        .unwrap();
    let mut ledger = ChatLedger::new(
        Realm::new(&group_bytes, 9).unwrap(),
        LedgerLimits {
            max_actions: 8,
            max_action_bytes: 1024 * 1024,
        },
    )
    .unwrap();
    let view = ledger.begin_projection_view_in(&group).unwrap();
    ledger.import(action.clone()).unwrap();
    ledger
        .authorize(
            action.id(),
            verifier.verify_create_semantics(&action).unwrap(),
        )
        .unwrap();
    while ledger.advance(128).remaining {}
    let token = verifier
        .accepted_create(&action, &record, 0, &[], &view)
        .unwrap();
    ledger.admit_projection(&token, 100).unwrap();
    assert!(
        matches!(ledger.guarded_projection(action.id(), 100).unwrap().with_publication(Clone::clone).unwrap(), GuardedProjection::Visible { text, .. } if text == "actual node to Core issuer")
    );
}
