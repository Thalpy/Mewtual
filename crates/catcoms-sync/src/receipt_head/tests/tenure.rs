//! The member-side tenure gate on a fresh owner proof (`complete_checkpoint_head_scoped`).
//!
//! A proof repeats its receipt's tenure start, and the owner signs it fresh for the requester's
//! nonce. So when the same device owns in two separate tenures, it can prove a receipt from its
//! FIRST tenure perfectly well: right key, designated committer, fresh nonce, and a start that is
//! not in the future. The honest serve path refuses to do that (it proves only under its own
//! authoring tenure). The member's check is what stops a modified or stale owner that does it
//! anyway: a member that observed the second tenure begin refuses any proof claiming another start.
//!
//! Review 2's M-1 asked for this gate to be pinned. The repeated-tenure actor test pins
//! `Receipt::verify_current_owner`'s tenure comparison, but production adoption never reaches that
//! comparison with an observed value: it is handed the proof's claim, gated here first.
use super::*;
use catcoms_mls::serialize_key_package;
use catcoms_replication::ReceiptHeadProof;

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

fn signed(node: &Node, tenure_start: u64) -> Receipt {
    Receipt::sign(
        registry_document(&node.group.group_id(), 4).unwrap(),
        0,
        [7; 32],
        [8; 32],
        tenure_start,
        InheritedCheckpoint::EpochZero,
        &node.device,
    )
    .unwrap()
}

/// B -> A' -> B, with C a witness throughout. B's first-tenure receipt, proved fresh by B in its
/// second tenure, is refused by C, while B's second-tenure receipt proved the same way is accepted.
#[tokio::test]
async fn a_member_refuses_a_fresh_proof_of_the_same_owners_earlier_tenure_receipt() {
    let (_hub, mut nodes, ids) = build_members(3).await;
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

    // --- B's first tenure, and a receipt B signs in it.
    remove_through_contest(&mut nodes, &ids[0]).await;
    assert_eq!(nodes[1].group.designated_committer(), Some(ids[1]));
    let first = nodes[1].authoring_owner_tenure_start().unwrap();
    let r_first = signed(&nodes[1], first);

    // --- A' (a fresh device, as the product makes one) takes the vacated lowest leaf and owns.
    let a_prime = MlsDevice::generate().unwrap();
    let a_prime_id = a_prime.device_id();
    let invite = nodes[1].mint_invite([8; 16], u64::MAX, vec![]).unwrap();
    let kp = a_prime
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    nodes[1]
        .admit_now(&invite, &serialize_key_package(&kp).unwrap(), 1000)
        .unwrap();
    let record = nodes[1].commit_log.back().unwrap().clone();
    assert!(nodes[2].apply_commit_in_order(&record));
    assert_eq!(nodes[2].group.designated_committer(), Some(a_prime_id));

    // --- B owns again: B and C remove A'. B is the lowest occupied leaf once more.
    remove_through_contest(&mut nodes, &a_prime_id).await;
    assert_eq!(nodes[1].group.designated_committer(), Some(ids[1]));
    let second = nodes[1].authoring_owner_tenure_start().unwrap();
    assert!(second > first, "B's second tenure is a new tenure");
    assert_eq!(
        nodes[2].verification_owner_tenure_start(),
        Some(second),
        "C observed B's second start"
    );
    // The receipt is otherwise valid: under its own claimed start it verifies at C. Only the
    // start C observed tells the two tenures apart.
    assert!(r_first.verify_current_owner(&nodes[2].group, first).is_ok());

    let (_, rest) = nodes.split_at_mut(1);
    let (b, c) = rest.split_at_mut(1);
    let (owner, member) = (&mut b[0], &mut c[0]);
    member.promote_member_peer_bound(owner.local_peer(), ids[1], true);
    owner.watch_registry_head(4);

    // B, modified, proves its first-tenure receipt fresh for C's nonce. C refuses it.
    let r = r_first.clone();
    let refused = spoof(member, owner, move |provider, pending| ReceiptHeadAnswer {
        proof: Some(
            ReceiptHeadProof::sign(&r, pending.requester, pending.nonce, &provider.device).unwrap(),
        ),
        receipt: Some(r),
        repair: None,
    })
    .await;
    assert!(
        matches!(refused, Err(SyncError::Unauthorized)),
        "a proof claiming the owner's earlier tenure is refused by a member that observed the later \
         one: {refused:?}"
    );

    // The positive oracle: the same path, the same key and a fresh proof, under B's second
    // tenure, is accepted. The refusal above is the tenure, not the path.
    let r_second = signed(owner, second);
    let accepted = spoof(member, owner, move |provider, pending| ReceiptHeadAnswer {
        proof: Some(
            ReceiptHeadProof::sign(
                &r_second,
                pending.requester,
                pending.nonce,
                &provider.device,
            )
            .unwrap(),
        ),
        receipt: Some(r_second),
        repair: None,
    })
    .await;
    assert!(
        matches!(accepted, Ok(Some(_))),
        "a fresh proof of the current tenure's receipt is accepted: {accepted:?}"
    );
}
