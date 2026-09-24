// The same contracts exercise Registry, Studio Index and Studio Flipnote through their real
// typed adapters. Each including module supplies only its document-specific fixture operations.

impl Fixture {
    fn decision(&self, selected: &Receipt, losing: &Receipt, sequence: u64) -> ReceiptRepair {
        ReceiptRepair::sign_in_tenure(
            self.source.logical.clone(),
            selected.tenure_id,
            [selected.hash(), losing.hash()],
            selected.hash(),
            sequence,
            0,
            &self.owner,
        )
        .unwrap()
    }
    fn apply(
        &mut self,
        selected: &Receipt,
        losing: &Receipt,
        sequence: u64,
    ) -> SourceRepairOutcome {
        let repair = self.decision(selected, losing, sequence);
        self.source
            .apply_receipt_repair(&repair, selected, losing, &self.group, 0)
            .unwrap()
    }
    fn with_baseline(&self, receipt: &Receipt) -> Receipt {
        Receipt::sign(
            receipt.document.clone(),
            receipt.closed_epoch,
            receipt.close_record_hash,
            receipt.seed_change_hash,
            0,
            InheritedCheckpoint::Checkpoint {
                epoch: 1,
                close_record_hash: [61; 32],
                seed_change_hash: [62; 32],
            },
            &self.owner,
        )
        .unwrap()
    }
    fn quarantine(&self) {
        assert_eq!(
            self.source
                .gate
                .admit_inbound(crate::AdmittedOperation {
                    op_hash: [91; 32],
                    domain_op_id: [92; 32],
                    author: self.owner.device_id(),
                    encoded_len: 100,
                })
                .unwrap(),
            crate::Admission::Quarantined
        );
    }
}

#[test]
fn repair_named_current_epoch_fault_preserves_work_and_closing_quarantine() {
    for mut f in fixtures() {
        f.edit(1);
        let (selected, _) = f.target(0, 2);
        let (losing, _) = f.target(0, 3);
        assert_eq!(
            f.source.seal(losing.clone(), &f.group, 0).unwrap(),
            ReceiptIngest::Advanced
        );
        f.quarantine();
        assert_eq!(
            f.source.seal(selected.clone(), &f.group, 0).unwrap(),
            ReceiptIngest::Fault
        );
        let log = f.source.doc.signed_log().to_vec();
        assert_eq!(
            f.apply(&selected, &losing, 1),
            SourceRepairOutcome::Applied(RepairDisposition::Transitioned)
        );
        assert_eq!(f.source.phase(), EpochPhase::Closing);
        assert!(!f.source.adopting);
        assert!(!f.source.repair_install_pending());
        assert_eq!(f.source.doc.signed_log(), log);
        assert_eq!(f.source.op_count(), 1);
        assert_eq!(f.source.gate.quarantined_len(), 1);
        assert!(f.source.fault_evidence().is_none());
        f.reopen();
        let before = f.source.snapshot().unwrap();
        assert_eq!(
            f.apply(&selected, &losing, 1),
            SourceRepairOutcome::AlreadyResolved(RepairDisposition::Transitioned)
        );
        assert_eq!(f.source.snapshot().unwrap(), before);
        // Authority is checked even for an otherwise exact retry, before touching gate owner.
        let repair = f.decision(&selected, &losing, 1);
        assert!(f
            .source
            .apply_receipt_repair(&repair, &selected, &losing, &f.group, 1)
            .is_err());
        assert_eq!(f.source.snapshot().unwrap(), before);
        assert_eq!(
            f.apply(&losing, &selected, 1),
            SourceRepairOutcome::Held(RepairHold::SequenceNotNewer)
        );
    }
}

#[test]
fn repair_installed_winner_reopens_or_retains_qualifying_seal() {
    for keep_seal in [false, true] {
        for mut f in fixtures() {
            let (selected, seed) = f.target(2, 2);
            let (losing, _) = f.target(2, 3);
            f.install(&selected, &seed);
            f.edit(1);
            let inbound = f.inbound(91);
            let head = if keep_seal {
                let (head, _) = f.target(3, 4);
                f.source.seal(head.clone(), &f.group, 0).unwrap();
                Some(head)
            } else {
                let (distant, _) = f.target(20, 5);
                f.begin(&distant);
                None
            };
            assert_eq!(
                f.source.ingest(&inbound, &f.group, &f.owner).unwrap(),
                crate::Admission::Quarantined
            );
            assert_eq!(
                f.source.seal(losing.clone(), &f.group, 0).unwrap(),
                ReceiptIngest::Fault
            );
            assert_eq!(
                f.apply(&selected, &losing, 1),
                SourceRepairOutcome::Applied(RepairDisposition::Transitioned)
            );
            let state = f.source.repair_state().unwrap();
            assert!(state.installed);
            assert!(!state.install_pending);
            assert!(!f.source.adopting);
            assert_eq!(f.source.op_count(), 1);
            if let Some(head) = head {
                assert_eq!(f.source.phase(), EpochPhase::Closing);
                assert_eq!(f.source.receipts.latest(), Some(&head));
                assert_eq!(f.source.gate.quarantined_len(), 1);
            } else {
                assert_eq!(f.source.phase(), EpochPhase::Open);
                assert_eq!(f.source.receipts.latest(), Some(&selected));
                assert_eq!(f.source.gate.quarantined_len(), 0);
            }
            f.reopen();
            if !keep_seal {
                assert_eq!(
                    f.source.ingest(&inbound, &f.group, &f.owner).unwrap(),
                    crate::Admission::Accepted
                );
                assert_eq!(f.source.op_count(), 2);
                f.reopen();
                assert_eq!(
                    f.source.ingest(&inbound, &f.group, &f.owner).unwrap(),
                    crate::Admission::Duplicate
                );
            }
            assert_eq!(
                f.source.repair_state().unwrap().disposition,
                RepairDisposition::Transitioned
            );
        }
    }
}

#[test]
fn repair_covered_opening_requires_adoption_even_when_winner_closes_current_epoch() {
    for exact_losing_opening in [false, true] {
        for mut f in fixtures() {
            let (opening, seed) = f.target(2, 2);
            f.install(&opening, &seed);
            let op = f.edit(1);
            let (selected, winner_seed) = f.target(3, 4);
            let selected = f.with_baseline(&selected);
            let losing = if exact_losing_opening {
                opening.clone()
            } else {
                let (losing, _) = f.target(3, 3);
                f.source.seal(losing.clone(), &f.group, 0).unwrap();
                losing
            };
            assert_eq!(
                f.source.seal(selected.clone(), &f.group, 0).unwrap(),
                ReceiptIngest::Fault
            );
            assert_eq!(
                f.apply(&selected, &losing, 1),
                SourceRepairOutcome::Applied(RepairDisposition::Transitioned)
            );
            assert!(f.source.repair_install_pending());
            f.reopen();
            let before = f.source.snapshot().unwrap();
            assert!(f
                .source
                .prepare_checkpoint_adoption(&selected, winner_seed.bytes(), &f.group, 0)
                .is_err());
            assert!(f
                .source
                .prepare_repair_adoption(&selected, b"bad seed", &f.group, 0)
                .is_err());
            assert_eq!(f.source.snapshot().unwrap(), before);
            let plan = f
                .source
                .prepare_repair_adoption(&selected, winner_seed.bytes(), &f.group, 0)
                .unwrap();
            let recovery = plan.recovery_snapshot().unwrap();
            assert_eq!(recovery.reason, RecoveryReason::Repair);
            assert!(recovery.applied_ops.contains(&op.id(&f.owner.device_id())));
            assert_eq!(
                recovery.base_close_record_hash,
                Some(opening.close_record_hash)
            );
            assert_eq!(
                f.apply(&selected, &losing, 2),
                SourceRepairOutcome::Held(RepairHold::RepairInProgress)
            );
            assert_eq!(f.source.snapshot().unwrap(), before);
            f.source = f.source.adopted_successor(&plan, &f.group, 0).unwrap();
            assert_eq!(f.source.epoch(), 4);
            assert!(f.source.repair_state().unwrap().installed);
            assert!(!f.source.repair_install_pending());
            f.reopen();
            assert_eq!(
                f.apply(&selected, &losing, 1),
                SourceRepairOutcome::AlreadyResolved(RepairDisposition::Transitioned)
            );
            // Ordinary later progress must carry the binding but not inherit Repair attribution.
            let (later, seed) = f.target(10, 5);
            let later = f.with_baseline(&later);
            assert_eq!(f.begin(&later), ReceiptIngest::Advanced);
            let later_plan = f.plan(&later, &seed);
            if let Some(recovery) = later_plan.recovery_snapshot() {
                assert_eq!(recovery.reason, RecoveryReason::Rewound);
            }
            f.source = f
                .source
                .adopted_successor(&later_plan, &f.group, 0)
                .unwrap();
            f.reopen();
            assert!(!f.source.repair_state().unwrap().installed);
            assert_eq!(
                f.apply(&selected, &losing, 1),
                SourceRepairOutcome::AlreadyResolved(RepairDisposition::Transitioned)
            );
        }
    }
}

#[test]
fn repair_same_epoch_adoption_and_healthy_losing_opening_keep_distinct_actions() {
    for healthy in [false, true] {
        for mut f in fixtures() {
            let (selected, seed) = f.target(0, 2);
            let (losing, losing_seed) = f.target(0, 3);
            if healthy {
                f.install(&losing, &losing_seed);
            } else {
                f.begin(&losing);
                assert_eq!(f.begin(&selected), ReceiptIngest::Fault);
            }
            let disposition = if healthy {
                RepairDisposition::Retargeted
            } else {
                RepairDisposition::Transitioned
            };
            assert_eq!(
                f.apply(&selected, &losing, 1),
                SourceRepairOutcome::Applied(disposition)
            );
            assert!(f.source.repair_install_pending());
            f.reopen();
            let plan = f
                .source
                .prepare_repair_adoption(&selected, seed.bytes(), &f.group, 0)
                .unwrap();
            f.source = f.source.adopted_successor(&plan, &f.group, 0).unwrap();
            f.reopen();
            assert_eq!(f.source.repair_state().unwrap().disposition, disposition);
            assert!(f.source.repair_state().unwrap().installed);
        }
    }
}

#[test]
fn repair_screening_does_not_claim_ordinary_adoption_or_replace_unrelated_fault() {
    for unrelated_fault in [false, true] {
        for mut f in fixtures() {
            f.edit(1);
            let (selected, seed) = f.target(10, 2);
            let (losing, _) = f.target(10, 3);
            let pair = if unrelated_fault {
                let (a, _) = f.target(0, 4);
                let (b, _) = f.target(0, 5);
                f.source.seal(a.clone(), &f.group, 0).unwrap();
                assert_eq!(
                    f.source.seal(b.clone(), &f.group, 0).unwrap(),
                    ReceiptIngest::Fault
                );
                Some((a, b))
            } else {
                f.begin(&selected);
                None
            };
            let gate = f.source.gate.encode().unwrap();
            let head = f.source.receipts.latest().cloned();
            assert_eq!(
                f.apply(&selected, &losing, 1),
                SourceRepairOutcome::Applied(RepairDisposition::Screened)
            );
            assert_eq!(f.source.gate.encode().unwrap(), gate);
            assert_eq!(f.source.receipts.latest(), head.as_ref());
            assert!(!f.source.repair_install_pending());
            assert!(f
                .source
                .prepare_repair_adoption(&selected, seed.bytes(), &f.group, 0)
                .is_err());
            f.reopen();
            if let Some((a, b)) = pair {
                let (x, y) = f.source.fault_evidence().unwrap();
                assert!((x == &a && y == &b) || (x == &b && y == &a));
                assert_eq!(f.source.phase(), EpochPhase::Fault);
                assert_eq!(
                    f.apply(&a, &b, 2),
                    SourceRepairOutcome::Applied(RepairDisposition::Transitioned)
                );
            } else {
                let plan = f.plan(&selected, &seed);
                assert_eq!(
                    plan.recovery_snapshot().unwrap().reason,
                    RecoveryReason::Rewound
                );
                f.source = f.source.adopted_successor(&plan, &f.group, 0).unwrap();
                assert!(f.source.repair_state().unwrap().installed);
                assert_eq!(
                    f.source.repair_state().unwrap().disposition,
                    RepairDisposition::Screened
                );
            }
            f.reopen();
        }
    }
}

#[test]
fn repair_v3_binding_is_strict_and_legacy_state_does_not_invent_provenance() {
    for mut f in fixtures() {
        let (selected, _) = f.target(10, 2);
        let (losing, _) = f.target(10, 3);
        f.begin(&losing);
        f.begin(&selected);
        f.apply(&selected, &losing, 1);
        let bytes = f.source.snapshot().unwrap();
        assert_eq!(&bytes[..4], &[3, 1, 1, 1]);
        for (offset, value) in [
            (1, 2),
            (2, 0),
            (2, 4),
            (3, 0),
            (3, 2),
            (7, 31),
            (8, bytes[8] ^ 1),
        ] {
            let mut bad = bytes.clone();
            bad[offset] = value;
            assert!(
                f.restore_bytes(&bad).is_err(),
                "accepted invalid v3 byte {offset}"
            );
        }
        // Strip exactly the new prefix, leaving the historical v2 body and resolved receipt
        // book untouched. It remains readable and byte-stable, but cannot claim typed action.
        let mut legacy = vec![2];
        legacy.extend_from_slice(&bytes[40..]);
        f.source = f.restore_bytes(&legacy).unwrap();
        assert_eq!(f.source.snapshot().unwrap(), legacy);
        assert!(f.source.repair_state().is_none());
        assert!(!f.source.repair_install_pending());
        assert_eq!(
            f.apply(&selected, &losing, 1),
            SourceRepairOutcome::Held(RepairHold::UnsupportedShape)
        );
        assert_eq!(f.source.snapshot().unwrap(), legacy);
    }
}

#[test]
fn repair_cross_tenure_unfaults_without_installing_old_owner_and_screens_healthy_sources() {
    for mode in 0..4 {
        for mut f in fixtures() {
            let epoch = if mode == 0 { 0 } else { 2 };
            let (selected, seed) = f.target(epoch, 2);
            let (losing, losing_seed) = f.target(epoch, 3);
            match mode {
                0 => {
                    f.source.seal(selected.clone(), &f.group, 0).unwrap();
                    f.source.seal(losing.clone(), &f.group, 0).unwrap();
                }
                1 => {
                    f.install(&losing, &losing_seed);
                    f.source.seal(selected.clone(), &f.group, 0).unwrap();
                }
                2 => f.install(&losing, &losing_seed),
                3 => {
                    f.begin(&selected);
                }
                _ => unreachable!(),
            }
            let gate = f.source.gate.encode().unwrap();
            let prior_head = f.source.receipts.latest().cloned();
            let successor = MlsDevice::generate().unwrap();
            let welcome = f
                .group
                .add_member(&f.owner, successor.key_package().unwrap())
                .unwrap()
                .welcome;
            let mut group = ServerGroup::join(&successor, &welcome).unwrap();
            group
                .remove_member(&successor, &f.owner.device_id())
                .unwrap();
            let tenure = group.epoch();
            let repair = ReceiptRepair::sign_in_tenure(
                selected.document.clone(),
                selected.tenure_id,
                [selected.hash(), losing.hash()],
                selected.hash(),
                1,
                tenure,
                &successor,
            )
            .unwrap();
            let disposition = if mode < 2 {
                RepairDisposition::Transitioned
            } else {
                RepairDisposition::Screened
            };
            assert_eq!(
                f.source
                    .apply_receipt_repair(&repair, &selected, &losing, &group, tenure)
                    .unwrap(),
                SourceRepairOutcome::Applied(disposition)
            );
            assert!(!f.source.repair_install_pending());
            assert!(f
                .source
                .prepare_repair_adoption(&selected, seed.bytes(), &group, tenure)
                .is_err());
            assert!(f
                .source
                .prepare_checkpoint_adoption(&selected, seed.bytes(), &group, tenure)
                .is_err());
            if mode < 2 {
                assert_eq!(f.source.phase(), EpochPhase::Open);
                assert!(!f.source.adopting);
                assert_eq!(f.source.receipts.latest(), f.source.opening.as_ref());
            } else {
                assert_eq!(f.source.gate.encode().unwrap(), gate);
                assert_eq!(f.source.receipts.latest(), prior_head.as_ref());
            }
            f.group = group;
            f.owner = successor;
            // Restoring refreshes current gate ownership, but retains the historical binding.
            let bytes = f.source.snapshot().unwrap();
            f.source = f.restore_bytes(&bytes).unwrap();
            f.reopen();
            assert_eq!(
                f.source
                    .apply_receipt_repair(&repair, &selected, &losing, &f.group, tenure)
                    .unwrap(),
                SourceRepairOutcome::AlreadyResolved(disposition)
            );
            // A fresh receipt in the independent current tenure remains usable.
            let (current, current_seed) = f.target(10, 7);
            let current = Receipt::sign(
                current.document,
                current.closed_epoch,
                current.close_record_hash,
                current.seed_change_hash,
                tenure,
                InheritedCheckpoint::EpochZero,
                &f.owner,
            )
            .unwrap();
            assert_eq!(
                f.source
                    .begin_checkpoint_adoption(current.clone(), &f.group, tenure)
                    .unwrap(),
                ReceiptIngest::Advanced
            );
            let plan = f
                .source
                .prepare_checkpoint_adoption(&current, current_seed.bytes(), &f.group, tenure)
                .unwrap();
            f.source = f.source.adopted_successor(&plan, &f.group, tenure).unwrap();
            f.reopen();
            assert_eq!(f.source.repair_state().unwrap().disposition, disposition);
        }
    }
}

#[test]
fn repair_rejects_bad_evidence_and_scope_without_refreshing_or_mutating_source() {
    for mut f in fixtures() {
        f.edit(1);
        let (selected, _) = f.target(0, 2);
        let (losing, _) = f.target(0, 3);
        f.source.seal(selected.clone(), &f.group, 0).unwrap();
        f.source.seal(losing.clone(), &f.group, 0).unwrap();
        let before = f.source.snapshot().unwrap();
        let repair = f.decision(&selected, &losing, 1);
        let mut bad = repair.clone();
        bad.signature[0] ^= 1;
        assert!(f
            .source
            .apply_receipt_repair(&bad, &selected, &losing, &f.group, 0)
            .is_err());
        let mut bad = losing.clone();
        bad.signature[0] ^= 1;
        assert!(f
            .source
            .apply_receipt_repair(&repair, &selected, &bad, &f.group, 0)
            .is_err());
        assert!(f
            .source
            .apply_receipt_repair(&repair, &selected, &selected, &f.group, 0)
            .is_err());
        let other_owner = MlsDevice::generate().unwrap();
        let other_group = ServerGroup::create(&other_owner).unwrap();
        assert!(f
            .source
            .apply_receipt_repair(&repair, &selected, &losing, &other_group, 0)
            .is_err());
        assert_eq!(f.source.snapshot().unwrap(), before);
    }
}

#[test]
fn repair_protocol_allowance_matches_actual_snapshot_growth() {
    for mut f in fixtures() {
        f.edit(1);
        let (selected, _) = f.target(10, 2);
        let (losing, _) = f.target(10, 3);
        let before = f.source.snapshot().unwrap();
        let protocol = f.source.storage_protocol_bytes().unwrap();
        assert_eq!(
            f.apply(&selected, &losing, 1),
            SourceRepairOutcome::Applied(RepairDisposition::Screened)
        );
        let after = f.source.snapshot().unwrap();
        assert_eq!(
            after.len() - before.len(),
            f.source.storage_protocol_bytes().unwrap() - protocol
        );
        assert_eq!(after[0], 3);
        // An invented non-screening action cannot explain a headless same-tenure Open source.
        for action in [1, 2] {
            let mut bad = after.clone();
            bad[2] = action;
            assert!(f.restore_bytes(&bad).is_err());
        }
        f.reopen();
    }
}

#[test]
fn repair_restore_rejects_covered_live_heads_and_screened_openings() {
    for installed_loser in [false, true] {
        for mut f in fixtures() {
            let (selected, _) = f.target(0, 2);
            let (losing, seed) = f.target(0, 3);
            let head = if installed_loser {
                f.install(&losing, &seed);
                f.target(1, 4).0
            } else {
                losing.clone()
            };
            f.source.seal(head.clone(), &f.group, 0).unwrap();
            let gate = f.source.gate.encode().unwrap();
            f.apply(&selected, &losing, 1);
            // Preserve genuine signed repair evidence but substitute a canonical live role.
            // These old gate/book validators accept this independently; v3 must reject it.
            let mut book = f.source.receipts.encode().unwrap();
            let selected_bytes = selected.encode();
            let offset = book
                .windows(selected_bytes.len())
                .position(|bytes| bytes == selected_bytes)
                .unwrap();
            book.splice(offset..offset + selected_bytes.len(), head.encode());
            f.source.receipts = ReceiptBook::decode(&book).unwrap();
            f.source.gate = EpochGate::decode(&gate).unwrap();
            for adopting in [false, true] {
                f.source.adopting = adopting;
                let mut bad = f.source.snapshot().unwrap();
                for action in [1, 2, 3] {
                    // Covered opening is valid only as non-screening adoption recovery.
                    if installed_loser && adopting && action != 3 {
                        continue;
                    }
                    bad[2] = action;
                    assert!(f.restore_bytes(&bad).is_err(),"accepted covered live role: opening={installed_loser}, adoption={adopting}, action={action}");
                }
            }
        }
    }
}

#[test]
fn repair_pending_continuation_fences_competing_public_adoption_and_seal() {
    for mut f in fixtures() {
        let (selected, seed) = f.target(0, 2);
        let (losing, losing_seed) = f.target(0, 3);
        f.install(&losing, &losing_seed);
        f.apply(&selected, &losing, 1);
        let before = f.source.snapshot().unwrap();
        let (later, _) = f.target(10, 4);
        let (conflict, _) = f.target(0, 5);
        for receipt in [&later, &conflict] {
            assert!(f
                .source
                .begin_checkpoint_adoption(receipt.clone(), &f.group, 0)
                .is_err());
            assert!(f.source.seal(receipt.clone(), &f.group, 0).is_err());
            assert_eq!(f.source.snapshot().unwrap(), before);
        }
        for receipt in [&selected, &losing] {
            let outcome = if receipt == &selected {
                ReceiptIngest::Duplicate
            } else {
                ReceiptIngest::Stale
            };
            assert_eq!(
                f.source
                    .begin_checkpoint_adoption(receipt.clone(), &f.group, 0)
                    .unwrap(),
                outcome
            );
            assert_eq!(
                f.source.seal(receipt.clone(), &f.group, 0).unwrap(),
                outcome
            );
            assert_eq!(f.source.snapshot().unwrap(), before);
            assert!(f
                .source
                .begin_checkpoint_adoption(receipt.clone(), &f.group, 1)
                .is_err());
        }
        assert!(f.source.repair_install_pending());
        assert_eq!(
            f.apply(&selected, &losing, 2),
            SourceRepairOutcome::Held(RepairHold::RepairInProgress)
        );
        f.reopen();
        let plan = f
            .source
            .prepare_repair_adoption(&selected, seed.bytes(), &f.group, 0)
            .unwrap();
        f.source = f.source.adopted_successor(&plan, &f.group, 0).unwrap();
        assert!(f.source.repair_state().unwrap().installed);
    }
}

#[test]
fn repair_restore_rejects_retargeted_without_adoption_or_installed_opening() {
    for mut f in fixtures() {
        let (selected, _) = f.target(0, 2);
        let (losing, _) = f.target(0, 3);
        f.source.seal(losing.clone(), &f.group, 0).unwrap();
        assert_eq!(
            f.apply(&selected, &losing, 1),
            SourceRepairOutcome::Applied(RepairDisposition::Retargeted)
        );
        f.reopen();
        f.source.adopting = false;
        let bad = f.source.snapshot().unwrap();
        assert!(f.restore_bytes(&bad).is_err());
    }
}
