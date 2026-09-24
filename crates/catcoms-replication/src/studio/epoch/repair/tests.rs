use super::*;
use crate::{
    CheckpointSeed, InheritedCheckpoint, ReceiptRepair, RecoveryReason, RepairDisposition,
    RepairHold,
};
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

fn fixtures() -> Vec<Fixture> {
    vec![Fixture::new(false), Fixture::new(true)]
}
impl Fixture {
    fn inbound(&mut self, nonce: u8) -> SealedOp {
        let bytes = self.source.snapshot().unwrap();
        let mut sender = self.restore_bytes(&bytes).unwrap();
        let operation = self.operation(nonce);
        sender
            .edit_or_reseal(&self.owner, &self.group, &mut self.rng, &operation, 100)
            .unwrap()
    }
    fn install(&mut self, receipt: &Receipt, seed: &CheckpointSeed) {
        self.source = StudioEpoch::from_checkpoint(
            &self.group,
            self.source.target,
            self.owner.device_id(),
            receipt.clone(),
            0,
            seed.bytes(),
        )
        .unwrap();
    }
    fn restore_bytes(&self, bytes: &[u8]) -> Result<StudioEpoch, ReplError> {
        StudioEpoch::restore(
            bytes,
            &self.group,
            self.source.target,
            self.owner.device_id(),
        )
    }
}
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/epoch/repair_transition/typed_tests.rs"
));
