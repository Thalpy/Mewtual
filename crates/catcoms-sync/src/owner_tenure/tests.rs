use super::*;
use crate::tests::build_members;
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
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
    assert_eq!(nodes[0].observed_owner_tenure_start(), Some(0));
    assert_eq!(nodes[1].observed_owner_tenure_start(), None);
    for node in &mut nodes {
        let before = node.observed_owner_tenure_start();
        let snap = node.snapshot().unwrap();
        let mut reopened = restore(&snap).unwrap();
        assert_eq!(reopened.observed_owner_tenure_start(), before);
        // Remove only the new length-framed tail to produce the exact previous snapshot format.
        let tail = node.owner_tenure.encode(&node.group).unwrap().len() + 4;
        let legacy = &snap[..snap.len() - tail];
        let mut upgraded = restore(legacy).unwrap();
        assert_eq!(upgraded.observed_owner_tenure_start(), None);
        let saved = upgraded.snapshot().unwrap();
        assert_eq!(restore(&saved).unwrap().observed_owner_tenure_start(), None);
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
    let tail = nodes[0].owner_tenure.encode(&nodes[0].group).unwrap().len() + 4;
    let mut owner = restore(&snap[..snap.len() - tail]).unwrap();
    assert!(owner.is_designated_committer());
    assert_eq!(owner.observed_owner_tenure_start(), None);
    let joining = MlsDevice::generate().unwrap();
    let invite = owner.mint_invite([5; 16], u64::MAX, vec![]).unwrap();
    let kp = joining
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    owner
        .admit_now(&invite, &serialize_key_package(&kp).unwrap(), 1)
        .unwrap();
    assert_eq!(owner.epoch(), 1);
    assert_eq!(owner.observed_owner_tenure_start(), None);
    assert_eq!(
        restore(&owner.snapshot().unwrap())
            .unwrap()
            .observed_owner_tenure_start(),
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
        assert_eq!(node.observed_owner_tenure_start(), None);
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
            node.observed_owner_tenure_start(),
            None,
            "staging is not an applied transition"
        );
        assert!(node.resolve_pending_if_expired());
        assert_eq!(node.epoch(), 4);
        assert_eq!(node.designated_committer_id(), Some(ids[1]));
        assert_eq!(node.observed_owner_tenure_start(), Some(4));
        assert_eq!(
            restore(&node.snapshot().unwrap())
                .unwrap()
                .observed_owner_tenure_start(),
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
    assert_eq!(nodes[1].observed_owner_tenure_start(), Some(5));
    let record = nodes[1].commit_log.back().unwrap().clone();
    for node in nodes.iter_mut().skip(2) {
        assert!(node.apply_commit_in_order(&record));
        assert_eq!(node.observed_owner_tenure_start(), Some(5));
        assert_eq!(
            restore(&node.snapshot().unwrap())
                .unwrap()
                .observed_owner_tenure_start(),
            Some(5)
        );
    }
}

#[tokio::test]
async fn owner_tenure_rejects_malformed_tail_and_unobserved_group_advance() {
    let (_, mut nodes, _) = build_members(1).await;
    let node = &mut nodes[0];
    let bytes = node.owner_tenure.encode(&node.group).unwrap();
    assert_eq!(bytes.len(), 57);
    let mut expected = vec![1];
    expected.extend_from_slice(&0u64.to_be_bytes());
    expected.extend_from_slice(&32u32.to_be_bytes());
    expected.extend_from_slice(node.device.device_id().as_bytes());
    expected.extend_from_slice(&8u32.to_be_bytes());
    expected.extend_from_slice(&0u64.to_be_bytes());
    assert_eq!(bytes, expected, "pin every field and length in the v1 tail");
    for index in [0, 8, 12, 13, 48, 56] {
        let mut corrupt = bytes.clone();
        corrupt[index] ^= 1;
        assert!(OwnerTenure::decode(&corrupt, &node.group).is_err());
    }
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
    assert_eq!(node.observed_owner_tenure_start(), None);
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
    assert_eq!(state.start(&group), None);
    for _ in 0..2 {
        group
            .add_member(
                &device,
                MlsDevice::generate().unwrap().key_package().unwrap(),
            )
            .unwrap();
    }
    state.applied(before, &group);
    assert_eq!(state.start(&group), None);
    assert!(state.encode(&group).is_ok());
}

#[tokio::test]
async fn owner_tenure_new_lowest_leaf_owner_is_unknown_and_ordinary_removal_preserves_known() {
    let (hub, mut nodes, ids) = build_members(2).await;
    // The local synchronous Remove path advances MLS without changing the founder's tenure.
    nodes[0].commit_remove_now(&ids[1]);
    assert_eq!(nodes[0].observed_owner_tenure_start(), Some(0));
    assert_eq!(
        restore(&nodes[0].snapshot().unwrap())
            .unwrap()
            .observed_owner_tenure_start(),
        Some(0)
    );

    // In a separate group, Bob witnesses a succession, then admits a fresh device into the
    // founder's recycled leaf. Existing Bob knows when it happened; the Welcome joiner doesn't.
    let (_, mut nodes, ids) = build_members(2).await;
    nodes[1].config.max_committer_rank = 1;
    nodes[1].config.stage_decision_window_ms = 0;
    nodes[1].remove(&ids[0]).await.unwrap();
    assert!(nodes[1].resolve_pending_if_expired());
    assert_eq!(nodes[1].observed_owner_tenure_start(), Some(2));
    let newcomer = MlsDevice::generate().unwrap();
    let invite = nodes[1].mint_invite([9; 16], u64::MAX, vec![]).unwrap();
    let kp = newcomer
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let (welcome, _, _) = nodes[1]
        .admit_now(&invite, &serialize_key_package(&kp).unwrap(), 1000)
        .unwrap();
    assert_eq!(nodes[1].observed_owner_tenure_start(), Some(3));
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
    assert_eq!(joined.observed_owner_tenure_start(), None);
    assert_eq!(
        restore(&joined.snapshot().unwrap())
            .unwrap()
            .observed_owner_tenure_start(),
        None
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
    assert_eq!(bob.observed_owner_tenure_start(), Some(2));
    bob.admit_device_now(&cert, &serialize_key_package(&kp).unwrap())
        .unwrap();
    assert_eq!(bob.epoch(), 3);
    assert_eq!(bob.designated_committer_id(), Some(companion.device_id()));
    assert_eq!(bob.observed_owner_tenure_start(), Some(3));
    assert_eq!(
        restore(&bob.snapshot().unwrap())
            .unwrap()
            .observed_owner_tenure_start(),
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
    assert_eq!(node.observed_owner_tenure_start(), Some(0));
    assert_eq!(
        restore(&node.snapshot().unwrap())
            .unwrap()
            .observed_owner_tenure_start(),
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
