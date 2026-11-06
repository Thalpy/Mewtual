//! Deterministic work regressions, not timing benchmarks. All actions and authority fixtures
//! carry real signatures. The inactive semantic core still has no production authority issuer.
use super::*;
use std::collections::BTreeSet;

fn original(
    f: &Fixture,
    nonce: u64,
    reply_to: Option<ActionId>,
) -> (SignedAction, VerifiedAuthorityBasis) {
    f.action(
        &f.subjects[0],
        nonce,
        Body::Create {
            text: format!("message {nonce}"),
            reply_to,
        },
        reply_to.into_iter().collect(),
    )
}

#[test]
fn typed_chat_long_edit_chain_projects_with_linear_edge_work() {
    let f = Fixture::standard();
    let (create, witness) = original(&f, 0, None);
    let mut ledger = f.ledger();
    admit(&mut ledger, &create, &witness);
    let mut parent = create.id();
    let mut earlier_ancestor = false;
    let mut later_ancestor = false;
    const EDITS: usize = 96;
    for nonce in 1..=EDITS {
        let (edit, witness) = f.action(
            &f.subjects[0],
            nonce as u64,
            Body::Edit {
                create: create.id(),
                text: format!("edit {nonce}"),
            },
            vec![parent],
        );
        if nonce > 1 {
            earlier_ancestor |= parent < edit.id();
            later_ancestor |= parent > edit.id();
        }
        parent = edit.id();
        admit(&mut ledger, &edit, &witness);
    }
    drain(&mut ledger);
    assert!(
        earlier_ancestor && later_ancestor,
        "real selected-ancestor edges exercise both CID traversal orders"
    );
    // A scan plus each root's parent edges and each ancestor expansion once. The former
    // per-root visited set needs quadratic work here and must fail this explicit allowance.
    let projected = ledger
        .projection(create.id(), 5 * EDITS + 32)
        .unwrap()
        .unwrap();
    assert_eq!(projected.visible_text.as_deref(), Some("edit 96"));
    assert_eq!(
        projected
            .alternatives
            .iter()
            .map(|edit| edit.action)
            .collect::<Vec<_>>(),
        vec![parent]
    );
    assert_eq!(
        ledger.causal_heads(create.id(), 3 * EDITS + 32).unwrap(),
        vec![parent]
    );
    assert_eq!(ledger.projection(create.id(), 0), Err(ChatError::WorkLimit));
}

#[test]
fn typed_chat_unrelated_messages_and_reply_creates_spend_no_target_scan_work() {
    let f = Fixture::standard();
    let (create, witness) = original(&f, 0, None);
    let mut ledger = f.ledger();
    admit(&mut ledger, &create, &witness);
    for nonce in 1..=96 {
        let (other, witness) = original(&f, nonce, (nonce % 2 == 0).then_some(create.id()));
        admit(&mut ledger, &other, &witness);
    }
    drain(&mut ledger);
    assert_eq!(
        ledger.message_action_ids(create.id()).collect::<Vec<_>>(),
        vec![create.id()]
    );
    assert_eq!(
        ledger
            .projection(create.id(), 1)
            .unwrap()
            .unwrap()
            .visible_text
            .as_deref(),
        Some("message 0")
    );
    assert_eq!(
        ledger.causal_heads(create.id(), 1).unwrap(),
        vec![create.id()]
    );
}

#[test]
fn typed_chat_reverse_import_and_raw_restore_rebuild_identical_message_candidates() {
    let f = Fixture::standard();
    let mut actions = vec![original(&f, 0, None)];
    let create = actions[0].0.id();
    let mut parent = create;
    for nonce in 1..=12 {
        let action = f.action(
            &f.subjects[0],
            nonce,
            Body::Edit {
                create,
                text: format!("edit {nonce}"),
            },
            vec![parent],
        );
        parent = action.0.id();
        actions.push(action);
    }
    actions.push(original(&f, 13, Some(create)));
    let mut forward = f.ledger();
    let mut reverse = f.ledger();
    for (action, witness) in &actions {
        admit(&mut forward, action, witness);
    }
    for (action, witness) in actions.iter().rev() {
        admit(&mut reverse, action, witness);
    }
    drain(&mut forward);
    drain(&mut reverse);
    let bytes = forward.snapshot().unwrap().encode();
    assert_eq!(reverse.snapshot().unwrap().encode(), bytes);
    let witnesses: Vec<_> = actions
        .iter()
        .map(|(action, witness)| (action.id(), witness.clone()))
        .collect();
    let mut restored = ChatLedger::restore(&bytes, limits(), &witnesses).unwrap();
    drain(&mut restored);
    for ledger in [&forward, &reverse, &restored] {
        assert_eq!(ledger.message_action_ids(create).count(), 13);
        assert_eq!(
            ledger
                .message_action_ids(actions[13].0.id())
                .collect::<Vec<_>>(),
            vec![actions[13].0.id()]
        );
        assert_eq!(
            ledger.projection(create, 100).unwrap(),
            forward.projection(create, 100).unwrap()
        );
        assert_eq!(ledger.causal_heads(create, 100).unwrap(), vec![parent]);
    }
    assert_eq!(
        restored.snapshot().unwrap().encode(),
        bytes,
        "derived index has no persistence encoding"
    );
}

#[test]
fn typed_chat_index_retains_pending_and_invalid_candidates_but_not_refused_or_duplicate_inputs() {
    let f = Fixture::standard();
    let (create, witness) = original(&f, 0, None);
    let mut ledger = ChatLedger::new(
        f.realm.clone(),
        LedgerLimits {
            max_actions: 1,
            max_action_bytes: 4096,
        },
    )
    .unwrap();
    let mut bad = create.encode();
    *bad.last_mut().unwrap() ^= 1;
    let bad = SignedAction::decode(&bad).unwrap();
    assert!(!bad.verify());
    assert_eq!(ledger.import(bad), Err(ChatError::Signature));
    assert_eq!(ledger.message_action_ids(create.id()).count(), 0);
    admit(&mut ledger, &create, &witness);
    assert_eq!(
        ledger.import(create.clone()),
        Ok(ImportOutcome::Duplicate(create.id()))
    );
    assert_eq!(ledger.message_action_ids(create.id()).count(), 1);
    let (refused, _) = original(&f, 1, None);
    assert_eq!(ledger.import(refused.clone()), Err(ChatError::Capacity));
    assert_eq!(ledger.message_action_ids(refused.id()).count(), 0);

    // The index supplies candidates, never authority or validity. A real signed cross-author
    // edit remains retained for audit, and a real signed child can precede its missing Create.
    let mut ledger = f.ledger();
    admit(&mut ledger, &create, &witness);
    let (invalid, invalid_witness) = f.action(
        &f.subjects[1],
        2,
        Body::Edit {
            create: create.id(),
            text: "not my message".into(),
        },
        vec![create.id()],
    );
    admit(&mut ledger, &invalid, &invalid_witness);
    let missing = ActionId::from_bytes([99; 32]);
    let (pending, pending_witness) = f.action(
        &f.subjects[0],
        3,
        Body::Edit {
            create: missing,
            text: "late original".into(),
        },
        vec![missing],
    );
    admit(&mut ledger, &pending, &pending_witness);
    drain(&mut ledger);
    assert_eq!(
        ledger.state(invalid.id()),
        Some(ActionState::Invalid(ChatError::Permission))
    );
    assert_eq!(
        ledger.state(pending.id()),
        Some(ActionState::Pending(PendingReason::Dependencies))
    );
    assert_eq!(ledger.message_action_ids(create.id()).count(), 2);
    assert_eq!(
        ledger.message_action_ids(missing).collect::<Vec<_>>(),
        vec![pending.id()]
    );
    assert_eq!(
        ledger
            .projection(create.id(), 2)
            .unwrap()
            .unwrap()
            .visible_text
            .as_deref(),
        Some("message 0")
    );
}

#[test]
fn typed_chat_shared_join_diamond_and_selected_ancestors_keep_exact_causal_winners() {
    let f = Fixture::standard();
    let (create, witness) = original(&f, 0, None);
    let mut actions = vec![(create.clone(), witness)];
    let (left, left_w) = f.action(
        &f.subjects[0],
        1,
        Body::Edit {
            create: create.id(),
            text: "left".into(),
        },
        vec![create.id()],
    );
    let (right, right_w) = f.action(
        &f.subjects[0],
        2,
        Body::Edit {
            create: create.id(),
            text: "right".into(),
        },
        vec![create.id()],
    );
    let (join, join_w) = f.action(
        &f.subjects[0],
        3,
        Body::Join {
            create: create.id(),
        },
        vec![left.id(), right.id()],
    );
    let (top1, top1_w) = f.action(
        &f.subjects[0],
        4,
        Body::Edit {
            create: create.id(),
            text: "top one".into(),
        },
        vec![join.id()],
    );
    let (top2, top2_w) = f.action(
        &f.subjects[0],
        5,
        Body::Edit {
            create: create.id(),
            text: "top two".into(),
        },
        vec![join.id()],
    );
    actions.extend([
        (left, left_w),
        (right, right_w),
        (join, join_w),
        (top1.clone(), top1_w),
        (top2.clone(), top2_w),
    ]);
    let expected: BTreeSet<_> = [top1.id(), top2.id()].into_iter().collect();
    for reverse in [false, true] {
        let mut ledger = f.ledger();
        for offset in 0..actions.len() {
            let index = if reverse {
                actions.len() - 1 - offset
            } else {
                offset
            };
            admit(&mut ledger, &actions[index].0, &actions[index].1);
        }
        drain(&mut ledger);
        let projected = ledger.projection(create.id(), 100).unwrap().unwrap();
        assert_eq!(
            projected
                .alternatives
                .iter()
                .map(|edit| edit.action)
                .collect::<BTreeSet<_>>(),
            expected
        );
        assert_eq!(
            ledger.causal_heads(create.id(), 100).unwrap(),
            expected.iter().copied().collect::<Vec<_>>()
        );
    }
}
