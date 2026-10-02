use super::*;
use crate::tests::build_members;
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

mod archive;
mod returning;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;

/// The start a **fully observed** tenure reports, asserting it is not merely imported.
///
/// Every case in this module observes transitions directly or drops the tenure tail entirely, so
/// every expectation here predates the v1 import path and must come out `Observed` or `Unknown`. This
/// helper is what stops that assumption being silent: a case that ever starts producing `Imported`
/// fails loudly here instead of quietly satisfying an `Option` comparison. A test that deliberately
/// wants an import asserts on `observed_owner_tenure()` directly.
fn observed_start(node: &Node) -> Option<u64> {
    match node.observed_owner_tenure() {
        ObservedOwnerTenure::Observed(start) => Some(start),
        ObservedOwnerTenure::Unknown => None,
        ObservedOwnerTenure::Imported(start) => {
            panic!("this case must be fully observed, not imported from v1 (start {start})")
        }
    }
}
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(1458)
}
fn restore(bytes: &[u8]) -> Result<Node, SyncError> {
    Node::restore(
        bytes,
        Hub::new().join(PeerId::from_u64(99)),
        rng(),
        Box::new(ManualClock::new(1)),
    )
}

#[tokio::test]
async fn owner_tenure_founder_joiner_and_legacy_snapshot_have_distinct_evidence() {
    let (_, mut nodes, _) = build_members(2).await;
    assert_eq!(observed_start(&nodes[0]), Some(0));
    assert_eq!(observed_start(&nodes[1]), None);
    for node in &mut nodes {
        let before = observed_start(node);
        let snap = node.snapshot().unwrap();
        let mut reopened = restore(&snap).unwrap();
        assert_eq!(observed_start(&reopened), before);
        // Remove all following extensions and the tenure tail to produce the pre-tenure format.
        let policy_tail = crate::group_policy::encode_pin(node.group_policy.as_ref()).len() + 4;
        let tail = node.owner_tenure.encode(&node.group).unwrap().len() + 4;
        let chat_tail = node.durable_chat.encode().unwrap().len() + 4;
        let pending_tail = node.pending_finalization_snapshot().unwrap().len() + 4;
        let legacy = &snap[..snap.len() - pending_tail - chat_tail - policy_tail - tail];
        let mut upgraded = restore(legacy).unwrap();
        assert_eq!(observed_start(&upgraded), None);
        let saved = upgraded.snapshot().unwrap();
        assert_eq!(observed_start(&restore(&saved).unwrap()), None);
        assert_eq!(reopened.snapshot().unwrap().len(), snap.len());
        // Strict partial/new tails cannot degrade to legacy Unknown.
        for extra in 1..tail {
            assert!(restore(&snap[..legacy.len() + extra]).is_err());
        }
    }
}

#[tokio::test]
async fn owner_tenure_unknown_owner_stays_unknown_after_valid_same_owner_adds() {
    let (_, mut nodes, _) = build_members(1).await;
    let snap = nodes[0].snapshot().unwrap();
    let tail = nodes[0].owner_tenure.encode(&nodes[0].group).unwrap().len()
        + 4
        + crate::group_policy::encode_pin(nodes[0].group_policy.as_ref()).len()
        + 4
        + nodes[0].durable_chat.encode().unwrap().len()
        + 4
        + nodes[0].pending_finalization_snapshot().unwrap().len()
        + 4;
    let mut owner = restore(&snap[..snap.len() - tail]).unwrap();
    assert!(owner.is_designated_committer());
    assert_eq!(observed_start(&owner), None);
    let joining = MlsDevice::generate().unwrap();
    let invite = owner.mint_invite([5; 16], u64::MAX, vec![]).unwrap();
    let kp = joining
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    owner
        .admit_now(&invite, &serialize_key_package(&kp).unwrap(), 1)
        .unwrap();
    assert_eq!(owner.epoch(), 1);
    assert_eq!(observed_start(&owner), None);
    assert_eq!(
        observed_start(&restore(&owner.snapshot().unwrap()).unwrap()),
        None
    );
}

#[tokio::test]
async fn owner_tenure_winning_losing_and_inbound_staged_commits_observe_same_transition() {
    let (_, mut nodes, ids) = build_members(4).await;
    let initial: Vec<_> = nodes[0].commit_log.iter().cloned().collect();
    for node in nodes.iter_mut().skip(1) {
        for record in &initial {
            if record.commit_epoch == node.epoch() {
                assert!(node.apply_commit_in_order(record));
            }
        }
        node.config.max_committer_rank = 2;
        node.config.stage_decision_window_ms = 0;
        assert_eq!(observed_start(node), None);
    }
    nodes[1].remove(&ids[0]).await.unwrap();
    nodes[2].remove(&ids[0]).await.unwrap();
    let candidates = [
        nodes[1].pending.as_ref().unwrap().best.clone(),
        nodes[2].pending.as_ref().unwrap().best.clone(),
    ];
    for node in nodes.iter_mut().skip(1) {
        for candidate in &candidates {
            node.contest_commit(candidate.clone());
        }
        assert_eq!(
            observed_start(node),
            None,
            "staging is not an applied transition"
        );
        assert!(node.resolve_pending_if_expired());
        assert_eq!(node.epoch(), 4);
        assert_eq!(node.designated_committer_id(), Some(ids[1]));
        assert_eq!(observed_start(node), Some(4));
        assert_eq!(
            observed_start(&restore(&node.snapshot().unwrap()).unwrap()),
            Some(4)
        );
    }

    // Re-admit the ORIGINAL full A identity into its vacated low leaf. Both the direct Add
    // producer and the ordered inbound path must observe A -> B -> A, not resurrect tenure 0.
    // This test does not attempt to rejoin A's in-process OpenMLS provider into its old group.
    let invite = nodes[1].mint_invite([6; 16], u64::MAX, vec![]).unwrap();
    let kp = nodes[0]
        .device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    nodes[1]
        .admit_now(&invite, &serialize_key_package(&kp).unwrap(), 1000)
        .unwrap();
    assert_eq!(nodes[1].designated_committer_id(), Some(ids[0]));
    assert_eq!(observed_start(&nodes[1]), Some(5));
    let record = nodes[1].commit_log.back().unwrap().clone();
    for node in nodes.iter_mut().skip(2) {
        assert!(node.apply_commit_in_order(&record));
        assert_eq!(observed_start(node), Some(5));
        assert_eq!(
            observed_start(&restore(&node.snapshot().unwrap()).unwrap()),
            Some(5)
        );
    }
}

/// CORE-005 witness, requested by Agent 3: the SAME `DeviceId` rejoining in a LATER commit is a new
/// tenure on every witness, and the tenure that rejoin ends is archived, not resumed.
///
/// This is the two-commit shape, and it is deliberately separate from the one-commit
/// remove-and-re-add that `catcoms-mls`'s M-1 refuses. Here the removal and the re-admission are
/// different commits, M-1 must let both through, and the result must still be unambiguous.
///
/// Agent 3's N17 requires that a v2 record signed in A's FIRST tenure is refused after A returns,
/// which holds only if no node still believes A's first tenure is current.
///
/// **What this proves, and the half it does not.** It proves the two-commit rejoin is ADMITTED (so
/// M-1 is scoped to the one-commit shape), and that every witness - the admitting one and the ones
/// that apply the commit - observes a new start that is neither A's first tenure nor B's.
///
/// It does NOT prove the rejoining device's own view, and for this shape it cannot: a same-identity
/// device made with `MlsDevice::duplicate` carries A's old group in its provider, and
/// `ServerGroup::join` refuses with "A group with this GroupId already exists".
///
/// **That is not a production gap, and the reason is worth keeping beside the test.** The product
/// never rejoins with the same identity: every join and every found mints a fresh `MlsDevice`, so a
/// removed owner returns as a NEW `DeviceId`. Same-key A -> B -> A is reachable only from a modified
/// client, and only the witnesses' view of it matters, which is what this test pins. The product's
/// own returning owner, including its own view, is `returning::a_removed_owner_returns_as_a_new_device_with_a_new_tenure_everywhere`.
#[tokio::test]
async fn owner_tenure_same_device_rejoin_in_a_later_commit_is_a_new_tenure_on_every_witness() {
    let (_hub, mut nodes, ids) = build_members(4).await;
    let initial: Vec<_> = nodes[0].commit_log.iter().cloned().collect();
    for node in nodes.iter_mut().skip(1) {
        for record in &initial {
            if record.commit_epoch == node.epoch() {
                assert!(node.apply_commit_in_order(record));
            }
        }
        node.config.max_committer_rank = 2;
        node.config.stage_decision_window_ms = 0;
    }
    let first_tenure = observed_start(&nodes[0]);
    assert!(
        first_tenure.is_some(),
        "the founder must know its first tenure, or 'differs from it' proves nothing"
    );

    // Commit one: A leaves. B takes office.
    nodes[1].remove(&ids[0]).await.unwrap();
    nodes[2].remove(&ids[0]).await.unwrap();
    let candidates = [
        nodes[1].pending.as_ref().unwrap().best.clone(),
        nodes[2].pending.as_ref().unwrap().best.clone(),
    ];
    for node in nodes.iter_mut().skip(1) {
        for candidate in &candidates {
            node.contest_commit(candidate.clone());
        }
        assert!(node.resolve_pending_if_expired());
        assert_eq!(node.designated_committer_id(), Some(ids[1]));
    }
    let b_tenure = observed_start(&nodes[1]);

    // Commit two, separate from commit one: the SAME A identity is re-admitted. `duplicate` keeps
    // A's signature key, so this is the same `DeviceId`, not a rotated one.
    let returning = nodes[0].device.duplicate().unwrap();
    assert_eq!(returning.device_id(), ids[0]);
    let invite = nodes[1].mint_invite([7; 16], u64::MAX, vec![]).unwrap();
    let kp = returning
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let _ = nodes[1]
        .admit_now(&invite, &serialize_key_package(&kp).unwrap(), 1000)
        .expect("a rejoin in a LATER commit is not the shape M-1 refuses, and must be admitted");
    assert_eq!(
        nodes[1].designated_committer_id(),
        Some(ids[0]),
        "the same DeviceId is the designated committer again"
    );
    let witness_start = observed_start(&nodes[1]);

    // The departing tenure is archived, not resumed - on the admitting witness and on every witness
    // that applies the commit.
    let record = nodes[1].commit_log.back().unwrap().clone();
    let mut starts = vec![("the admitting witness", witness_start)];
    for node in nodes.iter_mut().skip(2) {
        assert!(node.apply_commit_in_order(&record));
        starts.push(("an applying witness", observed_start(node)));
        assert_eq!(
            observed_start(&restore(&node.snapshot().unwrap()).unwrap()),
            witness_start,
            "a witness keeps the new tenure across a save and reload"
        );
    }
    for (who, start) in starts {
        assert!(start.is_some(), "{who} lost track of the tenure entirely");
        assert_eq!(start, witness_start, "{who} disagrees about the new tenure");
        assert_ne!(
            start, first_tenure,
            "{who} resumed A's first tenure; a record signed in it would then still verify"
        );
        assert_ne!(start, b_tenure, "{who} confused A's return with B's tenure");
    }

    // CORE-005, the archive half. The tenure the rejoin commit ENDED is B's, and every witness
    // watched it begin, so every witness archives exactly it.
    //
    // A's FIRST tenure is not archived anywhere among them, and that is the bounded design rather
    // than a gap: these witnesses joined by Welcome and never saw it begin, so for them it was
    // `Unknown`, and an Unknown departure mints nothing.
    let b_key: [u8; 32] = nodes[1].device.public_key_bytes().try_into().unwrap();
    let b_start = b_tenure.unwrap();
    let returned_at = witness_start.unwrap();
    let a_key: [u8; 32] = nodes[0].device.public_key_bytes().try_into().unwrap();
    let group_id = nodes[1].group.group_id();
    for node in nodes.iter_mut().skip(1) {
        let archived = node
            .owner_tenure
            .archived(&node.group)
            .expect("B's tenure was observed start to finish");
        assert_eq!(
            (
                archived.owner_key(),
                archived.start(),
                archived.retired_at()
            ),
            (&b_key, b_start, returned_at)
        );
        assert_eq!(
            restore(&node.snapshot().unwrap())
                .unwrap()
                .owner_tenure
                .archived(&node.group),
            Some(archived),
            "and keeps it across a save and reload"
        );
    }

    // Commit three: A leaves AGAIN. Now the witnesses archive A's SECOND tenure, which they did
    // observe. Same key as the first, different start, so a different tenure id: a pair signed in
    // A's first tenure can never match this witness.
    remove_through_contest_by(&mut nodes, &ids[0]).await;
    for node in &nodes[1..] {
        let archived = node.owner_tenure.archived(&node.group).unwrap();
        assert_eq!(archived.owner_key(), &a_key);
        assert_eq!(archived.start(), returned_at);
        assert_eq!(archived.retired_at(), node.epoch());
        assert_ne!(
            archived.tenure_id(),
            &catcoms_replication::tenure_id(&group_id, &a_key, first_tenure.unwrap()),
            "A's second tenure must not be mistaken for its first"
        );
    }
}

/// Remove `leaving` through the contested path, committed by nodes 1 and 2 and applied by 1..4.
async fn remove_through_contest_by(nodes: &mut [Node], leaving: &DeviceId) {
    nodes[1].remove(leaving).await.unwrap();
    nodes[2].remove(leaving).await.unwrap();
    let candidates = [
        nodes[1].pending.as_ref().unwrap().best.clone(),
        nodes[2].pending.as_ref().unwrap().best.clone(),
    ];
    for node in nodes.iter_mut().skip(1) {
        for candidate in &candidates {
            node.contest_commit(candidate.clone());
        }
        assert!(node.resolve_pending_if_expired());
    }
}

#[tokio::test]
async fn owner_tenure_rejects_malformed_tail_and_unobserved_group_advance() {
    let (_, mut nodes, _) = build_members(1).await;
    let node = &mut nodes[0];
    let bytes = node.owner_tenure.encode(&node.group).unwrap();
    // v2. The tail gained the committer's leaf identity - index plus a digest over index, signature
    // key and credential - and the import flag. Both are load-bearing: the leaf is what lets every
    // participant see the same discontinuity, and the flag is what stops a save-and-reload laundering
    // a v1-imported value into a fully observed one.
    let (leaf_index, leaf_digest) = node.group.designated_committer_leaf().unwrap();
    let mut expected = vec![2];
    expected.extend_from_slice(&0u64.to_be_bytes());
    expected.extend_from_slice(&32u32.to_be_bytes());
    expected.extend_from_slice(node.device.device_id().as_bytes());
    expected.extend_from_slice(&8u32.to_be_bytes());
    expected.extend_from_slice(&0u64.to_be_bytes());
    expected.extend_from_slice(&36u32.to_be_bytes());
    expected.extend_from_slice(&leaf_index.to_be_bytes());
    expected.extend_from_slice(&leaf_digest);
    expected.push(0);
    assert_eq!(bytes, expected, "pin every field and length in the v2 tail");
    assert_eq!(bytes.len(), 98);
    for index in [0, 8, 12, 13, 48, 56, 61, 70] {
        let mut corrupt = bytes.clone();
        corrupt[index] ^= 1;
        assert!(
            OwnerTenure::decode(&corrupt, &node.group).is_err(),
            "corrupting byte {index} must refuse"
        );
    }
    // The import flag is the one byte that does NOT refuse when flipped, and that is correct rather
    // than an oversight: setting it can only DOWNGRADE a fully observed value to an imported one,
    // which removes authoring authority and keeps verification. Refusing would turn a harmless bit
    // flip into an unopenable vault; accepting it fails closed in the direction that matters.
    let mut flagged = bytes.clone();
    flagged[97] ^= 1;
    let imported = OwnerTenure::decode(&flagged, &node.group).expect("a downgrade must decode");
    assert_eq!(
        imported.observed(&node.group),
        ObservedOwnerTenure::Imported(0),
        "the persisted flag must be honoured, not ignored"
    );
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(OwnerTenure::decode(&trailing, &node.group).is_err());
    // A missed mutation hook cannot leak stale tenure evidence, even for the same owner.
    node.group
        .add_member(
            &node.device,
            MlsDevice::generate().unwrap().key_package().unwrap(),
        )
        .unwrap();
    assert_eq!(observed_start(node), None);
    assert!(node.snapshot().is_err());
    assert!(OwnerTenure::decode(&bytes, &node.group).is_err());
}

#[test]
fn owner_tenure_gap_and_noop_never_invent_a_start() {
    let device = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&device).unwrap();
    let mut state = OwnerTenure::unknown(&group);
    let before = Position::of(&group);
    state.applied(before, &group);
    assert_eq!(state.observed(&group), ObservedOwnerTenure::Unknown);
    for _ in 0..2 {
        group
            .add_member(
                &device,
                MlsDevice::generate().unwrap().key_package().unwrap(),
            )
            .unwrap();
    }
    state.applied(before, &group);
    assert_eq!(state.observed(&group), ObservedOwnerTenure::Unknown);
    assert!(state.encode(&group).is_ok());
}

#[tokio::test]
async fn owner_tenure_joining_lowest_leaf_owner_agrees_with_its_witness_and_removal_preserves_known(
) {
    let (hub, mut nodes, ids) = build_members(2).await;
    // The local synchronous Remove path advances MLS without changing the founder's tenure.
    nodes[0].commit_remove_now(&ids[1]);
    assert_eq!(observed_start(&nodes[0]), Some(0));
    assert_eq!(
        observed_start(&restore(&nodes[0].snapshot().unwrap()).unwrap()),
        Some(0)
    );

    // In a separate group, Bob witnesses a succession, then admits a fresh device into the
    // founder's recycled leaf. Existing Bob knows when it happened; the Welcome joiner doesn't.
    let (_, mut nodes, ids) = build_members(2).await;
    nodes[1].config.max_committer_rank = 1;
    nodes[1].config.stage_decision_window_ms = 0;
    nodes[1].remove(&ids[0]).await.unwrap();
    assert!(nodes[1].resolve_pending_if_expired());
    assert_eq!(observed_start(&nodes[1]), Some(2));
    let newcomer = MlsDevice::generate().unwrap();
    let invite = nodes[1].mint_invite([9; 16], u64::MAX, vec![]).unwrap();
    let kp = newcomer
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let (welcome, _, _) = nodes[1]
        .admit_now(&invite, &serialize_key_package(&kp).unwrap(), 1000)
        .unwrap();
    assert_eq!(observed_start(&nodes[1]), Some(3));
    let group = ServerGroup::join(&newcomer, &welcome).unwrap();
    // Only tenure construction is under test. Routing transfer is deliberately absent; no
    // file/network operation is performed through this synthetic transport configuration.
    let mut joined = Node::new_joined(
        hub.join(PeerId::from_u64(99)),
        group,
        newcomer,
        rng(),
        Box::new(ManualClock::new(1000)),
        RoutingState::default(),
    );
    assert!(joined.is_designated_committer());
    // **This expectation is inverted from what it was, and the old one was the bug.**
    //
    // A device that joins into a recycled low leaf becomes the designated committer. With `unknown`
    // it held `None` and could never issue a receipt, while every witness already knew the answer -
    // and the disagreement does not self-correct, because a proof's claimed tenure is accepted when
    // the local value is absent. `joined` infers from current continuous membership: a tenure is an
    // uninterrupted run as committer, so this device's current tenure cannot predate its current
    // membership, which began at this epoch.
    //
    // The property worth asserting is therefore agreement, not ignorance: the joiner and the witness
    // must compute the SAME value.
    assert_eq!(observed_start(&joined), Some(3));
    assert_eq!(
        observed_start(&joined),
        observed_start(&nodes[1]),
        "the joining committer and the admitting witness must agree about the tenure start"
    );
    assert_eq!(
        observed_start(&restore(&joined.snapshot().unwrap()).unwrap()),
        Some(3),
        "and the inference must survive a save and reload"
    );
}

#[tokio::test]
async fn owner_tenure_companion_add_observes_recycled_leaf_only_after_success() {
    let (_, mut nodes, ids) = build_members(2).await;
    let bob = &mut nodes[1];
    bob.config.max_committer_rank = 1;
    bob.config.stage_decision_window_ms = 0;
    bob.remove(&ids[0]).await.unwrap();
    assert!(bob.resolve_pending_if_expired());
    let companion = MlsDevice::generate().unwrap();
    let mut cert = DeviceCertificate {
        origin_id: bob.device.device_id(),
        origin_public_key: bob.device.public_key_bytes().try_into().unwrap(),
        new_device_id: companion.device_id(),
        group_id: bob.group_id(),
        device_name: "companion".into(),
        issued_ts_ms: 1000,
        signature: [0; 64],
    };
    cert.signature = bob
        .device
        .sign(&DeviceCertificate::signing_payload(
            &cert.origin_id,
            &cert.origin_public_key,
            &cert.new_device_id,
            &cert.group_id,
            &cert.device_name,
            cert.issued_ts_ms,
        ))
        .unwrap();
    // The changed boundary is the synchronous post-admission MLS mutation helper; outer
    // device-add authorization is unchanged and covered by its existing endpoint tests.
    let kp = companion
        .key_package_for_invite(&cert.group_id, device_bind_nonce(&cert))
        .unwrap();
    assert!(bob.admit_device_now(&cert, b"bad-key-package").is_none());
    assert_eq!(bob.epoch(), 2);
    assert_eq!(observed_start(bob), Some(2));
    bob.admit_device_now(&cert, &serialize_key_package(&kp).unwrap())
        .unwrap();
    assert_eq!(bob.epoch(), 3);
    assert_eq!(bob.designated_committer_id(), Some(companion.device_id()));
    assert_eq!(observed_start(bob), Some(3));
    assert_eq!(
        observed_start(&restore(&bob.snapshot().unwrap()).unwrap()),
        Some(3)
    );
}

#[tokio::test]
async fn owner_tenure_post_merge_error_observes_actual_state_before_propagation() {
    let (_, mut nodes, _) = build_members(1).await;
    let node = &mut nodes[0];
    let result: Result<(), SyncError> = node.with_observed_mls_transition(|node| {
        node.group
            .add_member(
                &node.device,
                MlsDevice::generate().unwrap().key_package().unwrap(),
            )
            .unwrap();
        // Model the helper's fallible serialization/ledger step AFTER a real successful merge.
        Err(SyncError::Malformed)
    });
    assert!(
        result.is_err(),
        "observation never converts helper failure to success"
    );
    assert_eq!(node.epoch(), 1);
    assert_eq!(observed_start(node), Some(0));
    assert_eq!(
        observed_start(&restore(&node.snapshot().unwrap()).unwrap()),
        Some(0)
    );
    let before = node.snapshot().unwrap();
    let result: Result<(), SyncError> =
        node.with_observed_mls_transition(|_| Err(SyncError::Malformed));
    assert!(result.is_err());
    assert_eq!(
        node.snapshot().unwrap(),
        before,
        "pre-merge failure changes no evidence"
    );
}

/// The leaf-aware arm: a same-owner step whose committer leaf identity changed is a NEW tenure.
///
/// Without it a witness preserves the old start across a remove-and-re-add of the committer while the
/// rejoining device computes a new one, and the two then disagree about who may issue a receipt. The
/// disagreement does not self-correct, because a proof's claimed tenure is accepted when the local
/// value is absent.
///
/// Driven at the `Position`/`applied` level because the commit shape that produces it inside ONE
/// commit is exactly what `catcoms-mls`'s M-1 now refuses; this arm has to keep working for the shapes
/// M-1 permits, and the two rules protect different things.
#[tokio::test]
async fn owner_tenure_same_owner_on_a_new_leaf_identity_starts_a_new_tenure() {
    let (_, mut nodes, _) = build_members(2).await;
    let node = &mut nodes[0];
    assert!(
        node.epoch() >= 1,
        "the step below needs a predecessor epoch"
    );

    // Drive `applied` for real. `before` describes the predecessor epoch with a DIFFERENT committer
    // leaf identity; the live group is `after`. Fabricating the group itself is not possible, and
    // `before` is exactly the value the production seam captures, so this is the honest half to vary.
    let live = Position::of(&node.group);
    let (index, digest) = live.leaf.expect("a group has a committer");
    let mut other = digest;
    other[0] ^= 0xff;
    let before = Position {
        owner: live.owner,
        leaf: Some((index, other)),
        owner_key: live.owner_key,
        epoch: live.epoch - 1,
    };

    // Start from a state that KNOWS an older tenure, so preserving is the visible alternative.
    let mut state = OwnerTenure::unknown(&node.group);
    state.position = before;
    state.start = Some(0);
    state.applied(before, &node.group);
    assert_eq!(
        state.observed(&node.group),
        ObservedOwnerTenure::Observed(live.epoch),
        "a changed leaf identity under the same owner is a new tenure, not preserved knowledge"
    );

    // Guard the guard: the SAME leaf identity across the same step preserves knowledge instead. This
    // is the self-update case, and it is why the HPKE encryption key is excluded from the digest.
    let mut preserved = OwnerTenure::unknown(&node.group);
    let same = Position {
        owner: live.owner,
        leaf: live.leaf,
        owner_key: live.owner_key,
        epoch: live.epoch - 1,
    };
    preserved.position = same;
    preserved.start = Some(0);
    preserved.applied(same, &node.group);
    assert_eq!(
        preserved.observed(&node.group),
        ObservedOwnerTenure::Observed(0),
        "an unchanged leaf identity must preserve the tenure it already knew"
    );
}

/// The v1 migration. Only `start == epoch` is provably safe; everything else is imported.
#[tokio::test]
async fn owner_tenure_v1_snapshots_promote_only_the_provably_safe_shape() {
    // A group at epoch > 0. The previous version of this test used `ServerGroup::create`, which sits
    // at epoch 0, so its `start < epoch` case was guarded by `if epoch > 0` and **never ran** - the
    // entire Imported path had no executable coverage, and a mutation forcing every v1 record to
    // `Observed` passed the suite. The review caught it.
    let (_, mut nodes, _) = build_members(2).await;
    let node = &mut nodes[0];
    let device = &node.device;
    let group = &node.group;
    let epoch = group.epoch();
    assert!(
        epoch > 0,
        "this fixture must have advanced, or the unsafe case is unreachable again"
    );

    // A v1 tail is: [1][u64 epoch][owner bytes][start bytes]. Built here rather than by encoding,
    // because this build no longer writes v1.
    let v1 = |start: Option<u64>| {
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_u64(epoch);
        e.put_bytes(device.device_id().as_bytes()).unwrap();
        e.put_bytes(&start.map_or_else(Vec::new, |s| s.to_be_bytes().to_vec()))
            .unwrap();
        e.finish()
    };

    // start == epoch: the most recent applied step was a genuine owner change, visible under BOTH the
    // old and the new rule, so no hidden discontinuity can lie at that step. Promoted.
    let promoted = OwnerTenure::decode(&v1(Some(epoch)), group).expect("a safe v1 must decode");
    assert_eq!(
        promoted.observed(group),
        ObservedOwnerTenure::Observed(epoch),
        "start == epoch is provably safe and must be fully observed"
    );

    // No start at all: nothing to migrate.
    let unknown = OwnerTenure::decode(&v1(None), group).expect("a v1 with no start must decode");
    assert_eq!(unknown.observed(group), ObservedOwnerTenure::Unknown);

    // A start BELOW the epoch means at least one preserve step, which is exactly where an invisible
    // discontinuity hides. Imported: still verifies, refuses to author. Discarding it instead would
    // downgrade every existing server's owner, which on a single-owner server never recovers.
    let mut imported = OwnerTenure::decode(&v1(Some(epoch - 1)), group).unwrap();
    assert_eq!(
        imported.observed(group),
        ObservedOwnerTenure::Imported(epoch - 1)
    );

    // **And the import must not launder itself away on an ordinary commit.** The preserve arm carries
    // the start forward rather than deriving one, so treating "a start is present" as "a start is
    // fresh" promoted every imported server on its next same-owner step - which is precisely the
    // authority the migration withheld, handed back by an unrelated member add. A review demonstrated
    // it; this is the regression.
    //
    // The step has to REACH the preserve arm to prove anything. An earlier version of this passed
    // `Position::of(group)` as `before`, so `applied` returned at its own `if after == before`
    // no-op guard and never touched the flag: a review showed that mutating the flag line to
    // `self.imported = false` left this and every other sync test green. The synthetic `before` one
    // epoch back with the same owner and the same leaf is what the preserve arm actually looks
    // like, and it is the shape an ordinary member add produces.
    let live = Position::of(group);
    let before = Position {
        owner: live.owner,
        leaf: live.leaf,
        owner_key: live.owner_key,
        epoch: live.epoch - 1,
    };
    imported.position = before;
    imported.applied(before, group);
    assert_eq!(
        imported.observed(group),
        ObservedOwnerTenure::Imported(epoch - 1),
        "a step that only PRESERVES a start must not promote an imported tenure"
    );

    // It must also survive a round trip, or a save and reload would do the laundering instead.
    let bytes = imported.encode(group).unwrap();
    assert_eq!(
        OwnerTenure::decode(&bytes, group).unwrap().observed(group),
        ObservedOwnerTenure::Imported(epoch - 1)
    );
}

/// The accessor split, which had no test: swapping authoring for verification at the signing site
/// passed the entire suite.
///
/// The two have **opposite failure directions**, so a single `Option` could not serve both.
/// Verification wants the value present wherever it is sound, because
/// `complete_checkpoint_head_scoped` accepts a proof's own claimed tenure when the local value is
/// absent - so surfacing an imported one can only add refusals. Authoring wants it absent unless
/// fully observed, because signing as the owner on a tenure this build cannot verify is the failure
/// the distinction exists to prevent.
///
/// An `Imported` node is the only state where they differ, which is why this needs the v1 migration
/// to construct one.
#[tokio::test]
async fn owner_tenure_imported_verifies_but_cannot_author() {
    let (_, mut nodes, _) = build_members(2).await;
    let node = &mut nodes[0];
    let epoch = node.group.epoch();
    assert!(epoch > 0);

    // A v1 tail with `start < epoch`: the shape whose continuity this build cannot establish.
    let mut e = Encoder::new();
    e.put_u8(1);
    e.put_u64(epoch);
    e.put_bytes(node.device.device_id().as_bytes()).unwrap();
    e.put_bytes((epoch - 1).to_be_bytes().as_ref()).unwrap();
    node.owner_tenure = OwnerTenure::decode(&e.finish(), &node.group).unwrap();

    assert_eq!(
        node.observed_owner_tenure(),
        ObservedOwnerTenure::Imported(epoch - 1)
    );
    assert_eq!(
        node.verification_owner_tenure_start(),
        Some(epoch - 1),
        "verification must see it: hiding it would let a proof's own claim be accepted instead"
    );
    assert_eq!(
        node.authoring_owner_tenure_start(),
        None,
        "authoring must not: this build cannot verify the tenure it would be signing under"
    );

    // And the two agree again once the tenure is fully observed, so the split is about evidence
    // quality rather than a permanent divergence.
    let observed = ObservedOwnerTenure::Observed(epoch);
    let _ = observed;
    node.owner_tenure = OwnerTenure::new(&node.group);
    if node.observed_owner_tenure() != ObservedOwnerTenure::Unknown {
        assert_eq!(
            node.verification_owner_tenure_start(),
            node.authoring_owner_tenure_start(),
            "a fully observed tenure must look the same to both consumers"
        );
    }
}
