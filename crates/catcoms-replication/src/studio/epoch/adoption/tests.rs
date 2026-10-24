use super::*;
use crate::{CheckpointSeed, InheritedCheckpoint};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

struct Fixture {
    owner: MlsDevice,
    group: ServerGroup,
    source: StudioEpoch,
    rng: ChaCha20Rng,
}
impl Fixture {
    fn new(art: bool) -> Self {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let target = if art {
            StudioTarget::Flipnote {
                channel: [7; 16],
                object: [9; 16],
            }
        } else {
            StudioTarget::Index { channel: [7; 16] }
        };
        let source = StudioEpoch::new(&group, target, owner.device_id()).unwrap();
        Self {
            owner,
            group,
            source,
            rng: ChaCha20Rng::seed_from_u64(861),
        }
    }
    fn operation(&self, nonce: u8) -> DomainOp {
        let body = match self.source.target {
            StudioTarget::Index { .. } => IndexOp::PutObject {
                object: [nonce; 16],
                kind: StudioKind::Flipnote,
                title: format!("private version {nonce}"),
                created_by: self.owner.device_id(),
                ts: 100,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap(),
            _ => FlipnoteOp::InsertFrame {
                frame: [nonce; 16],
                after: None,
                cid: [nonce; 32],
                bytes: 32,
            }
            .encode()
            .unwrap(),
        };
        DomainOp {
            nonce: [nonce; 16],
            doc_type: self.source.logical.doc_type,
            logical_key: self.source.logical.logical_key.clone(),
            body,
        }
    }
    fn edit(&mut self, nonce: u8) -> DomainOp {
        let op = self.operation(nonce);
        self.source
            .edit_or_reseal(&self.owner, &self.group, &mut self.rng, &op, 100)
            .unwrap();
        op
    }
    fn target(&self, epoch: u64, close: u8) -> (Receipt, CheckpointSeed) {
        // Trusted owner fixture, not an unverified imported Automerge snapshot. The newcomer
        // deliberately lacks this predecessor closure and must rely on the owner's receipt.
        let mut projection = self.source.projection().unwrap();
        match &mut projection {
            StudioProjection::Index(p) => p.epoch = epoch,
            StudioProjection::Flipnote(p) => p.epoch = epoch,
        }
        let seed = projection.checkpoint([close; 32]).unwrap();
        let receipt = Receipt::sign(
            self.source.logical.clone(),
            epoch,
            [close; 32],
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
    fn plan(&mut self, receipt: &Receipt, seed: &CheckpointSeed) -> StudioAdoptionPlan {
        self.source
            .prepare_checkpoint_adoption(receipt, seed.bytes(), &self.group, 0)
            .unwrap()
    }
    fn reopen(&mut self) {
        let bytes = self.source.snapshot().unwrap();
        assert_eq!(
            StudioEpoch::validate_vault_snapshot(
                &bytes,
                &self.group.group_id(),
                self.source.target
            )
            .unwrap(),
            self.source.storage_protocol_bytes().unwrap()
        );
        self.source = StudioEpoch::restore(
            &bytes,
            &self.group,
            self.source.target,
            self.owner.device_id(),
        )
        .unwrap();
        assert_eq!(self.source.snapshot().unwrap(), bytes);
    }
}

#[test]
fn studio_adoption_above_registry_lineage_ceiling_restarts_closing_and_fault() {
    for art in [false, true] {
        for epoch in [4096, 4097] {
            let mut f = Fixture::new(art);
            f.edit(1);
            let (receipt, seed) = f.target(epoch, 7);
            assert_eq!(f.begin(&receipt), ReceiptIngest::Advanced);
            let bytes = f.source.snapshot().unwrap();
            let mut reopened =
                StudioEpoch::restore(&bytes, &f.group, f.source.target, f.owner.device_id())
                    .unwrap();
            let plan = reopened
                .prepare_checkpoint_adoption(&receipt, seed.bytes(), &f.group, 0)
                .unwrap();
            let next = reopened.adopted_successor(&plan, &f.group, 0).unwrap();
            assert_eq!(next.epoch(), epoch + 1);
            let (different, _) = f.target(epoch, 8);
            assert_eq!(
                reopened
                    .begin_checkpoint_adoption(different, &f.group, 0)
                    .unwrap(),
                ReceiptIngest::Fault
            );
            let bytes = reopened.snapshot().unwrap();
            let fault =
                StudioEpoch::restore(&bytes, &f.group, f.source.target, f.owner.device_id())
                    .unwrap();
            assert_eq!(fault.phase(), EpochPhase::Fault);
            assert_eq!(fault.op_count(), 1);
        }
    }
}

#[test]
fn studio_adoption_nonadjacent_closing_reopens_and_preserves_whole_version() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let op = f.edit(1);
        let (receipt, seed) = f.target(10, 10);
        let before = f.source.snapshot().unwrap().len();
        let protocol = f.source.storage_protocol_bytes().unwrap();
        assert_eq!(f.begin(&receipt), ReceiptIngest::Advanced);
        assert_eq!(f.source.phase(), EpochPhase::Closing);
        let sealed = f.source.snapshot().unwrap();
        assert_eq!(sealed[0], 2);
        assert_eq!(
            sealed.len() - before,
            f.source.storage_protocol_bytes().unwrap() - protocol
        );
        f.reopen();
        assert!(f
            .source
            .edit_or_reseal(&f.owner, &f.group, &mut f.rng, &op, 100)
            .is_err());
        let plan = f.plan(&receipt, &seed);
        let recovery = StudioRecovery::from_snapshot(
            plan.recovery_snapshot().unwrap(),
            &f.source.logical,
            f.source.target.channel(),
        )
        .unwrap();
        assert_eq!(recovery.selecting_receipt(), [0; 32]);
        assert_eq!(
            recovery.operations()[&op.id(&f.owner.device_id())].operation,
            op
        );
        assert_eq!(recovery.projection(), &f.source.projection().unwrap());
        let mut successor = f.source.adopted_successor(&plan, &f.group, 0).unwrap();
        assert_eq!((successor.epoch(), successor.op_count()), (11, 0));
        assert_eq!(successor.snapshot().unwrap()[0], 1);
        assert_eq!(f.source.op_count(), 1, "constructing is not replacing");
        let next = f.operation(2);
        successor
            .edit_or_reseal(&f.owner, &f.group, &mut f.rng, &next, 100)
            .unwrap();
        let signed = successor.doc.signed_log().last().unwrap();
        let change = automerge::Change::from_bytes(signed.delta.clone()).unwrap();
        assert_eq!(change.deps(), &[automerge::ChangeHash(seed.change_hash())]);
        f.source = successor;
        f.reopen();
        let saved = f.source.snapshot().unwrap();
        assert_eq!(f.begin(&receipt), ReceiptIngest::Duplicate);
        assert_eq!(
            f.source.snapshot().unwrap(),
            saved,
            "installed retry must not reseed over edits"
        );
    }
}

#[test]
fn studio_adoption_retarget_has_stable_recovery_and_fault_reopens_without_a_seed() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        f.edit(1);
        let (r10, s10) = f.target(10, 10);
        let (r20, s20) = f.target(20, 20);
        f.begin(&r10);
        let old = f.plan(&r10, &s10);
        let recovery = old.recovery_snapshot().unwrap().encode().unwrap();
        assert_eq!(f.begin(&r20), ReceiptIngest::Advanced);
        assert!(f.source.adopted_successor(&old, &f.group, 0).is_err());
        f.reopen();
        let newer = f.plan(&r20, &s20);
        assert_eq!(
            newer.recovery_snapshot().unwrap().encode().unwrap(),
            recovery
        );
        let (conflict, _) = f.target(20, 21);
        assert_eq!(f.begin(&conflict), ReceiptIngest::Fault);
        f.reopen();
        assert_eq!(f.source.phase(), EpochPhase::Fault);
        assert!(f.source.receipt_head().is_err());
        assert!(f
            .source
            .prepare_checkpoint_adoption(&r20, s20.bytes(), &f.group, 0)
            .is_err());
        assert_eq!(f.source.op_count(), 1);
    }
}

#[test]
fn studio_adoption_seed_and_scope_errors_never_reopen_or_replace_the_source() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        f.edit(1);
        let (receipt, seed) = f.target(5, 5);
        f.begin(&receipt);
        let before = f.source.snapshot().unwrap();
        let mut bad = seed.bytes().to_vec();
        bad[0] ^= 1;
        assert!(f
            .source
            .prepare_checkpoint_adoption(&receipt, &bad, &f.group, 0)
            .is_err());
        assert!(f
            .source
            .prepare_checkpoint_adoption(&receipt, seed.bytes(), &f.group, 1)
            .is_err());
        assert_eq!(f.source.snapshot().unwrap(), before);
        f.reopen();
        // A seeded source with no subsequent operations is still real historical content.
        let plan = f.plan(&receipt, &seed);
        f.source = f.source.adopted_successor(&plan, &f.group, 0).unwrap();
        let (next, next_seed) = f.target(9, 9);
        f.begin(&next);
        let plan = f.plan(&next, &next_seed);
        let recovery = StudioRecovery::from_snapshot(
            plan.recovery_snapshot().unwrap(),
            &f.source.logical,
            f.source.target.channel(),
        )
        .unwrap();
        assert!(recovery.operations().is_empty());
        assert_eq!(recovery.selecting_receipt(), receipt.hash());
        assert_eq!(recovery.projection(), &f.source.projection().unwrap());
    }
    let mut empty = Fixture::new(true);
    let (r, s) = empty.target(2, 2);
    empty.begin(&r);
    assert!(empty.plan(&r, &s).recovery_snapshot().is_none());
}
