use super::*;
mod acceptance;
mod fixtures;
mod moderation;
use fixtures::*;

#[test]
fn typed_chat_same_signer_forks_union_without_sequence_or_nonce_first_winner() {
    let f = Fixture::standard();
    let alice = key(1);
    let (a, aw) = f.action(
        &alice,
        0,
        Body::Create {
            text: "left".into(),
            reply_to: None,
        },
        vec![],
    );
    assert_eq!(f.witness(0, Rights::MEMBER), aw);
    let (b, bw) = f.action(
        &alice,
        0,
        Body::Create {
            text: "right".into(),
            reply_to: None,
        },
        vec![],
    );
    assert_ne!(a.id(), b.id());
    let mut left = f.ledger();
    let mut right = f.ledger();
    for (action, witness) in [(&a, &aw), (&b, &bw)] {
        admit(&mut left, action, witness);
    }
    for (action, witness) in [(&b, &bw), (&a, &aw)] {
        admit(&mut right, action, witness);
    }
    drain(&mut left);
    drain(&mut right);
    assert_eq!(
        left.action_ids().collect::<Vec<_>>(),
        right.action_ids().collect::<Vec<_>>()
    );
    assert_eq!(
        left.projection(a.id(), 1000).unwrap(),
        right.projection(a.id(), 1000).unwrap()
    );
    assert_eq!(
        left.projection(b.id(), 1000).unwrap(),
        right.projection(b.id(), 1000).unwrap()
    );
    assert!(matches!(
        left.import(a.clone()).unwrap(),
        ImportOutcome::Duplicate(_)
    ));
}

#[test]
fn typed_chat_causal_edit_wins_without_wall_clock_and_delete_never_resurrects() {
    let f = Fixture::standard();
    let alice = key(1);
    let (create, cw) = f.action(
        &alice,
        1,
        Body::Create {
            text: "original".into(),
            reply_to: None,
        },
        vec![],
    );
    let (a, aw) = f.action_with(
        &alice,
        2,
        Body::Edit {
            create: create.id(),
            text: "future-clock".into(),
        },
        vec![create.id()],
        Rights::MEMBER,
        f.basis,
        &[],
        u64::MAX,
    );
    let (b, bw) = f.action_with(
        &alice,
        2,
        Body::Edit {
            create: create.id(),
            text: "backwards-clock".into(),
        },
        vec![create.id()],
        Rights::MEMBER,
        f.basis,
        &[],
        0,
    );
    let (join, jw) = f.action(
        &alice,
        3,
        Body::Join {
            create: create.id(),
        },
        vec![a.id(), b.id()],
    );
    let (edit, ew) = f.action(
        &alice,
        4,
        Body::Edit {
            create: create.id(),
            text: "observed both".into(),
        },
        vec![join.id()],
    );
    let (delete, dw) = f.action(
        &alice,
        5,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    let mut ledger = f.ledger();
    for (action, witness) in [
        (&edit, &ew),
        (&b, &bw),
        (&a, &aw),
        (&join, &jw),
        (&create, &cw),
    ] {
        admit(&mut ledger, action, witness);
    }
    drain(&mut ledger);
    let projection = ledger.projection(create.id(), 10_000).unwrap().unwrap();
    assert_eq!(projection.visible_text.as_deref(), Some("observed both"));
    assert_eq!(projection.alternatives.len(), 1);
    admit(&mut ledger, &delete, &dw);
    drain(&mut ledger);
    assert!(ledger
        .projection(create.id(), 10_000)
        .unwrap()
        .unwrap()
        .visible_text
        .is_none());
    assert!(matches!(
        ledger.import(create.clone()).unwrap(),
        ImportOutcome::Duplicate(_)
    ));
    assert!(ledger
        .projection(create.id(), 10_000)
        .unwrap()
        .unwrap()
        .visible_text
        .is_none());
}

#[test]
fn typed_chat_cross_author_edit_refuses_and_delete_needs_exact_approval() {
    let f = Fixture::standard();
    let alice = key(1);
    let bob = key(2);
    let (create, cw) = f.action(
        &alice,
        1,
        Body::Create {
            text: "Alice".into(),
            reply_to: None,
        },
        vec![],
    );
    let (edit, ew) = f.action(
        &bob,
        2,
        Body::Edit {
            create: create.id(),
            text: "Bob pretending".into(),
        },
        vec![create.id()],
    );
    let (delete, dw) = f.action(
        &bob,
        3,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    let (moderate, mw) = f.action_with(
        &bob,
        4,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
        Rights::MODERATOR,
        f.basis,
        &[],
        1000,
    );
    let mut ledger = f.ledger();
    for (action, witness) in [(&create, &cw), (&edit, &ew), (&delete, &dw)] {
        admit(&mut ledger, action, witness);
    }
    drain(&mut ledger);
    assert_eq!(
        ledger.state(edit.id()),
        Some(ActionState::Invalid(ChatError::Permission))
    );
    assert_eq!(
        ledger.state(delete.id()),
        Some(ActionState::Pending(PendingReason::ModerationApproval))
    );
    assert_eq!(
        ledger.authorize(delete.id(), mw.clone()),
        Err(ChatError::Authority)
    );
    admit(&mut ledger, &moderate, &mw);
    drain(&mut ledger);
    assert_eq!(
        ledger.state(moderate.id()),
        Some(ActionState::Pending(PendingReason::ModerationApproval))
    );
    ledger
        .approve_delete(moderate.id(), f.approval(&moderate))
        .unwrap();
    drain(&mut ledger);
    assert_eq!(ledger.state(moderate.id()), Some(ActionState::Valid));
    assert!(ledger
        .projection(create.id(), 1000)
        .unwrap()
        .unwrap()
        .visible_text
        .is_none());
}

#[test]
fn typed_chat_snapshot_and_page_decode_never_restore_authority() {
    let f = Fixture::standard();
    let alice = key(1);
    let (create, cw) = f.action(
        &alice,
        1,
        Body::Create {
            text: "saved".into(),
            reply_to: None,
        },
        vec![],
    );
    let (delete, dw) = f.action(
        &alice,
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    let mut ledger = f.ledger();
    admit(&mut ledger, &delete, &dw);
    drain(&mut ledger);
    let pending_bytes = ledger.snapshot().unwrap().encode();
    let mut restored = ChatLedger::restore(&pending_bytes, limits(), &[]).unwrap();
    drain(&mut restored);
    assert_eq!(
        restored.state(delete.id()),
        Some(ActionState::Pending(PendingReason::Authority))
    );
    restored.authorize(delete.id(), dw.clone()).unwrap();
    admit(&mut restored, &create, &cw);
    drain(&mut restored);
    assert!(restored
        .projection(create.id(), 1000)
        .unwrap()
        .unwrap()
        .visible_text
        .is_none());
    let snapshot = restored.snapshot().unwrap();
    let first = snapshot.page(None, 1, MAX_PAGE_BYTES).unwrap();
    let second = snapshot.page(first.next(), 1, MAX_PAGE_BYTES).unwrap();
    let mut target = f.ledger();
    for page in [&first, &second] {
        let page = Page::decode(&page.encode()).unwrap();
        target.import_page(&page).unwrap();
    }
    drain(&mut target);
    assert!(target.action_ids().all(|id| matches!(
        target.state(id),
        Some(ActionState::Pending(PendingReason::Authority))
    )));
    target.authorize(create.id(), cw.clone()).unwrap();
    target.authorize(delete.id(), dw.clone()).unwrap();
    drain(&mut target);
    assert_eq!(
        target.projection(create.id(), 1000).unwrap(),
        restored.projection(create.id(), 1000).unwrap()
    );
    let bytes = target.snapshot().unwrap().encode();
    let mut again =
        ChatLedger::restore(&bytes, limits(), &[(create.id(), cw), (delete.id(), dw)]).unwrap();
    drain(&mut again);
    assert_eq!(
        again.projection(create.id(), 1000).unwrap(),
        target.projection(create.id(), 1000).unwrap()
    );
}

#[test]
fn typed_chat_scope_signature_and_authority_substitution_fail_without_new_records() {
    let f = Fixture::standard();
    let alice = key(1);
    let bob = key(2);
    let (create, cw) = f.action(
        &alice,
        1,
        Body::Create {
            text: "bound".into(),
            reply_to: None,
        },
        vec![],
    );
    let mut bytes = create.encode();
    *bytes.last_mut().unwrap() ^= 1;
    let bad = SignedAction::decode(&bytes).unwrap();
    let mut ledger = f.ledger();
    assert_eq!(ledger.import(bad), Err(ChatError::Signature));
    assert_eq!(ledger.action_ids().count(), 0);
    assert_eq!(
        ledger.authorize(create.id(), cw.clone()),
        Err(ChatError::Unknown)
    );
    ledger.import(create.clone()).unwrap();
    let wrong_subject = f.witness_with(&bob, Rights::MEMBER, f.basis, &[]);
    assert_eq!(
        ledger.authorize(create.id(), wrong_subject),
        Err(ChatError::Authority)
    );
    assert_eq!(
        ledger.state(create.id()),
        Some(ActionState::Pending(PendingReason::Authority))
    );
    let other = Realm::new(b"other-group", f.realm.channel()).unwrap();
    let mut other_ledger = ChatLedger::new(other, limits()).unwrap();
    assert_eq!(other_ledger.import(create), Err(ChatError::Scope));
    assert_eq!(other_ledger.action_ids().count(), 0);
}

#[test]
fn typed_chat_bounded_work_and_capacity_preserve_retained_evidence() {
    let f = Fixture::standard();
    let alice = key(1);
    let (create, cw) = f.action(
        &alice,
        1,
        Body::Create {
            text: "kept".into(),
            reply_to: None,
        },
        vec![],
    );
    let mut ledger = ChatLedger::new(
        f.realm.clone(),
        LedgerLimits {
            max_actions: 1,
            max_action_bytes: MAX_ACTION_BYTES,
        },
    )
    .unwrap();
    admit(&mut ledger, &create, &cw);
    assert_eq!(ledger.advance(0).work, 0);
    assert!(ledger.has_pending_work());
    drain(&mut ledger);
    let (other, _) = f.action(
        &alice,
        2,
        Body::Create {
            text: "refused".into(),
            reply_to: None,
        },
        vec![],
    );
    assert_eq!(ledger.import(other), Err(ChatError::Capacity));
    assert_eq!(ledger.action_ids().collect::<Vec<_>>(), [create.id()]);
    assert_eq!(ledger.projection(create.id(), 0), Err(ChatError::WorkLimit));
}

#[test]
fn typed_chat_more_than_parent_cap_heads_are_joined_without_forgetting_alternatives() {
    let f = Fixture::standard();
    let alice = key(1);
    let (create, cw) = f.action(
        &alice,
        1,
        Body::Create {
            text: "base".into(),
            reply_to: None,
        },
        vec![],
    );
    let mut ledger = f.ledger();
    admit(&mut ledger, &create, &cw);
    for index in 0..MAX_PARENTS + 7 {
        let (edit, ew) = f.action(
            &alice,
            index as u64 + 2,
            Body::Edit {
                create: create.id(),
                text: format!("branch{index}"),
            },
            vec![create.id()],
        );
        admit(&mut ledger, &edit, &ew);
    }
    drain(&mut ledger);
    let before = ledger.projection(create.id(), 100_000).unwrap().unwrap();
    assert_eq!(before.alternatives.len(), MAX_PARENTS + 7);
    let heads = ledger.causal_heads(create.id(), 100_000).unwrap();
    assert_eq!(
        heads.len(),
        MAX_PARENTS + 7,
        "frontier never truncates to a convenient first64"
    );
    let mut joins = Vec::new();
    for (index, chunk) in heads.chunks(MAX_PARENTS).enumerate() {
        let (join, jw) = f.action(
            &alice,
            1000 + index as u64,
            Body::Join {
                create: create.id(),
            },
            chunk.to_vec(),
        );
        joins.push(join.id());
        admit(&mut ledger, &join, &jw);
    }
    drain(&mut ledger);
    assert_eq!(
        ledger
            .projection(create.id(), 100_000)
            .unwrap()
            .unwrap()
            .alternatives,
        before.alternatives,
        "joining knowledge alone never edits text"
    );
    let (edit, ew) = f.action(
        &alice,
        2000,
        Body::Edit {
            create: create.id(),
            text: "observed every branch".into(),
        },
        joins,
    );
    admit(&mut ledger, &edit, &ew);
    drain(&mut ledger);
    let after = ledger.projection(create.id(), 100_000).unwrap().unwrap();
    assert_eq!(after.visible_text.as_deref(), Some("observed every branch"));
    assert_eq!(after.alternatives.len(), 1);
}

#[test]
fn typed_chat_fresh_same_key_basis_self_edit_does_not_inherit_privileged_rights() {
    let f = Fixture::standard();
    let alice = key(1);
    let bob = key(2);
    let (own, ow) = f.action(
        &alice,
        1,
        Body::Create {
            text: "same full signer".into(),
            reply_to: None,
        },
        vec![],
    );
    let (other, otherw) = f.action(
        &bob,
        2,
        Body::Create {
            text: "different signer".into(),
            reply_to: None,
        },
        vec![],
    );
    let next_basis = BasisId::from_bytes([8; 32]);
    let (edit, ew) = f.action_with(
        &alice,
        3,
        Body::Edit {
            create: own.id(),
            text: "fresh ordinary grant".into(),
        },
        vec![own.id()],
        Rights::MEMBER,
        next_basis,
        &[f.basis],
        0,
    );
    let (delete, dw) = f.action_with(
        &alice,
        4,
        Body::Delete { create: other.id() },
        vec![other.id()],
        Rights::MEMBER,
        next_basis,
        &[f.basis],
        0,
    );
    let mut ledger = f.ledger();
    for (action, witness) in [(&edit, &ew), (&delete, &dw), (&other, &otherw), (&own, &ow)] {
        admit(&mut ledger, action, witness);
    }
    drain(&mut ledger);
    assert_eq!(ledger.state(edit.id()), Some(ActionState::Valid));
    assert_eq!(
        ledger.state(delete.id()),
        Some(ActionState::Pending(PendingReason::ModerationApproval))
    );
    assert_eq!(
        ledger
            .projection(own.id(), 1000)
            .unwrap()
            .unwrap()
            .visible_text
            .as_deref(),
        Some("fresh ordinary grant")
    );
    assert_eq!(
        ledger
            .projection(other.id(), 1000)
            .unwrap()
            .unwrap()
            .visible_text
            .as_deref(),
        Some("different signer")
    );
}

#[test]
fn typed_chat_missing_causal_authority_is_pending_not_an_old_grant_upgrade() {
    let f = Fixture::standard();
    let alice = key(1);
    let moderator = key(2);
    let newer = BasisId::from_bytes([9; 32]);
    let (create, cw) = f.action_with(
        &alice,
        1,
        Body::Create {
            text: "new context".into(),
            reply_to: None,
        },
        vec![],
        Rights::MEMBER,
        newer,
        &[],
        0,
    );
    let (delete, dw) = f.action_with(
        &moderator,
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
        Rights::MODERATOR,
        f.basis,
        &[],
        0,
    );
    let mut ledger = f.ledger();
    admit(&mut ledger, &create, &cw);
    admit(&mut ledger, &delete, &dw);
    drain(&mut ledger);
    assert_eq!(
        ledger.state(delete.id()),
        Some(ActionState::Pending(PendingReason::CausalAuthority))
    );
    assert_eq!(
        ledger
            .projection(create.id(), 1000)
            .unwrap()
            .unwrap()
            .visible_text
            .as_deref(),
        Some("new context")
    );
    // This checkpoint has no production verifier that can decide revocation/old-context
    // moderation. Wire bytes cannot manufacture a coverage witness for a newer basis.
    let mut restored = ChatLedger::restore(
        &ledger.snapshot().unwrap().encode(),
        limits(),
        &[(create.id(), cw), (delete.id(), dw)],
    )
    .unwrap();
    drain(&mut restored);
    assert_eq!(
        restored.state(delete.id()),
        Some(ActionState::Pending(PendingReason::CausalAuthority))
    );
}

#[test]
fn typed_chat_300_external_bases_preserve_action_history_but_restore_needs_witnesses() {
    let f = Fixture::standard();
    let alice = key(1);
    let mut ledger = f.ledger();
    let mut witnesses = Vec::new();
    for index in 0..300u64 {
        let mut basis = [0; 32];
        basis[24..].copy_from_slice(&index.to_be_bytes());
        let (action, witness) = f.action_with(
            &alice,
            index,
            Body::Create {
                text: format!("context{index}"),
                reply_to: None,
            },
            vec![],
            Rights::MEMBER,
            BasisId::from_bytes(basis),
            &[],
            0,
        );
        admit(&mut ledger, &action, &witness);
        witnesses.push((action.id(), witness));
    }
    drain(&mut ledger);
    let snapshot = ledger.snapshot().unwrap().encode();
    let mut without = ChatLedger::restore(&snapshot, limits(), &[]).unwrap();
    drain(&mut without);
    assert_eq!(without.action_ids().count(), 300);
    assert!(without.action_ids().all(|id| matches!(
        without.state(id),
        Some(ActionState::Pending(PendingReason::Authority))
    )));
    let mut restored = ChatLedger::restore(&snapshot, limits(), &witnesses).unwrap();
    drain(&mut restored);
    assert_eq!(
        ledger.action_ids().collect::<Vec<_>>(),
        restored.action_ids().collect::<Vec<_>>()
    );
    assert!(restored
        .action_ids()
        .all(|id| restored.state(id) == Some(ActionState::Valid)));
    // These are signed externally-pinned fixture decisions, not300 actual MLS transitions
    // and not evidence that the unresolved native context archive is implemented.
}

#[test]
fn typed_chat_bad_page_tail_keeps_verified_prefix_for_exact_retry_and_cursor_binds_snapshot() {
    let f = Fixture::standard();
    let alice = key(1);
    let mut source = f.ledger();
    let mut originals = Vec::new();
    for index in 0..2 {
        let (action, witness) = f.action(
            &alice,
            index,
            Body::Create {
                text: format!("page{index}"),
                reply_to: None,
            },
            vec![],
        );
        admit(&mut source, &action, &witness);
        originals.push(action);
    }
    drain(&mut source);
    let snapshot = source.snapshot().unwrap();
    let page = snapshot.page(None, 2, MAX_PAGE_BYTES).unwrap();
    assert!(page.next().is_none());
    let mut bytes = page.encode();
    let last_signature_byte = bytes.len() - 2;
    bytes[last_signature_byte] ^= 1;
    let bad = Page::decode(&bytes).unwrap();
    let mut target = f.ledger();
    assert_eq!(target.import_page(&bad), Err(ChatError::Signature));
    assert_eq!(target.action_ids().count(), 1);
    assert_eq!(target.import_page(&page), Ok(1));
    assert_eq!(
        target.action_ids().collect::<Vec<_>>(),
        source.action_ids().collect::<Vec<_>>()
    );
    let first = snapshot.page(None, 1, MAX_PAGE_BYTES).unwrap();
    let (extra, extra_w) = f.action(
        &alice,
        3,
        Body::Create {
            text: "inventory changed".into(),
            reply_to: None,
        },
        vec![],
    );
    admit(&mut source, &extra, &extra_w);
    let changed = source.snapshot().unwrap();
    assert_eq!(
        changed.page(first.next(), 1, MAX_PAGE_BYTES).err(),
        Some(ChatError::Scope)
    );
}

#[test]
fn typed_chat_strict_codec_rejects_noncanonical_scope_parents_and_legacy_bytes() {
    let f = Fixture::standard();
    let alice = key(1);
    let (create, _) = f.action(
        &alice,
        0,
        Body::Create {
            text: "strict".into(),
            reply_to: None,
        },
        vec![],
    );
    assert_eq!(SignedAction::decode(&create.encode()).unwrap(), create);
    for length in 0..create.encode().len() {
        assert!(SignedAction::decode(&create.encode()[..length]).is_err());
    }
    let mut trailing = create.encode();
    trailing.push(0);
    assert!(SignedAction::decode(&trailing).is_err());
    let mut spec = create.spec().clone();
    spec.body = Body::Edit {
        create: create.id(),
        text: "x".into(),
    };
    spec.parents = vec![create.id(); MAX_PARENTS + 1];
    assert!(SignedAction::sign(&alice, spec.clone()).is_err());
    spec.parents = vec![create.id(), create.id()];
    assert!(SignedAction::sign(&alice, spec).is_err());
    let mut wrong_profile = create.encode();
    let offset = 4 + 4 + b"mewtual/chat/action/v1\0".len() + 4 + f.realm.group().len() + 16;
    wrong_profile[offset..offset + 2].copy_from_slice(&2u16.to_be_bytes());
    assert_eq!(
        SignedAction::decode(&wrong_profile).err(),
        Some(ChatError::Scope)
    );
    assert!(SignedAction::decode(b"catcoms/op/v1").is_err());
    assert_ne!(f.realm.id(), Realm::new(f.realm.group(), 42).unwrap().id());
    assert_ne!(
        f.realm.id(),
        Realm::new(b"another-group", f.realm.channel())
            .unwrap()
            .id()
    );
}
