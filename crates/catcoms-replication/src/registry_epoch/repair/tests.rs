use super::*;
use crate::registry::{PointerKey, RegistryOp};
use crate::{
    CheckpointSeed, InheritedCheckpoint, ReceiptRepair, RecoveryReason, RepairDisposition,
    RepairHold,
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

fn fixtures() -> Vec<Fixture> {
    vec![Fixture::new()]
}
impl Fixture {
    fn inbound(&mut self, nonce: u8) -> SealedOp {
        let bytes = self.source.snapshot().unwrap();
        let mut sender = self.restore_bytes(&bytes).unwrap();
        let operation = RegistryOp::Put {
            key: self.key.clone(),
            epoch: u64::from(nonce),
        }
        .domain_op(&self.group.group_id(), [nonce; 16])
        .unwrap();
        sender
            .edit(&self.owner, &self.group, &mut self.rng, &operation)
            .unwrap()
    }
    fn install(&mut self, receipt: &Receipt, seed: &CheckpointSeed) {
        self.install_seed_for_test(receipt.clone(), seed);
    }
    fn restore_bytes(&self, bytes: &[u8]) -> Result<RegistryEpoch, ReplError> {
        RegistryEpoch::restore(
            bytes,
            &self.group,
            self.key.bucket(),
            self.owner.device_id(),
        )
    }
}
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/epoch/repair_transition/typed_tests.rs"
));

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/epoch/repair_transition/joint_typed_tests.rs"
));
