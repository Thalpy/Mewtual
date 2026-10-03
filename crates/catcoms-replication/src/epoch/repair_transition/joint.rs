//! Joint preflight for the independently computed source and owner-journal candidates. This
//! contains no IO: the application still owns historical admission, custody and the persisted
//! repair claim, including when the journal is unchanged. Never infer those facts from a plan.

use super::*;

/// Immutable computation for one exact source version and one exact journal candidate.
///
/// Persist the journal candidate together with the admitted repair and target claim at B1;
/// only then apply the source candidate and persist the complete source at B2. Neither this
/// value nor its commit method attests that either write happened. Restart must reconstruct
/// a plan under the still-held transaction, repeating historical admission and live checks.
pub struct ReceiptRepairPlan {
    resolved: ResolvedRepair,
    source_version: Hash32,
    source: RepairPlan,
    expected: RepairStateStamp,
    original_journal: Vec<u8>,
    journal: OwnerReceiptJournal,
    journal_effect: JournalRepairEffect,
}

impl std::fmt::Debug for ReceiptRepairPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceiptRepairPlan")
            .field("source_outcome", &self.source_outcome())
            .field("journal_effect", &self.journal_effect)
            .finish_non_exhaustive()
    }
}

impl ReceiptRepairPlan {
    /// Signed decision, not historical owner admission or a current authority lease.
    pub fn repair(&self) -> &ReceiptRepair {
        &self.resolved.repair
    }

    /// Candidate to include in the atomic B1 owner record, even for `NoChange`.
    pub fn journal(&self) -> &OwnerReceiptJournal {
        &self.journal
    }

    /// Compare the actual journal BEFORE B1 under exclusive custody. A stale candidate must
    /// never overwrite a newly published/pending decision. This comparison is not an atomic
    /// storage CAS; keep custody through the write and check the enclosing owner record too.
    pub fn matches_original_journal(&self, current: &OwnerReceiptJournal) -> bool {
        current.encode() == self.original_journal
    }

    /// Independent journal action; it need not equal the source's action or selected head.
    pub fn journal_effect(&self) -> JournalRepairEffect {
        self.journal_effect
    }

    /// Expected in-memory source result. This is not durable completion or delivery.
    pub fn source_outcome(&self) -> SourceRepairOutcome {
        match &self.source {
            RepairPlan::Candidate(candidate) => {
                SourceRepairOutcome::Applied(candidate.binding.disposition)
            }
            RepairPlan::Unchanged(outcome) => *outcome,
        }
    }

    /// Local full-restart-unit fingerprint, not a wire hash or proof of custody/durability.
    pub(crate) fn source_version(snapshot: &[u8]) -> Hash32 {
        let mut hash = blake3::Hasher::new_derive_key("catcoms/joint-repair-source/v1");
        hash.update(snapshot);
        *hash.finalize().as_bytes()
    }
}

impl RepairSource<'_> {
    // The typed wrapper fingerprints its complete document as well as these exact private
    // protocol roles. The gate stamp is rechecked while locked at commit, closing a gate-only
    // race between serializing the source and applying its candidate.
    fn stamp(&self) -> Result<RepairStateStamp, ReplError> {
        Ok(RepairStateStamp {
            gate: self.gate.inner.lock().expect("epoch gate poisoned").clone(),
            book: self.book_bytes()?,
            adopting: *self.adopting,
            binding: *self.binding,
            opening: self.opening.cloned(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn plan_joint(
        &self,
        repair: &ReceiptRepair,
        a: &Receipt,
        b: &Receipt,
        journal: &OwnerReceiptJournal,
        retiring_close: Option<&CloseRecord>,
        source_version: Hash32,
        group: &ServerGroup,
        issuer_tenure: u64,
    ) -> Result<ReceiptRepairPlan, ReplError> {
        let expected = self.stamp()?;
        let source = self.plan(repair, a, b, group, issuer_tenure)?;
        if matches!(source, RepairPlan::Unchanged(SourceRepairOutcome::Held(_))) {
            return Err(ReplError::ReceiptConflict);
        }
        // NoChange must not bypass the OTHER transaction's nonterminal provenance or its
        // sequence. The leaf's role reconciliation intentionally leaves those to its caller.
        journal.check_repair_progress(repair)?;
        let mut next_journal = journal.clone();
        let journal_effect =
            next_journal.resolve_repair(repair, a, b, retiring_close, group, issuer_tenure)?;
        let (selected, losing) = if a.hash() == repair.selected_receipt_hash {
            (a, b)
        } else {
            (b, a)
        };
        let resolved = ResolvedRepair {
            repair: repair.clone(),
            selected: selected.clone(),
            losing: losing.clone(),
        };
        compatible_journal(&next_journal, &resolved)?;
        let (book, gate, adopting, binding) = match &source {
            RepairPlan::Candidate(candidate) => (
                &candidate.book,
                &candidate.gate,
                candidate.adopting,
                candidate.binding,
            ),
            RepairPlan::Unchanged(_) => (
                &*self.book,
                &expected.gate,
                *self.adopting,
                self.binding.ok_or(ReplError::Malformed)?,
            ),
        };
        compatible_source(book, gate, self.opening, adopting, binding, &resolved)?;
        Ok(ReceiptRepairPlan {
            resolved,
            source_version,
            source,
            expected,
            original_journal: journal.encode(),
            journal: next_journal,
            journal_effect,
        })
    }

    pub(crate) fn commit_joint(
        &mut self,
        plan: ReceiptRepairPlan,
        current_journal: &OwnerReceiptJournal,
        source_version: Hash32,
        group: &ServerGroup,
        issuer_tenure: u64,
    ) -> Result<SourceRepairOutcome, ReplError> {
        // Delayed plans and exact retries are not authority leases. This check must precede
        // every successful no-op; the caller independently rechecks custody/transaction claim.
        plan.repair().verify_current_owner(group, issuer_tenure)?;
        plan.repair()
            .check_evidence(&plan.resolved.selected, &plan.resolved.losing)?;
        if plan.repair().document != *self.document
            || source_version != plan.source_version
            || current_journal.encode() != plan.journal.encode()
            || self.book_bytes()? != plan.expected.book
            || *self.adopting != plan.expected.adopting
            || *self.binding != plan.expected.binding
            || self.opening != plan.expected.opening.as_ref()
        {
            return Err(ReplError::ReceiptConflict);
        }
        match plan.source {
            RepairPlan::Candidate(candidate) => self.commit(*candidate),
            RepairPlan::Unchanged(outcome) => {
                // Even an exact retry fences gate changes after the full snapshot comparison.
                self.gate.commit_repair(
                    &plan.expected.gate,
                    &plan.expected.gate,
                    RepairDisposition::Screened,
                    || {},
                )?;
                Ok(outcome)
            }
        }
    }
}

fn compatible_journal(
    journal: &OwnerReceiptJournal,
    resolved: &ResolvedRepair,
) -> Result<(), ReplError> {
    // Decode before exposing ANY candidate, including a v1/NoChange journal which bypassed
    // the repair leaf's v2 validation. Corruption is a refusal, never a reset to an empty role.
    OwnerReceiptJournal::decode(&journal.encode())?;
    if journal
        .effective_choice()
        .is_some_and(|r| resolved.covers(r))
    {
        return Err(ReplError::ReceiptConflict);
    }
    Ok(())
}

fn compatible_source(
    book: &ReceiptBook,
    gate: &EpochGateInner,
    opening: Option<&Receipt>,
    adopting: bool,
    binding: RepairBinding,
    resolved: &ResolvedRepair,
) -> Result<(), ReplError> {
    if binding.hash != resolved.repair.hash()
        || book.latest_repair() != Some(&resolved.repair)
        || match gate.phase {
            EpochPhase::Open => gate.receipt_hash.is_some() || adopting,
            EpochPhase::Closing => {
                gate.receipt_hash.is_none() || gate.receipt_hash != book.latest().map(Receipt::hash)
            }
            EpochPhase::Fault => false,
            EpochPhase::Settled => true,
        }
    {
        return Err(ReplError::ReceiptConflict);
    }
    if (gate.phase == EpochPhase::Fault) != book.is_faulted() {
        return Err(ReplError::Malformed);
    }
    // An unrelated fault blocks all proof/install/settlement. Old-tenure roles are historical
    // only: existing typed continuations always verify current receipt authority again.
    if book.is_faulted() || !same_issuer_tenure(resolved) {
        return Ok(());
    }
    if book.latest().is_some_and(|r| resolved.covers(r)) {
        return Err(ReplError::ReceiptConflict);
    }
    if opening
        .into_iter()
        .chain(book.previous_until_installed.iter())
        .any(|r| resolved.covers(r))
        && !binding
            .state(book, gate.phase, opening, adopting)
            .is_some_and(|state| state.install_pending)
    {
        // Losing installed material is permitted only behind the concrete repair adoption
        // fence, which saves whole-source recovery and forbids ordinary settlement.
        return Err(ReplError::ReceiptConflict);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joint_compatibility_rejects_covered_roles_and_inconsistent_gate_binding() {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let document =
            LogicalDocument::new(group.group_id(), DocType::StudioIndex, vec![7; 16]).unwrap();
        let sign = |salt| {
            Receipt::sign(
                document.clone(),
                0,
                [salt; 32],
                [salt; 32],
                0,
                InheritedCheckpoint::EpochZero,
                &owner,
            )
            .unwrap()
        };
        let selected = sign(1);
        let losing = sign(2);
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
        let resolved = ResolvedRepair {
            repair: repair.clone(),
            selected: selected.clone(),
            losing: losing.clone(),
        };
        let mut journal = OwnerReceiptJournal::default();
        journal.prepare(losing.clone(), &group, 0).unwrap();
        assert!(
            compatible_journal(&journal, &resolved).is_err(),
            "covered journal choice must refuse"
        );
        let gate = EpochGate::new(document, 1, 0, owner.device_id());
        let binding = RepairBinding {
            hash: repair.hash(),
            disposition: RepairDisposition::Retargeted,
        };
        let book = ReceiptBook {
            resolved_repair: Some(resolved.clone()),
            repair_sequence: 1,
            latest: Some(selected.clone()),
            ..ReceiptBook::default()
        };
        let mut inner = gate.inner.lock().unwrap().clone();
        inner.phase = EpochPhase::Closing;
        inner.receipt_hash = Some(selected.hash());
        assert!(compatible_source(&book, &inner, Some(&losing), true, binding, &resolved).is_ok());
        for change in 0..7 {
            let mut candidate = book.clone();
            let mut candidate_gate = inner.clone();
            let mut candidate_binding = binding;
            let mut opening = None;
            match change {
                0 => {
                    candidate.latest = Some(losing.clone());
                    candidate_gate.receipt_hash = Some(losing.hash());
                }
                1 => opening = Some(&losing),
                2 => candidate.previous_until_installed = Some(losing.clone()),
                3 => candidate_gate.receipt_hash = Some(losing.hash()),
                4 => candidate_gate.phase = EpochPhase::Fault,
                5 => candidate_binding.hash = [0; 32],
                6 => candidate.resolved_repair = None,
                _ => unreachable!(),
            }
            assert!(
                compatible_source(
                    &candidate,
                    &candidate_gate,
                    opening,
                    false,
                    candidate_binding,
                    &resolved
                )
                .is_err(),
                "unsafe source role {change} must refuse"
            );
        }
    }
    #[test]
    fn joint_commit_fences_gate_races_even_after_fingerprint_comparison_and_on_retry() {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let document =
            LogicalDocument::new(group.group_id(), DocType::StudioIndex, vec![7; 16]).unwrap();
        let sign = |salt| {
            Receipt::sign(
                document.clone(),
                0,
                [salt; 32],
                [salt; 32],
                0,
                InheritedCheckpoint::EpochZero,
                &owner,
            )
            .unwrap()
        };
        let selected = sign(1);
        let losing = sign(2);
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
        for retry in [false, true] {
            let gate = EpochGate::new(document.clone(), 1, 0, owner.device_id());
            let mut book = ReceiptBook::default();
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
            if retry {
                source
                    .apply(&repair, &selected, &losing, &group, 0)
                    .unwrap();
            }
            let journal = OwnerReceiptJournal::default();
            let version = [44; 32];
            let stale = source
                .plan_joint(
                    &repair, &selected, &losing, &journal, None, version, &group, 0,
                )
                .unwrap();
            let before_book = source.book_bytes().unwrap();
            let before_gate = gate.encode().unwrap();
            assert!(
                source
                    .commit_joint(stale, &journal, [45; 32], &group, 0)
                    .is_err(),
                "source fingerprint mismatch must refuse"
            );
            assert_eq!(source.book_bytes().unwrap(), before_book);
            assert_eq!(gate.encode().unwrap(), before_gate);
            let plan = source
                .plan_joint(
                    &repair, &selected, &losing, &journal, None, version, &group, 0,
                )
                .unwrap();
            assert_eq!(
                gate.admit_inbound(AdmittedOperation {
                    op_hash: [3; 32],
                    domain_op_id: [4; 32],
                    author: owner.device_id(),
                    encoded_len: 33
                })
                .unwrap(),
                Admission::Accepted
            );
            let before_gate = gate.encode().unwrap();
            let before_book = source.book_bytes().unwrap();
            // Deliberately reuse the already-compared snapshot fingerprint. Only the locked
            // final stamp can see an inbound gate mutation in this interval.
            assert!(
                source
                    .commit_joint(plan, &journal, version, &group, 0)
                    .is_err(),
                "gate-only race must refuse even on exact retry"
            );
            assert_eq!(source.book_bytes().unwrap(), before_book);
            assert_eq!(gate.encode().unwrap(), before_gate);
        }
    }
}
