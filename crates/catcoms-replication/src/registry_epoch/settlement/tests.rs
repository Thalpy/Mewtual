use super::*;
use crate::registry::{PointerKey, RegistryOp};
use crate::InheritedCheckpoint;
use automerge::transaction::{CommitOptions, Transactable};
use automerge::ROOT;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(in crate::registry_epoch) fn fill_close(
    source: &mut RegistryEpoch,
    owner: &MlsDevice,
    group: &ServerGroup,
    key: &PointerKey,
    rng: &mut ChaCha20Rng,
) {
    // Clone the current projection WITH its seed for rotated-source tests. The writer and
    // checked source remain separate; every authored change must still pass normal ingest.
    let mut writer = source.doc.doc().clone();
    writer.set_actor(automerge::ActorId::from(
        owner.device_id().as_bytes().to_vec(),
    ));
    for n in 0..10u8 {
        let domain = RegistryOp::Put {
            key: key.clone(),
            epoch: u64::from(n),
        }
        .domain_op(&group.group_id(), [n; 16])
        .unwrap();
        writer.put(ROOT, "bucket", u64::from(key.bucket())).unwrap();
        writer.put(ROOT, "epoch", source.epoch()).unwrap();
        writer
            .put(ROOT, "key", hex(&source.logical.logical_key))
            .unwrap();
        writer.put(ROOT, "kind", "registry").unwrap();
        writer.put(ROOT, "v", 1u64).unwrap();
        writer
            .put(
                ROOT,
                format!("p/0010/{}", hex(key.logical_key())),
                u64::from(n),
            )
            .unwrap();
        writer
            .put(
                ROOT,
                format!("_p1/op/{}", hex(&domain.id(&owner.device_id()))),
                1u64,
            )
            .unwrap();
        // Real signed bytes reach the 2-MiB close threshold; no test-only admission bypass.
        writer.commit_with(CommitOptions::default().with_message("x".repeat(220_000)));
        let signed = SignedOp::sign_domain(
            owner,
            DocType::DocRegistry,
            source.doc_id(),
            writer.get_last_local_change().unwrap().raw_bytes().to_vec(),
            &domain,
        )
        .unwrap();
        let sealed = SealedOp::seal(&signed, group, owner, rng).unwrap();
        assert_eq!(
            source.ingest(&sealed, group, owner).unwrap(),
            Admission::Accepted
        );
    }
}

struct Fixture {
    owner: MlsDevice,
    group: ServerGroup,
    key: PointerKey,
    source: RegistryEpoch,
    close: CloseRecord,
    receipt: Receipt,
    rng: ChaCha20Rng,
}

impl Fixture {
    fn new() -> Self {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let key = PointerKey::new(DocType::StudioObject, b"private-pointer".to_vec()).unwrap();
        let mut source = RegistryEpoch::new(&group, key.bucket(), owner.device_id()).unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(731);
        fill_close(&mut source, &owner, &group, &key, &mut rng);
        let close = CloseRecord::sign(
            &source.logical,
            source.doc_id(),
            0,
            source.doc.heads(),
            &owner,
        )
        .unwrap();
        let (seed, _) = checkpoint_registry_close(
            &mut source.doc,
            &source.gate,
            key.bucket(),
            &close,
            &group,
            None,
        )
        .unwrap();
        let receipt = Receipt::sign(
            source.logical.clone(),
            0,
            close.hash(),
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &owner,
        )
        .unwrap();
        Self {
            owner,
            group,
            key,
            source,
            close,
            receipt,
            rng,
        }
    }

    fn edit(&mut self, n: u8) -> DomainOp {
        let domain = RegistryOp::Put {
            key: self.key.clone(),
            epoch: u64::from(n),
        }
        .domain_op(&self.group.group_id(), [n; 16])
        .unwrap();
        self.source
            .edit(&self.owner, &self.group, &mut self.rng, &domain)
            .unwrap();
        domain
    }

    fn seal(&mut self) {
        self.source
            .seal(self.receipt.clone(), &self.group, 0)
            .unwrap();
    }

    fn plan(&mut self) -> Result<RegistrySettlementPlan, ReplError> {
        self.source.prepare_settlement(&self.close, &self.group, 0)
    }
}

#[test]
fn registry_adoption_mode_cannot_use_ordinary_settlement_even_at_the_same_epoch() {
    let mut f = Fixture::new();
    f.seal();
    let ordinary = f.plan().unwrap();
    assert_eq!(
        f.source
            .begin_checkpoint_adoption(f.receipt.clone(), &f.group, 0)
            .unwrap(),
        ReceiptIngest::Duplicate
    );
    assert!(f.source.adopting);
    assert_eq!(f.source.epoch(), f.receipt.closed_epoch);
    assert!(f.plan().is_err());
    assert!(f
        .source
        .checkpoint_successor(&ordinary, &f.group, 0)
        .is_err());
    // Adoption requires whole-version recovery even if this receipt would cover every op.
    let plan = f
        .source
        .prepare_checkpoint_adoption(&f.receipt, ordinary.checkpoint().bytes(), &f.group, 0)
        .unwrap();
    assert_eq!(
        plan.recovery_snapshot().unwrap().applied_ops.len(),
        f.source.op_count()
    );
    let mut successor = f.source.adopted_successor(&plan, &f.group, 0).unwrap();
    assert_eq!(successor.snapshot().unwrap()[0], 1);
}

#[test]
fn registry_checkpoint_successor_preserves_book_and_seed_only_dependencies() {
    let mut f = Fixture::new();
    f.edit(11);
    f.seal();
    // Exercise saved repair anti-replay state, which from_checkpoint alone would reset.
    let mut bytes = f.source.receipts.encode().unwrap();
    let end = bytes.len();
    bytes[end - 8..].copy_from_slice(&42u64.to_be_bytes());
    f.source.receipts = ReceiptBook::decode(&bytes).unwrap();
    let plan = f.plan().unwrap();
    let before = f.source.snapshot().unwrap();
    let mut successor = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
    assert_eq!(
        f.source.snapshot().unwrap(),
        before,
        "construction cannot prune"
    );
    assert_eq!(successor.epoch(), 1);
    assert_eq!(successor.op_count(), 0);
    assert_eq!(successor.projection().unwrap().pointers[&f.key], 9);
    let bytes = successor.receipts.encode().unwrap();
    assert_eq!(&bytes[bytes.len() - 8..], &42u64.to_be_bytes());
    let domain = RegistryOp::Put {
        key: f.key.clone(),
        epoch: 12,
    }
    .domain_op(&f.group.group_id(), [12; 16])
    .unwrap();
    successor
        .edit(&f.owner, &f.group, &mut f.rng, &domain)
        .unwrap();
    let mut graph = successor.doc.doc().clone();
    let change = graph.get_last_local_change().unwrap();
    assert_eq!(
        change.deps().iter().map(|h| h.0).collect::<Vec<_>>(),
        vec![plan.checkpoint().origin().seed_hash()]
    );
    assert!(successor.opened_by(&f.receipt));
    let snapshot = successor.snapshot().unwrap();
    let restored =
        RegistryEpoch::restore(&snapshot, &f.group, f.key.bucket(), f.owner.device_id()).unwrap();
    assert_eq!(restored.op_count(), 1);
    assert_eq!(restored.projection().unwrap().pointers[&f.key], 12);
}

#[test]
fn registry_checkpoint_opening_equivocation_faults_open_and_closing_successors_on_restart() {
    let mut f = Fixture::new();
    f.seal();
    let plan = f.plan().unwrap();
    for closing in [false, true] {
        let mut successor = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
        let domain = RegistryOp::Put {
            key: f.key.clone(),
            epoch: 12,
        }
        .domain_op(&f.group.group_id(), [12; 16])
        .unwrap();
        successor
            .edit(&f.owner, &f.group, &mut f.rng, &domain)
            .unwrap();
        if closing {
            let seed = successor
                .projection()
                .unwrap()
                .checkpoint([80; 32])
                .unwrap();
            let next = Receipt::sign(
                successor.logical.clone(),
                1,
                [80; 32],
                seed.change_hash(),
                0,
                InheritedCheckpoint::EpochZero,
                &f.owner,
            )
            .unwrap();
            successor.seal(next, &f.group, 0).unwrap();
        }
        let before = successor.snapshot().unwrap();
        let latest = successor.receipts.latest().unwrap().clone();
        let ordinary = successor.receipts.encode().unwrap();
        assert_eq!(ordinary[0], 1);
        assert_eq!(
            ReceiptBook::decode(&ordinary).unwrap().encode().unwrap(),
            ordinary
        );
        let mut noncanonical = ordinary.clone();
        noncanonical[0] = 2;
        assert!(ReceiptBook::decode(&noncanonical).is_err());
        assert_eq!(
            successor.seal(f.receipt.clone(), &f.group, 0).unwrap(),
            ReceiptIngest::Duplicate
        );
        assert_eq!(successor.snapshot().unwrap(), before);
        let mut bad = f.receipt.clone();
        bad.signature[0] ^= 1;
        assert!(successor.seal(bad, &f.group, 0).is_err());
        assert_eq!(successor.snapshot().unwrap(), before);
        let conflicting = Receipt::sign(
            f.receipt.document.clone(),
            0,
            [81; 32],
            [82; 32],
            0,
            InheritedCheckpoint::EpochZero,
            &f.owner,
        )
        .unwrap();
        assert_eq!(
            successor.seal(conflicting, &f.group, 0).unwrap(),
            ReceiptIngest::Fault
        );
        assert_eq!(
            successor.receipts.latest(),
            Some(&latest),
            "keep successor high-water"
        );
        let snapshot = successor.snapshot().unwrap();
        let fault_bytes = successor.receipts.encode().unwrap();
        assert_eq!(fault_bytes[0], if closing { 2 } else { 1 });
        assert_eq!(
            ReceiptBook::decode(&fault_bytes).unwrap().encode().unwrap(),
            fault_bytes
        );
        if closing {
            let mut wrong_version = fault_bytes;
            wrong_version[0] = 1;
            assert!(ReceiptBook::decode(&wrong_version).is_err());
        }
        let mut restored =
            RegistryEpoch::restore(&snapshot, &f.group, f.key.bucket(), f.owner.device_id())
                .unwrap();
        assert_eq!(restored.phase(), EpochPhase::Fault);
        assert_eq!(restored.op_count(), 1);
        assert_eq!(restored.projection().unwrap().pointers[&f.key], 12);
        assert!(restored
            .edit(&f.owner, &f.group, &mut f.rng, &domain)
            .is_err());
        assert!(!restored.opened_by(&f.receipt));
        // Real signatures and a real conflict are insufficient without the exact opening anchor.
        let mut unrelated = ReceiptBook::default();
        for n in [90, 91] {
            unrelated
                .ingest_verified(
                    Receipt::sign(
                        f.receipt.document.clone(),
                        0,
                        [n; 32],
                        [n; 32],
                        0,
                        InheritedCheckpoint::EpochZero,
                        &f.owner,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        restored.receipts = unrelated;
        let forged = restored.snapshot().unwrap();
        assert!(
            RegistryEpoch::restore(&forged, &f.group, f.key.bucket(), f.owner.device_id()).is_err()
        );
    }
}

#[test]
fn registry_settlement_rotated_source_preserves_inherited_seed_and_partitions_only_new_ops() {
    let mut f = Fixture::new();
    f.seal();
    let first = f.plan().unwrap();
    let mut source = RegistryEpoch::from_checkpoint(
        &f.group,
        f.key.bucket(),
        f.owner.device_id(),
        f.receipt.clone(),
        0,
        first.checkpoint().bytes(),
    )
    .unwrap();
    let key = (0..10_000)
        .map(|n| {
            PointerKey::new(DocType::StudioObject, format!("second-{n}").into_bytes()).unwrap()
        })
        .find(|key| key.bucket() == f.key.bucket())
        .unwrap();
    fill_close(&mut source, &f.owner, &f.group, &key, &mut f.rng);
    let close = CloseRecord::sign(
        &source.logical,
        source.doc_id(),
        1,
        source.doc.heads(),
        &f.owner,
    )
    .unwrap();
    let (seed, closure) = checkpoint_registry_close(
        &mut source.doc,
        &source.gate,
        key.bucket(),
        &close,
        &f.group,
        None,
    )
    .unwrap();
    assert_eq!(
        closure.operation_count, 10,
        "the inherited unsigned seed is not a user op"
    );
    let excluded = RegistryOp::Put {
        key: key.clone(),
        epoch: 11,
    }
    .domain_op(&f.group.group_id(), [11; 16])
    .unwrap();
    source
        .edit(&f.owner, &f.group, &mut f.rng, &excluded)
        .unwrap();
    let receipt = Receipt::sign(
        source.logical.clone(),
        1,
        close.hash(),
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &f.owner,
    )
    .unwrap();
    source.seal(receipt.clone(), &f.group, 0).unwrap();
    let before = source.snapshot().unwrap();
    let plan = source.prepare_settlement(&close, &f.group, 0).unwrap();
    let snapshot = plan.recovery_snapshot().unwrap().unwrap();
    assert_eq!(snapshot.base_close_record_hash, Some(f.close.hash()));
    let recovered = crate::registry::RegistryRecovery::from_snapshot(
        &snapshot,
        &source.logical,
        f.key.bucket(),
    )
    .unwrap();
    assert_eq!(recovered.projection(), plan.source_projection());
    assert_eq!(source.snapshot().unwrap(), before);
    assert_eq!(plan.included_operation_ids().len(), 10);
    assert_eq!(plan.excluded_operations().len(), 1);
    assert_eq!(
        plan.excluded_operations()[&excluded.id(&f.owner.device_id())].operation,
        excluded
    );
    assert_eq!(plan.source_projection().pointers[&key], 11);
    assert_eq!(plan.source_projection().pointers[&f.key], 9);
    let mut restored =
        RegistryEpoch::restore(&before, &f.group, f.key.bucket(), f.owner.device_id()).unwrap();
    let retry = restored.prepare_settlement(&close, &f.group, 0).unwrap();
    assert_eq!(plan.source_version(), retry.source_version());
    assert_eq!(plan.checkpoint().bytes(), retry.checkpoint().bytes());
    let mut next = RegistryEpoch::from_checkpoint(
        &f.group,
        f.key.bucket(),
        f.owner.device_id(),
        receipt,
        0,
        retry.checkpoint().bytes(),
    )
    .unwrap();
    assert_eq!(next.epoch(), 2);
    assert_eq!(
        next.projection().unwrap().pointers[&f.key],
        9,
        "seed-only pointer survives another rotation"
    );
    assert_eq!(next.projection().unwrap().pointers[&key], 9);
    next.edit(&f.owner, &f.group, &mut f.rng, &excluded)
        .unwrap();
    let change = automerge::Change::from_bytes(next.doc.signed_log()[0].delta.clone()).unwrap();
    assert_eq!(
        change.deps(),
        &[automerge::ChangeHash(
            plan.checkpoint().origin().seed_hash()
        )]
    );
}

#[test]
fn registry_recovery_plan_identity_ignores_quarantine_and_empty_settlement_needs_no_slot() {
    let mut f = Fixture::new();
    let original = f.source.snapshot().unwrap();
    f.seal();
    assert!(f.plan().unwrap().recovery_snapshot().unwrap().is_none());
    f.source =
        RegistryEpoch::restore(&original, &f.group, f.key.bucket(), f.owner.device_id()).unwrap();
    f.edit(10);
    let before_late = f.source.snapshot().unwrap();
    f.edit(11);
    let late = SealedOp::seal(
        f.source.doc.signed_log().last().unwrap(),
        &f.group,
        &f.owner,
        &mut f.rng,
    )
    .unwrap();
    f.source = RegistryEpoch::restore(&before_late, &f.group, f.key.bucket(), f.owner.device_id())
        .unwrap();
    f.seal();
    let plan = f.plan().unwrap();
    let snapshot = plan.recovery_snapshot().unwrap().unwrap();
    assert_eq!(snapshot.base_close_record_hash, None);
    let recovered = crate::registry::RegistryRecovery::from_snapshot(
        &snapshot,
        &f.source.logical,
        f.key.bucket(),
    )
    .unwrap();
    assert_eq!(recovered.excluded_operations().len(), 1);
    assert_eq!(snapshot.applied_ops.len(), 11);
    f.source.ingest(&late, &f.group, &f.owner).unwrap();
    let retry = f.plan().unwrap();
    assert_ne!(plan.source_version(), retry.source_version());
    assert_eq!(snapshot, retry.recovery_snapshot().unwrap().unwrap());
    assert_eq!(
        snapshot.id().unwrap(),
        retry.recovery_snapshot().unwrap().unwrap().id().unwrap()
    );
}

#[test]
fn registry_settlement_partitions_actual_source_without_mutation_and_is_restart_stable() {
    let mut f = Fixture::new();
    let first = f.edit(10);
    let second = f.edit(11);
    f.seal();
    let before = f.source.snapshot().unwrap();
    let plan = f.plan().unwrap();
    assert_eq!(plan.included_operation_ids().len(), 10);
    assert_eq!(plan.excluded_operations().len(), 2);
    for op in [first, second] {
        assert_eq!(
            plan.excluded_operations()[&op.id(&f.owner.device_id())],
            LocalIntent {
                author: f.owner.device_id(),
                operation: op
            }
        );
    }
    assert!(plan.excluded_operations().keys().is_sorted());
    assert_eq!(plan.receipt(), &f.receipt);
    assert_eq!(plan.source_projection().pointers[&f.key], 11);
    let mut successor = RegistryEpoch::from_checkpoint(
        &f.group,
        f.key.bucket(),
        f.owner.device_id(),
        f.receipt.clone(),
        0,
        plan.checkpoint().bytes(),
    )
    .unwrap();
    assert_eq!(successor.projection().unwrap().pointers[&f.key], 9);
    let op = RegistryOp::Put {
        key: f.key.clone(),
        epoch: 12,
    }
    .domain_op(&f.group.group_id(), [12; 16])
    .unwrap();
    successor.edit(&f.owner, &f.group, &mut f.rng, &op).unwrap();
    let change =
        automerge::Change::from_bytes(successor.doc.signed_log()[0].delta.clone()).unwrap();
    assert_eq!(
        change.deps(),
        &[automerge::ChangeHash(
            plan.checkpoint().origin().seed_hash()
        )]
    );
    assert_eq!(f.source.snapshot().unwrap(), before);
    let mut restored =
        RegistryEpoch::restore(&before, &f.group, f.key.bucket(), f.owner.device_id()).unwrap();
    assert!(plan.matches_source(&mut restored).unwrap());
    let retry = restored.prepare_settlement(&f.close, &f.group, 0).unwrap();
    assert_eq!(retry.source_version(), plan.source_version());
    assert_eq!(retry.checkpoint().bytes(), plan.checkpoint().bytes());
    assert_eq!(retry.excluded_operations(), plan.excluded_operations());
    assert_eq!(
        format!("{plan:?}"),
        "RegistrySettlementPlan { epoch: 0, included_operations: 10, excluded_operations: 2, .. }"
    );
}

#[test]
fn registry_settlement_same_receipt_does_not_hide_different_accepted_source_versions() {
    let mut f = Fixture::new();
    let early = f.source.snapshot().unwrap();
    f.edit(10);
    f.seal();
    let later = f.plan().unwrap();
    let mut early =
        RegistryEpoch::restore(&early, &f.group, f.key.bucket(), f.owner.device_id()).unwrap();
    early.seal(f.receipt.clone(), &f.group, 0).unwrap();
    let plan = early.prepare_settlement(&f.close, &f.group, 0).unwrap();
    assert_eq!(plan.checkpoint().bytes(), later.checkpoint().bytes());
    assert_eq!(plan.receipt(), later.receipt());
    assert!(plan.excluded_operations().is_empty());
    assert_eq!(later.excluded_operations().len(), 1);
    assert!(!later.matches_source(&mut early).unwrap());
}

#[test]
fn registry_settlement_rejects_open_wrong_close_malformed_input_wrong_seed_and_fault() {
    let mut f = Fixture::new();
    let open = f.source.snapshot().unwrap();
    assert!(matches!(f.plan(), Err(ReplError::EpochClosed)));
    assert_eq!(f.source.snapshot().unwrap(), open);
    f.edit(10);
    let alternate = CloseRecord::sign(
        &f.source.logical,
        f.source.doc_id(),
        0,
        f.source.doc.heads(),
        &f.owner,
    )
    .unwrap();
    let unsealed = f.source.snapshot().unwrap();
    f.seal();
    let before = f.source.snapshot().unwrap();
    assert!(matches!(
        f.source.prepare_settlement(&alternate, &f.group, 0),
        Err(ReplError::ReceiptConflict)
    ));
    let mut malformed = f.close.clone();
    malformed.heads = vec![[0; 32]; 65_536];
    assert!(matches!(
        f.source.prepare_settlement(&malformed, &f.group, 0),
        Err(ReplError::EpochBound)
    ));
    malformed.heads.clear();
    assert!(f
        .source
        .prepare_settlement(&malformed, &f.group, 0)
        .is_err());
    malformed = f.close.clone();
    malformed.signature[0] ^= 1;
    assert!(f
        .source
        .prepare_settlement(&malformed, &f.group, 0)
        .is_err());
    assert!(f.source.prepare_settlement(&f.close, &f.group, 1).is_err());
    assert_eq!(f.source.snapshot().unwrap(), before);
    // An authentic owner receipt naming the wrong expected seed is still not a valid plan.
    let wrong = Receipt::sign(
        f.source.logical.clone(),
        0,
        f.close.hash(),
        [42; 32],
        0,
        InheritedCheckpoint::EpochZero,
        &f.owner,
    )
    .unwrap();
    let mut source =
        RegistryEpoch::restore(&unsealed, &f.group, f.key.bucket(), f.owner.device_id()).unwrap();
    source.seal(wrong.clone(), &f.group, 0).unwrap();
    let before = source.snapshot().unwrap();
    assert!(source.prepare_settlement(&f.close, &f.group, 0).is_err());
    assert_eq!(source.snapshot().unwrap(), before);
    f.source.seal(wrong, &f.group, 0).unwrap();
    assert_eq!(f.source.phase(), EpochPhase::Fault);
    let before = f.source.snapshot().unwrap();
    assert!(matches!(f.plan(), Err(ReplError::EpochClosed)));
    assert_eq!(f.source.snapshot().unwrap(), before);
}

#[test]
fn registry_settlement_missing_heads_and_quarantined_content_are_not_recovery_inputs() {
    let mut f = Fixture::new();
    let before_edit = f.source.snapshot().unwrap();
    let domain = f.edit(10);
    let late = SealedOp::seal(
        f.source.doc.signed_log().last().unwrap(),
        &f.group,
        &f.owner,
        &mut f.rng,
    )
    .unwrap();
    f.source = RegistryEpoch::restore(&before_edit, &f.group, f.key.bucket(), f.owner.device_id())
        .unwrap();
    f.seal();
    f.source.ingest(&late, &f.group, &f.owner).unwrap();
    assert_eq!(f.source.quarantined_len(), 1);
    let before = f.source.snapshot().unwrap();
    let plan = f.plan().unwrap();
    assert!(plan.excluded_operations().is_empty());
    assert!(!plan
        .included_operation_ids()
        .contains(&domain.id(&f.owner.device_id())));
    assert_eq!(f.source.snapshot().unwrap(), before);
    // A receipt does not conjure up an absent dependency closure.
    let missing = CloseRecord::sign(
        &f.source.logical,
        f.source.doc_id(),
        0,
        vec![[77; 32]],
        &f.owner,
    )
    .unwrap();
    let receipt = Receipt::sign(
        f.source.logical.clone(),
        0,
        missing.hash(),
        f.receipt.seed_change_hash,
        0,
        InheritedCheckpoint::EpochZero,
        &f.owner,
    )
    .unwrap();
    let mut source =
        RegistryEpoch::restore(&before_edit, &f.group, f.key.bucket(), f.owner.device_id())
            .unwrap();
    source.seal(receipt, &f.group, 0).unwrap();
    let before = source.snapshot().unwrap();
    assert!(source.prepare_settlement(&missing, &f.group, 0).is_err());
    assert_eq!(source.snapshot().unwrap(), before);
}

#[test]
fn registry_settlement_preserves_source_tombstones_and_excluded_peer_authorship() {
    let mut f = Fixture::new();
    let peer = MlsDevice::generate().unwrap();
    let welcome = f
        .group
        .add_member(&f.owner, peer.key_package().unwrap())
        .unwrap()
        .welcome;
    let peer_group = ServerGroup::join(&peer, &welcome).unwrap();
    let mut peer_source = RegistryEpoch::restore(
        &f.source.snapshot().unwrap(),
        &peer_group,
        f.key.bucket(),
        peer.device_id(),
    )
    .unwrap();
    let domain = RegistryOp::Tombstone { key: f.key.clone() }
        .domain_op(&peer_group.group_id(), [29; 16])
        .unwrap();
    let sealed = peer_source
        .edit(&peer, &peer_group, &mut f.rng, &domain)
        .unwrap();
    f.source.ingest(&sealed, &f.group, &f.owner).unwrap();
    f.seal();
    let before = f.source.snapshot().unwrap();
    let plan = f.plan().unwrap();
    assert!(plan.source_projection().tombstones.contains(&f.key));
    assert!(!plan.source_projection().pointers.contains_key(&f.key));
    assert_eq!(
        plan.excluded_operations()[&domain.id(&peer.device_id())],
        LocalIntent {
            author: peer.device_id(),
            operation: domain
        }
    );
    assert_eq!(f.source.snapshot().unwrap(), before);
}

#[test]
fn registry_settlement_checks_current_owner_without_mutating_gate_and_allows_receipted_removed_author(
) {
    let mut f = Fixture::new();
    let unsealed = f.source.snapshot().unwrap();
    f.seal();
    let next = MlsDevice::generate().unwrap();
    let welcome = f
        .group
        .add_member(&f.owner, next.key_package().unwrap())
        .unwrap()
        .welcome;
    let mut next_group = ServerGroup::join(&next, &welcome).unwrap();
    next_group
        .remove_member(&next, &f.owner.device_id())
        .unwrap();
    let before = f.source.snapshot().unwrap();
    assert!(f
        .source
        .prepare_settlement(&f.close, &next_group, next_group.epoch())
        .is_err());
    assert_eq!(f.source.snapshot().unwrap(), before);
    let mut source =
        RegistryEpoch::restore(&unsealed, &next_group, f.key.bucket(), next.device_id()).unwrap();
    let receipt = Receipt::sign(
        source.logical.clone(),
        0,
        f.close.hash(),
        f.receipt.seed_change_hash,
        next_group.epoch(),
        InheritedCheckpoint::EpochZero,
        &next,
    )
    .unwrap();
    source
        .seal(receipt, &next_group, next_group.epoch())
        .unwrap();
    let before = source.snapshot().unwrap();
    let plan = source
        .prepare_settlement(&f.close, &next_group, next_group.epoch())
        .unwrap();
    assert_eq!(plan.included_operation_ids().len(), 10);
    assert_eq!(source.snapshot().unwrap(), before);
}
