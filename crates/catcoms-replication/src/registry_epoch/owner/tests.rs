use super::*;
use crate::registry::{PointerKey, RegistryOp};
use crate::registry_epoch::settlement::tests::fill_close;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

mod frozen;

struct Fixture {
    owner: MlsDevice,
    rejoining_owner: MlsDevice,
    group: ServerGroup,
    source: RegistryEpoch,
    key: PointerKey,
    rng: ChaCha20Rng,
}
impl Fixture {
    fn new() -> Self {
        let owner = MlsDevice::generate().unwrap();
        // A same-key provider without the original group models re-admission after leaving.
        let rejoining_owner = owner.duplicate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let key = PointerKey::new(DocType::StudioObject, b"owner-cat".to_vec()).unwrap();
        let source = RegistryEpoch::new(&group, key.bucket(), owner.device_id()).unwrap();
        Self {
            owner,
            rejoining_owner,
            group,
            source,
            key,
            rng: ChaCha20Rng::seed_from_u64(971),
        }
    }
    fn fill(&mut self) {
        fill_close(
            &mut self.source,
            &self.owner,
            &self.group,
            &self.key,
            &mut self.rng,
        );
    }
    fn decide(&mut self, previous: Option<&Receipt>) -> RegistryOwnerDecision {
        self.source
            .new_owner_decision(&self.group, &self.owner, 0, previous)
            .unwrap()
    }
    fn install(&mut self, decision: &RegistryOwnerDecision) {
        self.source
            .seal(decision.receipt().clone(), &self.group, 0)
            .unwrap();
        let plan = self
            .source
            .prepare_settlement(decision.close(), &self.group, 0)
            .unwrap();
        self.source = self
            .source
            .checkpoint_successor(&plan, &self.group, 0)
            .unwrap();
    }
}

#[test]
fn registry_owner_decision_uses_eligible_closure_and_resumes_original_heads() {
    let mut f = Fixture::new();
    assert!(f
        .source
        .new_owner_decision(&f.group, &f.owner, 0, None)
        .is_err());
    f.fill();
    let decision = f.decide(None);
    let receipt = decision.receipt().encode();
    let close = decision.close().encode();
    let op = RegistryOp::Put {
        key: f.key.clone(),
        epoch: 77,
    }
    .domain_op(&f.group.group_id(), [77; 16])
    .unwrap();
    f.source.edit(&f.owner, &f.group, &mut f.rng, &op).unwrap();
    let saved = f.source.snapshot().unwrap();
    f.source =
        RegistryEpoch::restore(&saved, &f.group, f.key.bucket(), f.owner.device_id()).unwrap();
    let resumed = f
        .source
        .resume_owner_decision(&f.group, &f.owner, 0, decision.receipt(), decision.close())
        .unwrap();
    assert_eq!(resumed.receipt().encode(), receipt);
    assert_eq!(resumed.close().encode(), close);
    f.source
        .seal(resumed.receipt().clone(), &f.group, 0)
        .unwrap();
    let plan = f
        .source
        .prepare_settlement(resumed.close(), &f.group, 0)
        .unwrap();
    assert_eq!(plan.included_operations().len(), 10);
    assert_eq!(plan.excluded_operations().len(), 1);
    assert_eq!(plan.source_projection().pointers[&f.key], 77);
    assert!(f
        .source
        .new_owner_decision(&f.group, &f.owner, 0, None)
        .is_err());
}

#[test]
fn registry_owner_decision_repeated_rotations_keep_inheritance_and_cannot_reseed_edits() {
    let mut f = Fixture::new();
    let mut previous = None;
    for epoch in 0..3 {
        f.fill();
        let decision = f.decide(previous.as_ref());
        assert_eq!(decision.receipt().closed_epoch, epoch);
        assert_eq!(decision.receipt().inherited, InheritedCheckpoint::EpochZero);
        f.install(&decision);
        let op = RegistryOp::Put {
            key: f.key.clone(),
            epoch: 80 + epoch,
        }
        .domain_op(&f.group.group_id(), [80 + epoch as u8; 16])
        .unwrap();
        f.source.edit(&f.owner, &f.group, &mut f.rng, &op).unwrap();
        let before = f.source.snapshot().unwrap();
        f.source
            .resume_owner_decision(&f.group, &f.owner, 0, decision.receipt(), decision.close())
            .unwrap();
        assert_eq!(f.source.snapshot().unwrap(), before);
        // A lost same-tenure journal cannot be replaced by a new inherited baseline.
        assert!(f
            .source
            .new_owner_decision(&f.group, &f.owner, 0, None)
            .is_err());
        previous = Some(decision.receipt().clone());
    }
}

#[test]
fn registry_owner_decision_refuses_wrong_owner_fault_and_wrong_saved_seed_or_close() {
    let mut f = Fixture::new();
    f.fill();
    let other = MlsDevice::generate().unwrap();
    assert!(f
        .source
        .new_owner_decision(&f.group, &other, 0, None)
        .is_err());
    assert!(f
        .source
        .new_owner_decision(&f.group, &f.owner, 1, None)
        .is_err());
    let decision = f.decide(None);
    let wrong = Receipt::sign(
        decision.receipt().document.clone(),
        0,
        decision.close().hash(),
        [9; 32],
        0,
        InheritedCheckpoint::EpochZero,
        &f.owner,
    )
    .unwrap();
    assert!(f
        .source
        .resume_owner_decision(&f.group, &f.owner, 0, &wrong, decision.close())
        .is_err());
    let mut close = decision.close().clone();
    close.signature[0] ^= 1;
    assert!(f
        .source
        .resume_owner_decision(&f.group, &f.owner, 0, decision.receipt(), &close)
        .is_err());
    f.source
        .seal(decision.receipt().clone(), &f.group, 0)
        .unwrap();
    f.source.seal(wrong, &f.group, 0).unwrap();
    assert_eq!(f.source.phase(), EpochPhase::Fault);
    assert!(f
        .source
        .resume_owner_decision(&f.group, &f.owner, 0, decision.receipt(), decision.close())
        .is_err());
}

#[test]
fn registry_owner_decision_refuses_more_than_64_heads_without_truncating_the_closure() {
    let mut f = Fixture::new();
    let mut members = Vec::new();
    for _ in 0..64 {
        let member = MlsDevice::generate().unwrap();
        f.group
            .add_member(&f.owner, member.key_package().unwrap())
            .unwrap();
        members.push(member);
    }
    f.fill(); // An eligible owner branch plus 64 independent member branches.
    for (n, member) in members.iter().enumerate() {
        let mut independent =
            RegistryEpoch::new(&f.group, f.key.bucket(), member.device_id()).unwrap();
        let op = RegistryOp::Put {
            key: f.key.clone(),
            epoch: 100 + n as u64,
        }
        .domain_op(&f.group.group_id(), [n as u8; 16])
        .unwrap();
        let sealed = independent.edit(member, &f.group, &mut f.rng, &op).unwrap();
        f.source.ingest(&sealed, &f.group, &f.owner).unwrap();
    }
    assert_eq!(f.source.doc.heads().len(), 65);
    let before = f.source.snapshot().unwrap();
    assert!(matches!(
        f.source.new_owner_decision(&f.group, &f.owner, 0, None),
        Err(ReplError::EpochBound)
    ));
    assert_eq!(f.source.snapshot().unwrap(), before);
}

#[test]
fn registry_owner_decision_succession_inherits_actual_opening_and_returning_owner_is_distinct() {
    let mut f = Fixture::new();
    f.fill();
    let first = f.decide(None);
    f.install(&first);
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
    let tenure = next_group.epoch();
    f.source = RegistryEpoch::restore(
        &f.source.snapshot().unwrap(),
        &next_group,
        f.key.bucket(),
        next.device_id(),
    )
    .unwrap();
    fill_close(&mut f.source, &next, &next_group, &f.key, &mut f.rng);
    let second = f
        .source
        .new_owner_decision(&next_group, &next, tenure, None)
        .unwrap();
    assert_eq!(
        second.receipt().inherited,
        InheritedCheckpoint::Checkpoint {
            epoch: 1,
            close_record_hash: first.receipt().close_record_hash,
            seed_change_hash: first.receipt().seed_change_hash,
        }
    );
    f.source
        .seal(second.receipt().clone(), &next_group, tenure)
        .unwrap();
    let plan = f
        .source
        .prepare_settlement(second.close(), &next_group, tenure)
        .unwrap();
    f.source = f
        .source
        .checkpoint_successor(&plan, &next_group, tenure)
        .unwrap();
    // A rejoins the vacated low leaf, becoming owner in a new, independently observed epoch.
    let welcome = next_group
        .add_member(&next, f.rejoining_owner.key_package().unwrap())
        .unwrap()
        .welcome;
    let returning_group = ServerGroup::join(&f.rejoining_owner, &welcome).unwrap();
    let returning_tenure = returning_group.epoch();
    assert!(returning_tenure > tenure);
    assert_eq!(
        returning_group.designated_committer(),
        Some(f.owner.device_id())
    );
    f.source = RegistryEpoch::restore(
        &f.source.snapshot().unwrap(),
        &returning_group,
        f.key.bucket(),
        f.owner.device_id(),
    )
    .unwrap();
    fill_close(
        &mut f.source,
        &f.rejoining_owner,
        &returning_group,
        &f.key,
        &mut f.rng,
    );
    let third = f
        .source
        .new_owner_decision(
            &returning_group,
            &f.rejoining_owner,
            returning_tenure,
            Some(first.receipt()),
        )
        .unwrap();
    assert_eq!(
        third.receipt().inherited,
        InheritedCheckpoint::Checkpoint {
            epoch: 2,
            close_record_hash: second.receipt().close_record_hash,
            seed_change_hash: second.receipt().seed_change_hash,
        }
    );
    assert_ne!(third.receipt().tenure_id, first.receipt().tenure_id);
}
