use super::*;
use crate::registry::{PointerKey, RegistryOp};
use crate::{
    CheckpointSeed, InheritedCheckpoint, RecoveryReason, RecoverySlots, RecoveryTransition,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

struct Fixture {
    owner: MlsDevice,
    group: ServerGroup,
    key: PointerKey,
    source: RegistryEpoch,
    rng: ChaCha20Rng,
}
impl Fixture {
    fn new() -> Self {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let key =
            PointerKey::new(DocType::StudioObject, b"private-newcomer-pointer".to_vec()).unwrap();
        let source = RegistryEpoch::new(&group, key.bucket(), owner.device_id()).unwrap();
        Self {
            owner,
            group,
            key,
            source,
            rng: ChaCha20Rng::seed_from_u64(573),
        }
    }
    fn edit(&mut self, nonce: u8) -> DomainOp {
        let op = RegistryOp::Put {
            key: self.key.clone(),
            epoch: u64::from(nonce),
        }
        .domain_op(&self.group.group_id(), [nonce; 16])
        .unwrap();
        self.source
            .edit(&self.owner, &self.group, &mut self.rng, &op)
            .unwrap();
        op
    }
    fn target(&self, closed_epoch: u64, salt: u8) -> (Receipt, CheckpointSeed) {
        // A newcomer deliberately does NOT possess or validate the owner's prior closure.
        // Build a canonical seed and a real current-owner signature, not fake gate admission.
        let mut projection = self.source.projection().unwrap();
        projection.epoch = closed_epoch;
        projection
            .pointers
            .insert(self.key.clone(), u64::from(salt));
        let seed = projection.checkpoint([salt; 32]).unwrap();
        let receipt = Receipt::sign(
            self.source.logical.clone(),
            closed_epoch,
            [salt; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &self.owner,
        )
        .unwrap();
        (receipt, seed)
    }
    fn begin(&mut self, receipt: &Receipt) -> ReceiptIngest {
        self.source
            .begin_checkpoint_adoption(receipt.clone(), &self.group, 0)
            .unwrap()
    }
    fn plan(&mut self, receipt: &Receipt, seed: &CheckpointSeed) -> RegistryAdoptionPlan {
        self.source
            .prepare_checkpoint_adoption(receipt, seed.bytes(), &self.group, 0)
            .unwrap()
    }
    fn reopen(&mut self) {
        let bytes = self.source.snapshot().unwrap();
        let protocol = self.source.storage_protocol_bytes().unwrap();
        assert_eq!(
            RegistryEpoch::validate_vault_snapshot(
                &bytes,
                &self.group.group_id(),
                self.key.bucket(),
            )
            .unwrap(),
            protocol
        );
        self.source = RegistryEpoch::restore(
            &bytes,
            &self.group,
            self.key.bucket(),
            self.owner.device_id(),
        )
        .unwrap();
        assert_eq!(self.source.snapshot().unwrap(), bytes);
    }
    fn install_seed_for_test(&mut self, receipt: Receipt, seed: &CheckpointSeed) {
        self.source = RegistryEpoch::from_checkpoint(
            &self.group,
            self.key.bucket(),
            self.owner.device_id(),
            receipt,
            0,
            seed.bytes(),
        )
        .unwrap();
    }
}

#[test]
fn registry_adoption_preserves_whole_source_and_constructs_one_isolated_successor() {
    let mut f = Fixture::new();
    let op = f.edit(1);
    // Adoption must carry anti-replay repair bookkeeping into the selected successor.
    let mut book = f.source.receipts.encode().unwrap();
    let end = book.len();
    book[end - 8..].copy_from_slice(&42u64.to_be_bytes());
    f.source.receipts = ReceiptBook::decode(&book).unwrap();
    let (receipt, seed) = f.target(10, 10);
    let before = f.source.snapshot().unwrap().len();
    let protocol = f.source.storage_protocol_bytes().unwrap();
    assert_eq!(f.begin(&receipt), ReceiptIngest::Advanced);
    assert_eq!(f.source.phase(), EpochPhase::Closing);
    assert_eq!(f.source.op_count(), 1);
    let after = f.source.snapshot().unwrap();
    assert_eq!(after[0], 2);
    assert_eq!(
        after.len() - before,
        f.source.storage_protocol_bytes().unwrap() - protocol
    );
    assert!(f.source.edit(&f.owner, &f.group, &mut f.rng, &op).is_err());
    f.reopen();
    let plan = f.plan(&receipt, &seed);
    let snapshot = plan.recovery_snapshot().unwrap();
    assert_eq!(snapshot.reason, RecoveryReason::Rewound);
    let recovery =
        RegistryRecovery::from_snapshot(snapshot, &f.source.logical, f.key.bucket()).unwrap();
    assert_eq!(recovery.projection().pointers[&f.key], 1);
    assert_eq!(recovery.receipt_hash(), [0; 32]);
    assert_eq!(
        recovery.excluded_operations()[&op.id(&f.owner.device_id())].operation,
        op
    );
    let mut successor = f.source.adopted_successor(&plan, &f.group, 0).unwrap();
    assert_eq!((successor.epoch(), successor.op_count()), (11, 0));
    assert_eq!(successor.projection().unwrap().pointers[&f.key], 10);
    assert_eq!(successor.snapshot().unwrap()[0], 1);
    let book = successor.receipts.encode().unwrap();
    assert_eq!(&book[book.len() - 8..], &42u64.to_be_bytes());
    assert_eq!(
        f.source.op_count(),
        1,
        "construction does not discard the source"
    );
    successor.edit(&f.owner, &f.group, &mut f.rng, &op).unwrap();
    let signed = successor.doc.signed_log().last().unwrap();
    let change = automerge::Change::from_bytes(signed.delta.clone()).unwrap();
    assert_eq!(change.deps(), &[automerge::ChangeHash(seed.change_hash())]);
    f.source = successor;
    f.reopen();
    let current = f.source.snapshot().unwrap();
    assert_eq!(f.begin(&receipt), ReceiptIngest::Duplicate);
    assert_eq!(
        f.source.snapshot().unwrap(),
        current,
        "retry must not reseed over edits"
    );
}

#[test]
fn registry_adoption_distant_opening_fault_survives_restart_without_lowering_high_water() {
    let mut f = Fixture::new();
    let (opening, seed) = f.target(0, 1);
    f.install_seed_for_test(opening.clone(), &seed);
    let (r10, _) = f.target(10, 10);
    let (r20, _) = f.target(20, 20);
    f.begin(&r10);
    f.begin(&r20);
    let (conflict, _) = f.target(0, 2);
    assert_eq!(f.begin(&conflict), ReceiptIngest::Fault);
    f.reopen();
    assert_eq!(f.source.phase(), EpochPhase::Fault);
    assert_eq!(f.source.receipts.latest(), Some(&r20));
    assert_eq!(f.source.projection().unwrap().pointers[&f.key], 1);
    assert!(f.source.receipt_head().is_err());
    assert!(ReceiptBook::decode(&f.source.receipts.encode_adoption().unwrap()).is_err());
    // A valid fault pair cannot be transplanted to a different source lacking its anchor.
    let (different_opening, seed) = f.target(0, 4);
    let mut wrong = RegistryEpoch::from_checkpoint(
        &f.group,
        f.key.bucket(),
        f.owner.device_id(),
        different_opening,
        0,
        seed.bytes(),
    )
    .unwrap();
    wrong.begin_checkpoint_adoption(r20, &f.group, 0).unwrap();
    // Match the wrong source's actual gate id/metadata, leaving ONLY the fault anchor wrong.
    let (different_conflict, _) = f.target(0, 5);
    wrong
        .begin_checkpoint_adoption(different_conflict, &f.group, 0)
        .unwrap();
    wrong.receipts = f.source.receipts.clone();
    let bytes = wrong.snapshot().unwrap();
    assert!(RegistryEpoch::restore(&bytes, &f.group, f.key.bucket(), f.owner.device_id()).is_err());
    let frozen = f.source.snapshot().unwrap();
    assert_eq!(f.begin(&opening), ReceiptIngest::Fault);
    assert_eq!(f.source.snapshot().unwrap(), frozen);
}

#[test]
fn registry_adoption_retained_prior_target_fault_is_not_hidden_as_stale() {
    let mut f = Fixture::new();
    f.edit(3);
    let (r10, _) = f.target(10, 10);
    let (r20, _) = f.target(20, 20);
    f.begin(&r10);
    f.begin(&r20);
    f.reopen();
    let before = f.source.snapshot().unwrap();
    assert_eq!(f.begin(&r10), ReceiptIngest::Stale);
    assert_eq!(f.source.snapshot().unwrap(), before);
    let (conflict, _) = f.target(10, 11);
    assert_eq!(f.begin(&conflict), ReceiptIngest::Fault);
    f.reopen();
    assert_eq!(f.source.receipts.latest(), Some(&r20));
    assert_eq!(f.source.op_count(), 1);
}

#[test]
fn registry_adoption_retarget_reuses_whole_version_and_eviction_deadline() {
    let mut f = Fixture::new();
    let mut slots = RecoverySlots::default();
    // Three genuinely different source versions fill the two retained + one staged slots.
    for n in 1..=3 {
        let (opening, seed) = f.target(0, n);
        f.install_seed_for_test(opening.clone(), &seed);
        let (target, seed) = f.target(10, 10);
        f.begin(&target);
        let plan = f.plan(&target, &seed);
        let snapshot = plan.recovery_snapshot().unwrap().clone();
        assert!(
            snapshot.applied_ops.is_empty(),
            "seed-only versions still need recovery"
        );
        assert_eq!(
            RegistryRecovery::from_snapshot(&snapshot, &f.source.logical, f.key.bucket())
                .unwrap()
                .receipt_hash(),
            opening.hash()
        );
        let transition = slots.stage(snapshot.clone(), 100).unwrap();
        if n == 3 {
            assert!(matches!(
                transition,
                RecoveryTransition::EvictionPending { .. }
            ));
            let (newer, seed) = f.target(20, 20);
            f.begin(&newer);
            f.reopen();
            let newer_plan = f.plan(&newer, &seed);
            assert!(!plan.matches_source(&mut f.source).unwrap());
            assert!(f.source.adopted_successor(&plan, &f.group, 0).is_err());
            let repeated = newer_plan.recovery_snapshot().unwrap();
            assert_eq!(snapshot.id().unwrap(), repeated.id().unwrap());
            assert_eq!(slots.stage(repeated.clone(), 500_000).unwrap(), transition);
        }
    }
}

#[test]
fn registry_adoption_empty_source_needs_no_snapshot_but_terminal_seed_does() {
    let mut f = Fixture::new();
    let (receipt, seed) = f.target(0, 1);
    f.begin(&receipt);
    assert!(f.plan(&receipt, &seed).recovery_snapshot().is_none());
    let (terminal, seed) = f.target(MAX_REGISTRY_EPOCH - 1, 2);
    f.install_seed_for_test(terminal, &seed);
    let (other_branch, seed) = f.target(0, 3);
    // Same-tenure rollback is not permitted; emulate a legitimate NEW owner below instead.
    let next = MlsDevice::generate().unwrap();
    let kp = next.key_package().unwrap();
    let welcome = f.group.add_member(&f.owner, kp).unwrap().welcome;
    let mut next_group = ServerGroup::join(&next, &welcome).unwrap();
    next_group
        .remove_member(&next, &f.owner.device_id())
        .unwrap();
    let tenure = next_group.epoch();
    f.source = RegistryEpoch::restore(
        &f.source.snapshot().unwrap(),
        &next_group,
        f.key.bucket(),
        next.device_id(),
    )
    .unwrap();
    let receipt = Receipt::sign(
        other_branch.document,
        other_branch.closed_epoch,
        other_branch.close_record_hash,
        other_branch.seed_change_hash,
        tenure,
        InheritedCheckpoint::EpochZero,
        &next,
    )
    .unwrap();
    assert_eq!(
        f.source
            .begin_checkpoint_adoption(receipt.clone(), &next_group, tenure)
            .unwrap(),
        ReceiptIngest::Advanced
    );
    let plan = f
        .source
        .prepare_checkpoint_adoption(&receipt, seed.bytes(), &next_group, tenure)
        .unwrap();
    let snapshot = plan.recovery_snapshot().unwrap();
    assert_eq!(snapshot.epoch, MAX_REGISTRY_EPOCH);
    RegistryRecovery::from_snapshot(snapshot, &f.source.logical, f.key.bucket()).unwrap();
    f.source = RegistryEpoch::restore(
        &f.source.snapshot().unwrap(),
        &next_group,
        f.key.bucket(),
        next.device_id(),
    )
    .unwrap();
    f.source = f
        .source
        .adopted_successor(&plan, &next_group, tenure)
        .unwrap();
    assert_eq!(f.source.phase(), EpochPhase::Open);
    assert_eq!(
        f.source.epoch(),
        1,
        "new-tenure adoption can rewind a terminal checkpoint"
    );
}

#[test]
fn registry_adoption_quarantine_invalidates_plan_without_churning_recovery() {
    let mut f = Fixture::new();
    f.edit(1);
    let mut late = RegistryEpoch::restore(
        &f.source.snapshot().unwrap(),
        &f.group,
        f.key.bucket(),
        f.owner.device_id(),
    )
    .unwrap();
    let operation = RegistryOp::Put {
        key: f.key.clone(),
        epoch: 99,
    }
    .domain_op(&f.group.group_id(), [99; 16])
    .unwrap();
    let sealed = late
        .edit(&f.owner, &f.group, &mut f.rng, &operation)
        .unwrap();
    let (receipt, seed) = f.target(10, 10);
    f.begin(&receipt);
    let plan = f.plan(&receipt, &seed);
    let snapshot = plan.recovery_snapshot().unwrap();
    let mut slots = RecoverySlots::default();
    for n in [80, 81] {
        let mut prior = RegistryEpoch::new(&f.group, f.key.bucket(), f.owner.device_id()).unwrap();
        let op = RegistryOp::Put {
            key: f.key.clone(),
            epoch: n,
        }
        .domain_op(&f.group.group_id(), [n as u8; 16])
        .unwrap();
        prior.edit(&f.owner, &f.group, &mut f.rng, &op).unwrap();
        prior
            .begin_checkpoint_adoption(receipt.clone(), &f.group, 0)
            .unwrap();
        let old = prior
            .prepare_checkpoint_adoption(&receipt, seed.bytes(), &f.group, 0)
            .unwrap();
        slots
            .stage(old.recovery_snapshot().unwrap().clone(), 100)
            .unwrap();
    }
    let warning = slots.stage(snapshot.clone(), 100).unwrap();
    assert!(matches!(
        warning,
        RecoveryTransition::EvictionPending { .. }
    ));
    assert_eq!(
        f.source.ingest(&sealed, &f.group, &f.owner).unwrap(),
        Admission::Quarantined
    );
    assert_eq!(f.source.quarantined_len(), 1);
    assert_eq!(f.source.op_count(), 1);
    assert!(!plan.matches_source(&mut f.source).unwrap());
    assert!(f.source.adopted_successor(&plan, &f.group, 0).is_err());
    f.reopen();
    let (newer, seed) = f.target(20, 20);
    f.begin(&newer);
    f.reopen();
    let repeated = f.plan(&newer, &seed);
    assert_eq!(
        snapshot.id().unwrap(),
        repeated.recovery_snapshot().unwrap().id().unwrap()
    );
    assert_eq!(
        slots
            .stage(repeated.recovery_snapshot().unwrap().clone(), 500_000)
            .unwrap(),
        warning
    );
    assert!(!repeated
        .recovery_snapshot()
        .unwrap()
        .applied_ops
        .contains(&operation.id(&f.owner.device_id())));
}

#[test]
fn registry_adoption_restart_rejects_same_tenure_target_below_the_actual_opening() {
    let mut f = Fixture::new();
    let (opening, seed) = f.target(10, 10);
    f.install_seed_for_test(opening, &seed);
    let (r0, _) = f.target(0, 1);
    let (r1, _) = f.target(1, 2);
    assert_eq!(f.begin(&r1), ReceiptIngest::Stale);
    // Synthesize a locally spliced v2 unit: the original source seed/id/epoch and gate match,
    // but the separate receipt book advances R0 -> R1 while omitting the true opening R10.
    f.source.receipts = ReceiptBook::default();
    for receipt in [r0, r1] {
        f.source
            .receipts
            .ingest_adoption(receipt, &f.group, 0, &f.source.gate, None)
            .unwrap();
    }
    f.source.adopting = true;
    let bytes = f.source.snapshot().unwrap();
    assert!(RegistryEpoch::restore(&bytes, &f.group, f.key.bucket(), f.owner.device_id()).is_err());
}

#[test]
fn registry_adoption_rejects_wrong_seed_authority_and_format_downgrades() {
    let mut f = Fixture::new();
    f.edit(1);
    let (receipt, seed) = f.target(10, 10);
    let before = f.source.snapshot().unwrap();
    let mut forged = receipt.clone();
    forged.signature[0] ^= 1;
    assert!(f
        .source
        .begin_checkpoint_adoption(forged, &f.group, 0)
        .is_err());
    assert_eq!(f.source.snapshot().unwrap(), before);
    f.begin(&receipt);
    let frozen = f.source.snapshot().unwrap();
    assert!(f
        .source
        .prepare_checkpoint_adoption(&receipt, &[], &f.group, 0)
        .is_err());
    assert!(f
        .source
        .prepare_checkpoint_adoption(&receipt, seed.bytes(), &f.group, 1)
        .is_err());
    assert_eq!(f.source.snapshot().unwrap(), frozen);
    let mut downgraded = frozen;
    downgraded[0] = 1;
    assert!(
        RegistryEpoch::restore(&downgraded, &f.group, f.key.bucket(), f.owner.device_id()).is_err()
    );
}
