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
fn repair_evidence_checks_canonical_signed_conflict_without_live_authority() {
    let f = Fixture::new(false);
    conflicting_receipt_pair(&f.document, &f.selected, &f.losing).unwrap();
    conflicting_receipt_pair(&f.document, &f.losing, &f.selected).unwrap();
    f.repair.check_evidence(&f.selected, &f.losing).unwrap();

    // These are valid signatures over genuinely different inheritance at different epochs.
    // A checker restricted to equal closed epochs would strand a real historical fault.
    let different_baseline = f.sign(3, baseline(1), 4);
    conflicting_receipt_pair(&f.document, &f.selected, &different_baseline).unwrap();

    // The historical pair remains authentic after removal of its author. No group or current
    // membership is an input to the evidence checker, and no verified capability is returned.
    let successor = MlsDevice::generate().unwrap();
    let mut old_group = f.group;
    let welcome = old_group
        .add_member(&f.owner, successor.key_package().unwrap())
        .unwrap()
        .welcome;
    let mut new_group = ServerGroup::join(&successor, &welcome).unwrap();
    new_group
        .remove_member(&successor, &f.owner.device_id())
        .unwrap();
    assert!(f.selected.verify_current_owner(&new_group, 0).is_err());
    conflicting_receipt_pair(&f.document, &f.selected, &f.losing).unwrap();

    for bad_side in [false, true] {
        let (mut a, mut b) = (f.selected.clone(), f.losing.clone());
        let bad = if bad_side { &mut a } else { &mut b };
        bad.signature[0] ^= 1;
        assert!(matches!(
            conflicting_receipt_pair(&f.document, &a, &b),
            Err(ReplError::EpochAuthority)
        ));
    }
}

#[test]
fn repair_self_signed_member_pair_does_not_prove_owner_authority() {
    let mut f = Fixture::new(false);
    let member = MlsDevice::generate().unwrap();
    f.group
        .add_member(&f.owner, member.key_package().unwrap())
        .unwrap();
    assert!(f.group.member_device_ids().contains(&member.device_id()));
    assert_eq!(f.group.designated_committer(), Some(f.owner.device_id()));
    let sign = |close| {
        Receipt::sign(
            f.document.clone(),
            2,
            [close; 32],
            [9; 32],
            0,
            InheritedCheckpoint::EpochZero,
            &member,
        )
        .unwrap()
    };
    let (a, b) = (sign(6), sign(7));
    // A malicious authenticated member can create canonical, correctly signed equivocation.
    // The primitive deliberately accepts it: no membership/owner history is an input. This is
    // a limitation test, NOT the future report-admission/no-write security regression.
    conflicting_receipt_pair(&f.document, &a, &b).unwrap();
    assert!(a.verify_current_owner(&f.group, 0).is_err());
    assert!(b.verify_current_owner(&f.group, 0).is_err());
}

#[test]
fn repair_evidence_rejects_shape_scope_tenure_duplicates_and_consistent_progress() {
    let f = Fixture::new(false);
    let mut malformed = f.losing.clone();
    // A signature alone is insufficient: an empty key is encodable and signable through public
    // fields, but LogicalDocument's decoder refuses it. Keep the signature valid to isolate shape.
    malformed.document.logical_key.clear();
    malformed.signature = f.owner.sign(&malformed.signature_hash()).unwrap();
    assert!(Receipt::decode(&malformed.encode()).is_err());
    assert!(conflicting_receipt_pair(&f.document, &f.selected, &malformed).is_err());

    let mut foreign_document = f.document.clone();
    foreign_document.logical_key[0] ^= 1;
    let foreign = Receipt::sign(
        foreign_document.clone(),
        2,
        [3; 32],
        [9; 32],
        0,
        InheritedCheckpoint::EpochZero,
        &f.owner,
    )
    .unwrap();
    foreign.verify_signature_only().unwrap();
    assert!(matches!(
        conflicting_receipt_pair(&f.document, &f.selected, &foreign),
        Err(ReplError::EpochScope)
    ));
    // Even two matching, valid receipts cannot be rebound by the enclosing target.
    assert!(matches!(
        conflicting_receipt_pair(&foreign_document, &f.selected, &f.losing),
        Err(ReplError::EpochScope)
    ));
    let other_tenure = Receipt::sign(
        f.document.clone(),
        2,
        [3; 32],
        [9; 32],
        1,
        InheritedCheckpoint::EpochZero,
        &f.owner,
    )
    .unwrap();
    other_tenure.verify_signature_only().unwrap();
    for receipt in [
        other_tenure,
        f.selected.clone(),
        f.sign(3, InheritedCheckpoint::EpochZero, 4),
    ] {
        assert!(matches!(
            conflicting_receipt_pair(&f.document, &f.selected, &receipt),
            Err(ReplError::ReceiptConflict)
        ));
    }
    let mut oversized = f.losing.clone();
    oversized.document.logical_key = vec![0; MAX_LOGICAL_KEY_BYTES + 1];
    assert!(matches!(
        conflicting_receipt_pair(&f.document, &f.selected, &oversized),
        Err(ReplError::EpochBound)
    ));
}

#[test]
fn repair_evidence_binds_the_exact_pair_even_when_it_shares_the_winner() {
    let f = Fixture::new(false);
    let third = f.sign(2, InheritedCheckpoint::EpochZero, 4);
    conflicting_receipt_pair(&f.document, &f.selected, &third).unwrap();
    assert!(f.repair.receipt_hashes.contains(&f.selected.hash()));
    // M5: only pair equality can reject this second genuine pair with the same selected receipt.
    assert!(
        matches!(
            f.repair.check_evidence(&f.selected, &third),
            Err(ReplError::ReceiptConflict)
        ),
        "repair evidence accepted a different pair sharing the winner"
    );
    f.repair.check_evidence(&f.selected, &f.losing).unwrap();
    f.repair.check_evidence(&f.losing, &f.selected).unwrap();
}

#[test]
fn repair_evidence_checks_decision_bindings_but_does_not_assert_its_authority() {
    let f = Fixture::new(false);
    for mutation in 0..5 {
        let mut repair = f.repair.clone();
        match mutation {
            0 => repair.tenure_id[0] ^= 1,
            1 => repair.receipt_hashes.swap(0, 1),
            2 => repair.selected_receipt_hash = [0; 32],
            3 => repair.issuer_tenure_start_group_epoch = None,
            4 => repair.repair_sequence = 0,
            _ => unreachable!(),
        }
        // All receipts are still canonical, authentic, and conflicting for each negative.
        conflicting_receipt_pair(&f.document, &f.selected, &f.losing).unwrap();
        assert!(
            matches!(
                repair.check_evidence(&f.selected, &f.losing),
                Err(ReplError::ReceiptConflict)
            ),
            "mutation {mutation}"
        );
    }
    let mut unsigned = f.repair.clone();
    unsigned.signature[0] ^= 1;
    unsigned.check_evidence(&f.selected, &f.losing).unwrap();
    assert!(unsigned.verify_current_owner(&f.group, 0).is_err());
    let resolved = ResolvedRepair {
        repair: unsigned,
        selected: f.selected.clone(),
        losing: f.losing.clone(),
    };
    assert!(resolved.verify(Some(&f.document), 1).is_err());
}

#[test]
fn repair_headless_book_roundtrips_and_retains_its_evidence_identity() {
    let mut f = Fixture::new(false);
    f.apply();
    // C-8 / N2b: the cross-tenure planner will use this epoch-zero shape. Exercising the
    // book codec directly isolates its identity derivation from later typed source validation.
    f.book.latest = None;
    f.book.tenure = None;
    for adoption in [false, true] {
        let bytes = f.book.encode_mode(adoption).unwrap();
        assert_eq!(bytes[0], if adoption { 5 } else { 4 });
        let restored =
            ReceiptBook::decode_mode(&bytes, adoption).expect("headless repair book must restore");
        assert_eq!(restored.encode_mode(adoption).unwrap(), bytes);
        assert_eq!(restored.document.as_ref(), Some(&f.document));
        assert!(restored.latest().is_none());
        assert!(restored.tenure.is_none());
        assert_eq!(restored.repair_sequence(), 1);
        assert_eq!(restored.latest_repair(), Some(&f.repair));
        assert!(restored.is_repaired_loser(&f.losing));
    }
}

#[test]
fn repair_headless_book_rejects_a_same_document_predecessor() {
    let mut f = Fixture::new(false);
    f.apply();
    f.book.latest = None;
    f.book.tenure = None;
    for adoption in [false, true] {
        // The headless repair evidence is otherwise valid, including its document and stored
        // sequence. A fully signed SAME-document receipt isolates predecessor presence from
        // the existing foreign-document rejection and from typed gate/opening validation.
        assert!(ReceiptBook::decode_mode(&f.book.encode_mode(adoption).unwrap(), adoption).is_ok());
        f.selected.verify_signature_only().unwrap();
        assert_eq!(f.selected.document, f.document);
        f.book.previous_until_installed = Some(f.selected.clone());
        let bytes = f.book.encode_mode(adoption).unwrap();
        assert_eq!(bytes[0], if adoption { 5 } else { 4 });
        assert!(
            matches!(
                ReceiptBook::decode_mode(&bytes, adoption),
                Err(ReplError::Malformed)
            ),
            "a headless repaired book must reject a same-document predecessor"
        );
        f.book.previous_until_installed = None;
        assert!(ReceiptBook::decode_mode(&f.book.encode_mode(adoption).unwrap(), adoption).is_ok());
    }
}

#[test]
fn repair_headless_decode_keeps_scope_sequence_and_legacy_constraints() {
    let mut f = Fixture::new(false);
    f.apply();
    f.book.latest = None;
    f.book.tenure = None;
    for mutation in 0..3 {
        let mut book = f.book.clone();
        match mutation {
            0 => book.tenure = Some(TenureSelection::from(&f.selected)),
            1 => book.repair_sequence = 2,
            2 => book.resolved_repair.as_mut().unwrap().selected = f.losing.clone(),
            _ => unreachable!(),
        }
        assert!(
            ReceiptBook::decode(&book.encode().unwrap()).is_err(),
            "mutation {mutation}"
        );
    }
    // Old tags still cannot carry repair bytes, and an empty legacy book gains no identity.
    for version in [1, 2, 3] {
        let mut bytes = f.book.encode().unwrap();
        bytes[0] = version;
        assert!(ReceiptBook::decode_mode(&bytes, version == 3).is_err());
    }
    let empty = ReceiptBook::default();
    for adoption in [false, true] {
        let bytes = empty.encode_mode(adoption).unwrap();
        assert_eq!(bytes[0], if adoption { 3 } else { 1 });
        let restored = ReceiptBook::decode_mode(&bytes, adoption).unwrap();
        assert!(restored.document.is_none());
        assert_eq!(restored.encode_mode(adoption).unwrap(), bytes);
    }
}

#[test]
fn repair_headed_book_checks_retained_predecessor_document() {
    let mut f = Fixture::new(false);
    f.apply();
    let predecessor = f.sign(1, InheritedCheckpoint::EpochZero, 5);
    let mut foreign = predecessor.clone();
    foreign.document.logical_key[0] ^= 1;
    foreign.signature = f.owner.sign(&foreign.signature_hash()).unwrap();
    foreign.verify_signature_only().unwrap();
    for adoption in [false, true] {
        // Keep a valid latest receipt and tenure so the independent headless-predecessor
        // guard cannot mask the document-scope check on this retained receipt.
        assert!(f.book.latest().is_some());
        f.book.previous_until_installed = Some(predecessor.clone());
        let valid = f.book.encode_mode(adoption).unwrap();
        let restored = ReceiptBook::decode_mode(&valid, adoption).unwrap();
        assert_eq!(restored.encode_mode(adoption).unwrap(), valid);

        f.book.previous_until_installed = Some(foreign.clone());
        assert!(
            matches!(
                ReceiptBook::decode_mode(&f.book.encode_mode(adoption).unwrap(), adoption),
                Err(ReplError::Malformed)
            ),
            "a headed repaired book must reject a foreign-document predecessor"
        );
    }
}

#[test]
fn repair_losing_adoption_anchors_do_not_recreate_a_resolved_fault() {
    for different_baseline in [false, true] {
        for retained_as_opening in [false, true] {
            for descendant in [false, true] {
                // A same-baseline repair identifies only the exact loser: its subsequent
                // ancestry is unknown. Different inheritance also proves its branch lost.
                if descendant && !different_baseline {
                    continue;
                }
                let mut f = Fixture::new(different_baseline);
                f.apply();
                // Both anchors are independently checked by admission and restart. Exercise each
                // alone so exempting only the opening or only the previous target cannot pass.
                let anchor = if descendant {
                    f.sign(9, f.losing.inherited.clone(), 8)
                } else {
                    f.losing.clone()
                };
                let opening = retained_as_opening.then_some(&anchor);
                if !retained_as_opening {
                    f.book.previous_until_installed = Some(anchor.clone());
                }
                let gate = EpochGate::new(f.document.clone(), 43, 3, f.owner.device_id());
                assert_eq!(
                    f.book
                        .ingest_adoption(f.selected.clone(), &f.group, 0, &gate, opening)
                        .unwrap(),
                    ReceiptIngest::Duplicate,
                    "the retained loser must not re-fault the selected checkpoint"
                );
                assert_eq!(gate.phase(), EpochPhase::Closing);
                assert!(!f.book.is_faulted());
                let bytes = f.book.encode_adoption().unwrap();
                let restored = ReceiptBook::decode_adoption(&bytes).unwrap();
                restored
                    .verify_adoption_state(&f.document, &gate.inner.lock().unwrap(), opening)
                    .expect("a repaired losing anchor must remain restorable");
                assert_eq!(restored.encode_adoption().unwrap(), bytes);
            }
        }
    }
}

#[test]
fn repair_anchor_exemption_does_not_hide_a_third_baseline_or_unrepaired_rollback() {
    let mut f = Fixture::new(true);
    f.apply();
    let gate = EpochGate::new(f.document.clone(), 43, 3, f.owner.device_id());
    let third = f.sign(3, baseline(2), 5);
    assert_eq!(
        f.book
            .ingest_adoption(third, &f.group, 0, &gate, Some(&f.selected))
            .unwrap(),
        ReceiptIngest::Fault
    );
    assert_eq!(gate.phase(), EpochPhase::Fault);

    // An unrepaired newer anchor still prevents an older target from being restorable.
    let mut f = Fixture::new(false);
    f.apply();
    f.book.previous_until_installed = Some(f.sign(3, InheritedCheckpoint::EpochZero, 6));
    let gate = EpochGate::new(f.document.clone(), 42, 2, f.owner.device_id());
    let mut inner = gate.inner.lock().unwrap();
    inner.phase = EpochPhase::Closing;
    inner.receipt_hash = Some(f.selected.hash());
    assert!(f
        .book
        .verify_adoption_state(&f.document, &inner, None)
        .is_err());
    f.book.previous_until_installed = None;
    let newer_opening = f.sign(3, InheritedCheckpoint::EpochZero, 6);
    assert!(f
        .book
        .verify_adoption_state(&f.document, &inner, Some(&newer_opening))
        .is_err());
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
