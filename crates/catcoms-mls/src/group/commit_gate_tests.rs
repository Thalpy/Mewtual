//! [`ServerGroup::apply_commit`] succeeds only for this group's next commit, and refuses every
//! other MLS content type without changing the group (`docs/design-mls-commit-log.md`).
//!
//! The sync layer records whatever this accepts into the commit log that catch-up serves, so each
//! case below is a message an authorized committer could wrap in a signed commit record.

use super::*;
use catcoms_wire::DocType;

/// Alice (committer) and Bob, both at epoch 1, plus Carol's pending admission at epoch 1.
struct Fixture {
    alice: MlsDevice,
    alice_group: ServerGroup,
    bob: MlsDevice,
    bob_group: ServerGroup,
}

fn fixture() -> Fixture {
    let alice = MlsDevice::generate().unwrap();
    let mut alice_group = ServerGroup::create(&alice).unwrap();
    let bob = MlsDevice::generate().unwrap();
    let added = alice_group
        .add_member(
            &alice,
            bob.key_package_for_invite(&alice_group.group_id(), [1; 16])
                .unwrap(),
        )
        .unwrap();
    let bob_group = ServerGroup::join(&bob, &added.welcome).unwrap();
    assert_eq!(bob_group.epoch(), 1);
    Fixture {
        alice,
        alice_group,
        bob,
        bob_group,
    }
}

/// Alice admits a fresh device; returns the commit and the epoch it was built at.
fn alice_adds_someone(f: &mut Fixture, nonce: u8) -> (Vec<u8>, u64) {
    let joiner = MlsDevice::generate().unwrap();
    let out = f
        .alice_group
        .add_member(
            &f.alice,
            joiner
                .key_package_for_invite(&f.alice_group.group_id(), [nonce; 16])
                .unwrap(),
        )
        .unwrap();
    (out.commit, out.commit_epoch)
}

/// Asserts `result` is the given refusal and that Bob's group did not move.
fn assert_refused(
    f: &Fixture,
    result: Result<AppliedCommit, MlsError>,
    expected: CommitRefusal,
    epoch: u64,
) {
    match result {
        Err(MlsError::NotACommit(refusal)) => assert_eq!(refusal, expected),
        other => panic!("expected {expected:?}, got {other:?}"),
    }
    assert_eq!(
        f.bob_group.epoch(),
        epoch,
        "a refusal must not move the group"
    );
}

#[test]
fn the_next_commit_applies_and_reports_a_removal() {
    let mut f = fixture();
    let (commit, at) = alice_adds_someone(&mut f, 2);
    assert_eq!(at, 1);
    let applied = f.bob_group.apply_commit(&f.bob, &commit, at).unwrap();
    assert_eq!(applied, AppliedCommit { removed: false });
    assert_eq!(f.bob_group.epoch(), 2);

    let staged = f
        .alice_group
        .stage_remove(&f.alice, &f.bob.device_id())
        .unwrap();
    f.alice_group.merge_staged_self(&f.alice).unwrap();
    // Bob applies the commit that removes him: still a merged commit, and it reports the removal.
    let applied = f
        .bob_group
        .apply_commit(&f.bob, &staged.commit, staged.commit_epoch)
        .unwrap();
    assert_eq!(applied, AppliedCommit { removed: true });
    assert_eq!(f.bob_group.epoch(), staged.commit_epoch + 1);
}

/// A wrapped application message is refused on its header, so it is never decrypted on this path:
/// the real recipient path can still open it afterwards.
#[test]
fn an_application_message_is_refused_before_decryption() {
    let mut f = fixture();
    let epoch = f.bob_group.epoch();
    let message = f
        .alice_group
        .create_application_message(&f.alice, b"not a commit")
        .unwrap();
    let result = f.bob_group.apply_commit(&f.bob, &message, epoch);
    assert_refused(
        &f,
        result,
        CommitRefusal::NotCommitContent("application"),
        epoch,
    );
    match f.bob_group.process_incoming(&f.bob, &message).unwrap() {
        Incoming::Application(plain) => assert_eq!(plain, b"not a commit"),
        other => panic!("the ratchet generation must still be unused, got {other:?}"),
    }
}

/// A proposal (here Alice's self-update proposal) is refused on its header and stays usable on
/// the ordinary path.
#[test]
fn a_proposal_is_refused_before_decryption() {
    let mut f = fixture();
    let epoch = f.bob_group.epoch();
    let proposal = f.alice_group.test_self_update_proposal(&f.alice).unwrap();
    let result = f.bob_group.apply_commit(&f.bob, &proposal, epoch);
    assert_refused(
        &f,
        result,
        CommitRefusal::NotCommitContent("proposal"),
        epoch,
    );
    assert!(matches!(
        f.bob_group.process_incoming(&f.bob, &proposal),
        Ok(Incoming::Other)
    ));
}

/// An external commit is a `PublicMessage` and is refused on the wire format, before
/// `process_incoming`'s own external-sender refusal is even reached.
#[test]
fn an_external_commit_is_refused() {
    let mut f = fixture();
    let epoch = f.bob_group.epoch();
    let outsider = MlsDevice::generate().unwrap();
    let commit = f
        .alice_group
        .test_external_commit(&f.alice, &outsider)
        .expect("openmls must let an outsider build the external commit, or this reaches nothing");
    let result = f.bob_group.apply_commit(&f.bob, &commit, epoch);
    assert_refused(&f, result, CommitRefusal::NotCiphertext, epoch);
}

/// A commit offered under the wrong epoch, either way round, is refused before processing.
#[test]
fn a_commit_for_another_epoch_is_refused() {
    let mut f = fixture();
    let (commit, at) = alice_adds_someone(&mut f, 2);
    // Labelled one ahead of what it is.
    let result = f.bob_group.apply_commit(&f.bob, &commit, at + 1);
    assert_refused(
        &f,
        result,
        CommitRefusal::WrongEpoch {
            expected: at + 1,
            found: at,
            current: at,
        },
        at,
    );
    // A commit built one epoch ahead of us, labelled truthfully: not ours to apply yet.
    let (next, next_at) = alice_adds_someone(&mut f, 3);
    assert_eq!(next_at, at + 1);
    let result = f.bob_group.apply_commit(&f.bob, &next, next_at);
    assert_refused(
        &f,
        result,
        CommitRefusal::WrongEpoch {
            expected: next_at,
            found: next_at,
            current: at,
        },
        at,
    );
    // The real next commit still applies.
    f.bob_group.apply_commit(&f.bob, &commit, at).unwrap();
}

/// An already-merged commit replayed, under its own label or relabelled to the current epoch.
#[test]
fn a_replayed_commit_is_refused() {
    let mut f = fixture();
    let (commit, at) = alice_adds_someone(&mut f, 2);
    f.bob_group.apply_commit(&f.bob, &commit, at).unwrap();
    let now = f.bob_group.epoch();
    let result = f.bob_group.apply_commit(&f.bob, &commit, at);
    assert_refused(
        &f,
        result,
        CommitRefusal::WrongEpoch {
            expected: at,
            found: at,
            current: now,
        },
        now,
    );
    let result = f.bob_group.apply_commit(&f.bob, &commit, now);
    assert_refused(
        &f,
        result,
        CommitRefusal::WrongEpoch {
            expected: now,
            found: at,
            current: now,
        },
        now,
    );
}

/// Our own pending commit echoed back passes the header but is refused by the own-leaf rule, and
/// stays pending.
#[test]
fn our_own_pending_commit_is_refused_and_stays_pending() {
    let mut f = fixture();
    let epoch = f.alice_group.epoch();
    let carol = MlsDevice::generate().unwrap();
    let staged = f
        .alice_group
        .stage_add(
            &f.alice,
            carol
                .key_package_for_invite(&f.alice_group.group_id(), [4; 16])
                .unwrap(),
        )
        .unwrap();
    let echoed = f
        .alice_group
        .apply_commit(&f.alice, &staged.commit, staged.commit_epoch);
    assert!(
        matches!(echoed, Err(MlsError::Protocol(_))),
        "got {echoed:?}"
    );
    assert_eq!(f.alice_group.epoch(), epoch);
    f.alice_group.merge_staged_self(&f.alice).unwrap();
    assert_eq!(f.alice_group.epoch(), epoch + 1);
}

/// Bytes that are not a protocol message, and a commit from another group.
#[test]
fn foreign_and_non_protocol_bytes_are_refused() {
    let mut f = fixture();
    let epoch = f.bob_group.epoch();
    let result = f.bob_group.apply_commit(&f.bob, &[7], epoch);
    assert_refused(&f, result, CommitRefusal::NotProtocolMessage, epoch);

    // A Welcome is an MLS message, but not a protocol message.
    let other = MlsDevice::generate().unwrap();
    let mut other_group = ServerGroup::create(&other).unwrap();
    let dave = MlsDevice::generate().unwrap();
    let out = other_group
        .add_member(
            &other,
            dave.key_package_for_invite(&other_group.group_id(), [5; 16])
                .unwrap(),
        )
        .unwrap();
    let result = f.bob_group.apply_commit(&f.bob, &out.welcome, epoch);
    assert_refused(&f, result, CommitRefusal::NotProtocolMessage, epoch);

    // Another group's commit at a matching epoch number.
    let (foreign, foreign_at) = {
        let eve = MlsDevice::generate().unwrap();
        let out = other_group
            .add_member(
                &other,
                eve.key_package_for_invite(&other_group.group_id(), [6; 16])
                    .unwrap(),
            )
            .unwrap();
        (out.commit, out.commit_epoch)
    };
    assert_eq!(foreign_at, epoch);
    let result = f.bob_group.apply_commit(&f.bob, &foreign, epoch);
    assert_refused(&f, result, CommitRefusal::WrongGroup, epoch);

    // A real commit with bytes appended. openmls's own parse ignores the tail and would apply it;
    // the exact header parse refuses it, and the clean commit still applies.
    let (commit, at) = alice_adds_someone(&mut f, 7);
    let mut padded = commit.clone();
    padded.extend_from_slice(b"tail");
    let result = f.bob_group.apply_commit(&f.bob, &padded, at);
    assert_refused(&f, result, CommitRefusal::NotProtocolMessage, epoch);
    f.bob_group.apply_commit(&f.bob, &commit, at).unwrap();

    // The group still works.
    assert_eq!(
        f.bob_group
            .channel_secret(&f.bob, DocType::Channel, 1)
            .unwrap(),
        f.alice_group
            .channel_secret(&f.alice, DocType::Channel, 1)
            .unwrap()
    );
}

/// Older builds: the frozen openmls 0.8.1 fixtures (`persist/compat_tests.rs`). A member snapshot
/// written by 0.8.1 refuses the 0.8.1 application message on the commit path (without consuming
/// it) and applies the 0.8.1 commit through `apply_commit`, so the gate does not strand state or
/// traffic from builds before it.
#[test]
fn openmls_0_8_1_traffic_passes_the_gate_and_its_application_message_does_not() {
    let (bob, mut group) = crate::persist::restore_server(include_bytes!(
        "../persist/fixtures/openmls-0.8.1/member.snapshot"
    ))
    .unwrap();
    let application = include_bytes!("../persist/fixtures/openmls-0.8.1/member_app.msg");
    let commit = include_bytes!("../persist/fixtures/openmls-0.8.1/member_commit.msg");
    let epoch = group.epoch();
    assert_eq!(commit_header(commit).unwrap().epoch, epoch);

    match group.apply_commit(&bob, application, epoch) {
        Err(MlsError::NotACommit(CommitRefusal::NotCommitContent("application"))) => {}
        other => panic!("expected a header refusal, got {other:?}"),
    }
    match group.process_incoming(&bob, application).unwrap() {
        Incoming::Application(plain) => assert_eq!(plain, b"written by openmls 0.8.1"),
        other => panic!("the 0.8.1 message must still be readable, got {other:?}"),
    }
    let applied = group.apply_commit(&bob, commit, epoch).unwrap();
    assert_eq!(applied, AppliedCommit { removed: false });
    assert_eq!(group.epoch(), epoch + 1);
}
