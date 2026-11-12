//! Real signed Core replay/selection, with an UNTRUSTED in-memory proof exporter. This
//! does not exercise an MLS node, App selection, disk archive or restart factory. The
//! exporter deliberately holds history outside Core, to test the bounded verifier boundary.
use super::*;
use crate::chat::acceptance_tree::{proof_plan::inclusion_ranges, AcceptanceLeaf};
use crate::chat::owner_facts::OwnerFactId;
use crate::chat::owner_header::OwnerStateCommitment;
use std::collections::BTreeMap;

#[derive(Default)]
struct Archive {
    nodes: BTreeMap<(u64, u64), V2SelectedHistoryHashData>,
    records: Vec<(SignedOwnerHeader, SignedOwnerHeader)>,
}
impl Archive {
    fn retain(&mut self, retired: &V2GeneralRetiredData, child: &SignedOwnerHeader) {
        for node in retired.selected_history_nodes_data() {
            assert!(self
                .nodes
                .insert((node.first_data(), node.count_data()), node.hash_data())
                .is_none());
        }
        self.records
            .push((retired.parent_data().header().clone(), child.clone()));
    }
    fn siblings(
        &self,
        tip: V2SelectedHistoryTipData,
        index: u64,
    ) -> Vec<V2SelectedHistoryHashData> {
        inclusion_ranges(tip.count, index)
            .unwrap()
            .ranges()
            .iter()
            .map(|range| {
                let mut nodes = range
                    .complete_nodes_right_to_left()
                    .map(|n| self.nodes[&(n.first(), n.count())]);
                let right = nodes.next().unwrap();
                nodes.fold(right, |right, left| tip.node_hash_data(left, right))
            })
            .collect()
    }
    fn proof<'a>(
        &'a self,
        tip: V2SelectedHistoryTipData,
        index: u64,
        siblings: &'a [V2SelectedHistoryHashData],
    ) -> V2SelectedParentProof<'a> {
        let (parent, selected_child) = &self.records[index as usize];
        V2SelectedParentProof {
            tip,
            index,
            parent,
            selected_child,
            siblings,
        }
    }
}

/// Canonical governing Append mathematics with unique public acceptance labels. These
/// are not App messages or evidence that the corresponding action bodies were received.
fn append_input(f: &Fixture, marker: u64) -> (SignedOwnerHeader, AppendBody, Vec<u8>) {
    let base = f.verifier.replay_data();
    let slot = base.slots().next().unwrap();
    let leaf = AcceptanceLeaf {
        channel: 42,
        acceptance: OwnerFactId::from_bytes(*blake3::hash(&marker.to_be_bytes()).as_bytes()),
    };
    let prepared = slot.frontier.prepare_append(&[leaf]).unwrap();
    let body = AppendBody::new(AppendSpec {
        group: GROUP.to_vec(),
        previous_tip: slot.tip_header,
        old_tip: prepared.old_tip(),
        new_tip: prepared.new_tip(),
        leaves: vec![leaf],
    })
    .unwrap();
    let plan = base.prepare_append_data(&body).unwrap();
    let image = plan.image_data().to_vec();
    (f.owner.header(&image), body, image)
}
fn select_append(f: &mut Fixture, archive: &mut Archive, marker: u64) {
    let (header, body, image) = append_input(f, marker);
    let old_tip = f.verifier.selected_history_tip_data();
    let candidate = f
        .verifier
        .derive_successor(&header, ReplayBody::Append(&body), &image)
        .unwrap();
    assert_eq!(
        candidate.selected_history_append_data().old_tip_data(),
        old_tip
    );
    let expected = candidate.selected_history_append_data().new_tip_data();
    let expected_nodes = candidate
        .selected_history_append_data()
        .created_nodes_data()
        .to_vec();
    let mut candidate = Some(candidate);
    let selected = V2HostSelectionExpectation::from_trusted_host(header.id(), header.spec().state);
    let retired = f
        .verifier
        .advance_host_selected(&selected, &mut candidate)
        .unwrap();
    assert!(candidate.is_none());
    assert_eq!(retired.prior_selected_history_tip_data(), old_tip);
    assert_eq!(retired.selected_history_nodes_data(), expected_nodes);
    assert_eq!(f.verifier.selected_history_tip_data(), expected);
    archive.retain(&retired, &header);
    // The original candidate claim drains only when this whole retired owner drops.
    drop(retired);
}
fn sibling(f: &Fixture, header: &SignedOwnerHeader) -> SignedOwnerHeader {
    let mut spec = header.spec().clone();
    spec.body = OwnerBodyId::from_bytes([219; 32]);
    SignedOwnerHeader::sign_using(spec, |bytes| Ok(f.owner.key.sign(bytes))).unwrap()
}
fn invalid_signature(header: &SignedOwnerHeader) -> SignedOwnerHeader {
    let mut bytes = header.encode();
    *bytes.last_mut().unwrap() ^= 1;
    let invalid = SignedOwnerHeader::decode(&bytes).unwrap();
    assert_eq!(invalid.id(), header.id());
    invalid
}
fn rejected(error: ChatError) -> V2ArchivedHeaderRefusal {
    V2ArchivedHeaderRefusal::Rejected(error)
}

/// Independent recursive reference: explicitly encode domains and recurse over leaf
/// slices, rather than reuse the production frontier, proof verifier or fold helper.
fn reference_root(
    context: &[u8; 32],
    records: &[(SignedOwnerHeader, SignedOwnerHeader)],
) -> V2SelectedHistoryHashData {
    let mut h = blake3::Hasher::new();
    if records.len() == 1 {
        h.update(b"mewtual/chat/selected-history-leaf/v1\0");
        h.update(context);
        h.update(records[0].0.id().as_bytes());
        h.update(records[0].1.id().as_bytes());
    } else {
        let left = 1usize << (usize::BITS - (records.len() - 1).leading_zeros() - 1);
        h.update(b"mewtual/chat/selected-history-node/v1\0");
        h.update(context);
        h.update(reference_root(context, &records[..left]).as_bytes());
        h.update(reference_root(context, &records[left..]).as_bytes());
    }
    V2SelectedHistoryHashData::from_bytes(*h.finalize().as_bytes())
}

#[test]
fn selected_nodes_and_every_small_tree_path_match_independent_roots() {
    let mut f = Fixture::new();
    let mut archive = Archive::default();
    for marker in 0..33 {
        select_append(&mut f, &mut archive, marker);
        let tip = f.verifier.selected_history_tip_data();
        assert_eq!(tip.root, reference_root(&tip.context, &archive.records));
        for index in 0..tip.count {
            let siblings = archive.siblings(tip, index);
            let proof = archive.proof(tip, index, &siblings);
            let child = &archive.records[index as usize].1;
            assert_eq!(
                f.verifier.observe_proven_archived_header(proof, child),
                Ok(())
            );
        }
    }
}

#[test]
fn delayed_sibling_beyond_4096_fences_and_keeps_original_evidence() {
    let mut f = Fixture::new();
    let mut archive = Archive::default();
    let invoice = V2GeneralPrefixVerifier::metadata_layout_bytes();
    let frontier_bytes = std::mem::size_of_val(f.verifier.history.as_ref());
    for marker in 0..4101 {
        select_append(&mut f, &mut archive, marker);
    }
    let tip = f.verifier.selected_history_tip_data();
    assert_eq!(tip.count, 4101);
    assert_eq!(
        std::mem::size_of_val(f.verifier.history.as_ref()),
        frontier_bytes
    );
    assert_eq!(V2GeneralPrefixVerifier::metadata_layout_bytes(), invoice);
    let siblings = archive.siblings(tip, 0);
    let alternate = sibling(&f, &archive.records[0].1);
    assert_eq!(
        f.verifier
            .observe_proven_archived_header(archive.proof(tip, 0, &siblings), &alternate),
        Err(rejected(ChatError::AuthorityConflict))
    );
    let founder = f.verifier.admissions[0].admission_data().admission_id();
    assert!(f.verifier.admission(founder).is_err());
    let (parent, first, second) = f.verifier.archived_contradiction_data().unwrap();
    assert_eq!(parent, &archive.records[0].0);
    assert_eq!(first, &archive.records[0].1);
    assert_eq!(second, &alternate);
    // A lost caller reply or a further closed request cannot erase the cached originals.
    assert!(f
        .verifier
        .observe_proven_archived_header(archive.proof(tip, 0, &siblings), &alternate)
        .is_err());
    assert_eq!(
        f.verifier.archived_contradiction_data().unwrap().2,
        &alternate
    );
    assert_eq!(f.verifier.selected_history_tip_data(), tip);
}

#[test]
fn full_exact_replay_reconstructs_commitment_without_adopting_hot_identity() {
    let mut a = Fixture::new();
    let mut b = Fixture::new();
    let mut archive = Archive::default();
    let mut second_archive = Archive::default();
    for marker in 0..17 {
        select_append(&mut a, &mut archive, marker);
        select_append(&mut b, &mut second_archive, marker);
    }
    assert!(!Arc::ptr_eq(&a.verifier.identity, &b.verifier.identity));
    let tip = a.verifier.selected_history_tip_data();
    assert_eq!(tip, b.verifier.selected_history_tip_data());
    let siblings = archive.siblings(tip, 2);
    assert_eq!(
        b.verifier.observe_proven_archived_header(
            archive.proof(tip, 2, &siblings),
            &archive.records[2].1
        ),
        Ok(())
    );
    // Existing opaque hot-parent escrows remain bound to the original verifier.
    let (header, body, image) = append_input(&a, 99);
    let mut retired = a.advance(&header, ReplayBody::Append(&body), &image);
    let mut hot = retired.take_known_parent_data().unwrap();
    assert_eq!(
        b.verifier.observe_archived_header(&mut hot, &header),
        Err(ChatError::Scope)
    );
}

#[test]
fn foreign_pinned_history_cannot_supply_a_cold_parent() {
    let mut a = Fixture::new();
    let mut b = Fixture::with_owner_seed(92);
    let mut archive = Archive::default();
    let mut other = Archive::default();
    select_append(&mut a, &mut archive, 1);
    select_append(&mut b, &mut other, 1);
    let tip = a.verifier.selected_history_tip_data();
    let original = b.verifier.selected_history_tip_data();
    let siblings = archive.siblings(tip, 0);
    assert_eq!(
        b.verifier.observe_proven_archived_header(
            archive.proof(tip, 0, &siblings),
            &archive.records[0].1
        ),
        Err(rejected(ChatError::Scope))
    );
    assert_eq!(b.verifier.selected_history_tip_data(), original);
    assert!(b.verifier.live().is_ok());
}

#[test]
fn malformed_paths_indices_roots_and_contexts_leave_source_unchanged() {
    let mut f = Fixture::new();
    let mut archive = Archive::default();
    for marker in 0..5 {
        select_append(&mut f, &mut archive, marker);
    }
    let tip = f.verifier.selected_history_tip_data();
    let siblings = archive.siblings(tip, 0);
    let alternate = sibling(&f, &archive.records[0].1);
    for case in 0..6 {
        let mut proof = archive.proof(tip, 0, &siblings);
        let mut changed = siblings.clone();
        match case {
            0 => proof.index = u64::MAX,
            1 => proof.tip.context[0] ^= 1,
            2 => proof.tip.root = V2SelectedHistoryHashData::from_bytes([0; 32]),
            3 => proof.siblings = &[],
            4 => {
                changed[0] = V2SelectedHistoryHashData::from_bytes([0; 32]);
                proof.siblings = &changed;
            }
            5 => {
                changed.resize(65, V2SelectedHistoryHashData::from_bytes([0; 32]));
                proof.siblings = &changed;
            }
            _ => unreachable!(),
        }
        assert!(f
            .verifier
            .observe_proven_archived_header(proof, &alternate)
            .is_err());
        assert!(f.verifier.live().is_ok());
        assert!(f.verifier.archived_contradiction_data().is_none());
        assert_eq!(f.verifier.selected_history_tip_data(), tip);
    }
    assert_eq!(
        f.verifier.observe_proven_archived_header(
            archive.proof(tip, 0, &siblings),
            &archive.records[0].1
        ),
        Ok(())
    );
}

#[test]
fn every_same_id_signature_representation_is_checked_before_duplicate_or_fork() {
    let mut f = Fixture::new();
    let mut archive = Archive::default();
    select_append(&mut f, &mut archive, 1);
    let tip = f.verifier.selected_history_tip_data();
    let siblings = archive.siblings(tip, 0);
    let parent = invalid_signature(&archive.records[0].0);
    let child = invalid_signature(&archive.records[0].1);
    let alternate = sibling(&f, &archive.records[0].1);
    for case in 0..4 {
        let mut proof = archive.proof(tip, 0, &siblings);
        let incoming = match case {
            0 => {
                proof.parent = &parent;
                &archive.records[0].1
            }
            1 => {
                proof.selected_child = &child;
                &archive.records[0].1
            }
            2 => &child,
            3 => {
                proof.selected_child = &child;
                &alternate
            }
            _ => unreachable!(),
        };
        assert_eq!(
            f.verifier.observe_proven_archived_header(proof, incoming),
            Err(rejected(ChatError::Signature))
        );
        assert!(f.verifier.live().is_ok());
        assert!(f.verifier.archived_contradiction_data().is_none());
        assert_eq!(f.verifier.selected_history_tip_data(), tip);
    }
}

#[test]
fn stale_read_is_typed_and_cannot_move_or_close_current_history() {
    let mut f = Fixture::new();
    let mut archive = Archive::default();
    select_append(&mut f, &mut archive, 1);
    let old = f.verifier.selected_history_tip_data();
    let siblings = archive.siblings(old, 0);
    select_append(&mut f, &mut archive, 2);
    let tip = f.verifier.selected_history_tip_data();
    assert_eq!(
        f.verifier.observe_proven_archived_header(
            archive.proof(old, 0, &siblings),
            &archive.records[0].1
        ),
        Err(V2ArchivedHeaderRefusal::StaleTip)
    );
    assert!(f.verifier.live().is_ok());
    assert_eq!(f.verifier.selected_history_tip_data(), tip);
    let mut future = tip;
    future.count = u64::MAX;
    assert_eq!(
        f.verifier.observe_proven_archived_header(
            archive.proof(future, 0, &siblings),
            &archive.records[0].1
        ),
        Err(V2ArchivedHeaderRefusal::StaleTip)
    );
}

#[test]
fn invalid_abandoned_and_refused_candidates_never_append_selected_history() {
    let mut f = Fixture::new();
    let tip = f.verifier.selected_history_tip_data();
    let (header, body, image) = append_input(&f, 1);
    assert!(f
        .verifier
        .derive_successor(&header, ReplayBody::Append(&body), &[])
        .is_err());
    assert_eq!(f.verifier.selected_history_tip_data(), tip);
    let mut candidate = Some(
        f.verifier
            .derive_successor(&header, ReplayBody::Append(&body), &image)
            .unwrap(),
    );
    assert_eq!(f.verifier.selected_history_tip_data(), tip);
    let wrong = V2HostSelectionExpectation::from_trusted_host(
        header.id(),
        OwnerStateCommitment::from_bytes([0; 32]),
    );
    assert!(f
        .verifier
        .advance_host_selected(&wrong, &mut candidate)
        .is_err());
    assert!(candidate.is_some());
    assert_eq!(f.verifier.selected_history_tip_data(), tip);
    drop(candidate);
    assert_eq!(f.verifier.selected_history_tip_data(), tip);
    let mut archive = Archive::default();
    select_append(&mut f, &mut archive, 1);
    assert_eq!(f.verifier.selected_history_tip_data().count, 1);
}

#[test]
fn unselected_valid_sibling_cannot_be_laundered_into_selected_evidence() {
    let mut f = Fixture::new();
    let mut archive = Archive::default();
    select_append(&mut f, &mut archive, 1);
    let tip = f.verifier.selected_history_tip_data();
    let siblings = archive.siblings(tip, 0);
    let alternate = sibling(&f, &archive.records[0].1);
    let mut proof = archive.proof(tip, 0, &siblings);
    proof.selected_child = &alternate;
    assert_eq!(
        f.verifier
            .observe_proven_archived_header(proof, &archive.records[0].1),
        Err(rejected(ChatError::Dependency))
    );
    assert!(f.verifier.live().is_ok());
    assert_eq!(f.verifier.selected_history_tip_data(), tip);
}

#[test]
fn membership_steps_are_committed_without_changing_original_removal_custody() {
    let mut f = Fixture::new();
    let original_parent = f.verifier.replay_data().header().clone();
    let key = DeviceKeypair::from_seed(&[33; 32]);
    let admission = f.add(&key, 80);
    let selected_add = f.verifier.replay_data().header().clone();
    let add_tip = f.verifier.selected_history_tip_data();
    assert_eq!(add_tip.count, 1);
    let mut archive = Archive::default();
    archive.nodes.insert((0, 1), add_tip.root);
    archive
        .records
        .push((original_parent, selected_add.clone()));
    let mut retired = f.remove(&admission);
    let selected_remove = f.verifier.replay_data().header().clone();
    archive.retain(&retired, &selected_remove);
    assert!(!admission.is_open_data());
    assert!(retired.take_frozen_data().is_some());
    drop(retired);
    let tip = f.verifier.selected_history_tip_data();
    assert_eq!(tip.count, 2);
    let siblings = archive.siblings(tip, 0);
    let alternate = sibling(&f, &selected_add);
    assert_eq!(
        f.verifier
            .observe_proven_archived_header(archive.proof(tip, 0, &siblings), &alternate),
        Err(rejected(ChatError::AuthorityConflict))
    );
    assert!(!admission.is_open_data());
}

#[test]
fn outstanding_retired_owner_still_excludes_another_candidate() {
    let mut f = Fixture::new();
    let (header, body, image) = append_input(&f, 1);
    let retired = f.advance(&header, ReplayBody::Append(&body), &image);
    let tip = f.verifier.selected_history_tip_data();
    let (next, next_body, next_image) = append_input(&f, 2);
    assert_eq!(
        f.verifier
            .derive_successor(&next, ReplayBody::Append(&next_body), &next_image)
            .unwrap_err(),
        ChatError::Capacity
    );
    assert_eq!(f.verifier.selected_history_tip_data(), tip);
    drop(retired);
    drop(
        f.verifier
            .derive_successor(&next, ReplayBody::Append(&next_body), &next_image)
            .unwrap(),
    );
    assert_eq!(f.verifier.selected_history_tip_data(), tip);
}
