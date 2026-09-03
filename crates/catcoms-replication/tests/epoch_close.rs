//! Adversarial P1 regressions. These tests exercise protocol records and the real shared epoch
//! gate rather than a UI-level model, so signature binding, crash replay, and seal/ingest races
//! cannot silently diverge from enforcement.

use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};
use std::thread;

use catcoms_crypto::DeviceId;
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::{
    Admission, AdmittedOperation, CloseRecord, EncryptedDoc, EpochGate, EpochPhase,
    InheritedCheckpoint, IntentLedger, LogicalDocument, OwnerReceiptJournal, Receipt, ReceiptBook,
    ReceiptHeadProof, ReceiptIngest, ReceiptRepair, RecoveryReason, RecoverySlots,
    RecoverySnapshot, RecoveryTransition, ReplError, SealedOp, SignedOp,
};
use catcoms_wire::DocType;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn solo() -> (MlsDevice, ServerGroup, LogicalDocument) {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let document = LogicalDocument::new(
        group.group_id(),
        DocType::StudioObject,
        b"score-17".to_vec(),
    )
    .unwrap();
    (owner, group, document)
}

fn receipt(
    owner: &MlsDevice,
    group: &ServerGroup,
    document: &LogicalDocument,
    epoch: u64,
    inherited: InheritedCheckpoint,
) -> Receipt {
    Receipt::sign(
        document.clone(),
        epoch,
        [epoch as u8; 32],
        [epoch.wrapping_add(1) as u8; 32],
        group.epoch(),
        inherited,
        owner,
    )
    .unwrap()
}

fn snapshot(epoch: u64) -> RecoverySnapshot {
    RecoverySnapshot {
        doc_type: DocType::StudioObject,
        logical_key: b"score-17".to_vec(),
        epoch,
        base_close_record_hash: None,
        reason: RecoveryReason::Excluded,
        projection: vec![epoch as u8],
        tombstones: Vec::new(),
        elements: Vec::new(),
        conflicts: Vec::new(),
        applied_ops: Vec::new(),
    }
}

fn metadata_gate(epoch: u64, owner: DeviceId) -> EpochGate {
    let document = LogicalDocument::new(
        b"test-server".to_vec(),
        DocType::StudioObject,
        b"gate-only".to_vec(),
    )
    .unwrap();
    let doc_id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    EpochGate::new(document, doc_id, epoch, owner)
}

fn ingest_receipt(
    book: &mut ReceiptBook,
    receipt: Receipt,
    group: &ServerGroup,
    expected_tenure_start_group_epoch: u64,
) -> ReceiptIngest {
    let gate = EpochGate::new(
        receipt.document.clone(),
        catcoms_replication::epoch_zero_id(
            receipt.document.doc_type,
            &receipt.document.logical_key,
        ),
        receipt.closed_epoch,
        group.designated_committer().unwrap(),
    );
    book.ingest_and_seal(receipt, group, expected_tenure_start_group_epoch, &gate)
        .unwrap()
        .0
}

#[test]
fn receipt_authenticates_the_inherited_seed_and_carries_recomputable_tenure_epoch() {
    let (owner, group, document) = solo();
    let inherited = InheritedCheckpoint::Checkpoint {
        epoch: 7,
        close_record_hash: [0x44; 32],
        seed_change_hash: [0x55; 32],
    };
    let signed = receipt(&owner, &group, &document, 7, inherited);
    let decoded = Receipt::decode(&signed.encode()).unwrap();
    decoded.verify_current_owner(&group, group.epoch()).unwrap();
    assert_eq!(decoded.inherited.epoch(), 7);
    assert_eq!(decoded.tenure_start_group_epoch, group.epoch());

    let mut seed_tamper = decoded.clone();
    seed_tamper.inherited = InheritedCheckpoint::Checkpoint {
        epoch: 7,
        close_record_hash: [0x44; 32],
        seed_change_hash: [0x56; 32],
    };
    assert!(matches!(
        seed_tamper.verify_current_owner(&group, group.epoch()),
        Err(ReplError::EpochAuthority)
    ));

    let mut epoch_tamper = decoded;
    epoch_tamper.tenure_start_group_epoch += 1;
    assert!(matches!(
        epoch_tamper.verify_current_owner(&group, group.epoch()),
        Err(ReplError::EpochAuthority)
    ));
}

#[test]
fn old_receipt_from_a_repeated_owner_tenure_needs_fresh_tenure_evidence() {
    let (owner, group, document) = solo();
    let old_tenure = receipt(&owner, &group, &document, 0, InheritedCheckpoint::EpochZero);
    assert!(matches!(
        old_tenure.verify_current_owner(&group, group.epoch() + 2),
        Err(ReplError::EpochAuthority)
    ));

    assert_ne!(
        old_tenure.tenure_id,
        catcoms_replication::tenure_id(
            &document.server_id,
            &owner.public_key_bytes(),
            group.epoch() + 2,
        )
    );
}

#[test]
fn current_head_proof_binds_receipt_requester_and_fresh_nonce() {
    let (owner, group, document) = solo();
    let signed = receipt(&owner, &group, &document, 0, InheritedCheckpoint::EpochZero);
    let requester = DeviceId::from_bytes([0x41; 32]);
    let proof = ReceiptHeadProof::sign(&signed, requester, [0x42; 16], &owner).unwrap();
    let decoded = ReceiptHeadProof::decode(&proof.encode()).unwrap();
    decoded
        .verify(&group, &signed, requester, &[0x42; 16])
        .unwrap();
    assert!(matches!(
        decoded.verify(&group, &signed, requester, &[0x43; 16]),
        Err(ReplError::EpochAuthority)
    ));
    assert!(matches!(
        decoded.verify(
            &group,
            &signed,
            DeviceId::from_bytes([0x44; 32]),
            &[0x42; 16],
        ),
        Err(ReplError::EpochAuthority)
    ));

    let other_receipt = receipt(&owner, &group, &document, 1, InheritedCheckpoint::EpochZero);
    assert!(matches!(
        decoded.verify(&group, &other_receipt, requester, &[0x42; 16]),
        Err(ReplError::EpochAuthority)
    ));
}

#[test]
fn first_tenure_decision_survives_restart_and_refuses_a_different_inheritance() {
    let (owner, group, document) = solo();
    let mut rejected_journal = OwnerReceiptJournal::default();
    let pristine = rejected_journal.encode();
    let invalid_first = receipt(&owner, &group, &document, 2, InheritedCheckpoint::EpochZero);
    assert!(matches!(
        rejected_journal.prepare(invalid_first, &group, group.epoch()),
        Err(ReplError::ReceiptConflict)
    ));
    assert_eq!(rejected_journal.encode(), pristine);

    let first = receipt(&owner, &group, &document, 0, InheritedCheckpoint::EpochZero);
    let mut journal = OwnerReceiptJournal::default();
    journal
        .prepare(first.clone(), &group, group.epoch())
        .unwrap();

    // The durable bytes are what the owner must commit before publishing. A crash at this point
    // restores the exact signature rather than re-evaluating whichever checkpoint is visible.
    let mut restored = OwnerReceiptJournal::decode(&journal.encode()).unwrap();
    assert_eq!(restored.in_flight(), Some(&first));

    let conflicting = receipt(
        &owner,
        &group,
        &document,
        3,
        InheritedCheckpoint::Checkpoint {
            epoch: 3,
            close_record_hash: [3; 32],
            seed_change_hash: [4; 32],
        },
    );
    assert!(matches!(
        restored.prepare(conflicting, &group, group.epoch()),
        Err(ReplError::ReceiptConflict)
    ));
    restored.mark_published(first.hash()).unwrap();
    assert!(restored.in_flight().is_none());

    let skipped = receipt(&owner, &group, &document, 2, InheritedCheckpoint::EpochZero);
    assert!(matches!(
        restored.prepare(skipped, &group, group.epoch()),
        Err(ReplError::ReceiptConflict)
    ));
}

#[test]
fn peer_enters_fault_for_inheritance_equivocation_in_either_delivery_order() {
    let (owner, group, document) = solo();
    let a = receipt(&owner, &group, &document, 5, InheritedCheckpoint::EpochZero);
    let b = receipt(
        &owner,
        &group,
        &document,
        6,
        InheritedCheckpoint::Checkpoint {
            epoch: 5,
            close_record_hash: [5; 32],
            seed_change_hash: [6; 32],
        },
    );
    a.verify_current_owner(&group, group.epoch()).unwrap();
    b.verify_current_owner(&group, group.epoch()).unwrap();

    let mut persisted = Vec::new();
    for (first, second) in [(a.clone(), b.clone()), (b.clone(), a.clone())] {
        let mut book = ReceiptBook::default();
        assert_eq!(
            ingest_receipt(&mut book, first, &group, group.epoch()),
            ReceiptIngest::Advanced
        );
        assert_eq!(
            ingest_receipt(&mut book, second, &group, group.epoch()),
            ReceiptIngest::Fault
        );
        assert!(book.is_faulted());
        let encoded = book.encode().unwrap();
        let restored = ReceiptBook::decode(&encoded).unwrap();
        assert!(restored.is_faulted());
        let third = receipt(&owner, &group, &document, 7, InheritedCheckpoint::EpochZero);
        assert_eq!(
            ingest_receipt(&mut book, third, &group, group.epoch()),
            ReceiptIngest::Fault
        );
        assert_eq!(book.encode().unwrap(), encoded);
        persisted.push(encoded);
    }
    assert_eq!(persisted[0], persisted[1]);
}

#[test]
fn owner_repair_selects_one_equivocating_receipt_and_returns_the_loser_for_recovery() {
    let (owner, group, document) = solo();
    let a = receipt(&owner, &group, &document, 5, InheritedCheckpoint::EpochZero);
    let b = receipt(
        &owner,
        &group,
        &document,
        6,
        InheritedCheckpoint::Checkpoint {
            epoch: 5,
            close_record_hash: [5; 32],
            seed_change_hash: [6; 32],
        },
    );
    let mut book = ReceiptBook::default();
    assert_eq!(
        ingest_receipt(&mut book, a.clone(), &group, group.epoch()),
        ReceiptIngest::Advanced
    );
    assert_eq!(
        ingest_receipt(&mut book, b.clone(), &group, group.epoch()),
        ReceiptIngest::Fault
    );

    let repair = ReceiptRepair::sign(
        document,
        a.tenure_id,
        [b.hash(), a.hash()],
        a.hash(),
        1,
        &owner,
    )
    .unwrap();
    let decoded = ReceiptRepair::decode(&repair.encode()).unwrap();
    decoded.verify_current_owner(&group).unwrap();
    assert_eq!(book.apply_repair(&decoded, &group).unwrap(), b);
    assert!(!book.is_faulted());
    assert_eq!(book.latest(), Some(&a));
    assert!(matches!(
        book.apply_repair(&decoded, &group),
        Err(ReplError::ReceiptConflict)
    ));
}

#[test]
fn receipt_state_refuses_cross_document_confusion() {
    let (owner, group, document) = solo();
    let other = LogicalDocument::new(
        document.server_id.clone(),
        document.doc_type,
        b"score-18".to_vec(),
    )
    .unwrap();
    let first = receipt(&owner, &group, &document, 0, InheritedCheckpoint::EpochZero);
    let foreign = receipt(&owner, &group, &other, 1, InheritedCheckpoint::EpochZero);

    let mut book = ReceiptBook::default();
    assert_eq!(
        ingest_receipt(&mut book, first.clone(), &group, group.epoch()),
        ReceiptIngest::Advanced
    );
    assert!(matches!(
        book.ingest_and_seal(
            foreign.clone(),
            &group,
            group.epoch(),
            &EpochGate::new(
                foreign.document.clone(),
                catcoms_replication::epoch_zero_id(
                    foreign.document.doc_type,
                    &foreign.document.logical_key,
                ),
                foreign.closed_epoch,
                group.designated_committer().unwrap(),
            ),
        ),
        Err(ReplError::EpochScope)
    ));

    let mut journal = OwnerReceiptJournal::default();
    journal
        .prepare(first.clone(), &group, group.epoch())
        .unwrap();
    journal.mark_published(first.hash()).unwrap();
    assert!(matches!(
        journal.prepare(foreign, &group, group.epoch()),
        Err(ReplError::EpochScope)
    ));
}

#[test]
fn close_signature_is_bound_to_server_document_epoch_and_heads() {
    let (owner, group, document) = solo();
    let id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let close = CloseRecord::sign(&document, id, 0, vec![[2; 32], [1; 32]], &owner).unwrap();
    close.verify_for(&document, id, &group, None).unwrap();
    assert_eq!(CloseRecord::decode(&close.encode()).unwrap(), close);

    let mut tampered = close.clone();
    tampered.closed_epoch = 1;
    assert!(matches!(
        tampered.verify_for(&document, id, &group, None),
        Err(ReplError::EpochAuthority)
    ));

    let other = LogicalDocument::new(
        document.server_id.clone(),
        DocType::PostReplies,
        document.logical_key.clone(),
    )
    .unwrap();
    assert!(matches!(
        close.verify_for(&other, id, &group, None),
        Err(ReplError::EpochScope)
    ));
}

#[test]
fn historical_close_authority_is_bound_to_the_exact_verified_receipt() {
    let (owner, group, document) = solo();
    let former_member = MlsDevice::generate().unwrap();
    let id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let close = CloseRecord::sign(&document, id, 0, vec![[7; 32]], &former_member).unwrap();
    let owner_receipt = Receipt::sign(
        document.clone(),
        0,
        close.hash(),
        [8; 32],
        group.epoch(),
        InheritedCheckpoint::EpochZero,
        &owner,
    )
    .unwrap();
    let verified_receipt = owner_receipt
        .verify_current_owner(&group, group.epoch())
        .unwrap();

    assert!(matches!(
        close.verify_for(&document, id, &group, None),
        Err(ReplError::EpochAuthority)
    ));
    close
        .verify_for(&document, id, &group, Some(&verified_receipt))
        .unwrap();
    let unrelated = receipt(&owner, &group, &document, 0, InheritedCheckpoint::EpochZero);
    let unrelated_verified = unrelated
        .verify_current_owner(&group, group.epoch())
        .unwrap();
    assert!(matches!(
        close.verify_for(&document, id, &group, Some(&unrelated_verified)),
        Err(ReplError::EpochAuthority)
    ));
}

#[test]
fn receipt_seal_and_inbound_ingest_have_only_the_two_allowed_interleavings() {
    let (owner_device, group, document) = solo();
    let doc_id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let signed_receipt = receipt(
        &owner_device,
        &group,
        &document,
        0,
        InheritedCheckpoint::EpochZero,
    );
    let receipt_hash = signed_receipt.hash();
    let verified = Arc::new(
        signed_receipt
            .verify_current_owner(&group, group.epoch())
            .unwrap(),
    );
    for sequence in 0..64u16 {
        let author = DeviceId::from_bytes([2; 32]);
        let gate = Arc::new(EpochGate::new(
            document.clone(),
            doc_id,
            0,
            owner_device.device_id(),
        ));
        let barrier = Arc::new(Barrier::new(3));
        let op = AdmittedOperation {
            op_hash: [sequence as u8; 32],
            domain_op_id: [sequence.wrapping_add(1) as u8; 32],
            author,
            encoded_len: 128,
        };

        let ingest_gate = Arc::clone(&gate);
        let ingest_barrier = Arc::clone(&barrier);
        let ingest = thread::spawn(move || {
            ingest_barrier.wait();
            ingest_gate.admit_inbound(op).unwrap()
        });
        let seal_gate = Arc::clone(&gate);
        let seal_barrier = Arc::clone(&barrier);
        let seal_receipt = Arc::clone(&verified);
        let seal = thread::spawn(move || {
            seal_barrier.wait();
            seal_gate.seal_verified_receipt(&seal_receipt).unwrap()
        });
        barrier.wait();

        let outcome = ingest.join().unwrap();
        let frozen = seal.join().unwrap();
        match outcome {
            Admission::Accepted => assert_eq!(frozen, vec![op]),
            Admission::Quarantined => {
                assert!(frozen.is_empty());
                assert_eq!(gate.quarantined_len(), 1);
            }
            other => panic!("unexpected gate race result: {other:?}"),
        }
        gate.mark_settled(receipt_hash).unwrap();
        assert_eq!(gate.phase(), EpochPhase::Settled);
        assert_eq!(gate.quarantined_len(), 0);
    }
}

#[test]
fn receipt_book_transition_seals_admission_and_persists_fault_as_one_gate_change() {
    let (owner, group, document) = solo();
    let doc_id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());
    let accepted = AdmittedOperation {
        op_hash: [1; 32],
        domain_op_id: [2; 32],
        author: owner.device_id(),
        encoded_len: 128,
    };
    gate.admit_local(accepted).unwrap();
    let first = receipt(&owner, &group, &document, 0, InheritedCheckpoint::EpochZero);
    let mut book = ReceiptBook::default();
    let (outcome, frozen) = book
        .ingest_and_seal(first, &group, group.epoch(), &gate)
        .unwrap();
    assert_eq!(outcome, ReceiptIngest::Advanced);
    assert_eq!(frozen, vec![accepted]);
    assert_eq!(gate.phase(), EpochPhase::Closing);
    assert!(matches!(
        gate.admit_local(AdmittedOperation {
            op_hash: [3; 32],
            domain_op_id: [4; 32],
            ..accepted
        }),
        Err(ReplError::EpochClosed)
    ));

    let conflicting = Receipt::sign(
        document,
        0,
        [0xee; 32],
        [0xef; 32],
        group.epoch(),
        InheritedCheckpoint::EpochZero,
        &owner,
    )
    .unwrap();
    let (outcome, _) = book
        .ingest_and_seal(conflicting, &group, group.epoch(), &gate)
        .unwrap();
    assert_eq!(outcome, ReceiptIngest::Fault);
    assert!(book.is_faulted());
    assert_eq!(gate.phase(), EpochPhase::Fault);
    assert_eq!(
        EpochGate::decode(&gate.encode().unwrap()).unwrap().phase(),
        EpochPhase::Fault
    );
}

#[test]
fn closing_gate_survives_restart_without_reopening_or_losing_quarantine() {
    let (owner_device, group, document) = solo();
    let owner = owner_device.device_id();
    let author = DeviceId::from_bytes([2; 32]);
    let accepted = AdmittedOperation {
        op_hash: [3; 32],
        domain_op_id: [13; 32],
        author,
        encoded_len: 100,
    };
    let late = AdmittedOperation {
        op_hash: [4; 32],
        domain_op_id: [14; 32],
        author,
        encoded_len: 100,
    };
    let doc_id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let signed_receipt = receipt(
        &owner_device,
        &group,
        &document,
        7,
        InheritedCheckpoint::EpochZero,
    );
    let receipt_hash = signed_receipt.hash();
    let verified = signed_receipt
        .verify_current_owner(&group, group.epoch())
        .unwrap();
    let gate = EpochGate::new(document, doc_id, 7, owner);
    assert_eq!(gate.admit_inbound(accepted).unwrap(), Admission::Accepted);
    assert_eq!(
        gate.seal_verified_receipt(&verified).unwrap(),
        vec![accepted]
    );
    assert_eq!(gate.admit_inbound(late).unwrap(), Admission::Quarantined);

    let restored = EpochGate::decode(&gate.encode().unwrap()).unwrap();
    assert_eq!(restored.epoch(), 7);
    assert_eq!(restored.phase(), EpochPhase::Closing);
    assert_eq!(restored.accepted_hashes(), vec![accepted.op_hash]);
    assert_eq!(restored.quarantined_len(), 1);
    assert!(matches!(
        restored.admit_local(late),
        Err(ReplError::EpochClosed)
    ));
    restored.mark_settled(receipt_hash).unwrap();
    assert_eq!(restored.quarantined_len(), 0);
}

#[test]
fn epoch_gate_rejects_conflicting_envelopes_with_one_domain_operation_id() {
    let owner = DeviceId::from_bytes([1; 32]);
    let author = DeviceId::from_bytes([2; 32]);
    let gate = metadata_gate(0, owner);
    let first = AdmittedOperation {
        op_hash: [1; 32],
        domain_op_id: [9; 32],
        author,
        encoded_len: 100,
    };
    let conflicting_replay = AdmittedOperation {
        op_hash: [2; 32],
        domain_op_id: [9; 32],
        author,
        encoded_len: 200,
    };
    assert_eq!(gate.admit_inbound(first).unwrap(), Admission::Accepted);
    assert!(matches!(
        gate.admit_inbound(conflicting_replay),
        Err(ReplError::IntentConflict)
    ));
    assert_eq!(gate.accepted_hashes(), vec![first.op_hash]);
}

#[test]
fn gate_restore_preserves_admissions_across_an_owner_change() {
    use catcoms_replication::epoch::MAX_DEVICE_OPERATIONS;

    let former_owner = DeviceId::from_bytes([1; 32]);
    let current_owner = DeviceId::from_bytes([2; 32]);
    let gate = metadata_gate(0, former_owner);
    for index in 0..=MAX_DEVICE_OPERATIONS {
        let mut op_hash = [0u8; 32];
        op_hash[..8].copy_from_slice(&(index as u64).to_be_bytes());
        let mut domain_op_id = op_hash;
        domain_op_id[31] ^= 0xff;
        assert_eq!(
            gate.admit_local(AdmittedOperation {
                op_hash,
                domain_op_id,
                author: former_owner,
                encoded_len: 32,
            })
            .unwrap(),
            Admission::Accepted
        );
    }
    gate.update_owner(current_owner);

    let restored = EpochGate::decode(&gate.encode().unwrap()).unwrap();
    let rejected = AdmittedOperation {
        op_hash: [0xfd; 32],
        domain_op_id: [0xfc; 32],
        author: former_owner,
        encoded_len: 32,
    };
    assert!(matches!(
        restored.admit_local(rejected),
        Err(ReplError::EpochBound)
    ));
    assert_eq!(
        restored
            .admit_local(AdmittedOperation {
                author: current_owner,
                ..rejected
            })
            .unwrap(),
        Admission::Accepted
    );
}

#[test]
fn third_recovery_snapshot_is_crash_resumable_and_never_makes_four_physical_copies() {
    let mut slots = RecoverySlots::default();
    assert_eq!(
        slots.stage(snapshot(1), 100).unwrap(),
        RecoveryTransition::Promoted
    );
    assert_eq!(
        slots.stage(snapshot(2), 200).unwrap(),
        RecoveryTransition::Promoted
    );
    let pending = slots.stage(snapshot(3), 300).unwrap();
    let (oldest, staged) = match pending {
        RecoveryTransition::EvictionPending {
            oldest_snapshot,
            staged_snapshot,
            ..
        } => (oldest_snapshot, staged_snapshot),
        other => panic!("unexpected recovery transition: {other:?}"),
    };
    assert_eq!(slots.stage(snapshot(3), 999).unwrap(), pending);
    assert!(matches!(
        slots.stage(snapshot(4), 400),
        Err(ReplError::RecoveryPending)
    ));

    let mut restored = RecoverySlots::decode(&slots.encode().unwrap()).unwrap();
    assert_eq!(restored.retained().len(), 2);
    assert!(restored.staged().is_some());
    assert!(matches!(
        restored.acknowledge_eviction([0xff; 32], staged),
        Err(ReplError::RecoveryPending)
    ));
    assert!(matches!(
        restored.acknowledge_eviction(oldest, [0xee; 32]),
        Err(ReplError::RecoveryPending)
    ));
    restored.acknowledge_eviction(oldest, staged).unwrap();
    assert_eq!(
        restored
            .retained()
            .map(|item| item.epoch)
            .collect::<Vec<_>>(),
        vec![3, 2]
    );
    assert!(restored.staged().is_none());
}

#[test]
fn recovery_slots_reject_cross_document_and_noncanonical_snapshot_state() {
    let mut slots = RecoverySlots::default();
    slots.stage(snapshot(1), 10).unwrap();
    let mut other = snapshot(2);
    other.logical_key = b"score-18".to_vec();
    assert!(matches!(slots.stage(other, 20), Err(ReplError::EpochScope)));
    assert_eq!(slots.retained().len(), 1);

    let mut noncanonical = snapshot(3);
    noncanonical.applied_ops = vec![[2; 32], [1; 32]];
    assert!(matches!(noncanonical.encode(), Err(ReplError::EpochBound)));
}

#[test]
fn domain_envelope_and_atomic_marker_survive_real_encrypted_replication() {
    use automerge::transaction::Transactable;
    use automerge::{ReadDoc, ROOT};

    let (owner, group, document) = solo();
    let doc_id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let mut author_doc = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    let author_gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());
    let mut rng = ChaCha20Rng::seed_from_u64(71);
    let operation = catcoms_replication::DomainOp {
        nonce: [0x71; 16],
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: br#"{"kind":"set_header","field":"title","value":"hello"}"#.to_vec(),
    };
    let (sealed, _) = author_doc
        .edit_domain_gated(
            &document,
            &author_gate,
            &owner,
            &group,
            &mut rng,
            &operation,
            |doc| {
                doc.put(ROOT, "title", "hello")?;
                Ok(())
            },
            |_, _| Ok(()),
        )
        .unwrap();

    let mut replica = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    let replica_gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());
    assert_eq!(
        replica
            .ingest_domain_gated(
                &document,
                &replica_gate,
                &sealed,
                &group,
                &owner,
                |_, _| Ok(()),
            )
            .unwrap(),
        Admission::Accepted
    );
    assert_eq!(
        replica
            .doc()
            .get(ROOT, "title")
            .unwrap()
            .unwrap()
            .0
            .into_string()
            .unwrap(),
        "hello"
    );

    // Restart must explicitly restore the device actor. Otherwise the next Automerge change is
    // signed by this device outside but names an implementation-selected actor inside.
    let author_snapshot = author_doc.snapshot().unwrap();
    let mut restarted =
        EncryptedDoc::restore_for_actor(&author_snapshot, &owner.device_id()).unwrap();
    let restarted_gate = EpochGate::decode(&author_gate.encode().unwrap()).unwrap();
    let second = catcoms_replication::DomainOp {
        nonce: [0x72; 16],
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: br#"{"kind":"set_header","field":"subtitle","value":"again"}"#.to_vec(),
    };
    let (sealed_after_restart, _) = restarted
        .edit_domain_gated(
            &document,
            &restarted_gate,
            &owner,
            &group,
            &mut rng,
            &second,
            |doc| {
                doc.put(ROOT, "subtitle", "again")?;
                Ok(())
            },
            |_, _| Ok(()),
        )
        .unwrap();
    assert_eq!(
        replica
            .ingest_domain_gated(
                &document,
                &replica_gate,
                &sealed_after_restart,
                &group,
                &owner,
                |_, _| Ok(()),
            )
            .unwrap(),
        Admission::Accepted
    );
}

#[test]
fn p1_types_reject_legacy_writes_and_run_semantic_validation_before_ingest() {
    use automerge::transaction::Transactable;
    use automerge::{ReadDoc, ROOT};

    let (owner, group, document) = solo();
    let doc_id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let mut rng = ChaCha20Rng::seed_from_u64(72);
    let mut legacy = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    assert!(matches!(
        legacy.edit(&owner, &group, &mut rng, |doc| doc
            .put(ROOT, "bypass", true)),
        Err(ReplError::EpochScope)
    ));
    assert!(legacy.doc().get(ROOT, "bypass").unwrap().is_none());

    let operation = catcoms_replication::DomainOp {
        nonce: [0x72; 16],
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: br#"{"kind":"set_header","field":"title","value":"bad"}"#.to_vec(),
    };
    let wrong_server = LogicalDocument::new(
        b"another-server".to_vec(),
        document.doc_type,
        document.logical_key.clone(),
    )
    .unwrap();
    let wrong_gate = EpochGate::new(wrong_server.clone(), doc_id, 0, owner.device_id());
    assert!(matches!(
        legacy.edit_domain_gated(
            &document,
            &wrong_gate,
            &owner,
            &group,
            &mut rng,
            &operation,
            |doc| {
                doc.put(ROOT, "cross-server", true)?;
                Ok(())
            },
            |_, _| Ok(()),
        ),
        Err(ReplError::EpochScope)
    ));
    assert!(matches!(
        legacy.edit_domain_gated(
            &wrong_server,
            &wrong_gate,
            &owner,
            &group,
            &mut rng,
            &operation,
            |doc| {
                doc.put(ROOT, "wrong-group", true)?;
                Ok(())
            },
            |_, _| Ok(()),
        ),
        Err(ReplError::EpochScope)
    ));
    assert!(legacy.doc().get(ROOT, "cross-server").unwrap().is_none());
    assert!(legacy.doc().get(ROOT, "wrong-group").unwrap().is_none());

    let author_gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());
    assert!(matches!(
        legacy.edit_domain_gated(
            &document,
            &author_gate,
            &owner,
            &group,
            &mut rng,
            &operation,
            |doc| {
                doc.put(ROOT, "title", "bad")?;
                Ok(())
            },
            |_, _| Err(ReplError::Malformed),
        ),
        Err(ReplError::Malformed)
    ));
    assert!(legacy.doc().get(ROOT, "title").unwrap().is_none());
    assert!(author_gate.accepted_hashes().is_empty());
    let (sealed, _) = legacy
        .edit_domain_gated(
            &document,
            &author_gate,
            &owner,
            &group,
            &mut rng,
            &operation,
            |doc| {
                doc.put(ROOT, "title", "bad")?;
                Ok(())
            },
            |_, _| Ok(()),
        )
        .unwrap();

    let mut receiver = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    let receiver_gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());
    assert!(matches!(
        receiver.ingest(&sealed, &group, &owner),
        Err(ReplError::EpochScope)
    ));
    assert!(matches!(
        receiver.ingest_domain_gated(
            &document,
            &receiver_gate,
            &sealed,
            &group,
            &owner,
            |_, _| Err(ReplError::Malformed),
        ),
        Err(ReplError::Malformed)
    ));
    assert!(receiver.doc().get(ROOT, "title").unwrap().is_none());
    assert_eq!(receiver.op_count(), 0);
    assert!(receiver_gate.accepted_hashes().is_empty());
}

#[test]
fn signed_domain_envelope_without_its_atomic_marker_is_rejected_before_state_changes() {
    use automerge::transaction::Transactable;
    use automerge::{ActorId, AutoCommit, ReadDoc, ROOT};

    let (owner, group, document) = solo();
    let doc_id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let operation = catcoms_replication::DomainOp {
        nonce: [0x19; 16],
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: br#"{"kind":"set_header","field":"title","value":"forged"}"#.to_vec(),
    };

    let mut malicious_change = AutoCommit::new();
    malicious_change.set_actor(ActorId::from(owner.device_id().as_bytes().to_vec()));
    malicious_change.put(ROOT, "title", "forged").unwrap();
    malicious_change.commit();
    let delta = malicious_change
        .get_last_local_change()
        .unwrap()
        .raw_bytes()
        .to_vec();
    let signed =
        SignedOp::sign_domain(&owner, document.doc_type, doc_id, delta, &operation).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(19);
    let sealed = SealedOp::seal(&signed, &group, &owner, &mut rng).unwrap();

    let mut replica = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    let gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());
    assert!(matches!(
        replica.ingest_domain_gated(&document, &gate, &sealed, &group, &owner, |_, _| Ok(())),
        Err(ReplError::Malformed)
    ));
    assert!(replica.doc().get(ROOT, "title").unwrap().is_none());
    assert_eq!(replica.op_count(), 0);
}

#[test]
fn signed_domain_envelope_cannot_forge_its_automerge_actor() {
    use automerge::transaction::Transactable;
    use automerge::{ActorId, AutoCommit, ReadDoc, ROOT};

    let (owner, group, document) = solo();
    let doc_id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let operation = catcoms_replication::DomainOp {
        nonce: [0x20; 16],
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: b"forged actor".to_vec(),
    };
    let mut malicious_change = AutoCommit::new();
    malicious_change.set_actor(ActorId::from(vec![0x99; 32]));
    malicious_change.put(ROOT, "title", "forged").unwrap();
    malicious_change.commit();
    let delta = malicious_change
        .get_last_local_change()
        .unwrap()
        .raw_bytes()
        .to_vec();
    let signed =
        SignedOp::sign_domain(&owner, document.doc_type, doc_id, delta, &operation).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(20);
    let sealed = SealedOp::seal(&signed, &group, &owner, &mut rng).unwrap();
    let gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());
    let mut receiver = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    assert!(matches!(
        receiver.ingest_domain_gated(&document, &gate, &sealed, &group, &owner, |_, _| Ok(()),),
        Err(ReplError::EpochAuthority)
    ));
    assert!(receiver.doc().get(ROOT, "title").unwrap().is_none());
    assert!(gate.accepted_hashes().is_empty());
}

#[test]
fn failed_domain_envelope_encoding_leaves_no_unsigned_change_in_live_state() {
    use automerge::transaction::Transactable;
    use automerge::{ReadDoc, ROOT};

    let (owner, group, document) = solo();
    let doc_id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let mut encrypted = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    let operation = catcoms_replication::DomainOp {
        nonce: [0x27; 16],
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: vec![0x27; 64 * 1024 + 1],
    };
    let mut rng = ChaCha20Rng::seed_from_u64(27);
    let gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());

    assert!(encrypted
        .edit_domain_gated(
            &document,
            &gate,
            &owner,
            &group,
            &mut rng,
            &operation,
            |doc| {
                doc.put(ROOT, "must-not-stick", true)?;
                Ok(())
            },
            |_, _| Ok(()),
        )
        .is_err());
    assert!(encrypted
        .doc()
        .get(ROOT, "must-not-stick")
        .unwrap()
        .is_none());
    assert_eq!(encrypted.op_count(), 0);
}

#[test]
fn gated_domain_edits_that_lose_the_seal_race_never_touch_live_state() {
    use automerge::transaction::Transactable;
    use automerge::{ReadDoc, ROOT};

    let (owner, group, document) = solo();
    let doc_id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let operation = catcoms_replication::DomainOp {
        nonce: [0x31; 16],
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: b"sealed-race".to_vec(),
    };
    let gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());
    let signed_receipt = receipt(&owner, &group, &document, 0, InheritedCheckpoint::EpochZero);
    let verified = signed_receipt
        .verify_current_owner(&group, group.epoch())
        .unwrap();
    gate.seal_verified_receipt(&verified).unwrap();
    let mut local = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    let mut rng = ChaCha20Rng::seed_from_u64(31);
    assert!(matches!(
        local.edit_domain_gated(
            &document,
            &gate,
            &owner,
            &group,
            &mut rng,
            &operation,
            |doc| {
                doc.put(ROOT, "late-local", true)?;
                Ok(())
            },
            |_, _| Ok(()),
        ),
        Err(ReplError::EpochClosed)
    ));
    assert!(local.doc().get(ROOT, "late-local").unwrap().is_none());

    let mut author = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    let author_gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());
    let (sealed, _) = author
        .edit_domain_gated(
            &document,
            &author_gate,
            &owner,
            &group,
            &mut rng,
            &operation,
            |doc| {
                doc.put(ROOT, "late-inbound", true)?;
                Ok(())
            },
            |_, _| Ok(()),
        )
        .unwrap();
    let mut receiver = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    assert_eq!(
        receiver
            .ingest_domain_gated(&document, &gate, &sealed, &group, &owner, |_, _| Ok(()))
            .unwrap(),
        Admission::Quarantined
    );
    assert!(receiver.doc().get(ROOT, "late-inbound").unwrap().is_none());
    assert_eq!(receiver.op_count(), 0);
}

#[test]
fn close_budget_is_computed_from_the_real_dependency_closed_signed_log() {
    use automerge::transaction::Transactable;
    use automerge::ROOT;

    let (owner, group, document) = solo();
    let doc_id = catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key);
    let mut encrypted = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    let gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());
    let mut rng = ChaCha20Rng::seed_from_u64(99);

    for index in 0..36u8 {
        let payload = vec![index; 30 * 1024];
        let operation = catcoms_replication::DomainOp {
            nonce: [index; 16],
            doc_type: document.doc_type,
            logical_key: document.logical_key.clone(),
            body: payload.clone(),
        };
        encrypted
            .edit_domain_gated(
                &document,
                &gate,
                &owner,
                &group,
                &mut rng,
                &operation,
                |doc| {
                    doc.put(ROOT, format!("chunk-{index}"), payload)?;
                    Ok(())
                },
                |_, _| Ok(()),
            )
            .unwrap();
    }

    let heads = encrypted.heads();
    let close = CloseRecord::sign(&document, doc_id, 0, heads, &owner).unwrap();
    let stats = close
        .verify_and_validate(&document, doc_id, &group, None, &mut encrypted, None)
        .unwrap();
    assert_eq!(stats.operation_count, 36);
    assert!(stats.encoded_bytes >= 2 * 1024 * 1024);

    let wrong_logical_key = LogicalDocument::new(
        document.server_id.clone(),
        document.doc_type,
        b"another-score".to_vec(),
    )
    .unwrap();
    assert!(matches!(
        close.verify_and_validate(
            &wrong_logical_key,
            doc_id,
            &group,
            None,
            &mut encrypted,
            None,
        ),
        Err(ReplError::EpochScope)
    ));

    // A close at a strict prefix below every lower-bound condition is rejected even though its
    // signature and head are otherwise valid.
    let mut tiny = EncryptedDoc::new(document.doc_type, doc_id, &owner.device_id());
    let operation = catcoms_replication::DomainOp {
        nonce: [0xfe; 16],
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: b"tiny".to_vec(),
    };
    let tiny_gate = EpochGate::new(document.clone(), doc_id, 0, owner.device_id());
    tiny.edit_domain_gated(
        &document,
        &tiny_gate,
        &owner,
        &group,
        &mut rng,
        &operation,
        |doc| {
            doc.put(ROOT, "tiny", true)?;
            Ok(())
        },
        |_, _| Ok(()),
    )
    .unwrap();
    let tiny_close = CloseRecord::sign(&document, doc_id, 0, tiny.heads(), &owner).unwrap();
    assert!(matches!(
        tiny_close.verify_and_validate(&document, doc_id, &group, None, &mut tiny, None),
        Err(ReplError::EpochBound)
    ));
}

#[test]
fn p1_records_reject_oversized_input_before_parsing() {
    use catcoms_replication::epoch::{
        MAX_CLOSE_RECORD_BYTES, MAX_EPOCH_GATE_BYTES, MAX_OWNER_RECEIPT_JOURNAL_BYTES,
        MAX_RECEIPT_BOOK_BYTES, MAX_RECEIPT_BYTES,
    };

    assert!(matches!(
        CloseRecord::decode(&vec![0; MAX_CLOSE_RECORD_BYTES + 1]),
        Err(ReplError::EpochBound)
    ));
    assert!(matches!(
        Receipt::decode(&vec![0; MAX_RECEIPT_BYTES + 1]),
        Err(ReplError::EpochBound)
    ));
    assert!(matches!(
        ReceiptBook::decode(&vec![0; MAX_RECEIPT_BOOK_BYTES + 1]),
        Err(ReplError::EpochBound)
    ));
    assert!(matches!(
        OwnerReceiptJournal::decode(&vec![0; MAX_OWNER_RECEIPT_JOURNAL_BYTES + 1]),
        Err(ReplError::EpochBound)
    ));
    assert!(matches!(
        EpochGate::decode(&vec![0; MAX_EPOCH_GATE_BYTES + 1]),
        Err(ReplError::EpochBound)
    ));
}

#[test]
fn local_intents_survive_restart_and_only_receipted_ids_are_removed() {
    let (owner, _group, document) = solo();
    let operation = catcoms_replication::DomainOp {
        nonce: [0x88; 16],
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        body: b"pending".to_vec(),
    };
    let mut ledger = IntentLedger::new(document);
    let id = ledger
        .prepare(owner.device_id(), operation.clone())
        .unwrap();
    assert_eq!(
        ledger
            .prepare(owner.device_id(), operation.clone())
            .unwrap(),
        id
    );

    let mut conflicting = operation;
    conflicting.body = b"same nonce, different operation".to_vec();
    assert!(matches!(
        ledger.prepare(owner.device_id(), conflicting),
        Err(ReplError::IntentConflict)
    ));

    let mut restored = IntentLedger::decode(&ledger.encode().unwrap()).unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored.remove_receipted(&BTreeSet::new()), 0);
    assert_eq!(restored.len(), 1);
    assert_eq!(restored.remove_receipted(&BTreeSet::from([id])), 1);
    assert!(restored.is_empty());
}
