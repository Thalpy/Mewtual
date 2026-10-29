//! Exact positive moderation decisions are separate from reusable member/grant authority.
//! These fixtures verify real Ed25519 signatures. No boolean or decoded approval flag is used.
use super::*;
use crate::chat::authority;

fn pair(
    f: &Fixture,
) -> (
    SignedAction,
    VerifiedAuthorityBasis,
    SignedAction,
    VerifiedAuthorityBasis,
) {
    let (create, cw) = f.action(
        &key(1),
        1,
        Body::Create {
            text: "retained original".into(),
            reply_to: None,
        },
        vec![],
    );
    let (delete, dw) = f.action(
        &key(2),
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    (create, cw, delete, dw)
}
fn approved_by(issuer_seed: u8, action: &SignedAction) -> VerifiedDeleteApproval {
    let issuer = key(issuer_seed);
    let pinned = issuer.verifying_key().to_bytes();
    let bytes = authority::fixture_delete_approval_bytes(&pinned, action).unwrap();
    authority::verified_delete_fixture(&pinned, action, issuer.sign(&bytes)).unwrap()
}

#[test]
fn exact_moderation_approval_is_pending_until_supplied_then_wakes_dependents() {
    let f = Fixture::standard();
    let (create, cw, delete, dw) = pair(&f);
    let (child, childw) = f.action(
        &key(2),
        3,
        Body::Join {
            create: create.id(),
        },
        vec![delete.id()],
    );
    let mut ledger = f.ledger();
    assert_eq!(
        ledger.approve_delete(delete.id(), f.approval(&delete)),
        Err(ChatError::Unknown)
    );
    assert_eq!(
        ledger.action_ids().count(),
        0,
        "unknown approvals cannot create a separate proof cache"
    );
    for (action, witness) in [(&delete, &dw), (&child, &childw), (&create, &cw)] {
        admit(&mut ledger, action, witness);
    }
    drain(&mut ledger);
    assert_eq!(
        ledger.state(delete.id()),
        Some(ActionState::Pending(PendingReason::ModerationApproval))
    );
    assert_eq!(
        ledger.state(child.id()),
        Some(ActionState::Pending(PendingReason::Dependencies))
    );
    assert_eq!(
        ledger
            .projection(create.id(), 1_000)
            .unwrap()
            .unwrap()
            .visible_text
            .as_deref(),
        Some("retained original")
    );
    ledger
        .approve_delete(delete.id(), f.approval(&delete))
        .unwrap();
    drain(&mut ledger);
    assert_eq!(ledger.state(delete.id()), Some(ActionState::Valid));
    assert_eq!(ledger.state(child.id()), Some(ActionState::Valid));
    let view = ledger.projection(create.id(), 1_000).unwrap().unwrap();
    assert!(view.visible_text.is_none());
    assert_eq!(view.deleted_by, vec![delete.id()]);
}

#[test]
fn exact_moderation_approval_and_member_authority_arrive_in_either_order_with_missing_create() {
    let f = Fixture::standard();
    let (create, cw, delete, dw) = pair(&f);
    let mut reference = None;
    for approval_first in [false, true] {
        for create_first in [false, true] {
            let mut ledger = f.ledger();
            ledger.import(delete.clone()).unwrap();
            if create_first {
                admit(&mut ledger, &create, &cw);
                drain(&mut ledger);
            }
            if approval_first {
                ledger
                    .approve_delete(delete.id(), f.approval(&delete))
                    .unwrap();
                drain(&mut ledger);
                assert_eq!(
                    ledger.state(delete.id()),
                    Some(ActionState::Pending(PendingReason::Authority)),
                    "specific approval alone is not a member/grant witness"
                );
                ledger.authorize(delete.id(), dw.clone()).unwrap();
            } else {
                ledger.authorize(delete.id(), dw.clone()).unwrap();
                drain(&mut ledger);
                assert_eq!(
                    ledger.state(delete.id()),
                    Some(ActionState::Pending(if create_first {
                        PendingReason::ModerationApproval
                    } else {
                        PendingReason::Dependencies
                    }))
                );
                ledger
                    .approve_delete(delete.id(), f.approval(&delete))
                    .unwrap();
            }
            drain(&mut ledger);
            if !create_first {
                assert_eq!(
                    ledger.state(delete.id()),
                    Some(ActionState::Pending(PendingReason::Dependencies))
                );
                admit(&mut ledger, &create, &cw);
                drain(&mut ledger);
            }
            assert_eq!(ledger.state(delete.id()), Some(ActionState::Valid));
            let view = ledger.projection(create.id(), 1_000).unwrap().unwrap();
            assert!(view.visible_text.is_none());
            let evidence = ledger.snapshot().unwrap().encode();
            if let Some((expected_view, expected_bytes)) = &reference {
                assert_eq!(&view, expected_view);
                assert_eq!(&evidence, expected_bytes);
            } else {
                reference = Some((view, evidence));
            }
        }
    }
}

#[test]
fn exact_moderation_approval_does_not_upgrade_grant_and_own_bare_delete_any_stays_valid() {
    let f = Fixture::standard();
    let (create, cw, _, _) = pair(&f);
    let (own, ow) = f.action(
        &key(2),
        4,
        Body::Create {
            text: "same full signer".into(),
            reply_to: None,
        },
        vec![],
    );
    let (own_delete, own_dw) = f.action_with(
        &key(2),
        5,
        Body::Delete { create: own.id() },
        vec![own.id()],
        Rights::DELETE_ANY,
        f.basis,
        &[],
        0,
    );
    let (cross, crossw) = f.action_with(
        &key(2),
        6,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
        Rights::DELETE_ANY,
        f.basis,
        &[],
        0,
    );
    let (no_base, no_basew) = f.action_with(
        &key(2),
        7,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
        Rights::CREATE,
        f.basis,
        &[],
        0,
    );
    let mut ledger = f.ledger();
    for (a, w) in [
        (&create, &cw),
        (&own, &ow),
        (&own_delete, &own_dw),
        (&cross, &crossw),
        (&no_base, &no_basew),
    ] {
        admit(&mut ledger, a, w);
    }
    drain(&mut ledger);
    assert_eq!(
        ledger.state(own_delete.id()),
        Some(ActionState::Valid),
        "existing same-full-signer DELETE_ANY behavior is preserved"
    );
    assert_eq!(
        ledger.state(cross.id()),
        Some(ActionState::Pending(PendingReason::ModerationApproval)),
        "bare reusable DELETE_ANY cannot moderate another signer"
    );
    for action in [&cross, &no_base] {
        ledger
            .approve_delete(action.id(), f.approval(action))
            .unwrap();
    }
    drain(&mut ledger);
    for action in [&cross, &no_base] {
        assert_eq!(
            ledger.state(action.id()),
            Some(ActionState::Invalid(ChatError::Permission)),
            "approval cannot manufacture ordinary SELF_DELETE/member authority"
        );
    }
    assert!(ledger
        .projection(own.id(), 1_000)
        .unwrap()
        .unwrap()
        .visible_text
        .is_none());
    assert!(ledger
        .projection(create.id(), 1_000)
        .unwrap()
        .unwrap()
        .visible_text
        .is_some());
}

#[test]
fn exact_moderation_bindings_reject_copied_proofs_without_poisoning_or_resurrecting() {
    let f = Fixture::standard();
    let (create, cw, delete, dw) = pair(&f);
    let good = f.approval(&delete);
    let mut ledger = f.ledger();
    admit(&mut ledger, &create, &cw);
    admit(&mut ledger, &delete, &dw);
    drain(&mut ledger);
    let mut wrong = Vec::new();
    for field in 0..8 {
        let mut spec = delete.spec().clone();
        let mut signer = key(2);
        match field {
            0 => spec.realm = Realm::new(b"another-approved-realm", spec.realm.channel()).unwrap(),
            1 => {
                spec.body = Body::Delete {
                    create: ActionId::from_bytes([41; 32]),
                }
            }
            2 => signer = key(3),
            3 => spec.basis = BasisId::from_bytes([42; 32]),
            4 => spec.grant = GrantId::from_bytes([43; 32]),
            5 => spec.nonce = [44; 16],
            6 => spec.presented_ms = spec.presented_ms.saturating_add(1),
            7 => {
                spec.parents.push(ActionId::from_bytes([46; 32]));
                spec.parents.sort();
                spec.parents.dedup();
            }
            _ => unreachable!(),
        }
        let action = SignedAction::sign(&signer, spec).unwrap();
        assert_ne!(action.id(), delete.id());
        wrong.push(f.approval(&action));
    }
    let before = ledger.snapshot().unwrap().encode();
    for proof in &wrong {
        assert_eq!(
            ledger.approve_delete(delete.id(), proof.clone()),
            Err(ChatError::Authority)
        );
        assert_eq!(
            ledger.state(delete.id()),
            Some(ActionState::Pending(PendingReason::ModerationApproval))
        );
        assert_eq!(ledger.snapshot().unwrap().encode(), before);
    }
    ledger.approve_delete(delete.id(), good).unwrap();
    drain(&mut ledger);
    let accepted = ledger.projection(create.id(), 1_000).unwrap().unwrap();
    assert!(accepted.visible_text.is_none());
    for proof in wrong {
        assert_eq!(
            ledger.approve_delete(delete.id(), proof),
            Err(ChatError::Authority)
        );
    }
    drain(&mut ledger);
    assert_eq!(ledger.state(delete.id()), Some(ActionState::Valid));
    assert_eq!(
        ledger.projection(create.id(), 1_000).unwrap().unwrap(),
        accepted
    );
    let mut spec = delete.spec().clone();
    spec.body = Body::Edit {
        create: create.id(),
        text: "cannot borrow delete approval".into(),
    };
    let edit = SignedAction::sign(&key(2), spec).unwrap();
    ledger.import(edit.clone()).unwrap();
    assert_eq!(
        ledger.approve_delete(edit.id(), f.approval(&delete)),
        Err(ChatError::Authority)
    );
}

#[test]
fn exact_moderation_bad_signatures_and_pins_cannot_poison_equivalent_positive_facts() {
    let f = Fixture::standard();
    let (create, cw, delete, dw) = pair(&f);
    let issuer = key(240);
    let pinned = issuer.verifying_key().to_bytes();
    let statement = authority::fixture_delete_approval_bytes(&pinned, &delete).unwrap();
    let signature = issuer.sign(&statement);
    let mut bad = signature;
    bad[0] ^= 1;
    let mut ledger = f.ledger();
    admit(&mut ledger, &create, &cw);
    admit(&mut ledger, &delete, &dw);
    drain(&mut ledger);
    assert_eq!(
        authority::verified_delete_fixture(&pinned, &delete, bad),
        Err(ChatError::Signature)
    );
    assert_eq!(
        authority::verified_delete_fixture(
            &key(241).verifying_key().to_bytes(),
            &delete,
            signature
        ),
        Err(ChatError::Signature)
    );
    let first = authority::verified_delete_fixture(&pinned, &delete, signature).unwrap();
    let second = approved_by(241, &delete);
    assert_eq!(first,second,"normalized positive facts do not retain proof/issuer representation after external verification");
    ledger.approve_delete(delete.id(), first).unwrap();
    drain(&mut ledger);
    let before = ledger.snapshot().unwrap().encode();
    let bytes = ledger.retained_action_bytes();
    for _ in 0..1000 {
        ledger.approve_delete(delete.id(), second.clone()).unwrap();
    }
    assert!(!ledger.has_pending_work());
    assert_eq!(ledger.retained_action_bytes(), bytes);
    assert_eq!(ledger.snapshot().unwrap().encode(), before);
    assert_eq!(
        authority::verified_delete_fixture(&pinned, &delete, bad),
        Err(ChatError::Signature)
    );
    assert_eq!(ledger.state(delete.id()), Some(ActionState::Valid));
    assert!(ledger
        .projection(create.id(), 1_000)
        .unwrap()
        .unwrap()
        .visible_text
        .is_none());
}

#[test]
fn exact_moderation_snapshot_retains_only_original_actions_and_restore_requires_resupply() {
    let f = Fixture::standard();
    let (create, cw, delete, dw) = pair(&f);
    let mut ledger = f.ledger();
    admit(&mut ledger, &create, &cw);
    admit(&mut ledger, &delete, &dw);
    drain(&mut ledger);
    let unapproved = ledger.snapshot().unwrap().encode();
    ledger
        .approve_delete(delete.id(), f.approval(&delete))
        .unwrap();
    drain(&mut ledger);
    let approved = ledger.snapshot().unwrap().encode();
    assert_eq!(
        approved, unapproved,
        "approval/status/rights bits never enter the immutable signed-action snapshot"
    );
    let witnesses = [(create.id(), cw.clone()), (delete.id(), dw.clone())];
    let mut restored = ChatLedger::restore(&approved, limits(), &witnesses).unwrap();
    drain(&mut restored);
    assert_eq!(
        restored.state(delete.id()),
        Some(ActionState::Pending(PendingReason::ModerationApproval))
    );
    assert!(restored
        .projection(create.id(), 1_000)
        .unwrap()
        .unwrap()
        .visible_text
        .is_some());
    assert_eq!(
        restored.action(delete.id()).unwrap().encode(),
        delete.encode()
    );
    restored
        .approve_delete(delete.id(), f.approval(&delete))
        .unwrap();
    drain(&mut restored);
    assert_eq!(
        restored.projection(create.id(), 1_000).unwrap(),
        ledger.projection(create.id(), 1_000).unwrap()
    );
    let mut again = ChatLedger::restore(&approved, limits(), &[]).unwrap();
    again
        .approve_delete(delete.id(), f.approval(&delete))
        .unwrap();
    drain(&mut again);
    assert_eq!(
        again.state(delete.id()),
        Some(ActionState::Pending(PendingReason::Authority))
    );
    for (id, witness) in witnesses {
        again.authorize(id, witness).unwrap();
    }
    drain(&mut again);
    assert_eq!(
        again.projection(create.id(), 1_000).unwrap(),
        ledger.projection(create.id(), 1_000).unwrap()
    );
}

#[test]
fn exact_moderation_proofs_do_not_create_unknown_or_over_capacity_records() {
    let f = Fixture::standard();
    let (create, _, delete, dw) = pair(&f);
    let mut ledger = ChatLedger::new(
        f.realm.clone(),
        LedgerLimits {
            max_actions: 1,
            max_action_bytes: MAX_ACTION_BYTES,
        },
    )
    .unwrap();
    ledger.import(delete.clone()).unwrap();
    ledger.authorize(delete.id(), dw).unwrap();
    ledger
        .approve_delete(delete.id(), f.approval(&delete))
        .unwrap();
    drain(&mut ledger);
    assert_eq!(
        ledger.state(delete.id()),
        Some(ActionState::Pending(PendingReason::Dependencies))
    );
    assert_eq!(ledger.import(create), Err(ChatError::Capacity));
    let mut spec = delete.spec().clone();
    spec.nonce = [45; 16];
    let unknown = SignedAction::sign(&key(2), spec).unwrap();
    assert_eq!(
        ledger.approve_delete(unknown.id(), f.approval(&unknown)),
        Err(ChatError::Unknown)
    );
    assert_eq!(ledger.action_ids().collect::<Vec<_>>(), vec![delete.id()]);
}

#[test]
fn exact_moderation_real_alternate_action_signatures_share_one_approval() {
    fn unhex(text: &str) -> Vec<u8> {
        assert_eq!(text.len() % 2, 0);
        text.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("delete-signature-variants.json")).unwrap();
    let actions: Vec<_> = vectors["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| SignedAction::decode(&unhex(value.as_str().unwrap())).unwrap())
        .collect();
    assert_eq!(actions.len(), 2);
    for action in &actions {
        assert!(action.verify());
        assert_eq!(
            action.signer().as_slice(),
            unhex(vectors["public_key"].as_str().unwrap())
        );
    }
    assert_eq!(actions[0].id(), actions[1].id());
    assert_eq!(actions[0].semantic_bytes(), actions[1].semantic_bytes());
    assert_ne!(actions[0].signature(), actions[1].signature());
    assert_ne!(actions[0].envelope_hash(), actions[1].envelope_hash());
    let f = Fixture::new(actions[0].spec().realm.clone(), 1);
    let approval = f.approval(&actions[0]);
    assert_eq!(
        approval,
        f.approval(&actions[1]),
        "signature representation is not a distinct moderation decision"
    );
    let mut reference = None;
    for order in [[0, 1], [1, 0]] {
        let mut ledger = f.ledger();
        ledger.import(actions[order[0]].clone()).unwrap();
        ledger
            .approve_delete(actions[0].id(), approval.clone())
            .unwrap();
        ledger.import(actions[order[1]].clone()).unwrap();
        ledger
            .approve_delete(actions[1].id(), approval.clone())
            .unwrap();
        drain(&mut ledger);
        assert_eq!(ledger.action_ids().count(), 1);
        assert_eq!(
            ledger.state(actions[0].id()),
            Some(ActionState::Pending(PendingReason::Authority)),
            "specific approval and real signatures cannot manufacture member/grant authority"
        );
        let before = ledger.snapshot().unwrap().encode();
        let mut bad = actions[0].encode();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(
            ledger.import(SignedAction::decode(&bad).unwrap()),
            Err(ChatError::Signature)
        );
        assert_eq!(ledger.snapshot().unwrap().encode(), before);
        if let Some(expected) = &reference {
            assert_eq!(&before, expected);
        } else {
            reference = Some(before);
        }
    }
}
