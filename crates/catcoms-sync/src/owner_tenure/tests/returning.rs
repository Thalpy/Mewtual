//! The returning owner as the product actually produces one.
//!
//! Every product join mints a fresh `MlsDevice` (the desktop's join and found paths both call
//! `MlsDevice::generate()`), so an owner who is removed and comes back is a NEW `DeviceId`, A',
//! never the same key. Same-key A -> B -> A is reachable only from a modified client, and is
//! covered elsewhere: M-1 refuses its one-commit form on the receive path and the leaf-aware rule
//! makes its later-commit form a new tenure. This is the form a real user reaches.
//!
//! What it shows, end to end with real MLS and real receipts:
//!
//! - A' joins by Welcome into the vacated lowest leaf and is the designated committer;
//! - A' itself and every witness hold the SAME new tenure start, distinct from A's and from B's,
//!   and keep it across a save and reload;
//! - A' can author: it holds the authoring tenure and mints a durable owner permit;
//! - a receipt A' signs in the new tenure verifies on every witness and on a newcomer;
//! - a receipt from A's first tenure is refused everywhere, and so is A's old key claiming the new
//!   tenure.
use super::*;
use catcoms_replication::{InheritedCheckpoint, LogicalDocument, Receipt};
use catcoms_wire::DocType;

fn document(group_id: Vec<u8>) -> LogicalDocument {
    LogicalDocument {
        server_id: group_id,
        doc_type: DocType::ChannelIndex,
        logical_key: b"returning-owner".to_vec(),
    }
}

fn sign(document: &LogicalDocument, tenure_start: u64, owner: &MlsDevice) -> Receipt {
    Receipt::sign(
        document.clone(),
        0,
        [1; 32],
        [2; 32],
        tenure_start,
        InheritedCheckpoint::EpochZero,
        owner,
    )
    .unwrap()
}

/// Remove `leaving` through the real contested-commit path, committed by nodes 1 and 2.
async fn remove_through_contest(nodes: &mut [Node], leaving: &DeviceId) {
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
async fn a_removed_owner_returns_as_a_new_device_with_a_new_tenure_everywhere() {
    let (hub, mut nodes, ids) = build_members(3).await;
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
    let doc = document(nodes[0].group_id());

    // A's founding tenure, and a receipt A signs in it.
    assert_eq!(observed_start(&nodes[0]), Some(0));
    let first = sign(&doc, 0, &nodes[0].device);
    assert!(first.verify_current_owner(&nodes[0].group, 0).is_ok());

    // A is removed. B takes office, observed by the witnesses.
    remove_through_contest(&mut nodes, &ids[0]).await;
    assert_eq!(nodes[1].designated_committer_id(), Some(ids[1]));
    let b_start = observed_start(&nodes[1]).unwrap();

    // A comes back the way the product brings it back: a fresh device, admitted by invite.
    let returning_device = MlsDevice::generate().unwrap();
    assert_ne!(returning_device.device_id(), ids[0]);
    let invite = nodes[1].mint_invite([8; 16], u64::MAX, vec![]).unwrap();
    let kp = returning_device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let (welcome, _, _) = nodes[1]
        .admit_now(&invite, &serialize_key_package(&kp).unwrap(), 1000)
        .unwrap();
    let record = nodes[1].commit_log.back().unwrap().clone();
    assert!(nodes[2].apply_commit_in_order(&record));
    let group = ServerGroup::join(&returning_device, &welcome).unwrap();
    let mut returning = Node::new_joined(
        hub.join(PeerId::from_u64(50)),
        group,
        returning_device,
        rng(),
        Box::new(ManualClock::new(1000)),
        RoutingState::default(),
    );
    assert!(
        returning.is_designated_committer(),
        "precondition: A' must land in the vacated lowest leaf, or it is not the returning owner"
    );

    // One new tenure, held identically by A' and every witness, across a save and reload.
    let start = observed_start(&returning).expect("A' knows its own tenure began at its join");
    for (who, node) in [("B", &nodes[1]), ("C", &nodes[2])] {
        assert_eq!(
            observed_start(node),
            Some(start),
            "{who} disagrees with A' about the new tenure"
        );
    }
    assert_ne!(start, 0, "A' must not resume A's founding tenure");
    assert_ne!(start, b_start, "A' must not inherit B's tenure");
    let (_, witnesses) = nodes.split_at_mut(1);
    for node in std::iter::once(&mut returning).chain(witnesses.iter_mut()) {
        let saved = node.snapshot().unwrap();
        assert_eq!(observed_start(&restore(&saved).unwrap()), Some(start));
    }

    // A' can author under it: the authoring accessor and a durable owner permit.
    assert_eq!(returning.authoring_owner_tenure_start(), Some(start));
    returning
        .prepare_receipt_head_snapshot(|_, _| Ok::<(), ()>(()))
        .unwrap()
        .expect("the returning owner must be able to mint its durable owner permit");

    // A newcomer who joins afterwards, and so never observed any tenure begin.
    let newcomer_device = MlsDevice::generate().unwrap();
    let invite = returning.mint_invite([9; 16], u64::MAX, vec![]).unwrap();
    let kp = newcomer_device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let (welcome, _, _) = returning
        .admit_now(&invite, &serialize_key_package(&kp).unwrap(), 2000)
        .unwrap();
    let record = returning.commit_log.back().unwrap().clone();
    for node in nodes.iter_mut().skip(1) {
        assert!(node.apply_commit_in_order(&record));
    }
    let newcomer_group = ServerGroup::join(&newcomer_device, &welcome).unwrap();
    assert_eq!(
        observed_start(&returning),
        Some(start),
        "admitting a member is not a tenure change"
    );

    // The new tenure's receipt verifies everywhere; the old tenure's, and A's old key claiming the
    // new tenure, verify nowhere. The newcomer has no local value, so it is given each receipt's
    // own claim, which is exactly the case a stale receipt would try to exploit.
    let fresh = sign(&doc, start, &returning.device);
    let replayed = sign(&doc, start, &nodes[0].device);
    // The tenure dimension on its own: the RIGHT key claiming a wrong start. Without these, every
    // refusal below would be explained by A's old key alone, and a witness holding the wrong start
    // would go unnoticed by the receipt checks.
    for wrong in [0, b_start] {
        let misdated = sign(&doc, wrong, &returning.device);
        for (who, group) in [
            ("A'", &returning.group),
            ("B", &nodes[1].group),
            ("C", &nodes[2].group),
        ] {
            assert!(
                misdated.verify_current_owner(group, start).is_err(),
                "{who} must refuse A' claiming the tenure that began at {wrong}"
            );
        }
    }
    let witnesses = [
        ("A'", &returning.group, Some(start)),
        ("B", &nodes[1].group, Some(start)),
        ("C", &nodes[2].group, Some(start)),
        ("newcomer", &newcomer_group, None),
    ];
    for (who, group, local) in witnesses {
        assert!(
            fresh
                .verify_current_owner(group, local.unwrap_or(start))
                .is_ok(),
            "{who} must accept the returning owner's receipt"
        );
        assert!(
            first
                .verify_current_owner(group, local.unwrap_or(0))
                .is_err(),
            "{who} must refuse a receipt from A's first tenure"
        );
        assert!(
            replayed
                .verify_current_owner(group, local.unwrap_or(start))
                .is_err(),
            "{who} must refuse A's old key claiming the new tenure"
        );
    }
}
