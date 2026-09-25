//! Shared source-side repair for Studio and Registry. This is an in-memory transition, not a
//! durable source/journal transaction or historical report admission. Typed owners keep their
//! documents private and save the complete restart unit before exposing any changed decision.

use super::repair_state::ResolvedRepair;
use super::*;

mod joint;
pub use joint::ReceiptRepairPlan;

/// What the original application did. Later progress must never change this persisted label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairDisposition {
    /// Ended this source's fault on the named pair.
    Transitioned,
    /// Redirected a nonfaulted source away from a provably losing branch.
    Retargeted,
    /// Recorded screening evidence, preserving every source role and any unrelated fault.
    Screened,
}

/// A valid signed repair that cannot presently change this source. No evidence is discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairHold {
    SequenceNotNewer,
    RepairInProgress,
    Settled,
    UnsupportedShape,
}

/// In-memory result only. The enclosing transaction must persist before reporting durable success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceRepairOutcome {
    Applied(RepairDisposition),
    AlreadyResolved(RepairDisposition),
    Held(RepairHold),
}

/// Committed historical evidence and the source's present continuation state. None of these
/// fields grants current serving/installation authority; live callers must verify it again.
#[derive(Clone, Debug)]
pub struct SourceRepairState {
    pub repair: ReceiptRepair,
    pub selected: Receipt,
    pub losing: Receipt,
    pub installed: bool,
    pub install_pending: bool,
    pub disposition: RepairDisposition,
}

/// Local v3 provenance binds the original action to one exact signed repair (whose hash also
/// commits its sequence). Receipt-book v4/v5 bytes remain unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RepairBinding {
    disposition: RepairDisposition,
    hash: Hash32,
}

impl RepairBinding {
    pub(crate) fn encode_prefix(binding: Option<&Self>, adopting: bool, e: &mut Encoder) {
        let Some(binding) = binding else {
            e.put_u8(if adopting { 2 } else { 1 });
            return;
        };
        e.put_u8(3);
        e.put_u8(u8::from(adopting));
        e.put_u8(match binding.disposition {
            RepairDisposition::Transitioned => 1,
            RepairDisposition::Retargeted => 2,
            RepairDisposition::Screened => 3,
        });
        e.put_u8(1);
        put_hash(e, &binding.hash);
    }

    pub(crate) fn decode_prefix(d: &mut Decoder<'_>) -> Result<(bool, Option<Self>), ReplError> {
        match d.get_u8().map_err(|_| ReplError::Malformed)? {
            1 => Ok((false, None)),
            2 => Ok((true, None)),
            3 => {
                let adopting = match d.get_u8().map_err(|_| ReplError::Malformed)? {
                    0 => false,
                    1 => true,
                    _ => return Err(ReplError::Malformed),
                };
                let disposition = match d.get_u8().map_err(|_| ReplError::Malformed)? {
                    1 => RepairDisposition::Transitioned,
                    2 => RepairDisposition::Retargeted,
                    3 => RepairDisposition::Screened,
                    _ => return Err(ReplError::Malformed),
                };
                if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
                    return Err(ReplError::Malformed);
                }
                Ok((
                    adopting,
                    Some(Self {
                        disposition,
                        hash: get_fixed(d)?,
                    }),
                ))
            }
            _ => Err(ReplError::Malformed),
        }
    }

    pub(crate) fn validate(
        &self,
        book: &ReceiptBook,
        gate: &EpochGate,
        opening: Option<&Receipt>,
        adopting: bool,
    ) -> Result<(), ReplError> {
        let resolved = book.resolved_repair.as_ref().ok_or(ReplError::Malformed)?;
        resolved.verify(Some(&gate.document), book.repair_sequence)?;
        if self.hash != resolved.repair.hash() {
            return Err(ReplError::Malformed);
        }
        let same_tenure = same_issuer_tenure(resolved);
        if self.disposition == RepairDisposition::Retargeted && !same_tenure {
            return Err(ReplError::Malformed);
        }
        if same_tenure && !book.is_faulted() {
            // Historical proof must never re-authorize a live losing head on restart. Ordinary
            // gate/book codecs intentionally also accept legacy history without typed provenance.
            if book.latest().is_some_and(|r| resolved.covers(r)) {
                return Err(ReplError::Malformed);
            }
            if self.disposition == RepairDisposition::Screened
                && opening
                    .into_iter()
                    .chain(book.previous_until_installed.iter())
                    .any(|r| resolved.covers(r))
            {
                return Err(ReplError::Malformed);
            }
            // Retargeting always begins in adoption; every completed successor has an opening.
            // Losing that mode bit must not silently convert whole-source recovery to settlement.
            if self.disposition == RepairDisposition::Retargeted && !adopting && opening.is_none() {
                return Err(ReplError::Malformed);
            }
        }
        // Progress can change opening/head/phase after the repair. Validate present authority
        // roles, not an invented reconstruction of the original operation from today's phase.
        if self.disposition != RepairDisposition::Screened && !book.is_faulted() {
            if same_tenure && gate.phase() == EpochPhase::Open && opening.is_none() {
                return Err(ReplError::Malformed);
            }
            if same_tenure && !adopting && opening.is_some_and(|r| resolved.covers(r)) {
                return Err(ReplError::Malformed);
            }
            if !same_tenure && adopting && book.latest() == Some(&resolved.selected) {
                return Err(ReplError::Malformed);
            }
        }
        Ok(())
    }

    /// A local post-application fence, not the durable B1-through-recycling target claim. Even
    /// public ordinary adoption/seal calls must not erase an outstanding Repair continuation.
    /// The typed caller verifies current receipt authority and scope before consulting this.
    pub(crate) fn pending_admission(
        &self,
        book: &ReceiptBook,
        phase: EpochPhase,
        opening: Option<&Receipt>,
        adopting: bool,
        receipt: &Receipt,
    ) -> Option<Result<ReceiptIngest, ReplError>> {
        if !self.state(book, phase, opening, adopting)?.install_pending {
            return None;
        }
        Some(if book.is_repaired_loser(receipt) {
            Ok(ReceiptIngest::Stale)
        } else if book.latest() == Some(receipt) {
            Ok(ReceiptIngest::Duplicate)
        } else {
            Err(ReplError::ReceiptConflict)
        })
    }

    pub(crate) fn state(
        &self,
        book: &ReceiptBook,
        phase: EpochPhase,
        opening: Option<&Receipt>,
        adopting: bool,
    ) -> Option<SourceRepairState> {
        let r = book.resolved_repair.as_ref()?;
        if self.hash != r.repair.hash() || book.repair_sequence != r.repair.repair_sequence {
            return None;
        }
        let installed =
            opening == Some(&r.selected) && matches!(phase, EpochPhase::Open | EpochPhase::Closing);
        Some(SourceRepairState {
            repair: r.repair.clone(),
            selected: r.selected.clone(),
            losing: r.losing.clone(),
            installed,
            // A screened repair may merely accompany ordinary adoption of this same receipt.
            // It must not take ownership of that continuation or relabel its recovery.
            install_pending: self.disposition != RepairDisposition::Screened
                && adopting
                && phase == EpochPhase::Closing
                && book.latest() == Some(&r.selected)
                && !installed,
            disposition: self.disposition,
        })
    }
}

fn same_issuer_tenure(resolved: &ResolvedRepair) -> bool {
    resolved.repair.owner_public_key == resolved.selected.owner_public_key
        && resolved.repair.issuer_tenure_start_group_epoch
            == Some(resolved.selected.tenure_start_group_epoch)
}

impl ReceiptBook {
    /// Both full receipts of the currently blocking fault. Screening a different repair leaves
    /// this evidence intact; the resolved repair's pair is a separate historical role.
    pub fn fault_evidence(&self) -> Option<(&Receipt, &Receipt)> {
        self.fault.as_ref().map(|(a, b)| (a, b))
    }
}

/// Exact private plan stamp, including accepted/quarantined work and accounting. The exclusive
/// typed source borrow protects its document; this stamp additionally fences gate-only races.
struct RepairStateStamp {
    gate: EpochGateInner,
    book: Vec<u8>,
    adopting: bool,
    binding: Option<RepairBinding>,
    opening: Option<Receipt>,
}

struct RepairCandidate {
    expected: RepairStateStamp,
    book: ReceiptBook,
    gate: EpochGateInner,
    adopting: bool,
    binding: RepairBinding,
}

enum RepairPlan {
    Candidate(Box<RepairCandidate>),
    Unchanged(SourceRepairOutcome),
}

/// Borrowed only by the two private typed-source adapters. Supplying a self-signed pair is not
/// historical owner admission: the future store adapter must establish that independently and
/// enforce custody, source/journal compatibility and the durable transaction claim even on retry.
pub(crate) struct RepairSource<'a> {
    pub(crate) document: &'a LogicalDocument,
    pub(crate) gate: &'a EpochGate,
    pub(crate) book: &'a mut ReceiptBook,
    pub(crate) opening: Option<&'a Receipt>,
    pub(crate) adopting: &'a mut bool,
    pub(crate) binding: &'a mut Option<RepairBinding>,
}

impl RepairSource<'_> {
    pub(crate) fn apply(
        &mut self,
        repair: &ReceiptRepair,
        a: &Receipt,
        b: &Receipt,
        group: &ServerGroup,
        issuer_tenure: u64,
    ) -> Result<SourceRepairOutcome, ReplError> {
        match self.plan(repair, a, b, group, issuer_tenure)? {
            RepairPlan::Unchanged(outcome) => Ok(outcome),
            RepairPlan::Candidate(candidate) => self.commit(*candidate),
        }
    }

    fn book_bytes(&self) -> Result<Vec<u8>, ReplError> {
        self.book.encode_mode(*self.adopting)
    }

    fn plan(
        &self,
        repair: &ReceiptRepair,
        a: &Receipt,
        b: &Receipt,
        group: &ServerGroup,
        issuer_tenure: u64,
    ) -> Result<RepairPlan, ReplError> {
        repair.verify_current_owner(group, issuer_tenure)?;
        repair.check_evidence(a, b)?;
        if repair.document != *self.document || self.gate.document != *self.document {
            return Err(ReplError::EpochScope);
        }
        let held = |reason| Ok(RepairPlan::Unchanged(SourceRepairOutcome::Held(reason)));
        let inner = self.gate.inner.lock().expect("epoch gate poisoned").clone();
        if inner.phase == EpochPhase::Settled {
            return held(RepairHold::Settled);
        }
        if self.book.latest_repair() == Some(repair) {
            let Some(binding) = self.binding.as_ref() else {
                // Legacy bookkeeping cannot reconstruct what the original typed application did.
                return held(RepairHold::UnsupportedShape);
            };
            binding.validate(self.book, self.gate, self.opening, *self.adopting)?;
            return Ok(RepairPlan::Unchanged(SourceRepairOutcome::AlreadyResolved(
                binding.disposition,
            )));
        }
        if repair.repair_sequence <= self.book.repair_sequence {
            return held(RepairHold::SequenceNotNewer);
        }
        if self
            .binding
            .as_ref()
            .and_then(|b| b.state(self.book, inner.phase, self.opening, *self.adopting))
            .is_some_and(|state| state.install_pending)
        {
            return held(RepairHold::RepairInProgress);
        }
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
        let same_tenure = selected.verify_current_owner(group, issuer_tenure).is_ok();
        let same_fault = self
            .book
            .fault
            .as_ref()
            .is_some_and(|(x, y)| (x == a && y == b) || (x == b && y == a));
        if (inner.phase == EpochPhase::Fault) != self.book.is_faulted() {
            return Err(ReplError::Malformed);
        }
        let expected = RepairStateStamp {
            gate: inner.clone(),
            book: self.book_bytes()?,
            adopting: *self.adopting,
            binding: *self.binding,
            opening: self.opening.cloned(),
        };
        let mut candidate = RepairCandidate {
            expected,
            book: self.book.clone(),
            gate: inner,
            adopting: *self.adopting,
            binding: RepairBinding {
                disposition: RepairDisposition::Screened,
                hash: repair.hash(),
            },
        };
        if same_fault {
            // Keep the original core authority/pair/sequence/retry checks as defense in depth.
            candidate.book.apply_repair(repair, group, issuer_tenure)?;
            candidate.binding.disposition = RepairDisposition::Transitioned;
            if !same_tenure {
                // Old-tenure choices are screening history, never a new checkpoint install.
                candidate.set_head(self.opening.cloned());
                candidate.gate.phase = EpochPhase::Open;
                candidate.gate.receipt_hash = None;
                candidate.adopting = false;
            } else if self.opening == Some(selected) {
                let qualified = self.book.latest().filter(|head| {
                    head.closed_epoch == self.gate.epoch
                        && TenureSelection::from(*head) == TenureSelection::from(selected)
                        && !resolved.covers(head)
                });
                candidate.set_head(qualified.cloned().or_else(|| self.opening.cloned()));
                candidate.adopting = false;
                candidate.gate.phase = if qualified.is_some() {
                    EpochPhase::Closing
                } else {
                    EpochPhase::Open
                };
                candidate.gate.receipt_hash = qualified.map(Receipt::hash);
            } else if self.opening.is_some_and(|opening| resolved.covers(opening)) {
                // Even a winner closing E cannot use ordinary settlement on a repudiated
                // installed seed. Exact losers AND losing-baseline openings require adoption.
                candidate.adopt(selected);
            } else if selected.closed_epoch == self.gate.epoch {
                candidate.set_head(Some(selected.clone()));
                candidate.gate.phase = EpochPhase::Closing;
                candidate.gate.receipt_hash = Some(selected.hash());
                candidate.adopting = *self.adopting;
            } else if *self.adopting {
                candidate.adopt(selected);
            } else {
                return held(RepairHold::UnsupportedShape);
            }
        } else {
            candidate.book.document = Some(self.document.clone());
            candidate.book.repair_sequence = repair.repair_sequence;
            candidate.book.resolved_repair = Some(resolved.clone());
            let covered = self
                .book
                .latest
                .iter()
                .chain(self.opening)
                .chain(self.book.previous_until_installed.iter())
                .any(|r| resolved.covers(r));
            if !self.book.is_faulted() && same_tenure && covered {
                candidate.binding.disposition = RepairDisposition::Retargeted;
                candidate.adopt(selected);
            }
            // An unrelated Fault is preserved, including its entire pair. Cross-tenure healthy
            // sources likewise retain their roles; old receipts still fail live install checks.
        }
        if candidate.gate.phase == EpochPhase::Open
            && candidate.binding.disposition != RepairDisposition::Screened
        {
            candidate.gate.quarantine.clear();
        }
        // Reuse the exact persisted-state validators before committing. A new repair must never
        // manufacture a Closing/Open state that becomes stranded immediately after restart.
        let next_gate = EpochGate {
            document: self.gate.document.clone(),
            doc_id: self.gate.doc_id,
            epoch: self.gate.epoch,
            inner: Mutex::new(candidate.gate.clone()),
        };
        let encoded = candidate.book.encode_mode(candidate.adopting)?;
        ReceiptBook::decode_mode(&encoded, candidate.adopting)?;
        let operations: Vec<_> = candidate.gate.operations.values().copied().collect();
        if next_gate
            .verify_restart_mode(
                &operations,
                &candidate.book,
                self.opening,
                candidate.adopting,
            )
            .is_err()
        {
            return held(RepairHold::UnsupportedShape);
        }
        candidate.binding.validate(
            &candidate.book,
            &next_gate,
            self.opening,
            candidate.adopting,
        )?;
        Ok(RepairPlan::Candidate(Box::new(candidate)))
    }

    fn commit(&mut self, candidate: RepairCandidate) -> Result<SourceRepairOutcome, ReplError> {
        if self.book_bytes()? != candidate.expected.book
            || *self.adopting != candidate.expected.adopting
            || *self.binding != candidate.expected.binding
            || self.opening != candidate.expected.opening.as_ref()
        {
            return Err(ReplError::ReceiptConflict);
        }
        let disposition = candidate.binding.disposition;
        self.gate.commit_repair(
            &candidate.expected.gate,
            &candidate.gate,
            disposition,
            || {
                *self.book = candidate.book;
                *self.adopting = candidate.adopting;
                *self.binding = Some(candidate.binding);
            },
        )?;
        Ok(SourceRepairOutcome::Applied(disposition))
    }
}

impl RepairCandidate {
    fn set_head(&mut self, receipt: Option<Receipt>) {
        self.book.tenure = receipt.as_ref().map(TenureSelection::from);
        self.book.latest = receipt;
        self.book.previous_until_installed = None;
    }

    fn adopt(&mut self, selected: &Receipt) {
        self.set_head(Some(selected.clone()));
        self.gate.phase = EpochPhase::Closing;
        self.gate.receipt_hash = Some(selected.hash());
        self.adopting = true;
    }
}

impl EpochGate {
    /// Private validated-candidate commit. Accepted operations/accounting remain exactly intact;
    /// only an Open repair clears rejected quarantine. Screening has a separate unchanged-gate
    /// path and cannot become a general Fault-to-Fault writer.
    fn commit_repair<F>(
        &self,
        expected: &EpochGateInner,
        next: &EpochGateInner,
        disposition: RepairDisposition,
        commit: F,
    ) -> Result<(), ReplError>
    where
        F: FnOnce(),
    {
        let mut inner = self.inner.lock().expect("epoch gate poisoned");
        if *inner != *expected || inner.phase == EpochPhase::Settled {
            return Err(ReplError::ReceiptConflict);
        }
        if disposition == RepairDisposition::Screened {
            if next != expected {
                return Err(ReplError::ReceiptConflict);
            }
        } else {
            if !matches!(
                (next.phase, next.receipt_hash),
                (EpochPhase::Open, None) | (EpochPhase::Closing, Some(_))
            ) {
                return Err(ReplError::ReceiptConflict);
            }
            let mut unchanged = next.clone();
            unchanged.phase = expected.phase;
            unchanged.receipt_hash = expected.receipt_hash;
            if next.phase == EpochPhase::Open {
                if !next.quarantine.is_empty() {
                    return Err(ReplError::ReceiptConflict);
                }
                unchanged.quarantine = expected.quarantine.clone();
            }
            if &unchanged != expected {
                return Err(ReplError::ReceiptConflict);
            }
        }
        *inner = next.clone();
        commit();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
