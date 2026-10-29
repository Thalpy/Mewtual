//! Signed-history acceptance for independent activity bubbles. This exercises the
//! inactive ledger, not real networking or the still-absent native authority issuer.
//! Every fixture authority token comes from the private signed-grant verifier.

use super::super::*;
use super::fixtures::{admit, drain, limits, Fixture};
use std::collections::{BTreeMap, BTreeSet};

struct Record {
    action: SignedAction,
    witness: VerifiedAuthorityBasis,
    bubble: usize,
    unauthorized: bool,
}

struct ExpectedMessage {
    create: ActionId,
    author: [u8; 32],
    text: String,
    deleted: bool,
}

/// Same nonce and signer deliberately occur on different fork bodies. Rejoined
/// writers retain the same key but receive a distinct, externally verified grant.
fn corpus(f: &Fixture, bubbles: usize) -> (Vec<Record>, Vec<ExpectedMessage>) {
    let mut records = Vec::new();
    let mut messages = Vec::new();
    let next_basis = BasisId::from_bytes([9; 32]);
    for (index, subject) in f.subjects.iter().enumerate() {
        let bubble = index % bubbles;
        let (create, cw) = f.action(
            subject,
            1,
            Body::Create {
                text: format!("message from {index}"),
                reply_to: None,
            },
            vec![],
        );
        let target = create.id();
        let (left, lw) = f.action_with(
            subject,
            2,
            Body::Edit {
                create: target,
                text: format!("{index}: cloned writer left"),
            },
            vec![target],
            Rights::MEMBER,
            f.basis,
            &[],
            u64::MAX,
        );
        let (right, rw) = f.action_with(
            subject,
            2,
            Body::Edit {
                create: target,
                text: format!("{index}: cloned writer right"),
            },
            vec![target],
            Rights::MEMBER,
            f.basis,
            &[],
            0,
        );
        assert_ne!(left.id(), right.id(), "a nonce is not a first-arrival slot");

        let basis = if index % 2 == 0 { f.basis } else { next_basis };
        let covered = if basis == f.basis {
            vec![]
        } else {
            vec![f.basis]
        };
        let text = format!("{index}: reconciled both branches");
        let (resolved, ew) = f.action_with(
            subject,
            3,
            Body::Edit {
                create: target,
                text: text.clone(),
            },
            vec![left.id(), right.id()],
            Rights::MEMBER,
            basis,
            &covered,
            1,
        );

        // Another genuinely admitted member still cannot edit this author's row.
        // Its extreme timestamp must not grant authority or select the display.
        let foreign = &f.subjects[(index + 1) % f.subjects.len()];
        let (forged, fw) = f.action_with(
            foreign,
            1000 + index as u64,
            Body::Edit {
                create: target,
                text: "cross-author substitution".into(),
            },
            vec![target],
            Rights::MEMBER,
            f.basis,
            &[],
            u64::MAX,
        );
        records.push(Record {
            action: forged,
            witness: fw,
            bubble,
            unauthorized: true,
        });

        let deleted = index % 7 == 0;
        if deleted {
            let (delete, dw) = f.action_with(
                subject,
                4,
                Body::Delete { create: target },
                vec![resolved.id()],
                Rights::MEMBER,
                basis,
                &covered,
                0,
            );
            records.push(Record {
                action: delete,
                witness: dw,
                bubble,
                unauthorized: false,
            });
        }
        // Two Creates from a cloned writer can reuse its original nonce too.
        // They are separate messages, not competing occupants of an actor sequence.
        if index % 9 == 0 {
            let fork_text = format!("{index}: independent cloned Create");
            let (fork, witness) = f.action(
                subject,
                1,
                Body::Create {
                    text: fork_text.clone(),
                    reply_to: None,
                },
                vec![],
            );
            assert_ne!(fork.id(), target);
            messages.push(ExpectedMessage {
                create: fork.id(),
                author: *fork.signer(),
                text: fork_text,
                deleted: false,
            });
            records.push(Record {
                action: fork,
                witness,
                bubble,
                unauthorized: false,
            });
        }
        messages.push(ExpectedMessage {
            create: target,
            author: *create.signer(),
            text,
            deleted,
        });
        for (action, witness) in [(resolved, ew), (right, rw), (left, lw), (create, cw)] {
            records.push(Record {
                action,
                witness,
                bubble,
                unauthorized: false,
            });
        }
    }
    (records, messages)
}

fn assert_complete(ledger: &ChatLedger, records: &[Record], messages: &[ExpectedMessage]) {
    let expected: BTreeSet<_> = records.iter().map(|record| record.action.id()).collect();
    assert_eq!(ledger.action_ids().collect::<BTreeSet<_>>(), expected);
    for record in records {
        let expected = if record.unauthorized {
            ActionState::Invalid(ChatError::Permission)
        } else {
            ActionState::Valid
        };
        assert_eq!(ledger.state(record.action.id()), Some(expected));
        assert_eq!(
            ledger.action(record.action.id()).unwrap().encode(),
            record.action.encode(),
            "the original signed envelope survives forks, paging and restart"
        );
    }
    for message in messages {
        let view = ledger.projection(message.create, 100_000).unwrap().unwrap();
        assert_eq!(view.author, message.author);
        assert_eq!(
            view.visible_text.as_deref(),
            (!message.deleted).then_some(message.text.as_str())
        );
        assert_eq!(view.deleted_by.len(), usize::from(message.deleted));
        if !view.alternatives.is_empty() {
            assert_eq!(
                view.alternatives.len(),
                1,
                "the causal successor observes both fork edits"
            );
            assert_eq!(view.alternatives[0].text, message.text);
        }
    }
}

#[test]
fn typed_chat_twenty_and_fifty_writers_converge_after_partition_restart_and_reordered_pages() {
    // Five independent bubbles each retain only their own writers before repair.
    // The fixture models signed data arrival; it does not claim 50 real TCP peers.
    for writers in [20, 50] {
        const BUBBLES: usize = 5;
        let f = Fixture::new(
            Realm::new(b"partitioned-history", writers as u128).unwrap(),
            writers,
        );
        let (records, messages) = corpus(&f, BUBBLES);
        let witnesses: Vec<_> = records
            .iter()
            .map(|record| (record.action.id(), record.witness.clone()))
            .collect();
        let mut replicas = Vec::new();
        let mut wire_pages = Vec::new();

        for bubble in 0..BUBBLES {
            let mut local = f.ledger();
            let own: Vec<_> = records
                .iter()
                .filter(|record| record.bubble == bubble)
                .collect();
            for (position, record) in own.iter().rev().enumerate() {
                local.import(record.action.clone()).unwrap();
                // Some evidence is durably retained before its authority arrives.
                if position % 2 == 0 {
                    local
                        .authorize(record.action.id(), record.witness.clone())
                        .unwrap();
                }
                assert!(local.advance(1).work <= 1);
            }
            drain(&mut local);
            let saved = local.snapshot().unwrap();
            let bytes = saved.encode();
            let mut reopened = ChatLedger::restore(&bytes, limits(), &[]).unwrap();
            drain(&mut reopened);
            assert_eq!(reopened.action_ids().count(), own.len());
            assert!(
                reopened.action_ids().all(|id| matches!(
                    reopened.state(id),
                    Some(ActionState::Pending(PendingReason::Authority))
                )),
                "serialized status must never manufacture an authority witness"
            );

            let mut cursor = None;
            loop {
                let page = saved.page(cursor, 7, 8 * 1024).unwrap();
                cursor = page.next();
                wire_pages.push(page.encode());
                if cursor.is_none() {
                    break;
                }
            }
            replicas.push(reopened);
        }

        let by_id: BTreeMap<_, _> = records
            .iter()
            .map(|record| (record.action.id(), record))
            .collect();
        for (index, replica) in replicas.iter_mut().enumerate() {
            let mut order: Vec<_> = (0..wire_pages.len()).collect();
            // Fixed, different delivery permutations without ambient randomness.
            order.sort_by_key(|position| {
                let mut h = blake3::Hasher::new();
                h.update(&(index as u64).to_be_bytes());
                h.update(&(*position as u64).to_be_bytes());
                *h.finalize().as_bytes()
            });
            for position in order {
                let page = Page::decode(&wire_pages[position]).unwrap();
                replica.import_page(&page).unwrap();
                assert_eq!(
                    replica.import_page(&page).unwrap(),
                    0,
                    "lost replies replay exact pages"
                );
                for action in page.actions() {
                    // Simulate delayed grant availability as well as delayed parents.
                    if action.id().as_bytes()[0] % 2 == 0 {
                        replica
                            .authorize(action.id(), by_id[&action.id()].witness.clone())
                            .unwrap();
                    }
                }
                assert!(replica.advance(1).work <= 1);
            }
            // Restart again while some records still lack permission evidence.
            let saved = replica.snapshot().unwrap().encode();
            *replica = ChatLedger::restore(&saved, limits(), &witnesses).unwrap();
            drain(replica);
            assert_complete(replica, &records, &messages);
        }

        let canonical = replicas[0].snapshot().unwrap().encode();
        for replica in &replicas[1..] {
            assert_eq!(
                replica.snapshot().unwrap().encode(),
                canonical,
                "all signed bodies, including rejected foreign edits, converge additively"
            );
            for message in &messages {
                assert_eq!(
                    replica.projection(message.create, 100_000).unwrap(),
                    replicas[0].projection(message.create, 100_000).unwrap()
                );
            }
        }
    }
}

#[test]
fn typed_chat_edit_projection_does_not_walk_unrelated_replied_message_history() {
    let f = Fixture::new(Realm::new(b"long-reply-thread", 1).unwrap(), 1);
    let subject = &f.subjects[0];
    let mut ledger = f.ledger();
    let mut previous = None;
    let mut history = Vec::new();
    for index in 0..80 {
        let (action, witness) = f.action(
            subject,
            index,
            Body::Create {
                text: format!("reply {index}"),
                reply_to: previous,
            },
            previous.into_iter().collect(),
        );
        previous = Some(action.id());
        history.push((action, witness));
    }
    let create = previous.unwrap();
    let (edit, witness) = f.action(
        subject,
        100,
        Body::Edit {
            create,
            text: "updated latest reply".into(),
        },
        vec![create],
    );
    history.push((edit, witness));
    for (action, witness) in history.into_iter().rev() {
        let id = action.id();
        ledger.import(action).unwrap();
        ledger.authorize(id, witness).unwrap();
    }
    drain(&mut ledger);

    // Inventory scanning is charged separately. Once we reach this message's
    // Create, earlier replied-to messages cannot contain an edit of that Create.
    // Their thread length must not consume this message's causal edit allowance.
    let budget = ledger.action_ids().count() + 8;
    let view = ledger.projection(create, budget).unwrap().unwrap();
    assert_eq!(view.visible_text.as_deref(), Some("updated latest reply"));
    assert_eq!(view.alternatives.len(), 1);
}

#[test]
fn typed_chat_restore_keeps_per_action_proofs_across_many_old_bases() {
    let f = Fixture::new(Realm::new(b"many-old-authority-bases", 1).unwrap(), 1);
    let subject = &f.subjects[0];
    let current = BasisId::from_bytes([240; 32]);
    let mut live = f.ledger();
    let mut witnesses = Vec::new();
    let mut targets = Vec::new();
    for index in 0..70u8 {
        let older = BasisId::from_bytes([index; 32]);
        let (create, cw) = f.action_with(
            subject,
            u64::from(index),
            Body::Create {
                text: format!("old context {index}"),
                reply_to: None,
            },
            vec![],
            Rights::MEMBER,
            older,
            &[],
            1000,
        );
        let (edit, ew) = f.action_with(
            subject,
            1000 + u64::from(index),
            Body::Edit {
                create: create.id(),
                text: format!("current edit {index}"),
            },
            vec![create.id()],
            Rights::MEMBER,
            current,
            &[older],
            1,
        );
        admit(&mut live, &create, &cw);
        admit(&mut live, &edit, &ew);
        targets.push(create.id());
        witnesses.extend([(create.id(), cw), (edit.id(), ew)]);
    }
    drain(&mut live);
    let bytes = live.snapshot().unwrap().encode();
    // Each action needs only ONE old basis, well within its fixed dependency cap.
    // Combining unrelated actions' selected coverage must not impose a global
    // 65-context ceiling on a long-running group or strand a valid saved ledger.
    let mut restored = ChatLedger::restore(&bytes, limits(), &witnesses).unwrap();
    drain(&mut restored);
    assert_eq!(restored.snapshot().unwrap().encode(), bytes);
    for target in targets {
        assert_eq!(
            restored.projection(target, 100_000).unwrap(),
            live.projection(target, 100_000).unwrap()
        );
    }
}

#[test]
fn typed_chat_restore_preserves_exact_coverage_for_a_still_missing_parent() {
    let f = Fixture::new(Realm::new(b"missing-parent-after-restart", 1).unwrap(), 1);
    let subject = &f.subjects[0];
    let older = BasisId::from_bytes([44; 32]);
    let newer = BasisId::from_bytes([45; 32]);
    let (create, cw) = f.action_with(
        subject,
        1,
        Body::Create {
            text: "delayed original".into(),
            reply_to: None,
        },
        vec![],
        Rights::MEMBER,
        older,
        &[],
        1000,
    );
    let (edit, ew) = f.action_with(
        subject,
        2,
        Body::Edit {
            create: create.id(),
            text: "arrived first".into(),
        },
        vec![create.id()],
        Rights::MEMBER,
        newer,
        &[older],
        0,
    );
    let mut ledger = f.ledger();
    admit(&mut ledger, &edit, &ew);
    drain(&mut ledger);
    let snapshot = ledger.snapshot().unwrap().encode();
    assert_eq!(
        ChatLedger::restore(&snapshot, limits(), &[(edit.id(), cw.clone())]).err(),
        Some(ChatError::Authority),
        "restore cannot associate another grant with this action"
    );
    let mut restored = ChatLedger::restore(&snapshot, limits(), &[(edit.id(), ew)]).unwrap();
    drain(&mut restored);
    assert_eq!(
        restored.state(edit.id()),
        Some(ActionState::Pending(PendingReason::Dependencies))
    );
    admit(&mut restored, &create, &cw);
    drain(&mut restored);
    assert_eq!(restored.state(edit.id()), Some(ActionState::Valid));
    assert_eq!(
        restored
            .projection(create.id(), 1000)
            .unwrap()
            .unwrap()
            .visible_text
            .as_deref(),
        Some("arrived first")
    );
}

#[test]
fn typed_chat_short_snapshot_cannot_allocate_an_advertised_billion_actions() {
    let f = Fixture::new(Realm::new(b"short-snapshot", 1).unwrap(), 1);
    let valid_empty = f.ledger().snapshot().unwrap().encode();
    let allowance = LedgerLimits {
        max_actions: 1_000_000_000,
        max_action_bytes: 1024,
    };
    assert!(ChatSnapshot::decode(&valid_empty, allowance).is_ok());
    for advertised in [1u32, 1_000_000_000] {
        let mut malformed = valid_empty.clone();
        let count = malformed.len() - 4;
        malformed[count..].copy_from_slice(&advertised.to_be_bytes());
        assert_eq!(
            ChatSnapshot::decode(&malformed, allowance).err(),
            Some(ChatError::Malformed)
        );
    }
}
