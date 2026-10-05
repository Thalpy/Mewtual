//! A faulted Registry bucket blocks Index and Flipnote discovery, so its repair is the same
//! transaction as Studio's: issuance at B1, application at B2, recycling back to ordinary.
use super::*;
use crate::store::{OfferedRepairEvidence, StudioRepairOutcome, StudioRepairRequest};
use catcoms_replication::{ReceiptRepair, RepairDisposition, ReplError};
use catcoms_rt::ManualClock;

fn faulted(f: &mut Fixture, store: &mut ServerStore) -> [Receipt; 2] {
    let mut b = budget(store, f);
    let op = f.op(1);
    f.ingest(store, &op, &mut b).unwrap();
    let pair = [f.receipt(7), f.receipt(8)];
    for receipt in &pair {
        store
            .seal_registry_epoch(
                SERVER,
                &f.group,
                f.key.bucket(),
                &f.device,
                receipt.clone(),
                0,
                &mut rng(),
                &mut b,
            )
            .unwrap();
    }
    assert_eq!(f.load(store).unwrap().phase(), EpochPhase::Fault);
    let mut sorted = pair;
    sorted.sort_by_key(Receipt::hash);
    sorted
}

fn issue(
    f: &Fixture,
    store: &mut ServerStore,
    pair: &[Receipt; 2],
    selected: &Receipt,
) -> Result<(ReceiptRepair, StudioRepairOutcome, EpochRegistryState), AppError> {
    let mut b = budget(store, f);
    store.issue_registry_repair(
        SERVER,
        &f.group,
        f.key.bucket(),
        &f.device,
        0,
        StudioRepairRequest {
            receipt_a: pair[0].hash(),
            receipt_b: pair[1].hash(),
            selected: selected.hash(),
        },
        None,
        &ManualClock::new(1000),
        &mut rng(),
        &mut b,
    )
}

#[test]
fn registry_repair_sequence_exhaustion_matches_studio_without_wrapping() {
    use super::super::super::epoch_owner::next_repair_sequence;

    assert_eq!(next_repair_sequence(7, u64::MAX - 1).unwrap(), u64::MAX);
    assert!(matches!(
        next_repair_sequence(7, u64::MAX),
        Err(ReplError::RepairSequenceExhausted)
    ));
    assert_eq!(next_repair_sequence(8, 3).unwrap(), 9);
}

#[test]
fn a_faulted_bucket_is_repaired_recycled_and_served_without_a_held_proof() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let pair = faulted(&mut f, &mut store);
    // A stale echo naming another selection refuses before any write.
    let path = f.path(&store);
    let before = fs::read(&path).unwrap();
    let mut bogus = pair.clone();
    bogus[1] = f.receipt(9);
    assert!(issue(&f, &mut store, &bogus, &pair[0]).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);

    let (repair, outcome, state) = issue(&f, &mut store, &pair, &pair[0]).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::Repaired);
    assert_eq!(state.phase(), EpochPhase::Closing);
    assert!(
        store.load_epoch_owner_receipts(SERVER, &f.document).is_ok(),
        "terminal recycling returned the owner record to ordinary"
    );
    drop(store);
    let mut store = open(root.path());
    let restored = f.load(&store).unwrap();
    assert_eq!(restored.phase(), EpochPhase::Closing);
    let resolved = restored.unit.repair_state().unwrap();
    assert_eq!(resolved.repair, repair);
    assert_eq!(resolved.disposition, RepairDisposition::Transitioned);
    // Head service carries the applied repair to peers faulted on the same pair.
    let mut b = budget(&mut store, &f);
    let (_, served) = store
        .prepare_registry_head_with_fault_repair(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            Some(0),
            None,
            None,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(served.as_ref(), Some(&repair));
    // Nothing remains decidable once the bucket has left Fault.
    assert!(issue(&f, &mut store, &pair, &pair[0]).is_err());
}

#[test]
fn repair_only_registry_head_repeats_uncertain_b2_durability_before_service() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let pair = faulted(&mut f, &mut store);
    let mut b = budget(&mut store, &f);
    let error = store
        .issue_registry_repair_with_io(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            0,
            StudioRepairRequest {
                receipt_a: pair[0].hash(),
                receipt_b: pair[1].hash(),
                selected: pair[0].hash(),
            },
            None,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
            &mut WriteHooks::fail_after_write(FailError::NotDurable(
                "uncertain Registry B2 source",
            ))
            .at(WriteTag::Source),
        )
        .unwrap_err();
    assert!(matches!(error, AppError::CommittedButNotDurable(_)));
    drop(store);

    let mut store = open(root.path());
    let repair = f
        .load(&store)
        .unwrap()
        .unit
        .repair_state()
        .expect("the visible bucket carries the repair")
        .repair;
    let mut b = budget(&mut store, &f);
    let refused = store.prepare_registry_head_and_repair(
        SERVER,
        &f.group,
        f.key.bucket(),
        &f.device,
        Some(0),
        None,
        None,
        &mut rng(),
        &mut b,
        &mut WriteHooks::fail_before_sync(FailError::NotDurable(
            "Registry source durability still unavailable",
        ))
        .at(WriteTag::Source),
    );
    assert!(refused.is_err(), "repair service bypassed the source flush");

    let mut b = budget(&mut store, &f);
    let refused = store.prepare_registry_head_and_repair(
        SERVER,
        &f.group,
        f.key.bucket(),
        &f.device,
        Some(0),
        None,
        None,
        &mut rng(),
        &mut b,
        &mut WriteHooks::fail_before_write(FailError::NotDurable(
            "Registry owner repair journal durability still unavailable",
        ))
        .at(WriteTag::Journal),
    );
    assert!(
        refused.is_err(),
        "repair service bypassed the owner-record re-save"
    );

    let mut b = budget(&mut store, &f);
    let refused = store.prepare_registry_head_and_repair(
        SERVER,
        &f.group,
        f.key.bucket(),
        &f.device,
        Some(0),
        None,
        None,
        &mut rng(),
        &mut b,
        &mut WriteHooks::fail_after_write(FailError::NotDurable(
            "Registry owner repair journal replacement is visible but uncertain",
        ))
        .at(WriteTag::Journal),
    );
    assert!(
        matches!(refused, Err(AppError::CommittedButNotDurable(_))),
        "repair service treated an uncertain B3 owner-record replacement as durable"
    );
    drop(store);

    // Re-open so the final service result proves that no in-memory success leaked through the
    // uncertain owner-record write. The clean retry must repeat both durability barriers.
    let mut store = open(root.path());

    let mut b = budget(&mut store, &f);
    let (selection, served) = store
        .prepare_registry_head_with_fault_repair(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            Some(0),
            None,
            None,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert!(!selection.prove);
    assert_eq!(served, Some(repair));
}

#[test]
fn a_held_bucket_decision_fences_adoption_defers_installs_and_resumes_to_ordinary() {
    let root = tempfile::tempdir().unwrap();
    let mut f = Fixture::new();
    let mut store = open(root.path());
    let pair = faulted(&mut f, &mut store);
    let mut fail_source = |tag: WriteTag, _: &Path, _: &[u8]| {
        if tag == WriteTag::Source {
            Intercept::Fail(AppError::Io("b2".into()))
        } else {
            Intercept::Continue
        }
    };
    let mut b = budget(&mut store, &f);
    assert!(store
        .issue_registry_repair_with_io(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            0,
            StudioRepairRequest {
                receipt_a: pair[0].hash(),
                receipt_b: pair[1].hash(),
                selected: pair[0].hash(),
            },
            None,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
            &mut WriteHooks::Hooked {
                before: Some(&mut fail_source),
                before_sync: None,
                before_unlink: None,
                after: None,
            },
        )
        .is_err());
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    assert!(store
        .load_epoch_owner_receipts(SERVER, &f.document)
        .is_err());
    // Another owner-proved receipt cannot install into the bucket the decision owns.
    let other = f.receipt(9);
    let mut b = budget(&mut store, &f);
    assert!(
        store
            .adopt_registry_checkpoint(
                SERVER,
                &f.group,
                f.key.bucket(),
                &f.device,
                &other,
                None,
                0,
                &ManualClock::new(1000),
                &mut rng(),
                &mut b,
            )
            .is_err(),
        "ordinary adoption must not bypass a held bucket decision"
    );
    // Before B2 the bucket owes nothing, so only the held decision defers the install.
    assert!(store
        .owed_registry_repair(SERVER, &f.group, f.key.bucket(), &f.device)
        .unwrap()
        .is_none());
    assert!(store
        .registry_install_deferred_by_repair(SERVER, &f.group, f.key.bucket(), None, &other)
        .unwrap());
    // Legacy maintenance does not run against the held bucket.
    assert!(!store.epoch_owner_is_ordinary(SERVER, &f.document).unwrap());
    // Resuming the exact decision completes it and recycles the record.
    let (_, outcome, state) = issue(&f, &mut store, &pair, &pair[0]).unwrap();
    assert_eq!(outcome, StudioRepairOutcome::Repaired);
    assert_eq!(state.phase(), EpochPhase::Closing);
    assert!(store.load_epoch_owner_receipts(SERVER, &f.document).is_ok());
    assert!(!store
        .registry_install_deferred_by_repair(SERVER, &f.group, f.key.bucket(), None, &other)
        .unwrap());
}

#[test]
fn a_peer_applies_an_owner_bucket_repair_and_keeps_no_owner_record() {
    let root = tempfile::tempdir().unwrap();
    let mut owner = Fixture::new();
    let peer = MlsDevice::generate().unwrap();
    let welcome = owner
        .group
        .add_member(&owner.device, peer.key_package().unwrap())
        .unwrap()
        .welcome;
    let peer_group = ServerGroup::join(&peer, &welcome).unwrap();
    let mut store = open(root.path());
    // The peer's own bucket, faulted on two receipts the owner signed.
    let mut f = Fixture {
        source: RegistryEpoch::new(&peer_group, owner.key.bucket(), peer.device_id()).unwrap(),
        device: peer,
        group: peer_group,
        key: owner.key.clone(),
        document: owner.document.clone(),
    };
    let mut b = budget(&mut store, &f);
    let op = f.op(1);
    f.ingest(&mut store, &op, &mut b).unwrap();
    let signed = |close: u8| {
        let seed = f
            .source
            .projection()
            .unwrap()
            .checkpoint([close; 32])
            .unwrap();
        Receipt::sign(
            f.document.clone(),
            0,
            [close; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &owner.device,
        )
        .unwrap()
    };
    let mut pair = [signed(7), signed(8)];
    for receipt in &pair {
        store
            .seal_registry_epoch(
                SERVER,
                &f.group,
                f.key.bucket(),
                &f.device,
                receipt.clone(),
                0,
                &mut rng(),
                &mut b,
            )
            .unwrap();
    }
    pair.sort_by_key(Receipt::hash);
    let repair = ReceiptRepair::sign_in_tenure(
        f.document.clone(),
        pair[0].tenure_id,
        [pair[0].hash(), pair[1].hash()],
        pair[1].hash(),
        1,
        0,
        &owner.device,
    )
    .unwrap();
    // Offered evidence, read before any work: the faulted bucket holds both receipts.
    let evidence = |store: &ServerStore, repair: &ReceiptRepair| {
        store
            .registry_repair_evidence(SERVER, &f.group, f.key.bucket(), &f.device, repair, None)
            .unwrap()
    };
    assert!(
        matches!(evidence(&store, &repair), OfferedRepairEvidence::Pair(held) if *held == pair)
    );
    // A repair naming a receipt this device never held cannot be applied, but may become
    // applicable later, so it is not terminal.
    let mut unheld = [pair[0].hash(), [9; 32]];
    unheld.sort();
    let unheld = ReceiptRepair::sign_in_tenure(
        f.document.clone(),
        pair[0].tenure_id,
        unheld,
        pair[0].hash(),
        1,
        0,
        &owner.device,
    )
    .unwrap();
    assert!(matches!(
        evidence(&store, &unheld),
        OfferedRepairEvidence::Unverifiable
    ));
    let mut b = budget(&mut store, &f);
    let (outcome, state) = store
        .apply_registry_repair(
            SERVER,
            &f.group,
            f.key.bucket(),
            &f.device,
            &repair,
            &pair,
            0,
            None,
            &ManualClock::new(1000),
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(outcome, StudioRepairOutcome::Repaired);
    assert_eq!(state.phase(), EpochPhase::Closing);
    assert!(store
        .epoch_owner_receipt_inventory_record(SERVER, &f.document)
        .unwrap()
        .is_none());
    // Applied and not owing a replacement: later answers carrying it are terminal here.
    assert!(matches!(
        evidence(&store, &repair),
        OfferedRepairEvidence::Terminal
    ));
}
