//! Only merged commits enter the commit log (`docs/design-mls-commit-log.md`).
//!
//! A commit record's signature covers whatever bytes the committer put in `mls_commit`, so an
//! authorized committer can sign an application message, a proposal, an external commit or a
//! relabelled old commit. Each test here sends such a record through the real control-topic,
//! catch-up, restore or fork-contest path and checks that it is refused: the epoch does not move,
//! nothing is counted as applied, and the log that catch-up serves never holds it.
use super::*;
use crate::tests::{
    build_members, commit_catchup_between, converge_and_publish_test_routes, framed_commit,
    prove_live_member,
};
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

type Member = ChannelSync<MemNetwork, ChaCha20Rng>;

/// `bytes` wrapped in a record signed by `committer`, labelled `commit_epoch`.
fn signed(committer: &Member, commit_epoch: u64, bytes: &[u8]) -> CommitRecord {
    committer.sign_add_record(
        commit_epoch,
        bytes,
        committer.group.epoch_authenticator_id(),
    )
}

/// The founder admits a fresh device through the real `serve_join`; the commit is recorded in its
/// log and queued for gossip, but not delivered to anyone. Returns that commit's record.
fn admit_quietly(founder: &mut Member, nonce: u8) -> CommitRecord {
    let device = MlsDevice::generate().unwrap();
    let invite = founder.mint_invite([nonce; 16], 60_000, vec![]).unwrap();
    let package = device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let request = encode_join_req(&invite, &serialize_key_package(&package).unwrap());
    let answer = founder
        .serve_join(PeerId::from_u64(900 + u64::from(nonce)), &request)
        .expect("the founder admits");
    assert_eq!(answer[0], 1, "admitted synchronously");
    founder.commit_log.back().cloned().unwrap()
}

/// Every record in `node`'s log is a commit for its own label, and none carries `bytes`.
fn log_is_clean(node: &Member, bytes: &[&[u8]]) {
    for record in &node.commit_log {
        assert!(
            record.header_matches_label(),
            "the log holds a record that is not a commit for its epoch"
        );
        assert!(
            !bytes.contains(&record.mls_commit.as_slice()),
            "a refused record reached the log"
        );
    }
}

/// An application message, a proposal and an external commit, each signed into a record by the
/// authorized committer at the right epoch, are all refused on the live control path. The real
/// commit for that epoch then applies.
#[tokio::test]
async fn a_non_commit_from_the_authorized_committer_is_refused_and_never_logged() {
    let (_hub, mut members, _) = build_members(2).await;
    converge_and_publish_test_routes(&mut members);
    let mut bob = members.pop().unwrap();
    let mut alice = members.pop().unwrap();
    let alice_peer = alice.local_peer();
    let here = bob.epoch();
    let log_len = bob.commit_log.len();
    let applied = bob.stats().commits_applied;

    let application = alice
        .group
        .create_application_message(&alice.device, b"not a commit")
        .unwrap();
    let proposal = bob.group.test_self_update_proposal(&bob.device).unwrap();
    let outsider = MlsDevice::generate().unwrap();
    let external = alice
        .group
        .test_external_commit(&alice.device, &outsider)
        .unwrap();
    let junk: [&[u8]; 3] = [&application, &proposal, &external];
    for (i, bytes) in junk.iter().enumerate() {
        bob.on_control(alice_peer, &framed_commit(&signed(&alice, here, bytes)));
        assert_eq!(bob.epoch(), here, "case {i}: the epoch must not move");
        assert_eq!(bob.commit_log.len(), log_len, "case {i}: nothing logged");
        assert_eq!(
            bob.stats().commits_applied,
            applied,
            "case {i}: nothing counted"
        );
        assert_eq!(bob.stats().commit_records_refused, i as u64 + 1);
    }

    // The application message was refused on its header, so it was never decrypted here.
    match bob
        .group
        .process_incoming(&bob.device, &application)
        .unwrap()
    {
        catcoms_mls::Incoming::Application(plain) => assert_eq!(plain, b"not a commit"),
        other => panic!("the wrapped message must still be unread, got {other:?}"),
    }

    let real = admit_quietly(&mut alice, 40);
    assert_eq!(real.commit_epoch, here);
    bob.on_control(alice_peer, &framed_commit(&real));
    assert_eq!(bob.epoch(), here + 1, "the real commit still applies");
    assert_eq!(bob.commit_log.len(), log_len + 1);
    assert_eq!(bob.stats().commits_applied, applied + 1);
    log_is_clean(&bob, &junk);
}

/// A commit labelled one epoch ahead of what it is gets buffered (the label is all a buffer can
/// see), then refused when its slot comes up; a replay relabelled to the current epoch is refused
/// outright. Neither is logged, and the honest chain carries on.
#[tokio::test]
async fn relabelled_and_replayed_commits_are_refused_and_never_logged() {
    let (_hub, mut members, _) = build_members(2).await;
    converge_and_publish_test_routes(&mut members);
    let mut bob = members.pop().unwrap();
    let mut alice = members.pop().unwrap();
    let alice_peer = alice.local_peer();
    let here = bob.epoch();

    let real = admit_quietly(&mut alice, 41);
    let relabelled = signed(&alice, here + 1, &real.mls_commit);
    bob.on_control(alice_peer, &framed_commit(&relabelled));
    assert_eq!(bob.epoch(), here, "a future label is only buffered");
    bob.on_control(alice_peer, &framed_commit(&real));
    assert_eq!(bob.epoch(), here + 1, "the real commit applies");
    assert_eq!(
        bob.stats().commit_records_refused,
        1,
        "the buffered relabel came up and was refused"
    );

    // The same commit replayed, relabelled to the epoch Bob is now at.
    bob.on_control(
        alice_peer,
        &framed_commit(&signed(&alice, here + 1, &real.mls_commit)),
    );
    assert_eq!(bob.epoch(), here + 1);
    assert_eq!(bob.stats().commit_records_refused, 2);
    // And replayed under its own label: already applied, ignored as before.
    bob.on_control(alice_peer, &framed_commit(&real));
    assert_eq!(bob.stats().commit_records_refused, 2);

    let real_count = bob
        .commit_log
        .iter()
        .filter(|r| r.mls_commit == real.mls_commit)
        .count();
    assert_eq!(
        real_count, 1,
        "the commit is logged once, under its real label"
    );
    log_is_clean(&bob, &[]);

    // The chain continues past the refusals.
    let next = admit_quietly(&mut alice, 42);
    bob.on_control(alice_peer, &framed_commit(&next));
    assert_eq!(bob.epoch(), here + 2);
}

/// Catch-up from a peer whose log already holds a non-commit (an older build recorded any
/// successful `process_incoming`): the requester refuses it, applies the real commits around it,
/// reports only those as applied, and its own log, which it serves onward, stays clean.
#[tokio::test]
async fn catch_up_past_a_non_commit_in_the_serving_log() {
    let (_hub, mut members, ids) = build_members(2).await;
    converge_and_publish_test_routes(&mut members);
    let mut bob = members.pop().unwrap();
    let mut alice = members.pop().unwrap();
    let (alice_peer, bob_peer) = (alice.local_peer(), bob.local_peer());
    prove_live_member(&mut alice, bob_peer, ids[1]);
    prove_live_member(&mut bob, alice_peer, ids[0]);
    let here = bob.epoch();

    // Alice moves two epochs on without Bob hearing.
    let first = admit_quietly(&mut alice, 43);
    let second = admit_quietly(&mut alice, 44);
    // Her log, as an older build could have left it: a signed non-commit at Bob's epoch, ahead of
    // the real commit for it.
    let application = alice
        .group
        .create_application_message(&alice.device, b"logged by an older build")
        .unwrap();
    let poisoned = signed(&alice, here, &application);
    let honest: Vec<_> = alice.commit_log.drain(..).collect();
    alice.commit_log.push_back(poisoned.clone());
    alice.commit_log.extend(honest);

    let outcome = commit_catchup_between(&mut bob, &mut alice, alice_peer, bob_peer, here).await;
    assert_eq!(
        outcome,
        CommitCatchupOutcome::Verified { applied: 2 },
        "only the two real commits count as progress"
    );
    assert_eq!(bob.epoch(), here + 2);
    assert_eq!(bob.stats().commit_records_refused, 1);
    log_is_clean(&bob, &[&application]);
    let tail: Vec<_> = bob
        .commit_log
        .iter()
        .rev()
        .take(2)
        .map(|r| r.mls_commit.clone())
        .collect();
    assert_eq!(tail, vec![second.mls_commit, first.mls_commit]);
}

/// A persisted log written by an older build may hold records that are not commits. A restore
/// drops them by header (nothing is decrypted) and keeps every real commit.
#[tokio::test]
async fn restore_drops_logged_non_commits_and_keeps_real_commits() {
    let (_hub, mut members, _) = build_members(1).await;
    let mut alice = members.pop().unwrap();
    let here = alice.epoch();
    let real = admit_quietly(&mut alice, 45);
    let application = alice
        .group
        .create_application_message(&alice.device, b"logged by an older build")
        .unwrap();
    alice
        .commit_log
        .push_back(signed(&alice, here + 1, &application));
    alice.commit_log.push_back(signed(&alice, here + 1, &[7]));
    // A real commit under the wrong label is not a commit for that label either.
    alice
        .commit_log
        .push_back(signed(&alice, here + 1, &real.mls_commit));
    assert_eq!(alice.commit_log.len(), 4);

    let snapshot = alice.snapshot().unwrap();
    let hub = Hub::new();
    let restored = Member::restore(
        &snapshot,
        hub.join(PeerId::from_u64(98)),
        ChaCha20Rng::seed_from_u64(9),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert_eq!(
        restored.commit_log.len(),
        1,
        "only the real commit survives"
    );
    assert_eq!(restored.commit_log[0].mls_commit, real.mls_commit);
    assert_eq!(restored.commit_log[0].commit_epoch, real.commit_epoch);
}

/// Fork contests (`max_committer_rank >= 1`, off by default): a second committer wraps an
/// application message and grinds its `commit_id` below the real candidate's. Without the
/// contest's header check it would win, the real committer would abort its own staged removal
/// for it, and nobody would advance. With it, the real removal wins.
#[tokio::test]
async fn a_non_commit_cannot_win_a_fork_contest() {
    let (_hub, mut members, ids) = build_members(3).await;
    converge_and_publish_test_routes(&mut members);
    let config = SyncConfig {
        max_committer_rank: 1,
        stage_decision_window_ms: 0,
        ..SyncConfig::default()
    };
    for member in &mut members {
        member.set_config(config);
    }
    let carol = ids[2];
    let mut bob = members.remove(1);
    let alice = &mut members[0];
    let here = alice.epoch();
    alice.remove(&carol).await.unwrap();
    let real_id = alice.pending.as_ref().unwrap().best.commit_id();

    // Bob (rank 1, inside the window) grinds an application message below Alice's candidate.
    let junk = (0u32..)
        .map(|i| {
            let message = bob
                .group
                .create_application_message(&bob.device, &i.to_be_bytes())
                .unwrap();
            signed(&bob, here, &message)
        })
        .find(|record| record.commit_id() < real_id)
        .unwrap();
    let bob_peer = bob.local_peer();
    alice.on_control(bob_peer, &framed_commit(&junk));

    assert!(alice.resolve_pending_if_expired());
    assert_eq!(alice.epoch(), here + 1, "the real removal won and merged");
    assert!(!alice.group.contains_device(&carol));
    assert_eq!(alice.stats().forks_lost, 0);
    assert_eq!(
        alice.stats().commit_records_refused,
        1,
        "refused at the contest"
    );
    log_is_clean(alice, &[&junk.mls_commit]);
}

/// The residual the contest check does not close (design note section 6): a candidate whose
/// header says "commit for this epoch" but which does not apply. It can still win the tie-break;
/// then nothing is recorded or counted as resolved, the epoch stays put, and commit catch-up is
/// queued to heal.
#[tokio::test]
async fn an_unappliable_commit_that_wins_a_contest_is_not_recorded() {
    let (_hub, mut members, ids) = build_members(3).await;
    converge_and_publish_test_routes(&mut members);
    let config = SyncConfig {
        max_committer_rank: 1,
        stage_decision_window_ms: 0,
        ..SyncConfig::default()
    };
    for member in &mut members {
        member.set_config(config);
    }
    let carol = ids[2];
    let bob = members.remove(1);
    let alice = &mut members[0];
    let here = alice.epoch();
    alice.remove(&carol).await.unwrap();
    let real = alice.pending.as_ref().unwrap().best.clone();

    // Bob flips the AEAD tag of Alice's commit: the header still parses exactly, decryption fails.
    let garbage = (1u8..=255)
        .map(|flip| {
            let mut bytes = real.mls_commit.clone();
            *bytes.last_mut().unwrap() ^= flip;
            signed(&bob, here, &bytes)
        })
        .find(|record| record.commit_id() < real.commit_id())
        .unwrap();
    assert!(
        garbage.header_matches_label(),
        "fixture: a header-valid commit"
    );
    alice.on_control(bob.local_peer(), &framed_commit(&garbage));
    let log_len = alice.commit_log.len();

    assert!(alice.resolve_pending_if_expired());
    assert_eq!(alice.epoch(), here, "the unappliable winner moved nobody");
    assert!(alice.group.contains_device(&carol));
    assert_eq!(alice.commit_log.len(), log_len, "nothing recorded");
    assert_eq!(alice.stats().forks_resolved, 0);
    assert!(alice
        .catchup_queue
        .iter()
        .any(|task| matches!(task, CatchupTask::Commits { .. })));
}
