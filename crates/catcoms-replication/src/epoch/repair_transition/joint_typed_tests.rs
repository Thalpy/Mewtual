// Included by the same real Registry/Index/Flipnote fixtures as typed_tests.rs.
use crate::{JournalRepairEffect, OwnerReceiptJournal, ReceiptRepairPlan};

impl Fixture {
    fn joint_plan(
        &mut self,
        selected: &Receipt,
        losing: &Receipt,
        sequence: u64,
        journal: &OwnerReceiptJournal,
        close: Option<&CloseRecord>,
    ) -> Result<ReceiptRepairPlan, ReplError> {
        let repair = self.decision(selected, losing, sequence);
        self.source.prepare_receipt_repair(
            &repair,
            selected,
            losing,
            journal,
            close,
            &self.group,
            0,
        )
    }

    fn journal_receipt(&self, epoch: u64, salt: u8) -> (Receipt, CloseRecord) {
        let close = CloseRecord::sign(
            &self.source.logical,
            u128::from(salt),
            epoch,
            vec![[salt; 32]],
            &self.owner,
        )
        .unwrap();
        let receipt = Receipt::sign(
            self.source.logical.clone(),
            epoch,
            close.hash(),
            [salt; 32],
            0,
            InheritedCheckpoint::EpochZero,
            &self.owner,
        )
        .unwrap();
        (receipt, close)
    }

    fn journal_at(&self, head: &Receipt, pending: bool) -> OwnerReceiptJournal {
        let mut journal = OwnerReceiptJournal::default();
        // Public owner decisions must walk the actual adjacency chain to a higher H.
        for epoch in 0..head.closed_epoch {
            let (prior, _) = self.journal_receipt(epoch, 80);
            journal.prepare(prior.clone(), &self.group, 0).unwrap();
            journal.mark_published(prior.hash()).unwrap();
        }
        journal.prepare(head.clone(), &self.group, 0).unwrap();
        if !pending {
            journal.mark_published(head.hash()).unwrap();
        }
        journal
    }
}

#[test]
fn joint_repair_transitioned_allows_unchanged_published_or_pending_higher_head() {
    for pending in [false, true] {
        for mut f in fixtures() {
            let (selected, seed) = f.target(2, 2);
            let (losing, _) = f.target(2, 3);
            f.install(&selected, &seed);
            f.edit(1);
            let (head, _) = f.target(3, 4);
            f.source.seal(head.clone(), &f.group, 0).unwrap();
            f.source.seal(losing.clone(), &f.group, 0).unwrap();
            let journal = f.journal_at(&head, pending);
            let before_source = f.source.snapshot().unwrap();
            let before_journal = journal.encode();
            let plan = f.joint_plan(&selected, &losing, 1, &journal, None).unwrap();
            assert_eq!(
                plan.source_outcome(),
                SourceRepairOutcome::Applied(RepairDisposition::Transitioned)
            );
            assert_eq!(plan.journal_effect(), JournalRepairEffect::NoChange);
            assert_eq!(plan.journal().encode(), before_journal);
            assert_eq!(f.source.snapshot().unwrap(), before_source);
            assert_eq!(journal.encode(), before_journal);
            f.source
                .apply_planned_receipt_repair(plan, &journal, &f.group, 0)
                .unwrap();
            assert_eq!(f.source.receipts.latest(), Some(&head));
            assert_eq!(f.source.phase(), EpochPhase::Closing);
            assert!(!f.source.adopting);
            f.reopen();
            let before = f.source.snapshot().unwrap();
            let plan = f.joint_plan(&selected, &losing, 1, &journal, None).unwrap();
            assert_eq!(
                plan.source_outcome(),
                SourceRepairOutcome::AlreadyResolved(RepairDisposition::Transitioned)
            );
            f.source
                .apply_planned_receipt_repair(plan, &journal, &f.group, 0)
                .unwrap();
            assert_eq!(f.source.snapshot().unwrap(), before);
        }
    }
}

#[test]
fn joint_repair_screened_allows_journal_replacement_and_preserves_unrelated_fault() {
    for pending in [false, true] {
        for faulted in [false, true] {
            for mut f in fixtures() {
                let (selected, _) = f.journal_receipt(0, 2);
                let (losing, close) = f.journal_receipt(0, 3);
                if faulted {
                    let (a, _) = f.target(0, 4);
                    let (b, _) = f.target(0, 5);
                    f.source.seal(a, &f.group, 0).unwrap();
                    f.source.seal(b, &f.group, 0).unwrap();
                } else {
                    f.edit(1);
                }
                let gate = f.source.gate.encode().unwrap();
                let fault = f
                    .source
                    .fault_evidence()
                    .map(|(a, b)| (a.clone(), b.clone()));
                let journal = f.journal_at(&losing, pending);
                let before = f.source.snapshot().unwrap();
                let plan = f
                    .joint_plan(&selected, &losing, 1, &journal, pending.then_some(&close))
                    .unwrap();
                assert_eq!(
                    plan.source_outcome(),
                    SourceRepairOutcome::Applied(RepairDisposition::Screened)
                );
                assert_eq!(
                    plan.journal_effect(),
                    if pending {
                        JournalRepairEffect::RetirePendingAndReplace
                    } else {
                        JournalRepairEffect::Replace
                    }
                );
                assert_eq!(f.source.snapshot().unwrap(), before);
                let candidate = OwnerReceiptJournal::decode(&plan.journal().encode()).unwrap();
                assert_eq!(candidate.effective_choice(), Some(&selected));
                assert_eq!(candidate.retired_pending().is_some(), pending);
                f.source
                    .apply_planned_receipt_repair(plan, &candidate, &f.group, 0)
                    .unwrap();
                assert_eq!(f.source.gate.encode().unwrap(), gate);
                assert_eq!(
                    f.source
                        .fault_evidence()
                        .map(|(a, b)| (a.clone(), b.clone())),
                    fault
                );
                assert!(!f.source.repair_install_pending());
                f.reopen();
            }
        }
    }
}

#[test]
fn joint_repair_retargeted_keeps_guarded_losing_opening_and_unrelated_journal_head() {
    for mut f in fixtures() {
        let (selected, _) = f.target(0, 2);
        let (losing, seed) = f.target(0, 3);
        f.install(&losing, &seed);
        f.edit(1);
        let (head, _) = f.journal_receipt(4, 6);
        let journal = f.journal_at(&head, false);
        let plan = f.joint_plan(&selected, &losing, 1, &journal, None).unwrap();
        assert_eq!(
            plan.source_outcome(),
            SourceRepairOutcome::Applied(RepairDisposition::Retargeted)
        );
        assert_eq!(plan.journal_effect(), JournalRepairEffect::NoChange);
        f.source
            .apply_planned_receipt_repair(plan, &journal, &f.group, 0)
            .unwrap();
        assert_eq!(f.source.opening.as_ref(), Some(&losing));
        assert_eq!(f.source.receipts.latest(), Some(&selected));
        assert!(f.source.repair_install_pending());
        f.reopen();
    }
}

#[test]
fn joint_repair_preflight_refuses_missing_close_bad_evidence_authority_and_sequence() {
    for mut f in fixtures() {
        let (selected, _) = f.journal_receipt(0, 2);
        let (losing, close) = f.journal_receipt(0, 3);
        let (other, wrong_close) = f.journal_receipt(0, 4);
        let journal = f.journal_at(&losing, true);
        let before_source = f.source.snapshot().unwrap();
        let before_journal = journal.encode();
        assert!(f.joint_plan(&selected, &losing, 1, &journal, None).is_err());
        assert!(f
            .joint_plan(&selected, &losing, 1, &journal, Some(&wrong_close))
            .is_err());
        let repair = f.decision(&selected, &losing, 1);
        assert!(f
            .source
            .prepare_receipt_repair(
                &repair,
                &selected,
                &other,
                &journal,
                Some(&close),
                &f.group,
                0
            )
            .is_err());
        assert!(f
            .source
            .prepare_receipt_repair(
                &repair,
                &selected,
                &losing,
                &journal,
                Some(&close),
                &f.group,
                1
            )
            .is_err());
        assert_eq!(f.source.snapshot().unwrap(), before_source);
        assert_eq!(journal.encode(), before_journal);
        // A local source high-water must fence issuance even with an empty owner journal.
        f.apply(&selected, &losing, 2);
        let before = f.source.snapshot().unwrap();
        assert!(f
            .joint_plan(&selected, &losing, 1, &OwnerReceiptJournal::default(), None)
            .is_err());
        assert_eq!(f.source.snapshot().unwrap(), before);
    }
}

#[test]
fn joint_repair_nochange_cannot_bypass_unfinished_journal_provenance() {
    for source_fault in [false, true] {
        for mut f in fixtures() {
            let (selected, _) = f.journal_receipt(0, 2);
            let (losing, _) = f.journal_receipt(0, 3);
            let (next_selected, _) = f.journal_receipt(1, 4);
            let (next_losing, _) = f.journal_receipt(1, 5);
            let repair = f.decision(&selected, &losing, 1);
            let mut journal = f.journal_at(&selected, false);
            journal
                .resolve_repair(&repair, &selected, &losing, None, &f.group, 0)
                .unwrap();
            // Normalized evidence-only provenance is unfinished even though H is not covered
            // by the new pair. Source may itself be healthy or faulted on that new pair.
            if source_fault {
                f.begin(&next_selected);
                f.begin(&next_losing);
            }
            let before_source = f.source.snapshot().unwrap();
            let before_journal = journal.encode();
            assert!(
                f.joint_plan(&next_selected, &next_losing, 2, &journal, None)
                    .is_err(),
                "unfinished journal must fence even NoChange"
            );
            assert_eq!(f.source.snapshot().unwrap(), before_source);
            assert_eq!(journal.encode(), before_journal);
            // Same transaction remains resumable despite independent source/journal effects.
            assert!(f.joint_plan(&selected, &losing, 1, &journal, None).is_ok());
            journal.mark_repair_source_finalized(repair.hash()).unwrap();
            assert!(f
                .joint_plan(&next_selected, &next_losing, 2, &journal, None)
                .is_ok());
        }
    }
}

#[test]
fn joint_repair_delayed_plan_rejects_source_journal_and_authority_changes() {
    for change in 0..4 {
        for mut f in fixtures() {
            let (selected, _) = f.target(0, 2);
            let (losing, _) = f.target(0, 3);
            let journal = OwnerReceiptJournal::default();
            let plan = f.joint_plan(&selected, &losing, 1, &journal, None).unwrap();
            assert!(plan.matches_original_journal(&journal));
            let mut candidate = plan.journal().clone();
            match change {
                0 => {
                    f.edit(1);
                }
                1 => {
                    f.source.seal(selected.clone(), &f.group, 0).unwrap();
                }
                2 => {
                    candidate.prepare(losing.clone(), &f.group, 0).unwrap();
                    assert!(!plan.matches_original_journal(&candidate));
                }
                3 => {}
                _ => unreachable!(),
            }
            let before = f.source.snapshot().unwrap();
            let journal_before = candidate.encode();
            let tenure = if change == 3 { 1 } else { 0 };
            assert!(
                f.source
                    .apply_planned_receipt_repair(plan, &candidate, &f.group, tenure)
                    .is_err(),
                "stale joint plan must refuse"
            );
            assert_eq!(f.source.snapshot().unwrap(), before);
            assert_eq!(candidate.encode(), journal_before);
        }
    }
}

#[test]
fn joint_repair_exact_retry_plan_is_fenced_after_later_edit() {
    for mut f in fixtures() {
        let (selected, _) = f.target(0, 2);
        let (losing, _) = f.target(0, 3);
        f.apply(&selected, &losing, 1);
        let journal = OwnerReceiptJournal::default();
        let plan = f.joint_plan(&selected, &losing, 1, &journal, None).unwrap();
        assert_eq!(
            plan.source_outcome(),
            SourceRepairOutcome::AlreadyResolved(RepairDisposition::Screened)
        );
        f.edit(1);
        let before = f.source.snapshot().unwrap();
        assert!(f
            .source
            .apply_planned_receipt_repair(plan, &journal, &f.group, 0)
            .is_err());
        assert_eq!(f.source.snapshot().unwrap(), before);
    }
}

#[test]
fn joint_repair_plan_refuses_real_owner_turnover_before_application() {
    for retry in [false, true] {
        for mut f in fixtures() {
            let (selected, _) = f.target(0, 2);
            let (losing, _) = f.target(0, 3);
            if retry {
                f.apply(&selected, &losing, 1);
            }
            let journal = OwnerReceiptJournal::default();
            let plan = f.joint_plan(&selected, &losing, 1, &journal, None).unwrap();
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
            let before = f.source.snapshot().unwrap();
            assert!(f
                .source
                .apply_planned_receipt_repair(plan, &journal, &group, group.epoch())
                .is_err());
            assert_eq!(f.source.snapshot().unwrap(), before);
        }
    }
}

#[test]
fn joint_repair_cross_tenure_roles_remain_historical_and_cannot_install() {
    for faulted in [false, true] {
        for mut f in fixtures() {
            let (selected, seed) = f.target(0, 2);
            let (losing, losing_seed) = f.target(0, 3);
            f.install(&losing, &losing_seed);
            if faulted {
                f.source.seal(selected.clone(), &f.group, 0).unwrap();
            }
            let journal = f.journal_at(&losing, false);
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
            let plan = f
                .source
                .prepare_receipt_repair(&repair, &selected, &losing, &journal, None, &group, tenure)
                .unwrap();
            assert_eq!(
                plan.source_outcome(),
                SourceRepairOutcome::Applied(if faulted {
                    RepairDisposition::Transitioned
                } else {
                    RepairDisposition::Screened
                })
            );
            assert_eq!(plan.journal_effect(), JournalRepairEffect::Replace);
            let candidate = plan.journal().clone();
            f.source
                .apply_planned_receipt_repair(plan, &candidate, &group, tenure)
                .unwrap();
            assert!(!f.source.repair_install_pending());
            assert!(f
                .source
                .prepare_repair_adoption(&selected, seed.bytes(), &group, tenure)
                .is_err());
            assert!(f
                .source
                .prepare_checkpoint_adoption(&selected, seed.bytes(), &group, tenure)
                .is_err());
        }
    }
}

#[test]
fn joint_repair_finalized_unpublished_proof_allows_only_newer_nochange() {
    for mut f in fixtures() {
        let (selected, _) = f.journal_receipt(0, 2);
        let (losing, _) = f.journal_receipt(0, 3);
        let (next_selected, _) = f.journal_receipt(1, 4);
        let (next_losing, _) = f.journal_receipt(1, 5);
        let repair = f.decision(&selected, &losing, 1);
        let mut journal = f.journal_at(&losing, false);
        journal
            .resolve_repair(&repair, &selected, &losing, None, &f.group, 0)
            .unwrap();
        journal.mark_repair_source_finalized(repair.hash()).unwrap();
        assert_eq!(journal.retained_repair(), Some(&repair));
        assert_eq!(journal.reconciled(), Some(&selected));
        let before = journal.encode();
        assert!(f
            .joint_plan(&next_selected, &next_losing, 1, &journal, None)
            .is_err());
        let plan = f
            .joint_plan(&next_selected, &next_losing, 2, &journal, None)
            .unwrap();
        assert_eq!(plan.journal_effect(), JournalRepairEffect::NoChange);
        assert_eq!(plan.journal().encode(), before);
        assert_eq!(journal.encode(), before);
    }
}
