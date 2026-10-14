//! Exercise the real signed P1 consumer, including retained exact retries, scoped restart,
//! recovery-first settlement evidence and a reusable seeded slot. No store/native authority is
//! fabricated by these core tests; successor construction alone is deliberately not durability.
use super::*;
use crate::{EpochPhase, LocalIntent, RecoveryReason};
use catcoms_mls::{MlsDevice, ServerGroup};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::collections::BTreeMap;

fn target() -> StudioTarget {
    StudioTarget::Score {
        channel: [7; 16],
        object: [9; 16],
    }
}
fn operation(unit: &StudioEpoch, op: &ScoreOp, n: u128) -> DomainOp {
    DomainOp {
        nonce: n.to_be_bytes(),
        doc_type: DocType::StudioObject,
        logical_key: unit.document().logical_key.clone(),
        body: op.encode().unwrap(),
    }
}
fn drum() -> ScoreOp {
    ScoreOp::SetCell {
        step: 0,
        row: 0,
        value: ScoreCell::Drum { pad: 0 },
    }
}

#[test]
fn pristine_score_restart_binds_family_before_any_authored_root_header() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let mut score = StudioEpoch::new(&group, target(), owner.device_id()).unwrap();
    let bytes = score.snapshot().unwrap();
    assert!(StudioEpoch::restore(&bytes, &group, target(), owner.device_id()).is_ok());
    let art = StudioTarget::Flipnote {
        channel: [7; 16],
        object: [9; 16],
    };
    assert!(StudioEpoch::restore(&bytes, &group, art, owner.device_id()).is_err());
    assert!(StudioEpoch::restore(
        &bytes[..bytes.len() - 1],
        &group,
        target(),
        owner.device_id()
    )
    .is_err());
    let mut legacy = StudioEpoch::new(&group, art, owner.device_id()).unwrap();
    assert!(StudioEpoch::restore(
        &legacy.snapshot().unwrap(),
        &group,
        target(),
        owner.device_id()
    )
    .is_err());
}

#[test]
fn signed_score_restarts_and_exact_retry_does_not_resurrect_a_clear() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let mut rng = ChaCha20Rng::from_seed([66; 32]);
    let mut unit = StudioEpoch::new(&group, target(), owner.device_id()).unwrap();
    let set = operation(&unit, &drum(), 1);
    let first = unit
        .edit_or_reseal(&owner, &group, &mut rng, &set, 100)
        .unwrap();
    let clear = operation(&unit, &ScoreOp::ClearCell { step: 0, row: 0 }, 2);
    unit.edit_or_reseal(&owner, &group, &mut rng, &clear, 101)
        .unwrap();
    let bytes = unit.snapshot().unwrap();
    let mut restored = StudioEpoch::restore(&bytes, &group, target(), owner.device_id()).unwrap();
    assert_eq!(restored.projection().unwrap(), unit.projection().unwrap());
    let again = restored
        .edit_or_reseal(&owner, &group, &mut rng, &set, 999)
        .unwrap();
    let key = group
        .channel_secret(&owner, first.doc_type, first.doc_id)
        .unwrap();
    assert_eq!(
        first.open(&key).unwrap().encode(),
        again.open(&key).unwrap().encode()
    );
    assert_eq!(restored.snapshot().unwrap(), bytes);
    let StudioProjection::Score(p) = restored.projection().unwrap() else {
        panic!()
    };
    assert!(p.cells[&(0, 0)].removed());
    assert!(StudioEpoch::restore(
        &bytes,
        &group,
        StudioTarget::Flipnote {
            channel: [7; 16],
            object: [9; 16]
        },
        owner.device_id()
    )
    .is_err());
    assert!(restored.blob_cids().unwrap().is_empty());
    let retype = operation(&restored, &drum(), 3);
    restored
        .edit_or_reseal(&owner, &group, &mut rng, &retype, 102)
        .unwrap();
    let mut wrong = set;
    wrong.body = ScoreOp::ClearCell { step: 0, row: 0 }.encode().unwrap();
    assert!(matches!(
        restored.validate_local_edit(&owner, &group, &wrong, 0),
        Err(ReplError::IntentConflict)
    ));
}

#[test]
fn score_settlement_preserves_removed_evidence_then_reuses_slot_after_seed() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let mut rng = ChaCha20Rng::from_seed([67; 32]);
    let mut unit = StudioEpoch::new(&group, target(), owner.device_id()).unwrap();
    // Meet the EXISTING P1 close minimum with real admitted signed metadata, as the owner
    // integration fixture does. This is not a lower-bound override or a forged gate counter.
    use automerge::{
        transaction::{CommitOptions, Transactable},
        ActorId, AutoCommit, ROOT,
    };
    let mut writer =
        AutoCommit::new().with_actor(ActorId::from(owner.device_id().as_bytes().to_vec()));
    for n in 1..=10 {
        let body = match n {
            1 => drum(),
            2 => ScoreOp::ClearCell { step: 0, row: 0 },
            _ => ScoreOp::SetHeader(ScoreHeader::Title(format!("title {n}"))),
        };
        let domain = operation(&unit, &body, n);
        prepare(
            &ScoreProjection::read(unit.document(), [7; 16], 0, &writer).unwrap(),
            &domain,
            &owner.device_id(),
        )
        .unwrap()
        .write(&mut writer)
        .unwrap();
        writer
            .put(
                ROOT,
                format!("_p1/op/{}", hex(&domain.id(&owner.device_id()))),
                1u64,
            )
            .unwrap();
        writer.commit_with(CommitOptions::default().with_message("x".repeat(220_000)));
        let signed = crate::SignedOp::sign_domain(
            &owner,
            DocType::StudioObject,
            unit.doc_id(),
            writer.get_last_local_change().unwrap().raw_bytes().to_vec(),
            &domain,
        )
        .unwrap();
        let sealed = crate::SealedOp::seal(&signed, &group, &owner, &mut rng).unwrap();
        assert_eq!(
            unit.ingest(&sealed, &group, &owner).unwrap(),
            crate::Admission::Accepted
        );
    }
    let decision = unit.new_owner_decision(&group, &owner, 0, None).unwrap();
    unit.seal(decision.receipt().clone(), &group, 0).unwrap();
    assert_eq!(unit.phase(), EpochPhase::Closing);
    let plan = unit
        .prepare_settlement(decision.close(), &group, 0)
        .unwrap();
    let recovery = plan
        .recovery_snapshot()
        .expect("even fully included removal needs recovery before omission");
    let recovered = StudioRecovery::from_snapshot(recovery, unit.document(), [7; 16]).unwrap();
    let StudioProjection::Score(old) = recovered.projection() else {
        panic!()
    };
    assert!(old.cells[&(0, 0)].removed());
    assert!(recovered.blob_cids().unwrap().is_empty());
    assert!(
        StudioRecovery::inspect_vault_references(recovery, unit.document())
            .unwrap()
            .is_empty()
    );
    let mut next = unit.checkpoint_successor(&plan, &group, 0).unwrap();
    let StudioProjection::Score(p) = next.projection().unwrap() else {
        panic!()
    };
    assert!(p.cells.is_empty());
    assert_eq!(next.epoch(), 1);
    let set = operation(&next, &drum(), 11);
    next.edit_or_reseal(&owner, &group, &mut rng, &set, 3)
        .unwrap();
    let bytes = next.snapshot().unwrap();
    let restored = StudioEpoch::restore(&bytes, &group, target(), owner.device_id()).unwrap();
    let StudioProjection::Score(p) = restored.projection().unwrap() else {
        panic!()
    };
    assert!(p.cells[&(0, 0)].value().is_some());
}

#[test]
fn score_recovery_validates_family_before_returning_empty_blob_references() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let unit = StudioEpoch::new(&group, target(), owner.device_id()).unwrap();
    let op = operation(&unit, &drum(), 1);
    let projection = unit.projection().unwrap();
    let mut operations = BTreeMap::from([(
        op.id(&owner.device_id()),
        LocalIntent {
            author: owner.device_id(),
            operation: op,
        },
    )]);
    let snapshot = StudioRecovery::snapshot(
        &projection,
        None,
        RecoveryReason::Rewound,
        [0; 32],
        &operations,
    )
    .unwrap();
    assert!(
        StudioRecovery::from_snapshot(&snapshot, unit.document(), [7; 16])
            .unwrap()
            .blob_cids()
            .unwrap()
            .is_empty()
    );
    let intent = operations.values_mut().next().unwrap();
    intent.operation.body = FlipnoteOp::InsertFrame {
        frame: [1; 16],
        after: None,
        cid: [2; 32],
        bytes: 10,
    }
    .encode()
    .unwrap();
    assert!(StudioRecovery::snapshot(
        &projection,
        None,
        RecoveryReason::Rewound,
        [0; 32],
        &operations
    )
    .is_err());
    let mut truncated = snapshot;
    truncated.projection.pop();
    assert!(StudioRecovery::inspect_vault_references(&truncated, unit.document()).is_err());
    let invalid = operation(&unit, &drum(), 2);
    let mut invalid = invalid;
    invalid.body = b"{}".to_vec();
    assert!(target()
        .operation_blob_cid(unit.document(), &invalid)
        .is_err());
}
