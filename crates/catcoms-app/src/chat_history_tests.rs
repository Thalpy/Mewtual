//! Adversarial channel projection tests. These deliberately use real Automerge changes rather
//! than a mock merge: disconnected first writers create conflicting containers, dependencies
//! arrive backwards, the same change is replayed, and clocks disagree. Network/MLS acceptance
//! needs separate coverage; a converged projection alone does not establish authorization.

use super::*;
use automerge::ActorId;
use catcoms_mls::{InviteLedger, ServerGroup};
use catcoms_replication::{EncryptedDoc, ReplError, SignedOp};
use rand_core::{RngCore, SeedableRng};

fn replica(actor: u8) -> AutoCommit {
    let mut doc = AutoCommit::new();
    doc.set_actor(ActorId::from(vec![actor; 32]));
    doc
}

fn conflicted() -> (AutoCommit, String) {
    let mut left = replica(1);
    let mut right = replica(2);
    append_message(&mut left, "left", "alice", "left original", 100, "").unwrap();
    append_message(&mut right, "right", "bob", "right original", 100, "").unwrap();
    left.merge(&mut right).unwrap();
    let (_, winner) = left.get(ROOT, MESSAGES).unwrap().unwrap();
    let winning_ids: HashSet<_> = left
        .list_range(&winner, ..)
        .map(|item| str_field(&left, &item.id(), MSG_ID))
        .collect();
    let losing = read_messages(&left)
        .into_iter()
        .find(|message| !winning_ids.contains(&message.id))
        .expect("the other visible message belongs to the losing container")
        .id;
    (left, losing)
}

#[test]
fn a_new_post_does_not_hide_valid_history_when_a_concurrent_non_list_value_wins_the_root() {
    // A large actor/counter makes the malformed scalar the winner without relying on randomness.
    let mut history = replica(1);
    append_message(&mut history, "keep", "alice", "keep this history", 100, "").unwrap();
    let mut malformed = replica(250);
    for index in 0..10 {
        malformed
            .put(ROOT, format!("padding-{index}"), true)
            .unwrap();
    }
    malformed.put(ROOT, MESSAGES, "not a list").unwrap();
    history.merge(&mut malformed).unwrap();
    assert!(matches!(
        history.get(ROOT, MESSAGES).unwrap().unwrap().0,
        Value::Scalar(_)
    ));
    assert_eq!(read_messages(&history).len(), 1);
    assert!(edit_message_in_doc(&mut history, "keep", "still addressable", 1).unwrap());
    append_message(&mut history, "new", "bob", "new addition", 200, "").unwrap();
    let messages = read_messages(&history);
    assert_eq!(
        messages.len(),
        2,
        "new additions cannot replace an observed valid message list"
    );
    assert_eq!(messages[0].text, "still addressable");
    assert_eq!(messages[1].text, "new addition");
}

#[test]
fn conflicting_id_scalars_address_only_the_same_winner_the_reader_displays() {
    let mut original = replica(1);
    append_message(&mut original, "initial", "alice", "original", 100, "").unwrap();
    let (_, list) = original.get(ROOT, MESSAGES).unwrap().unwrap();
    let (_, message) = original.get(&list, 0).unwrap().unwrap();
    let mut other = original.fork();
    other.set_actor(ActorId::from(vec![2; 32]));
    original.put(&message, MSG_ID, "left-id").unwrap();
    other.put(&message, MSG_ID, "right-id").unwrap();
    original.merge(&mut other).unwrap();
    assert_eq!(original.get_all(&message, MSG_ID).unwrap().len(), 2);
    let visible = read_messages(&original)[0].id.clone();
    let hidden = if visible == "left-id" {
        "right-id"
    } else {
        "left-id"
    };
    assert!(!edit_message_in_doc(&mut original, hidden, "wrong target", 2).unwrap());
    assert!(edit_message_in_doc(&mut original, &visible, "visible target", 1).unwrap());
    assert_eq!(read_messages(&original)[0].text, "visible target");
}

#[test]
fn an_edit_reaches_the_visible_message_in_a_losing_first_writer_list() {
    let (mut doc, losing) = conflicted();
    assert!(edit_message_in_doc(&mut doc, &losing, "corrected", 1).unwrap());
    let messages = read_messages(&doc);
    assert_eq!(messages.len(), 2);
    let changed = messages
        .iter()
        .find(|message| message.id == losing)
        .unwrap();
    assert_eq!(changed.text, "corrected");
    assert_eq!(
        changed.edited, 1,
        "a backwards wall clock cannot invalidate an edit"
    );
    assert!(messages
        .iter()
        .any(|message| message.id != losing && message.text.ends_with("original")));
}

#[test]
fn a_delete_reaches_the_visible_message_in_a_losing_first_writer_list() {
    let (mut doc, losing) = conflicted();
    assert!(delete_message_in_doc(&mut doc, &losing).unwrap());
    let messages = read_messages(&doc);
    assert_eq!(messages.len(), 1);
    assert_ne!(messages[0].id, losing);
}

#[test]
fn a_reaction_reaches_the_visible_message_in_a_losing_first_writer_list() {
    let (mut doc, losing) = conflicted();
    assert!(toggle_reaction_in_doc(&mut doc, &losing, "👍", "carol").unwrap());
    let messages = read_messages(&doc);
    assert_eq!(messages.len(), 2);
    let changed = messages
        .iter()
        .find(|message| message.id == losing)
        .unwrap();
    assert_eq!(
        changed.reactions,
        vec![Reaction {
            emoji: "👍".into(),
            by: vec!["carol".into()]
        }]
    );
    assert!(toggle_reaction_in_doc(&mut doc, &losing, "👍", "carol").unwrap());
    assert!(read_messages(&doc)
        .iter()
        .all(|message| message.reactions.is_empty()));
}

#[test]
fn a_pin_reaches_the_visible_message_in_a_losing_first_writer_list() {
    let (mut doc, losing) = conflicted();
    assert!(set_pin_in_doc(&mut doc, &losing, true).unwrap());
    assert_eq!(
        read_messages(&doc)
            .iter()
            .filter(|message| message.pinned)
            .count(),
        1
    );
    assert!(read_messages(&doc)
        .iter()
        .any(|message| message.id == losing && message.pinned));
}

#[test]
fn duplicate_message_ids_cannot_redirect_an_authorized_action_to_another_object() {
    for separate_lists in [false, true] {
        let mut doc = replica(1);
        append_message(&mut doc, "collision", "victim", "keep this", 10, "").unwrap();
        if separate_lists {
            let mut other = replica(2);
            append_message(&mut other, "collision", "attacker", "other row", 20, "").unwrap();
            doc.merge(&mut other).unwrap();
        } else {
            append_message(&mut doc, "collision", "attacker", "other row", 20, "").unwrap();
        }
        let before = read_messages(&doc);
        assert!(!edit_message_in_doc(&mut doc, "collision", "rewritten", 30).unwrap());
        assert!(!delete_message_in_doc(&mut doc, "collision").unwrap());
        assert!(!toggle_reaction_in_doc(&mut doc, "collision", "👍", "attacker").unwrap());
        assert!(!set_pin_in_doc(&mut doc, "collision", true).unwrap());
        assert_eq!(
            read_messages(&doc),
            before,
            "ambiguity never selects one author's object for another"
        );
    }
}

fn shuffled<T>(items: &mut [T], seed: u64) {
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(seed);
    for index in (1..items.len()).rev() {
        items.swap(index, (rng.next_u64() % (index as u64 + 1)) as usize);
    }
}

/// Fifty independent initial containers are more hostile than one pre-created shared list.
/// Exercise both proposed group sizes, then replay the real changes in unrelated orders.
#[test]
fn twenty_and_fifty_first_writers_preserve_the_union_and_all_messages_remain_mutable() {
    for count in [20, 50] {
        let mut merged = replica(200);
        for index in (0..count).rev() {
            let mut bubble = replica(index + 1);
            append_message(
                &mut bubble,
                &format!("m{index}"),
                &format!("a{index}"),
                "original",
                1_000,
                "",
            )
            .unwrap();
            merged.merge(&mut bubble).unwrap();
        }
        assert_eq!(read_messages(&merged).len(), count as usize);
        let before_edits = merged.get_heads();
        for index in 0..count {
            let id = format!("m{index}");
            assert!(
                edit_message_in_doc(
                    &mut merged,
                    &id,
                    &format!("edited {index}"),
                    u64::from(index)
                )
                .unwrap(),
                "message {id} at group size {count}"
            );
            if index % 3 == 0 {
                assert!(delete_message_in_doc(&mut merged, &id).unwrap());
            }
        }
        merged.commit();
        let expected = read_messages(&merged);
        assert_eq!(
            expected.len(),
            (0..count).filter(|index| index % 3 != 0).count()
        );
        let historical = merged.fork_at(&before_edits).unwrap();
        assert_eq!(
            read_messages(&historical).len(),
            count as usize,
            "deletion changes the projection, not retained history"
        );
        assert!(read_messages(&historical)
            .iter()
            .all(|message| message.text == "original"));

        let all_changes = merged.get_changes(&[]);
        for seed in 0..4 {
            let mut changes = all_changes.clone();
            shuffled(&mut changes, seed);
            let mut recipient = replica(210 + seed as u8);
            for change in changes.iter().chain(changes.iter().rev()) {
                recipient.apply_changes([change.clone()]).unwrap();
            }
            assert_eq!(
                read_messages(&recipient),
                expected,
                "same visible state after reordered duplicated history, size={count}, seed={seed}"
            );
            assert_eq!(recipient.get_heads(), merged.get_heads());
            let restored = AutoCommit::load(&recipient.save()).unwrap();
            assert_eq!(
                read_messages(&restored),
                expected,
                "restart cannot resurrect deleted rows"
            );
        }
    }
}

#[test]
fn deletion_wins_over_a_concurrent_edit_and_replaying_old_history_cannot_resurrect_it() {
    let mut base = replica(1);
    append_message(&mut base, "target", "alice", "original", u64::MAX, "").unwrap();
    append_message(&mut base, "unrelated", "bob", "keep", 100, "").unwrap();
    let base_heads = base.get_heads();
    let mut deleting = base.fork();
    deleting.set_actor(ActorId::from(vec![2; 32]));
    let mut editing = base.fork();
    editing.set_actor(ActorId::from(vec![3; 32]));
    assert!(delete_message_in_doc(&mut deleting, "target").unwrap());
    assert!(edit_message_in_doc(&mut editing, "target", "late edit", 0).unwrap());
    deleting.merge(&mut editing).unwrap();
    let changes = deleting.get_changes(&[]);
    for seed in 0..12 {
        let mut arrival = changes.clone();
        shuffled(&mut arrival, seed);
        let mut recipient = replica(100);
        for change in arrival
            .iter()
            .chain(changes.iter())
            .chain(arrival.iter().rev())
        {
            recipient.apply_changes([change.clone()]).unwrap();
        }
        let live = read_messages(&recipient);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, "unrelated");
        let old = recipient.fork_at(&base_heads).unwrap();
        assert!(read_messages(&old)
            .iter()
            .any(|message| message.id == "target" && message.text == "original"));
    }
}

#[test]
fn a_dependent_delete_survives_a_restart_before_its_message_arrives() {
    let mut source = replica(1);
    append_message(&mut source, "deleted", "alice", "original", 500, "").unwrap();
    let original = source.get_heads();
    assert!(delete_message_in_doc(&mut source, "deleted").unwrap());
    source.commit();
    let deletion = source.get_last_local_change().unwrap();
    let history = source.get_changes(&[]);
    let mut receiver = replica(2);
    receiver.apply_changes([deletion]).unwrap();
    assert!(read_messages(&receiver).is_empty());
    let mut receiver = AutoCommit::load(&receiver.save()).unwrap();
    for change in history.into_iter().rev() {
        receiver.apply_changes([change]).unwrap();
    }
    assert_eq!(receiver.get_heads(), source.get_heads());
    assert!(read_messages(&receiver).is_empty());
    assert_eq!(
        read_messages(&receiver.fork_at(&original).unwrap())[0].text,
        "original"
    );
}

#[test]
fn independent_deletions_merge_without_hiding_a_concurrent_unrelated_addition() {
    let mut base = replica(1);
    append_message(&mut base, "target", "alice", "original", 500, "").unwrap();
    let mut left = base.fork();
    left.set_actor(ActorId::from(vec![2; 32]));
    let mut right = base.fork();
    right.set_actor(ActorId::from(vec![3; 32]));
    assert!(delete_message_in_doc(&mut left, "target").unwrap());
    assert!(delete_message_in_doc(&mut right, "target").unwrap());
    append_message(&mut right, "new", "bob", "keep the addition", 0, "").unwrap();
    left.merge(&mut right).unwrap();
    let history = left.get_changes(&[]);
    for seed in 0..12 {
        let mut arrival = history.clone();
        shuffled(&mut arrival, seed);
        let mut recipient = replica(4);
        for change in arrival.iter().chain(arrival.iter().rev()) {
            recipient.apply_changes([change.clone()]).unwrap();
        }
        let messages = read_messages(&recipient);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, "new");
        assert_eq!(messages[0].text, "keep the addition");
        assert_eq!(recipient.get_heads(), left.get_heads());
    }
}

#[test]
fn concurrent_edits_converge_and_a_causal_followup_wins_despite_a_backwards_clock() {
    let mut base = replica(1);
    append_message(&mut base, "target", "alice", "original", 5_000, "").unwrap();
    let mut left = base.fork();
    left.set_actor(ActorId::from(vec![2; 32]));
    let mut right = base.fork();
    right.set_actor(ActorId::from(vec![3; 32]));
    assert!(edit_message_in_doc(&mut left, "target", "left edit", 9_999_999).unwrap());
    assert!(edit_message_in_doc(&mut right, "target", "right edit", 2).unwrap());
    left.merge(&mut right).unwrap();
    let concurrent = left.get_heads();
    let (_, message) = left.get(ROOT, MESSAGES).unwrap().unwrap();
    let (_, message) = left.get(&message, 0).unwrap().unwrap();
    let alternatives = left.get_all(&message, TEXT).unwrap();
    assert_eq!(
        alternatives.len(),
        2,
        "neither concurrent edit is erased from the CRDT"
    );
    assert!(edit_message_in_doc(&mut left, "target", "after seeing both", 1).unwrap());
    left.commit();
    let expected = read_messages(&left);
    assert_eq!(expected[0].text, "after seeing both");
    assert_eq!(expected[0].edited, 1);
    let changes = left.get_changes(&[]);
    for seed in 0..12 {
        let mut arrival = changes.clone();
        shuffled(&mut arrival, seed);
        let mut recipient = replica(4);
        for change in arrival.iter().chain(arrival.iter().rev()) {
            recipient.apply_changes([change.clone()]).unwrap();
        }
        assert_eq!(read_messages(&recipient), expected);
        let old = recipient.fork_at(&concurrent).unwrap();
        let (_, list) = old.get(ROOT, MESSAGES).unwrap().unwrap();
        let (_, msg) = old.get(&list, 0).unwrap().unwrap();
        assert_eq!(old.get_all(&msg, TEXT).unwrap().len(), 2);
    }
}

#[tokio::test]
async fn actual_server_commands_reject_ambiguous_ids_without_posting_an_operation() {
    use catcoms_rt::{Hub, ManualClock, PeerId};
    let hub = Hub::new();
    let mut server = Server::found(
        hub.join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rand_chacha::ChaCha20Rng::seed_from_u64(1),
        Box::new(ManualClock::new(1_000)),
        "owner",
    )
    .unwrap();
    let channel = channel_id("general");
    server.open_channel(channel).await.unwrap();
    let me = server.my_fingerprint();
    server
        .sync
        .post(DocType::Channel, channel, |doc| {
            append_message(doc, "duplicated", "victim", "leave this alone", 100, "")?;
            append_message(doc, "duplicated", &me, "mine", 200, "")
        })
        .await
        .unwrap();
    let before = server.messages(channel);
    let count = server
        .sync
        .doc(DocType::Channel, channel)
        .unwrap()
        .op_count();
    assert!(server
        .edit_message(channel, "duplicated", "changed")
        .await
        .is_err());
    assert!(server.delete_message(channel, "duplicated").await.is_err());
    assert!(server
        .toggle_reaction(channel, "duplicated", "👍")
        .await
        .is_err());
    assert!(server.set_pin(channel, "duplicated", true).await.is_err());
    assert_eq!(server.messages(channel), before);
    assert_eq!(
        server
            .sync
            .doc(DocType::Channel, channel)
            .unwrap()
            .op_count(),
        count
    );
}

#[tokio::test]
async fn actual_server_edits_and_deletes_its_losing_list_message_with_new_signed_operations() {
    use catcoms_rt::{Hub, ManualClock, PeerId};
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let alice_peer = PeerId::from_u64(1);
    let mut alice = Server::found(
        hub.join(alice_peer),
        MlsDevice::generate().unwrap(),
        rand_chacha::ChaCha20Rng::seed_from_u64(1),
        Box::new(clock.clone()),
        "alice",
    )
    .unwrap();
    alice.subscribe_control().await.unwrap();
    let invite = alice.mint_invite([7; 16], u64::MAX, vec![]).unwrap();
    let (bob, _) = tokio::join!(
        Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            rand_chacha::ChaCha20Rng::seed_from_u64(2),
            Box::new(clock.clone()),
            "bob",
            alice_peer,
            &invite
        ),
        alice.sync_once(),
    );
    let mut bob = bob.unwrap();
    let channel = channel_id("general");
    alice.open_channel(channel).await.unwrap();
    bob.open_channel(channel).await.unwrap();
    // No receive poll before both posts: the real command creates each first-writer list.
    alice.send_message(channel, "alice original").await.unwrap();
    bob.send_message(channel, "bob original").await.unwrap();
    // A wall-clock bound is only a hung-test guard. Protocol decisions use the injected clock.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        for server in [&mut alice, &mut bob] {
            while server.messages(channel).len() != 2 {
                assert!(server.sync_once().await.unwrap());
            }
        }
    })
    .await
    .unwrap();
    let (_, winner) = alice
        .sync
        .doc(DocType::Channel, channel)
        .unwrap()
        .doc()
        .get(ROOT, MESSAGES)
        .unwrap()
        .unwrap();
    let winner_author = read_message(
        alice.sync.doc(DocType::Channel, channel).unwrap().doc(),
        &alice
            .sync
            .doc(DocType::Channel, channel)
            .unwrap()
            .doc()
            .list_range(&winner, ..)
            .next()
            .unwrap()
            .id(),
    )
    .author;
    let losing_is_bob = winner_author == alice.my_fingerprint();
    let own_id = {
        let server = if losing_is_bob { &mut bob } else { &mut alice };
        let own = server
            .messages(channel)
            .into_iter()
            .find(|message| message.author == server.my_fingerprint())
            .unwrap();
        let count = server
            .sync
            .doc(DocType::Channel, channel)
            .unwrap()
            .op_count();
        server
            .edit_message(channel, &own.id, "actual correction")
            .await
            .unwrap();
        assert_eq!(
            server
                .sync
                .doc(DocType::Channel, channel)
                .unwrap()
                .op_count(),
            count + 1
        );
        assert_eq!(
            server
                .messages(channel)
                .iter()
                .find(|message| message.id == own.id)
                .unwrap()
                .text,
            "actual correction"
        );
        server
            .toggle_reaction(channel, &own.id, "👍")
            .await
            .unwrap();
        assert_eq!(
            server
                .sync
                .doc(DocType::Channel, channel)
                .unwrap()
                .op_count(),
            count + 2
        );
        assert_eq!(
            server
                .messages(channel)
                .iter()
                .find(|message| message.id == own.id)
                .unwrap()
                .reactions
                .len(),
            1
        );
        own.id
    };
    let count = alice
        .sync
        .doc(DocType::Channel, channel)
        .unwrap()
        .op_count();
    alice.set_pin(channel, &own_id, true).await.unwrap();
    assert_eq!(
        alice
            .sync
            .doc(DocType::Channel, channel)
            .unwrap()
            .op_count(),
        count + 1
    );
    assert!(alice
        .messages(channel)
        .iter()
        .any(|message| message.id == own_id && message.pinned));
    let server = if losing_is_bob { &mut bob } else { &mut alice };
    let count = server
        .sync
        .doc(DocType::Channel, channel)
        .unwrap()
        .op_count();
    server.delete_message(channel, &own_id).await.unwrap();
    assert_eq!(
        server
            .sync
            .doc(DocType::Channel, channel)
            .unwrap()
            .op_count(),
        count + 1
    );
    assert_eq!(server.messages(channel).len(), 1);
    assert!(server
        .messages(channel)
        .iter()
        .all(|message| message.author != server.my_fingerprint()));
}

fn admit(
    owner: &MlsDevice,
    group: &mut ServerGroup,
    member: &MlsDevice,
    ledger: &mut InviteLedger,
    nonce: u8,
) -> (ServerGroup, Vec<u8>) {
    let invite = group
        .mint_invite(owner, [nonce; 16], u64::MAX, vec![])
        .unwrap();
    let package = member
        .key_package_for_invite(&group.group_id(), invite.invite_nonce)
        .unwrap();
    let added = group
        .add_member_via_invite(owner, package, &invite, ledger, 1_000)
        .unwrap();
    (
        ServerGroup::join(member, &added.welcome).unwrap(),
        added.commit,
    )
}

/// The old bubble writes actual chat insertion/edit/deletion changes while ten Adds advance
/// the other side beyond the default eight-key window. Current-key catch-up must preserve its
/// exact inner-signed operations, not replace either branch with a rendered snapshot.
#[test]
fn chat_history_merges_across_ten_mls_epochs_and_retains_removed_authors_edits_and_deletes() {
    let alice = MlsDevice::generate().unwrap();
    let bob = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&alice).unwrap();
    let mut ledger = InviteLedger::new();
    let (mut bob_group, _) = admit(&alice, &mut group, &bob, &mut ledger, 1);
    let channel = channel_id("general");
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(40);
    let mut current = EncryptedDoc::new(DocType::Channel, channel, &alice.device_id());
    let mut bubble = EncryptedDoc::new(DocType::Channel, channel, &bob.device_id());
    let old_key = bob_group
        .channel_secret(&bob, DocType::Channel, channel)
        .unwrap();
    let author = fingerprint(&bob.device_id());
    let mut original = Vec::new();
    for (id, text) in [("edited", "original text"), ("deleted", "removed text")] {
        let operation = bubble
            .edit(&bob, &bob_group, &mut rng, |doc| {
                append_message(doc, id, &author, text, 1_000, "")
            })
            .unwrap();
        original.push(operation.open(&old_key).unwrap());
    }
    let operation = bubble
        .edit(&bob, &bob_group, &mut rng, |doc| {
            edit_message_in_doc(doc, "edited", "corrected text", 2).map(|changed| assert!(changed))
        })
        .unwrap();
    original.push(operation.open(&old_key).unwrap());
    let operation = bubble
        .edit(&bob, &bob_group, &mut rng, |doc| {
            delete_message_in_doc(doc, "deleted").map(|changed| assert!(changed))
        })
        .unwrap();
    original.push(operation.open(&old_key).unwrap());

    let mut commits = Vec::new();
    let mut last = None;
    for index in 0..10 {
        let member = MlsDevice::generate().unwrap();
        let (joined, commit) = admit(&alice, &mut group, &member, &mut ledger, index + 2);
        commits.push(commit);
        last = Some((member, joined));
        current
            .edit(&alice, &group, &mut rng, |doc| {
                append_message(
                    doc,
                    &format!("current-{index}"),
                    "alice",
                    "other bubble",
                    1_000,
                    "",
                )
            })
            .unwrap();
    }
    assert_eq!(group.epoch(), bob_group.epoch() + 10);
    let old_sealed = bubble.export_catchup(&bob_group, &bob, &mut rng).unwrap();
    assert!(matches!(
        current.ingest(&old_sealed[0], &group, &alice),
        Err(ReplError::EpochUnavailable(_))
    ));
    for commit in commits {
        bob_group.process_incoming(&bob, &commit).unwrap();
    }
    let mut catchup = bubble.export_catchup(&bob_group, &bob, &mut rng).unwrap();
    let key = group
        .channel_secret(&alice, DocType::Channel, channel)
        .unwrap();
    let rewrapped: Vec<SignedOp> = catchup
        .iter()
        .map(|operation| operation.open(&key).unwrap())
        .collect();
    assert_eq!(
        rewrapped, original,
        "epoch resealing preserves original authors, bytes and signatures"
    );
    catchup.reverse();
    assert_eq!(current.import_catchup(&catchup, &group, &alice).unwrap(), 4);
    let messages = read_messages(current.doc());
    assert_eq!(messages.len(), 11);
    assert_eq!(
        messages
            .iter()
            .find(|message| message.id == "edited")
            .unwrap()
            .text,
        "corrected text"
    );
    assert!(messages.iter().all(|message| message.id != "deleted"));

    // Removing an author changes future group access, never the accepted history. A late member
    // must receive the old author's signatures through a current member's current encryption.
    let (late, mut late_group) = last.unwrap();
    let removal = group.stage_remove(&alice, &bob.device_id()).unwrap();
    group.merge_staged_self(&alice).unwrap();
    late_group.process_incoming(&late, &removal.commit).unwrap();
    let export = current.export_catchup(&group, &alice, &mut rng).unwrap();
    let mut recipient = EncryptedDoc::new(DocType::Channel, channel, &late.device_id());
    // A pending dependent deletion is saved BEFORE its dependencies arrive. Reopen must keep it.
    let late_key = late_group
        .channel_secret(&late, DocType::Channel, channel)
        .unwrap();
    let deletion = export
        .iter()
        .find(|operation| operation.open(&late_key).unwrap().hash() == original[3].hash())
        .unwrap();
    recipient.ingest(deletion, &late_group, &late).unwrap();
    assert!(read_messages(recipient.doc()).is_empty());
    assert_eq!(recipient.op_count(), 1);
    let saved = recipient.snapshot().unwrap();
    let mut recipient = EncryptedDoc::restore_for_actor(&saved, &late.device_id()).unwrap();
    let mut shuffled_export = export.clone();
    shuffled(&mut shuffled_export, 99);
    recipient
        .import_catchup(&shuffled_export, &late_group, &late)
        .unwrap();
    assert_eq!(
        recipient
            .import_catchup(&export, &late_group, &late)
            .unwrap(),
        0
    );
    assert_eq!(recipient.op_count(), 14);
    assert_eq!(recipient.heads(), current.heads());
    assert_eq!(read_messages(recipient.doc()), messages);
    let relayed = recipient
        .export_catchup(&late_group, &late, &mut rng)
        .unwrap();
    let held: HashSet<_> = relayed
        .iter()
        .map(|operation| operation.open(&late_key).unwrap().hash())
        .collect();
    for operation in original {
        assert!(
            held.contains(&operation.hash()),
            "all insertion/edit/delete evidence survives removal, relay and restart"
        );
    }
}
