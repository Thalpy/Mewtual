use super::*;

fn gate() -> EpochGate {
    EpochGate::new(
        LogicalDocument::new(b"repair-test".to_vec(), DocType::StudioIndex, vec![7; 16]).unwrap(),
        1,
        0,
        DeviceId::from_bytes([1; 32]),
    )
}

#[test]
fn repair_commit_checks_every_gate_field_and_never_runs_callback_on_failure() {
    for field in 0..9 {
        let gate = gate();
        let mut expected = gate.inner.lock().unwrap().clone();
        expected.phase = EpochPhase::Fault;
        *gate.inner.lock().unwrap() = expected.clone();
        let mut next = expected.clone();
        next.phase = EpochPhase::Open;
        {
            let mut current = gate.inner.lock().unwrap();
            match field {
                0 => current.owner = DeviceId::from_bytes([2; 32]),
                1 => current.phase = EpochPhase::Settled,
                2 => current.receipt_hash = Some([3; 32]),
                3 => {
                    current.quarantine.push_back([4; 32]);
                }
                4 => {
                    current.operation_ids.insert([5; 32], [6; 32]);
                }
                5 => {
                    current.operations.insert(
                        [6; 32],
                        AdmittedOperation {
                            op_hash: [6; 32],
                            domain_op_id: [5; 32],
                            author: DeviceId::from_bytes([1; 32]),
                            encoded_len: 1,
                        },
                    );
                }
                6 => current.total_bytes += 1,
                7 => {
                    current
                        .by_device
                        .insert(DeviceId::from_bytes([1; 32]), (1, 1));
                }
                8 => {
                    next.quarantine.push_back([7; 32]);
                }
                _ => unreachable!(),
            }
        }
        let before = gate.inner.lock().unwrap().clone();
        let mut called = false;
        assert!(gate
            .commit_repair(&expected, &next, RepairDisposition::Transitioned, || {
                called = true
            })
            .is_err());
        assert!(!called);
        assert_eq!(*gate.inner.lock().unwrap(), before);
    }
}

#[test]
fn repair_commit_preserves_closing_quarantine_and_acceptance_accounting() {
    let gate = gate();
    let op = AdmittedOperation {
        op_hash: [1; 32],
        domain_op_id: [2; 32],
        author: DeviceId::from_bytes([1; 32]),
        encoded_len: 33,
    };
    assert_eq!(gate.admit_inbound(op).unwrap(), Admission::Accepted);
    let mut expected = gate.inner.lock().unwrap().clone();
    expected.phase = EpochPhase::Fault;
    expected.quarantine.push_back([3; 32]);
    *gate.inner.lock().unwrap() = expected.clone();
    let mut next = expected.clone();
    next.phase = EpochPhase::Closing;
    next.receipt_hash = Some([4; 32]);
    let mut bad = next.clone();
    bad.quarantine.clear();
    assert!(gate
        .commit_repair(
            &expected,
            &bad,
            RepairDisposition::Transitioned,
            || panic!()
        )
        .is_err());
    let mut bad = next.clone();
    bad.total_bytes = 0;
    assert!(gate
        .commit_repair(
            &expected,
            &bad,
            RepairDisposition::Transitioned,
            || panic!()
        )
        .is_err());
    assert!(gate
        .commit_repair(&expected, &next, RepairDisposition::Screened, || panic!())
        .is_err());
    gate.commit_repair(&expected, &next, RepairDisposition::Transitioned, || {})
        .unwrap();
    assert_eq!(*gate.inner.lock().unwrap(), next);
    assert_eq!(gate.admit_inbound(op).unwrap(), Admission::Duplicate);
    assert_eq!(gate.quarantined_len(), 1);
}

#[test]
fn repair_plan_stamp_fences_book_opening_binding_and_adoption_changes() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let document =
        LogicalDocument::new(group.group_id(), DocType::StudioIndex, vec![7; 16]).unwrap();
    let sign = |salt| {
        Receipt::sign(
            document.clone(),
            0,
            [salt; 32],
            [9; 32],
            0,
            InheritedCheckpoint::EpochZero,
            &owner,
        )
        .unwrap()
    };
    let selected = sign(1);
    let losing = sign(2);
    let third = sign(3);
    let repair = ReceiptRepair::sign_in_tenure(
        document.clone(),
        selected.tenure_id,
        [selected.hash(), losing.hash()],
        selected.hash(),
        1,
        0,
        &owner,
    )
    .unwrap();
    for change in 0..6 {
        let gate = EpochGate::new(document.clone(), 1, 0, owner.device_id());
        let mut book = ReceiptBook::default();
        book.ingest_and_seal(selected.clone(), &group, 0, &gate)
            .unwrap();
        book.ingest_and_seal(losing.clone(), &group, 0, &gate)
            .unwrap();
        let mut adopting = false;
        let mut binding = None;
        let mut source = RepairSource {
            document: &document,
            gate: &gate,
            book: &mut book,
            opening: None,
            adopting: &mut adopting,
            binding: &mut binding,
        };
        let RepairPlan::Candidate(candidate) =
            source.plan(&repair, &selected, &losing, &group, 0).unwrap()
        else {
            panic!("expected a validated candidate");
        };
        match change {
            0 => source.book.repair_sequence = 2,
            1 => source.book.fault = Some((selected.clone(), third.clone())),
            2 => source.book.latest = Some(third.clone()),
            3 => *source.adopting = true,
            4 => *source.binding = Some(candidate.binding),
            5 => source.opening = Some(&third),
            _ => unreachable!(),
        }
        let before_book = source.book_bytes().unwrap();
        let before_gate = gate.encode().unwrap();
        let before_binding = *source.binding;
        let before_adopting = *source.adopting;
        assert!(source.commit(*candidate).is_err());
        assert_eq!(source.book_bytes().unwrap(), before_book);
        assert_eq!(gate.encode().unwrap(), before_gate);
        assert_eq!(*source.binding, before_binding);
        assert_eq!(*source.adopting, before_adopting);
    }
}
