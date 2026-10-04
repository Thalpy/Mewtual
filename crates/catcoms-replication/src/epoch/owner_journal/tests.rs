use super::*;

struct Fixture {
    owner: MlsDevice,
    group: ServerGroup,
    document: LogicalDocument,
}

fn baseline(epoch: u64, marker: u8) -> InheritedCheckpoint {
    InheritedCheckpoint::Checkpoint {
        epoch,
        close_record_hash: [marker; 32],
        seed_change_hash: [marker; 32],
    }
}

impl Fixture {
    fn new() -> Self {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let document =
            LogicalDocument::new(group.group_id(), DocType::StudioIndex, vec![7; 16]).unwrap();
        Self {
            owner,
            group,
            document,
        }
    }

    fn receipt(
        &self,
        epoch: u64,
        inherited: InheritedCheckpoint,
        marker: u8,
    ) -> (Receipt, CloseRecord) {
        self.receipt_in_tenure(epoch, inherited, marker, 0)
    }

    fn receipt_in_tenure(
        &self,
        epoch: u64,
        inherited: InheritedCheckpoint,
        marker: u8,
        start: u64,
    ) -> (Receipt, CloseRecord) {
        let close = CloseRecord::sign(
            &self.document,
            u128::from(marker),
            epoch,
            vec![[marker; 32]],
            &self.owner,
        )
        .unwrap();
        let receipt = Receipt::sign(
            self.document.clone(),
            epoch,
            close.hash(),
            [marker; 32],
            start,
            inherited,
            &self.owner,
        )
        .unwrap();
        (receipt, close)
    }

    fn repair(&self, winner: &Receipt, loser: &Receipt, sequence: u64) -> ReceiptRepair {
        ReceiptRepair::sign_in_tenure(
            self.document.clone(),
            winner.tenure_id,
            [winner.hash(), loser.hash()],
            winner.hash(),
            sequence,
            0,
            &self.owner,
        )
        .unwrap()
    }

    fn resolve(
        &self,
        journal: &mut OwnerReceiptJournal,
        winner: &Receipt,
        loser: &Receipt,
        close: Option<&CloseRecord>,
    ) -> Result<JournalRepairEffect, ReplError> {
        journal.resolve_repair(
            &self.repair(winner, loser, 1),
            winner,
            loser,
            close,
            &self.group,
            0,
        )
    }

    fn published(&self, receipt: &Receipt) -> OwnerReceiptJournal {
        let mut j = OwnerReceiptJournal::default();
        j.prepare(receipt.clone(), &self.group, 0).unwrap();
        j.mark_published(receipt.hash()).unwrap();
        reopen(&mut j);
        j
    }
}

fn reopen(journal: &mut OwnerReceiptJournal) {
    let bytes = journal.encode();
    *journal = OwnerReceiptJournal::decode(&bytes)
        .expect("journal must restore at every lifecycle boundary");
    assert_eq!(journal.encode(), bytes);
    assert!(bytes.len() <= MAX_OWNER_RECEIPT_JOURNAL_BYTES);
}

fn unchanged<T>(
    j: &mut OwnerReceiptJournal,
    action: impl FnOnce(&mut OwnerReceiptJournal) -> Result<T, ReplError>,
) {
    let before = j.encode();
    assert!(action(j).is_err(), "operation must refuse");
    assert_eq!(
        j.encode(),
        before,
        "refusal must preserve all roles and evidence"
    );
}

#[test]
fn v1_codec_and_pending_retry_contract_are_preserved() {
    // Independently encoded legacy fixture: fixed public key/signature bytes are intentionally
    // opaque vault-local history. This golden does not invoke any current encoding helper.
    let hex: String = include_str!("v1-golden.hex").split_whitespace().collect();
    let golden: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let restored = OwnerReceiptJournal::decode(&golden).unwrap();
    assert_eq!(restored.encode(), golden);
    assert_eq!(restored.published().unwrap().closed_epoch, 0);
    assert_eq!(restored.in_flight().unwrap().closed_epoch, 1);
    let f = Fixture::new();
    assert_eq!(OwnerReceiptJournal::default().encode(), [1, 0, 0, 0]);
    let (first, _) = f.receipt(2, baseline(2, 1), 2);
    let (next, _) = f.receipt(3, first.inherited.clone(), 3);
    let mut j = f.published(&first);
    j.prepare(next.clone(), &f.group, 0).unwrap();
    // Frozen pre-C7 v1 layout, deliberately independent of OwnerReceiptJournal::encode.
    let mut legacy = Encoder::new();
    legacy.put_u8(1);
    put_receipt(&mut legacy, Some(&first));
    put_receipt(&mut legacy, Some(&next));
    put_tenure(&mut legacy, Some(&TenureSelection::from(&first)));
    let bytes = legacy.finish();
    assert_eq!(j.encode(), bytes);
    j = OwnerReceiptJournal::decode(&bytes).unwrap();
    assert_eq!(j.encode(), bytes);
    unchanged(&mut j, |j| j.prepare(first.clone(), &f.group, 0));
    j.prepare(next.clone(), &f.group, 0).unwrap();
    j.mark_published(first.hash()).unwrap();
    assert_eq!(j.in_flight(), Some(&next));
    j.mark_published(next.hash()).unwrap();
    reopen(&mut j);

    // Legacy vault restore intentionally never acquired a new signature-authentication rule.
    let mut opaque = first.clone();
    opaque.signature[0] ^= 1;
    let mut legacy = Encoder::new();
    legacy.put_u8(1);
    put_receipt(&mut legacy, Some(&opaque));
    put_receipt(&mut legacy, None);
    put_tenure(&mut legacy, Some(&TenureSelection::from(&opaque)));
    let bytes = legacy.finish();
    assert_eq!(OwnerReceiptJournal::decode(&bytes).unwrap().encode(), bytes);
    assert!(OwnerReceiptJournal::decode(&vec![1; MAX_V1_BYTES + 1]).is_err());
}

#[test]
fn ordinary_prepare_cannot_change_inheritance_or_skip_adjacency() {
    let f = Fixture::new();
    let (first, _) = f.receipt(2, baseline(2, 1), 2);
    let mut j = f.published(&first);
    for (epoch, inherited) in [
        (3, baseline(2, 9)),
        (4, first.inherited.clone()),
        (2, first.inherited.clone()),
    ] {
        let (bad, _) = f.receipt(epoch, inherited, 9);
        unchanged(&mut j, |j| j.prepare(bad, &f.group, 0));
    }
}

#[test]
fn rewind_retires_pending_with_full_close_and_rejects_stale_callbacks() {
    let f = Fixture::new();
    let (old, _) = f.receipt(4, baseline(4, 1), 4);
    let (pending, close) = f.receipt(5, old.inherited.clone(), 5);
    let (winner, _) = f.receipt(2, baseline(2, 2), 2);
    let mut j = f.published(&old);
    j.prepare(pending.clone(), &f.group, 0).unwrap();
    reopen(&mut j);
    assert_eq!(
        f.resolve(&mut j, &winner, &old, Some(&close)).unwrap(),
        JournalRepairEffect::RetirePendingAndReplace
    );
    assert_eq!(j.published(), Some(&old));
    assert_eq!(j.canonical_head(), Some(&winner));
    assert_eq!(j.effective_choice(), Some(&winner));
    assert_eq!(j.retired_pending(), Some((&pending, &close)));
    assert!(j.in_flight().is_none());
    reopen(&mut j);
    unchanged(&mut j, |j| j.mark_published(pending.hash()));
    let before = j.encode();
    j.mark_published(old.hash()).unwrap();
    assert_eq!(
        j.encode(),
        before,
        "historical completion retry cannot override repaired canonical choice"
    );
    j.mark_published(winner.hash()).unwrap();
    assert_eq!(j.published(), Some(&winner));
    assert!(j.reconciled().is_none());
    assert!(j.retired_pending().is_some());
    reopen(&mut j);
    let (next, _) = f.receipt(3, winner.inherited.clone(), 3);
    unchanged(&mut j, |j| j.prepare(next.clone(), &f.group, 0));
    unchanged(&mut j, |j| j.mark_published(next.hash()));
    unchanged(&mut j, |j| j.mark_repair_source_finalized([99; 32]));
    j.mark_repair_source_finalized(f.repair(&winner, &old, 1).hash())
        .unwrap();
    assert!(j.retained_repair().is_none());
    reopen(&mut j);
    // Two successive decisions demonstrate that cleanup releases the bounded evidence hold.
    for epoch in 3..=4 {
        let (next, _) = f.receipt(epoch, winner.inherited.clone(), epoch as u8);
        j.prepare(next.clone(), &f.group, 0).unwrap();
        reopen(&mut j);
        j.mark_published(next.hash()).unwrap();
        reopen(&mut j);
    }
}

#[test]
fn pending_retirement_requires_exact_canonical_signed_close_before_mutation() {
    let f = Fixture::new();
    let (loser, close) = f.receipt(2, baseline(2, 1), 2);
    let (winner, _) = f.receipt(2, baseline(2, 1), 3);
    let mut j = OwnerReceiptJournal::default();
    j.prepare(loser.clone(), &f.group, 0).unwrap();
    unchanged(&mut j, |j| f.resolve(j, &winner, &loser, None));
    for mutation in 0..7 {
        let mut bad = close.clone();
        match mutation {
            0 => bad.signature[0] ^= 1,
            1 => bad.doc_id += 1,
            2 => bad.closed_epoch += 1,
            3 => bad.server_id.push(0),
            4 => bad.heads.push(bad.heads[0]),
            5 => bad.author_public_key.push(0),
            _ => bad.heads = vec![[0; 32]; MAX_HEADS + 1],
        }
        unchanged(&mut j, |j| f.resolve(j, &winner, &loser, Some(&bad)));
    }
    assert_eq!(
        f.resolve(&mut j, &winner, &loser, Some(&close)).unwrap(),
        JournalRepairEffect::RetirePendingAndReplace
    );
    assert_eq!(j.retired_pending(), Some((&loser, &close)));
    reopen(&mut j);
}

#[test]
fn source_finalization_before_publication_keeps_proof_and_blocks_tenure_reset() {
    let f = Fixture::new();
    let (loser, _) = f.receipt(4, baseline(4, 1), 4);
    let (winner, _) = f.receipt(2, baseline(2, 2), 2);
    let mut j = f.published(&loser);
    assert_eq!(
        f.resolve(&mut j, &winner, &loser, None).unwrap(),
        JournalRepairEffect::Replace
    );
    let repair = f.repair(&winner, &loser, 1);
    j.mark_repair_source_finalized(repair.hash()).unwrap();
    reopen(&mut j);
    j.mark_repair_source_finalized(repair.hash()).unwrap();
    assert!(j.retained_repair().is_some());
    let (new_tenure, _) = f.receipt_in_tenure(0, InheritedCheckpoint::EpochZero, 9, 1);
    // The authority-independent private seam isolates the evidence fence from MLS rotation.
    unchanged(&mut j, |j| j.prepare_verified(new_tenure.clone()));
    j.mark_published(winner.hash()).unwrap();
    assert!(j.retained_repair().is_none());
    reopen(&mut j);
    unchanged(&mut j, |j| j.mark_repair_source_finalized(repair.hash()));
    j.prepare_verified(new_tenure.clone()).unwrap();
    assert_eq!(j.in_flight(), Some(&new_tenure));
    assert!(j.published().is_none());
    reopen(&mut j);
}

#[test]
fn pending_successor_outranks_reconciliation_and_leaves_only_one_step_evidence() {
    let f = Fixture::new();
    let (loser, _) = f.receipt(4, baseline(4, 1), 4);
    let (winner, _) = f.receipt(2, baseline(2, 2), 2);
    let mut j = f.published(&loser);
    f.resolve(&mut j, &winner, &loser, None).unwrap();
    let (next, _) = f.receipt(3, winner.inherited.clone(), 3);
    let (wrong_selection, _) = f.receipt(3, baseline(2, 8), 8);
    unchanged(&mut j, |j| j.prepare(wrong_selection, &f.group, 0));
    j.prepare(next.clone(), &f.group, 0).unwrap();
    assert_eq!(j.effective_choice(), Some(&next));
    assert_eq!(j.canonical_head(), Some(&winner));
    reopen(&mut j);
    unchanged(&mut j, |j| j.mark_published(winner.hash()));
    j.mark_published(next.hash()).unwrap();
    assert_eq!(j.published(), Some(&next));
    reopen(&mut j);
    let mut impossible = j.clone();
    impossible.high_water = Some(f.receipt(4, winner.inherited.clone(), 4).0);
    assert!(
        OwnerReceiptJournal::decode(&impossible.encode()).is_err(),
        "same baseline is not arbitrary ancestry evidence"
    );
    j.mark_repair_source_finalized(f.repair(&winner, &loser, 1).hash())
        .unwrap();
    reopen(&mut j);
}

#[test]
fn reselected_historical_publication_completes_its_new_pending_obligation() {
    for finalize_first in [false, true] {
        let f = Fixture::new();
        let (h2, _) = f.receipt(2, baseline(2, 1), 2);
        let (h3, _) = f.receipt(3, h2.inherited.clone(), 3);
        let (h4, _) = f.receipt(4, h2.inherited.clone(), 4);
        let mut j = f.published(&h2);
        for r in [&h3, &h4] {
            j.prepare(r.clone(), &f.group, 0).unwrap();
            j.mark_published(r.hash()).unwrap();
            reopen(&mut j);
        }
        let (alternate, _) = f.receipt(2, baseline(2, 9), 9);
        f.resolve(&mut j, &alternate, &h4, None).unwrap();
        j.mark_repair_source_finalized(f.repair(&alternate, &h4, 1).hash())
            .unwrap();
        reopen(&mut j);
        let repair = f.repair(&h3, &alternate, 2);
        j.resolve_repair(&repair, &h3, &alternate, None, &f.group, 0)
            .unwrap();
        reopen(&mut j);
        if finalize_first {
            j.mark_repair_source_finalized(repair.hash()).unwrap();
            reopen(&mut j);
        }
        j.prepare(h4.clone(), &f.group, 0).unwrap();
        assert_eq!(j.published(), j.in_flight());
        reopen(&mut j);
        j.mark_published(h4.hash()).unwrap();
        assert!(j.in_flight().is_none(), "active reselected publication must complete even when its hash equals historical high-water");
        assert!(j.reconciled().is_none());
        reopen(&mut j);
        if !finalize_first {
            j.mark_repair_source_finalized(repair.hash()).unwrap();
            reopen(&mut j);
        }
        let (h5, _) = f.receipt(5, h2.inherited.clone(), 5);
        j.prepare(h5.clone(), &f.group, 0).unwrap();
        j.mark_published(h5.hash()).unwrap();
        reopen(&mut j);
    }
}

#[test]
fn repeated_repairs_require_prior_source_barrier_and_replace_only_current_choice() {
    let f = Fixture::new();
    let (old, _) = f.receipt(4, baseline(4, 1), 4);
    let (middle, _) = f.receipt(2, baseline(2, 2), 2);
    let (winner, _) = f.receipt(3, baseline(3, 3), 3);
    let mut j = f.published(&old);
    f.resolve(&mut j, &middle, &old, None).unwrap();
    let first = f.repair(&middle, &old, 1);
    let second = f.repair(&winner, &middle, 2);
    unchanged(&mut j, |j| {
        j.resolve_repair(&second, &winner, &middle, None, &f.group, 0)
    });
    j.mark_repair_source_finalized(first.hash()).unwrap();
    reopen(&mut j);
    let before = j.encode();
    assert_eq!(
        f.resolve(&mut j, &middle, &old, None).unwrap(),
        JournalRepairEffect::NoChange
    );
    assert_eq!(
        j.encode(),
        before,
        "exact retry must not reset finalization"
    );
    let stale = f.repair(&winner, &middle, 1);
    unchanged(&mut j, |j| {
        j.resolve_repair(&stale, &winner, &middle, None, &f.group, 0)
    });
    assert_eq!(
        j.resolve_repair(&second, &winner, &middle, None, &f.group, 0)
            .unwrap(),
        JournalRepairEffect::Replace
    );
    assert_eq!(j.published(), Some(&old));
    assert_eq!(j.reconciled(), Some(&winner));
    assert!(!j.provenance.as_ref().unwrap().source_finalized);
    reopen(&mut j);
    unchanged(&mut j, |j| j.mark_repair_source_finalized(first.hash()));
    let before = j.encode();
    assert_eq!(
        f.resolve(&mut j, &middle, &old, None).unwrap(),
        JournalRepairEffect::NoChange
    );
    assert_eq!(
        j.encode(),
        before,
        "historical-only repair cannot overwrite a different canonical choice"
    );
    j.mark_published(winner.hash()).unwrap();
    j.mark_repair_source_finalized(second.hash()).unwrap();
    reopen(&mut j);
}

#[test]
fn already_published_winner_normalizes_without_phantom_reconciliation() {
    let f = Fixture::new();
    let (winner, _) = f.receipt(2, baseline(2, 1), 2);
    let (loser, _) = f.receipt(3, baseline(2, 2), 3);
    let mut j = f.published(&winner);
    assert_eq!(
        f.resolve(&mut j, &winner, &loser, None).unwrap(),
        JournalRepairEffect::Normalize
    );
    assert!(j.reconciled().is_none());
    assert_eq!(j.published(), Some(&winner));
    assert!(j.retained_repair().is_some());
    reopen(&mut j);
    j.mark_repair_source_finalized(f.repair(&winner, &loser, 1).hash())
        .unwrap();
    reopen(&mut j);
}

#[test]
fn returning_to_published_winner_normalizes_and_retains_pending_evidence() {
    let f = Fixture::new();
    let (winner, _) = f.receipt(2, baseline(2, 1), 2);
    let (middle, _) = f.receipt(4, baseline(4, 2), 4);
    let mut j = f.published(&winner);
    f.resolve(&mut j, &middle, &winner, None).unwrap();
    j.mark_repair_source_finalized(f.repair(&middle, &winner, 1).hash())
        .unwrap();
    let (pending, close) = f.receipt(5, middle.inherited.clone(), 5);
    j.prepare(pending.clone(), &f.group, 0).unwrap();
    reopen(&mut j);
    let repair = f.repair(&winner, &middle, 2);
    assert_eq!(
        j.resolve_repair(&repair, &winner, &middle, Some(&close), &f.group, 0)
            .unwrap(),
        JournalRepairEffect::Normalize
    );
    assert!(j.reconciled().is_none());
    assert!(j.in_flight().is_none());
    assert_eq!(j.published(), Some(&winner));
    assert_eq!(j.retired_pending(), Some((&pending, &close)));
    reopen(&mut j);
    unchanged(&mut j, |j| j.mark_published(pending.hash()));
    j.mark_repair_source_finalized(repair.hash()).unwrap();
    reopen(&mut j);
}

#[test]
fn unrelated_higher_pending_and_historical_tenure_repairs_preserve_every_role() {
    let f = Fixture::new();
    let (old, _) = f.receipt(2, baseline(2, 1), 2);
    let (winner, _) = f.receipt(2, old.inherited.clone(), 8);
    let (higher, _) = f.receipt(3, old.inherited.clone(), 3);
    let mut j = f.published(&old);
    j.prepare(higher.clone(), &f.group, 0).unwrap();
    let before = j.encode();
    assert_eq!(
        f.resolve(&mut j, &winner, &old, None).unwrap(),
        JournalRepairEffect::NoChange
    );
    assert_eq!(
        j.encode(),
        before,
        "same-baseline unknown ancestry must preserve pending work"
    );
    j.mark_published(higher.hash()).unwrap();
    let before = j.encode();
    assert_eq!(
        f.resolve(&mut j, &winner, &old, None).unwrap(),
        JournalRepairEffect::NoChange
    );
    assert_eq!(j.encode(), before);
    let (returning, _) = f.receipt_in_tenure(0, InheritedCheckpoint::EpochZero, 7, 7);
    j.prepare_verified(returning.clone()).unwrap();
    for published in [false, true] {
        if published {
            j.mark_published(returning.hash()).unwrap();
        }
        let before = j.encode();
        assert_eq!(
            f.resolve(&mut j, &winner, &old, None).unwrap(),
            JournalRepairEffect::NoChange
        );
        assert_eq!(
            j.encode(),
            before,
            "historical pair must not require unrelated current journal identity"
        );
    }
}

#[test]
fn live_authority_and_complete_evidence_precede_noop_and_exact_retry() {
    let f = Fixture::new();
    let (old, _) = f.receipt(2, baseline(2, 1), 2);
    let (winner, _) = f.receipt(2, old.inherited.clone(), 8);
    let mut j = f.published(&old);
    f.resolve(&mut j, &winner, &old, None).unwrap();
    let repair = f.repair(&winner, &old, 1);
    unchanged(&mut j, |j| {
        j.resolve_repair(&repair, &winner, &old, None, &f.group, 1)
    });
    let mut bad = old.clone();
    bad.signature[0] ^= 1;
    unchanged(&mut j, |j| {
        j.resolve_repair(&repair, &winner, &bad, None, &f.group, 0)
    });
    let mut bad_repair = repair.clone();
    bad_repair.signature[0] ^= 1;
    unchanged(&mut j, |j| {
        j.resolve_repair(&bad_repair, &winner, &old, None, &f.group, 0)
    });
    let mut empty = OwnerReceiptJournal::default();
    unchanged(&mut empty, |j| {
        j.resolve_repair(&repair, &winner, &old, None, &f.group, 1)
    });
    assert_eq!(
        f.resolve(&mut empty, &winner, &old, None).unwrap(),
        JournalRepairEffect::NoChange
    );
    assert_eq!(empty.encode(), [1, 0, 0, 0]);
}

#[test]
fn strict_restore_rejects_mixed_roles_identity_selection_and_retired_evidence() {
    let f = Fixture::new();
    let (old, _) = f.receipt(4, baseline(4, 1), 4);
    let (pending, close) = f.receipt(5, old.inherited.clone(), 5);
    let (winner, _) = f.receipt(2, baseline(2, 2), 2);
    let mut j = f.published(&old);
    j.prepare(pending.clone(), &f.group, 0).unwrap();
    f.resolve(&mut j, &winner, &old, Some(&close)).unwrap();
    for mutation in 0..13 {
        let mut bad = j.clone();
        match mutation {
            0 => bad.tenure.as_mut().unwrap().key[0] ^= 1,
            1 => bad.tenure.as_mut().unwrap().start += 1,
            2 => bad.tenure.as_mut().unwrap().id[0] ^= 1,
            3 => bad.reconciled = Some(old.clone()),
            4 => bad.provenance = None,
            5 => bad.provenance.as_mut().unwrap().resolved.repair.signature[0] ^= 1,
            6 => bad.provenance.as_mut().unwrap().resolved.selected = old.clone(),
            7 => {
                bad.provenance
                    .as_mut()
                    .unwrap()
                    .retired
                    .as_mut()
                    .unwrap()
                    .1
                    .signature[0] ^= 1
            }
            8 => bad.provenance.as_mut().unwrap().retired.as_mut().unwrap().0 = winner.clone(),
            9 => bad.high_water.as_mut().unwrap().signature[0] ^= 1,
            10 => bad.in_flight = Some(f.receipt(3, baseline(2, 8), 8).0),
            11 => {
                bad.high_water = Some(
                    f.receipt_in_tenure(0, InheritedCheckpoint::EpochZero, 1, 1)
                        .0,
                )
            }
            _ => {
                let mut other_doc = Fixture::new();
                other_doc.document.server_id = f.document.server_id.clone();
                bad.high_water = Some(other_doc.receipt(0, InheritedCheckpoint::EpochZero, 0).0);
            }
        }
        assert!(
            OwnerReceiptJournal::decode(&bad.encode()).is_err(),
            "malformed journal mutation {mutation} restored"
        );
    }
    let bytes = j.encode();
    for cut in 0..bytes.len() {
        assert!(OwnerReceiptJournal::decode(&bytes[..cut]).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(OwnerReceiptJournal::decode(&trailing).is_err());
    let mut unknown = bytes;
    unknown[0] = 3;
    assert!(OwnerReceiptJournal::decode(&unknown).is_err());
    assert!(OwnerReceiptJournal::decode(&vec![2; MAX_OWNER_RECEIPT_JOURNAL_BYTES + 1]).is_err());
    j.mark_published(winner.hash()).unwrap();
    let mut bad = j.clone();
    bad.provenance.as_mut().unwrap().source_finalized = true;
    assert!(
        OwnerReceiptJournal::decode(&bad.encode()).is_err(),
        "already-cleaned evidence-only role is not canonical"
    );
}

#[test]
fn published_winner_can_precede_equal_or_follow_historical_epoch() {
    for epoch in [1, 4, 8] {
        let f = Fixture::new();
        let (old, _) = f.receipt(4, baseline(4, 1), 4);
        let (winner, _) = f.receipt(epoch, baseline(epoch, 2), 2);
        let mut j = f.published(&old);
        f.resolve(&mut j, &winner, &old, None).unwrap();
        reopen(&mut j);
        j.mark_published(winner.hash()).unwrap();
        assert_eq!(j.published(), Some(&winner));
        reopen(&mut j);
    }
}

#[test]
fn version_two_bound_covers_all_receipt_roles_and_maximum_close_heads() {
    let f = Fixture::new();
    let (old, _) = f.receipt(4, baseline(4, 1), 4);
    let (winner, _) = f.receipt(2, baseline(2, 2), 2);
    let close = CloseRecord::sign(
        &f.document,
        123,
        5,
        (0..MAX_HEADS).map(|i| [i as u8; 32]).collect(),
        &f.owner,
    )
    .unwrap();
    let pending = Receipt::sign(
        f.document.clone(),
        5,
        close.hash(),
        [5; 32],
        0,
        old.inherited.clone(),
        &f.owner,
    )
    .unwrap();
    let mut j = f.published(&old);
    j.prepare(pending, &f.group, 0).unwrap();
    f.resolve(&mut j, &winner, &old, Some(&close)).unwrap();
    let (next, _) = f.receipt(3, winner.inherited.clone(), 3);
    j.prepare(next, &f.group, 0).unwrap();
    assert!(j.encode().len() < MAX_OWNER_RECEIPT_JOURNAL_BYTES);
    assert_eq!(MAX_OWNER_RECEIPT_JOURNAL_BYTES, 12 * 1024);
    reopen(&mut j);
}
