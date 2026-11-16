//! Real closed-node regressions for an immutable existing Header loan. These are
//! SDK DATA tests, never Root SavedS/Current or durable B-selection evidence.
use super::*;

fn unsigned(
    node: &mut UnboundV2GenesisNode<MemNetwork, ChaCha20Rng>,
    token: u8,
) -> (ReplayMath, PreparedV2AppendData, V2AppendPlanObservation) {
    let d = node.data();
    let model = ReplayMath::from_genesis(d.profile, d.admission, d.header, d.state).unwrap();
    let mut created = None;
    node.prepare_create_data(request(token, 9, "existing exact Header"), &mut created)
        .unwrap();
    let action = created
        .as_ref()
        .unwrap()
        .borrow_data(node)
        .unwrap()
        .action
        .clone();
    let mut append = None;
    node.prepare_create_append_data(&model, &action, &mut append)
        .unwrap();
    let append = append.unwrap();
    let original = append.observe_plan_data(node, &model).unwrap();
    (model, append, original)
}

#[test]
fn existing_materialized_loan_preserves_absence_then_borrows_same_original_outputs() {
    let (mut node, _reservation, _opened, _count) = prepared_node(0xc170);
    let (model, mut prepared, original) = unsigned(&mut node, 17);
    let before = snapshot(&node);
    append::reset_for_test();
    assert!(matches!(
        prepared.borrow_existing_materialized_append_data(&original),
        Err(V2CreateRefusal::Terminal)
    ));
    assert_eq!(prepared.payload_presence_for_test(), (false, false));
    assert_eq!(append::signs_for_test(), 0);
    node.sign_reserved_append_bytes_data(&original, &mut prepared)
        .unwrap();
    assert!(matches!(
        prepared.borrow_existing_materialized_append_data(&original),
        Err(V2CreateRefusal::Terminal)
    ));
    assert_eq!(prepared.payload_presence_for_test(), (true, false));
    node.materialize_signed_append_data(&model, &mut prepared)
        .unwrap();
    let complete = prepared.borrow_data(&node, &model).unwrap();
    for _ in 0..3 {
        let cached = prepared
            .borrow_existing_materialized_append_data(&original)
            .unwrap();
        assert!(std::ptr::eq(cached.header, complete.header));
        assert!(std::ptr::eq(cached.body, complete.body));
        assert!(std::ptr::eq(cached.record, complete.record));
        assert!(std::ptr::eq(cached.action, complete.action));
        assert!(std::ptr::eq(cached.image, complete.image));
    }
    assert_eq!(append::signs_for_test(), 1);
    assert_eq!(snapshot(&node).as_slice(), before.as_slice());
}

#[test]
fn existing_materialized_loan_checks_exact_weak_identity_before_output_presence() {
    let (mut node, _reservation, _opened, _count) = prepared_node(0xc171);
    let (model, mut first, original) = unsigned(&mut node, 18);
    let action = first
        .borrow_unsigned_data(&node, &model)
        .unwrap()
        .action
        .clone();
    let mut second = None;
    node.prepare_create_append_data(&model, &action, &mut second)
        .unwrap();
    let mut second = second.unwrap();
    let second_original = second.observe_plan_data(&node, &model).unwrap();
    // Equal unsigned bytes do not adopt another original, even before it has raw.
    assert!(matches!(
        second.borrow_existing_materialized_append_data(&original),
        Err(V2CreateRefusal::RequestMismatch)
    ));
    append::reset_for_test();
    node.sign_reserved_append_bytes_data(&original, &mut first)
        .unwrap();
    node.materialize_signed_append_data(&model, &mut first)
        .unwrap();
    node.sign_reserved_append_bytes_data(&second_original, &mut second)
        .unwrap();
    node.materialize_signed_append_data(&model, &mut second)
        .unwrap();
    assert_eq!(
        first
            .borrow_existing_materialized_append_data(&original)
            .unwrap()
            .header,
        second
            .borrow_existing_materialized_append_data(&second_original)
            .unwrap()
            .header
    );
    assert!(matches!(
        second.borrow_existing_materialized_append_data(&original),
        Err(V2CreateRefusal::RequestMismatch)
    ));
    drop(first);
    assert!(matches!(
        second.borrow_existing_materialized_append_data(&original),
        Err(V2CreateRefusal::RequestMismatch)
    ));
    assert!(second
        .borrow_existing_materialized_append_data(&second_original)
        .is_ok());
    assert_eq!(append::signs_for_test(), 2);
}

#[test]
fn header_cached_before_materialization_postcheck_survives_failed_source_and_node_drop() {
    let (mut node, reservation, _opened, count) = prepared_node(0xc172);
    let (model, mut prepared, original) = unsigned(&mut node, 19);
    let view = prepared.borrow_unsigned_data(&node, &model).unwrap();
    let signing = view.signing_bytes.to_vec();
    let image = view.image.to_vec();
    let signer = *view.action.signer();
    append::reset_for_test();
    node.sign_reserved_append_bytes_data(&original, &mut prepared)
        .unwrap();
    let owner = reservation.owner.clone();
    append::after_materialize_for_test(Box::new(move || {
        owner.send_replace(true);
    }));
    assert!(node
        .materialize_signed_append_data(&model, &mut prepared)
        .is_err());
    assert_eq!(prepared.payload_presence_for_test(), (true, true));
    assert!(prepared.borrow_data(&node, &model).is_err());
    assert!(node
        .materialize_signed_append_data(&model, &mut prepared)
        .is_err());
    let raw = *prepared
        .borrow_existing_raw_signature_data(&original)
        .unwrap();
    let header = prepared
        .borrow_existing_materialized_append_data(&original)
        .unwrap()
        .header
        .encode();
    drop(node);
    assert_eq!(count.load(Ordering::Acquire), 0);
    for _ in 0..3 {
        let cached = prepared
            .borrow_existing_materialized_append_data(&original)
            .unwrap();
        assert_eq!(cached.header.encode(), header);
        assert_eq!(cached.header.semantic_bytes(), signing);
        assert_eq!(cached.image, image.as_slice());
        assert!(catcoms_crypto::verify_with_public_bytes(
            &signer, &signing, &raw
        ));
    }
    assert_eq!(append::signs_for_test(), 1);
}

#[test]
fn cancellation_before_materialization_never_fabricates_a_header() {
    let (mut node, reservation, _opened, _count) = prepared_node(0xc173);
    let (model, mut prepared, original) = unsigned(&mut node, 20);
    append::reset_for_test();
    node.sign_reserved_append_bytes_data(&original, &mut prepared)
        .unwrap();
    reservation.owner.send_replace(true);
    assert!(node
        .materialize_signed_append_data(&model, &mut prepared)
        .is_err());
    for _ in 0..3 {
        assert!(matches!(
            prepared.borrow_existing_materialized_append_data(&original),
            Err(V2CreateRefusal::Terminal)
        ));
        assert_eq!(prepared.payload_presence_for_test(), (true, false));
    }
    assert_eq!(append::signs_for_test(), 1);
}
