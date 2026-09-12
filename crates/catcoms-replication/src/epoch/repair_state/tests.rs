use super::*;

struct Fixture {
    owner: MlsDevice,
    returning: MlsDevice,
    group: ServerGroup,
    document: LogicalDocument,
    selected: Receipt,
    losing: Receipt,
    repair: ReceiptRepair,
    book: ReceiptBook,
}

impl Fixture {
    fn new(different_baseline: bool) -> Self {
        let owner = MlsDevice::generate().unwrap();
        let returning = owner.duplicate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let document =
            LogicalDocument::new(group.group_id(), DocType::StudioIndex, vec![7; 16]).unwrap();
        let sign = |inherited, close| {
            Receipt::sign(
                document.clone(),
                2,
                [close; 32],
                [9; 32],
                0,
                inherited,
                &owner,
            )
            .unwrap()
        };
        let selected = sign(InheritedCheckpoint::EpochZero, 2);
        let losing = sign(
            if different_baseline {
                baseline(1)
            } else {
                InheritedCheckpoint::EpochZero
            },
            3,
        );
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
        let mut book = ReceiptBook::default();
        assert_eq!(
            book.ingest_verified(selected.clone()).unwrap(),
            ReceiptIngest::Advanced
        );
        assert_eq!(
            book.ingest_verified(losing.clone()).unwrap(),
            ReceiptIngest::Fault
        );
        Self {
            owner,
            returning,
            group,
            document,
            selected,
            losing,
            repair,
            book,
        }
    }

    fn apply(&mut self) {
        assert_eq!(
            self.book
                .apply_repair(&self.repair, &self.group, 0)
                .unwrap(),
            (ReceiptRepairIngest::Applied, self.losing.clone())
        );
    }

    fn sign(&self, epoch: u64, inherited: InheritedCheckpoint, close: u8) -> Receipt {
        Receipt::sign(
            self.document.clone(),
            epoch,
            [close; 32],
            [9; 32],
            0,
            inherited,
            &self.owner,
        )
        .unwrap()
    }

    fn reopen(&mut self) {
        let bytes = self.book.encode().unwrap();
        self.book = ReceiptBook::decode(&bytes).unwrap();
        assert_eq!(self.book.encode().unwrap(), bytes);
    }
}

fn baseline(epoch: u64) -> InheritedCheckpoint {
    InheritedCheckpoint::Checkpoint {
        epoch,
        close_record_hash: [epoch as u8; 32],
        seed_change_hash: [8; 32],
    }
}

#[test]
fn repair_replay_cannot_restore_resolved_fault_across_all_ingest_paths_and_restart() {
    for different_baseline in [false, true] {
        let mut f = Fixture::new(different_baseline);
        f.apply();
        for _ in 0..2 {
            f.reopen();
            let before = f.book.encode().unwrap();
            let gate = EpochGate::new(f.document.clone(), 42, 2, f.owner.device_id());
            assert_eq!(
                f.book
                    .ingest_and_seal(f.losing.clone(), &f.group, 0, &gate)
                    .unwrap()
                    .0,
                ReceiptIngest::Stale
            );
            assert_eq!(gate.phase(), EpochPhase::Open);
            let successor = EpochGate::new(f.document.clone(), 43, 3, f.owner.device_id());
            assert_eq!(
                f.book
                    .check_opening_receipt(f.losing.clone(), &f.selected, &f.group, 0, &successor)
                    .unwrap(),
                ReceiptIngest::Stale
            );
            assert_eq!(successor.phase(), EpochPhase::Open);
            assert_eq!(
                f.book
                    .ingest_adoption(f.losing.clone(), &f.group, 0, &successor, Some(&f.selected))
                    .unwrap(),
                ReceiptIngest::Stale
            );
            assert_eq!(successor.phase(), EpochPhase::Open);
            assert_eq!(f.book.encode().unwrap(), before);
        }
    }
}

#[test]
fn repair_losing_baseline_descendants_stay_stale_but_new_equivocation_faults() {
    let mut f = Fixture::new(true);
    f.apply();
    for epoch in [3, 10, 10_000] {
        let loser = f.sign(epoch, f.losing.inherited.clone(), 4);
        assert_eq!(
            f.book.ingest_verified(loser.clone()).unwrap(),
            ReceiptIngest::Stale
        );
        let gate = EpochGate::new(f.document.clone(), 43, 3, f.owner.device_id());
        assert_eq!(
            f.book
                .ingest_adoption(loser, &f.group, 0, &gate, Some(&f.selected))
                .unwrap(),
            ReceiptIngest::Stale
        );
        assert_eq!(gate.phase(), EpochPhase::Open);
        f.reopen();
    }
    let third = f.sign(4, baseline(2), 5);
    assert_eq!(f.book.ingest_verified(third).unwrap(), ReceiptIngest::Fault);
    let before = f.book.encode().unwrap();
    assert!(f.book.apply_repair(&f.repair, &f.group, 0).is_err());
    assert_eq!(
        f.book.encode().unwrap(),
        before,
        "old repair cannot clear a new fault"
    );
    f.reopen();
    assert!(f.book.is_faulted());

    let mut same = Fixture::new(false);
    same.apply();
    let third = same.sign(2, InheritedCheckpoint::EpochZero, 5);
    assert_eq!(
        same.book.ingest_verified(third).unwrap(),
        ReceiptIngest::Fault
    );
}

#[test]
fn repair_exact_retry_preserves_newer_head_and_named_pair_mismatch_holds() {
    let mut f = Fixture::new(true);
    f.apply();
    let later = f.sign(8, InheritedCheckpoint::EpochZero, 8);
    assert_eq!(
        f.book.ingest_verified(later.clone()).unwrap(),
        ReceiptIngest::Advanced
    );
    f.book.mark_latest_installed();
    f.reopen();
    let before = f.book.encode().unwrap();
    assert_eq!(
        f.book.apply_repair(&f.repair, &f.group, 0).unwrap().0,
        ReceiptRepairIngest::Duplicate
    );
    assert_eq!(f.book.latest(), Some(&later));
    assert_eq!(f.book.encode().unwrap(), before);

    // A different frozen pair is not the pair the owner signed, even when it is related to
    // the same losing baseline. Orchestration needs the exact signed named evidence first.
    let mut other = ReceiptBook::default();
    other.ingest_verified(f.selected.clone()).unwrap();
    other.ingest_verified(f.sign(9, baseline(1), 9)).unwrap();
    let before = other.encode().unwrap();
    assert!(other.apply_repair(&f.repair, &f.group, 0).is_err());
    assert_eq!(other.encode().unwrap(), before);
}

#[test]
fn repair_v1_is_byte_compatible_but_live_apply_requires_v2_current_tenure() {
    let mut f = Fixture::new(true);
    let legacy = ReceiptRepair::sign(
        f.document.clone(),
        f.selected.tenure_id,
        [f.selected.hash(), f.losing.hash()],
        f.selected.hash(),
        1,
        &f.owner,
    )
    .unwrap();
    // Independent legacy transcript pins v1 field order/domains rather than round-tripping
    // the same new encoder on both sides of a compatibility assertion.
    let mut unsigned = Encoder::new();
    unsigned.put_u8(1);
    unsigned.put_bytes(&legacy.document.server_id).unwrap();
    unsigned.put_u16(legacy.document.doc_type.tag());
    unsigned.put_bytes(&legacy.document.logical_key).unwrap();
    for hash in [
        legacy.tenure_id,
        legacy.receipt_hashes[0],
        legacy.receipt_hashes[1],
        legacy.selected_receipt_hash,
    ] {
        put_hash(&mut unsigned, &hash);
    }
    unsigned.put_u64(legacy.repair_sequence);
    unsigned.put_bytes(&legacy.owner_public_key).unwrap();
    let unsigned = unsigned.finish();
    assert_eq!(legacy.unsigned_bytes(), unsigned);
    assert_eq!(
        legacy.signature_hash(),
        hash_parts("catcoms-repair-sig:v1", &[&unsigned])
    );
    assert_eq!(
        legacy.hash(),
        hash_parts("catcoms-repair:v1", &[&unsigned, &legacy.signature])
    );
    assert_eq!(ReceiptRepair::decode(&legacy.encode()).unwrap(), legacy);
    let before = f.book.encode().unwrap();
    assert!(legacy.verify_current_owner(&f.group, 0).is_err());
    assert!(f.book.apply_repair(&legacy, &f.group, 0).is_err());
    for tenure in [1, u64::MAX] {
        assert!(f.book.apply_repair(&f.repair, &f.group, tenure).is_err());
    }
    assert_eq!(f.book.encode().unwrap(), before);
    assert_eq!(ReceiptRepair::decode(&f.repair.encode()).unwrap(), f.repair);
    for len in 0..f.repair.encode().len() {
        assert!(ReceiptRepair::decode(&f.repair.encode()[..len]).is_err());
    }
    assert!(ReceiptRepair::decode(&vec![0; MAX_RECEIPT_BYTES + 1]).is_err());
    let mut bad = f.repair.clone();
    bad.signature[0] ^= 1;
    assert!(f.book.apply_repair(&bad, &f.group, 0).is_err());
    let mut bad = f.repair.clone();
    bad.issuer_tenure_start_group_epoch = Some(1);
    assert!(f.book.apply_repair(&bad, &f.group, 0).is_err());
    let stranger = MlsDevice::generate().unwrap();
    let other = ServerGroup::create(&stranger).unwrap();
    assert!(f.book.apply_repair(&f.repair, &other, 0).is_err());
    assert_eq!(f.book.encode().unwrap(), before);
}

#[test]
fn repair_old_owner_tenure_cannot_bypass_authority_via_exact_retry_after_return() {
    let mut f = Fixture::new(true);
    f.apply();
    let next = MlsDevice::generate().unwrap();
    let welcome = f
        .group
        .add_member(&f.owner, next.key_package().unwrap())
        .unwrap()
        .welcome;
    let mut group = ServerGroup::join(&next, &welcome).unwrap();
    group.remove_member(&next, &f.owner.device_id()).unwrap();
    assert!(f
        .book
        .apply_repair(&f.repair, &group, group.epoch())
        .is_err());
    let welcome = group
        .add_member(&next, f.returning.key_package().unwrap())
        .unwrap()
        .welcome;
    group = ServerGroup::join(&f.returning, &welcome).unwrap();
    assert_eq!(group.designated_committer(), Some(f.owner.device_id()));
    let before = f.book.encode().unwrap();
    assert!(f
        .book
        .apply_repair(&f.repair, &group, group.epoch())
        .is_err());
    assert_eq!(f.book.encode().unwrap(), before);
    // Vault history survives succession but grants no new live authority.
    f.reopen();
    assert_eq!(f.book.latest_repair(), Some(&f.repair));
}

#[test]
fn repair_restart_rejects_corrupt_full_evidence_and_preserves_legacy_book_versions() {
    let mut f = Fixture::new(true);
    let ordinary = f.book.encode().unwrap();
    assert_eq!(ordinary[0], 1);
    let adoption = f.book.encode_adoption().unwrap();
    assert_eq!(adoption[0], 3);
    assert_eq!(
        ReceiptBook::decode(&ordinary).unwrap().encode().unwrap(),
        ordinary
    );
    assert_eq!(
        ReceiptBook::decode_adoption(&adoption)
            .unwrap()
            .encode_adoption()
            .unwrap(),
        adoption
    );
    f.apply();
    assert_eq!(f.book.encode().unwrap()[0], 4);
    let adoption = f.book.encode_adoption().unwrap();
    assert_eq!(adoption[0], 5);
    assert_eq!(
        ReceiptBook::decode_adoption(&adoption)
            .unwrap()
            .encode_adoption()
            .unwrap(),
        adoption
    );
    assert!(ReceiptBook::decode(&adoption).is_err());
    assert!(ReceiptBook::decode_adoption(&f.book.encode().unwrap()).is_err());
    for mutation in 0..8 {
        let mut corrupt = f.book.clone();
        let r = corrupt.resolved_repair.as_mut().unwrap();
        match mutation {
            0 => r.losing.signature[0] ^= 1,
            1 => r.selected.signature[0] ^= 1,
            2 => r.repair.signature[0] ^= 1,
            3 => corrupt.repair_sequence += 1,
            4 => std::mem::swap(&mut r.selected, &mut r.losing),
            5 => r.losing = r.selected.clone(),
            6 => r.repair.receipt_hashes[0][0] ^= 1,
            7 => r.repair.document.logical_key[0] ^= 1,
            _ => unreachable!(),
        }
        assert!(
            ReceiptBook::decode(&corrupt.encode().unwrap()).is_err(),
            "case {mutation}"
        );
    }
}

#[test]
fn repair_same_baseline_does_not_invent_descendant_ancestry() {
    let mut f = Fixture::new(false);
    f.apply();
    let next = f.sign(3, InheritedCheckpoint::EpochZero, 7);
    assert_eq!(
        f.book.ingest_verified(next).unwrap(),
        ReceiptIngest::Advanced
    );
    f.reopen();
    assert_eq!(
        f.book.ingest_verified(f.losing.clone()).unwrap(),
        ReceiptIngest::Stale
    );
}

#[test]
fn repair_v2_transcript_golden_vector() {
    // A codec/hash vector with deliberately synthetic key/signature bytes, not signed authority.
    let repair = ReceiptRepair {
        document: LogicalDocument::new(vec![1, 2], DocType::StudioIndex, vec![3; 16]).unwrap(),
        tenure_id: [4; 32],
        issuer_tenure_start_group_epoch: Some(7),
        receipt_hashes: [[5; 32], [6; 32]],
        selected_receipt_hash: [5; 32],
        repair_sequence: 9,
        owner_public_key: vec![7; 32],
        signature: [8; 64],
    };
    let expected = [
        vec![2, 0, 0, 0, 2, 1, 2, 0, 15, 0, 0, 0, 16],
        vec![3; 16],
        vec![0, 0, 0, 32],
        vec![4; 32],
        vec![0, 0, 0, 0, 0, 0, 0, 7],
        vec![0, 0, 0, 32],
        vec![5; 32],
        vec![0, 0, 0, 32],
        vec![6; 32],
        vec![0, 0, 0, 32],
        vec![5; 32],
        vec![0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 32],
        vec![7; 32],
    ]
    .concat();
    assert_eq!(repair.unsigned_bytes(), expected);
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(
        hex(&repair.signature_hash()),
        "015c4c844859c5cd59ed9ee001cd7a26722b820ba275b2157395453dd0184c75"
    );
    assert_eq!(
        hex(&repair.hash()),
        "658af54006076161efaba03dfe5380a7d2b10352e960f86106159e0860c696cc"
    );
    assert_eq!(ReceiptRepair::decode(&repair.encode()).unwrap(), repair);
}

#[test]
fn repair_maximal_scope_and_full_book_evidence_fit_unchanged_eight_kib_cap() {
    let owner = MlsDevice::generate().unwrap();
    let document = LogicalDocument::new(
        vec![1; MAX_SERVER_ID_BYTES],
        DocType::StudioIndex,
        vec![2; MAX_LOGICAL_KEY_BYTES],
    )
    .unwrap();
    let sign = |epoch, close| {
        Receipt::sign(
            document.clone(),
            epoch,
            [close; 32],
            [9; 32],
            0,
            baseline(1),
            &owner,
        )
        .unwrap()
    };
    let selected = sign(2, 1);
    let losing = sign(2, 2);
    let repair = ReceiptRepair::sign_in_tenure(
        document.clone(),
        selected.tenure_id,
        [selected.hash(), losing.hash()],
        selected.hash(),
        7,
        0,
        &owner,
    )
    .unwrap();
    let latest = sign(9, 3);
    let other = sign(9, 4);
    let book = ReceiptBook {
        document: Some(document.clone()),
        tenure: Some(TenureSelection::from(&latest)),
        latest: Some(latest.clone()),
        previous_until_installed: Some(sign(8, 5)),
        fault: Some(canonical_receipt_pair(latest, other)),
        repair_sequence: 7,
        resolved_repair: Some(ResolvedRepair {
            repair,
            selected,
            losing,
        }),
    };
    let bytes = book.encode().unwrap();
    assert!(
        bytes.len() > 5 * 1024,
        "fixture must include all seven maximal-scope records"
    );
    assert!(bytes.len() <= MAX_RECEIPT_BOOK_BYTES);
    assert_eq!(
        ReceiptBook::decode(&bytes).unwrap().encode().unwrap(),
        bytes
    );
}
