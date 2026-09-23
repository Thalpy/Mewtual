//! Owner-local decision lifecycle. Publication facts, pending obligations and repair choices
//! have separate roles. This leaf does not implement the store's atomic source/journal commit,
//! historical report admission, permanent repair sequence, custody or current serving fences.

use super::repair_state::ResolvedRepair;
use super::*;

const MAX_V1_BYTES: usize = 3 * MAX_RECEIPT_BYTES + 256;

#[derive(Clone, Debug, PartialEq, Eq)]
struct OwnerTenure {
    id: Hash32,
    start: u64,
    key: Vec<u8>,
}

impl From<&Receipt> for OwnerTenure {
    fn from(receipt: &Receipt) -> Self {
        Self {
            id: receipt.tenure_id,
            start: receipt.tenure_start_group_epoch,
            key: receipt.owner_public_key.clone(),
        }
    }
}

/// Derived journal effect. This describes a validated in-memory candidate, not permission to
/// publish, install, or bypass the source transaction's independent compatibility checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalRepairEffect {
    /// Effective losing choice replaced by the selected canonical receipt.
    Replace,
    /// Pending loser retained with its close as evidence and replaced by the canonical choice.
    RetirePendingAndReplace,
    /// Winner was already published; evidence is retained without an unpublished reconciliation.
    Normalize,
    /// Every journal role is unchanged. The source repair may still require a transaction.
    NoChange,
}

#[derive(Clone, Debug)]
struct Provenance {
    resolved: ResolvedRepair,
    retired: Option<(Receipt, CloseRecord)>,
    // A trusted local acknowledgement of matching durable source/recovery barriers, never an
    // authority certificate. Publication and this acknowledgement can complete in either order.
    source_finalized: bool,
}

/// Crash-journaled owner state for one logical document.
///
/// Mutations only prepare local candidates. Persist the whole enclosing owner record atomically
/// before publication, and recheck live authority/custody plus source transaction ownership at
/// the store boundary. In particular, an unresolved source repair must fence ordinary writes even
/// when its journal effect was `NoChange` and therefore left no provenance in this leaf.
#[derive(Clone, Debug, Default)]
pub struct OwnerReceiptJournal {
    document: Option<LogicalDocument>,
    tenure: Option<OwnerTenure>,
    high_water: Option<Receipt>,
    in_flight: Option<Receipt>,
    reconciled: Option<Receipt>,
    provenance: Option<Provenance>,
    // Never-repaired journals retain the exact v1 codec. Once upgraded, cleanup keeps v2.
    version_two: bool,
}

fn adjacent(base: &Receipt, next: &Receipt) -> bool {
    base.document == next.document
        && TenureSelection::from(base) == TenureSelection::from(next)
        && base.closed_epoch.checked_add(1) == Some(next.closed_epoch)
}

impl OwnerReceiptJournal {
    /// Prepare an irrevocable publication obligation after checking present owner authority.
    /// Only a repair may change inherited selection within an owner tenure.
    pub fn prepare(
        &mut self,
        receipt: Receipt,
        group: &ServerGroup,
        expected_tenure_start_group_epoch: u64,
    ) -> Result<(), ReplError> {
        receipt.verify_current_owner(group, expected_tenure_start_group_epoch)?;
        if self.version_two {
            // Public values can bypass the decoder; every newly persisted v2 role must have
            // the same canonical shape required on restart. Authority above bounds allocation.
            Receipt::decode(&receipt.encode())?;
        }
        self.prepare_verified(receipt)
    }

    pub(super) fn prepare_verified(&mut self, receipt: Receipt) -> Result<(), ReplError> {
        if self
            .document
            .as_ref()
            .is_some_and(|doc| doc != &receipt.document)
        {
            return Err(ReplError::EpochScope);
        }
        let tenure = OwnerTenure::from(&receipt);
        let changes_tenure = self.tenure.as_ref().is_some_and(|old| old != &tenure);
        if (self.tenure.is_none() || changes_tenure)
            && (receipt.closed_epoch != receipt.inherited.epoch()
                || self
                    .tenure
                    .as_ref()
                    .is_some_and(|old| tenure.start <= old.start))
        {
            return Err(ReplError::ReceiptConflict);
        }
        // Evidence-only provenance cannot advance beyond its bounded publication witness.
        // A new tenure also cannot erase a repair that still needs publication or recovery.
        if self.provenance.is_some() && (self.reconciled.is_none() || changes_tenure) {
            return Err(ReplError::ReceiptConflict);
        }
        if !changes_tenure {
            // Preserve v1 pending-before-completed retry semantics.
            if let Some(pending) = &self.in_flight {
                return if pending.hash() == receipt.hash() {
                    Ok(())
                } else {
                    Err(ReplError::ReceiptConflict)
                };
            }
            if let Some(base) = self.canonical_head() {
                if base.hash() == receipt.hash() {
                    return Ok(());
                }
                if !adjacent(base, &receipt) {
                    return Err(ReplError::ReceiptConflict);
                }
            }
        }
        // Every refusal precedes mutation. Ordinary verified tenure changes preserve legacy
        // replacement semantics, but only after all repair provenance has been safely cleared.
        self.document
            .get_or_insert_with(|| receipt.document.clone());
        if changes_tenure {
            self.high_water = None;
        }
        self.tenure = Some(tenure);
        self.in_flight = Some(receipt);
        Ok(())
    }

    /// Sole outstanding ordinary publication obligation. Retired evidence never appears here.
    pub fn in_flight(&self) -> Option<&Receipt> {
        self.in_flight.as_ref()
    }

    /// Last completed publication in this tenure; historical state, not present owner authority.
    pub fn published(&self) -> Option<&Receipt> {
        self.high_water.as_ref()
    }

    /// Repair-selected decision, possibly never published.
    pub fn reconciled(&self) -> Option<&Receipt> {
        self.reconciled.as_ref()
    }

    /// Base for the next ordinary receipt. A repaired choice outranks historical publication.
    pub fn canonical_head(&self) -> Option<&Receipt> {
        self.reconciled.as_ref().or(self.high_water.as_ref())
    }

    /// Pending publication takes precedence over the canonical adjacency base.
    pub fn effective_choice(&self) -> Option<&Receipt> {
        self.in_flight.as_ref().or(self.canonical_head())
    }

    /// Retained proof, including evidence-only state after publication. This is historical local
    /// evidence, never sufficient to serve a repair without fresh current-authority validation.
    pub fn retained_repair(&self) -> Option<&ReceiptRepair> {
        self.provenance.as_ref().map(|p| &p.resolved.repair)
    }

    /// Full retired obligation and close, available only for matching source recovery.
    pub fn retired_pending(&self) -> Option<(&Receipt, &CloseRecord)> {
        self.provenance
            .as_ref()?
            .retired
            .as_ref()
            .map(|(r, c)| (r, c))
    }

    /// Reconcile an exact loser or a provable differing-baseline descendant.
    ///
    /// The caller must have independently admitted the historical pair and must enforce global
    /// sequence, source compatibility and nonterminal transaction fences even on `NoChange`.
    /// This leaf checks live repair authority before shortcuts; its self-signature checks do not
    /// establish that the pair's signer was historically an owner. Save the candidate with the
    /// source transaction at B1 before retiring any external obligation or serving the winner.
    pub fn resolve_repair(
        &mut self,
        repair: &ReceiptRepair,
        a: &Receipt,
        b: &Receipt,
        retiring_close: Option<&CloseRecord>,
        group: &ServerGroup,
        issuer_tenure_start_group_epoch: u64,
    ) -> Result<JournalRepairEffect, ReplError> {
        repair.verify_current_owner(group, issuer_tenure_start_group_epoch)?;
        repair.check_evidence(a, b)?;
        if self
            .document
            .as_ref()
            .is_some_and(|doc| doc != &repair.document)
        {
            return Err(ReplError::EpochScope);
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
        if self
            .provenance
            .as_ref()
            .is_some_and(|p| p.resolved.repair.hash() == repair.hash())
        {
            return Ok(JournalRepairEffect::NoChange);
        }
        let covered = self.effective_choice().is_some_and(|r| resolved.covers(r));
        let normalize = self.in_flight.is_none()
            && self.reconciled.is_none()
            && self
                .high_water
                .as_ref()
                .is_some_and(|r| r.hash() == selected.hash());
        if !covered && !normalize {
            // Historical high-water alone cannot displace a different effective choice.
            return Ok(JournalRepairEffect::NoChange);
        }
        if self.tenure.as_ref() != Some(&OwnerTenure::from(selected)) {
            return Err(ReplError::EpochScope);
        }
        if self.provenance.as_ref().is_some_and(|p| {
            !p.source_finalized || repair.repair_sequence <= p.resolved.repair.repair_sequence
        }) {
            return Err(ReplError::ReceiptConflict);
        }
        let retired = match &self.in_flight {
            Some(pending) => {
                let close = retiring_close.ok_or(ReplError::ReceiptConflict)?;
                validate_retired(&resolved, pending, close)?;
                Some((pending.clone(), close.clone()))
            }
            None => None,
        };
        let normalized = self
            .high_water
            .as_ref()
            .is_some_and(|r| r.hash() == selected.hash());
        let effect = if normalized {
            JournalRepairEffect::Normalize
        } else if retired.is_some() {
            JournalRepairEffect::RetirePendingAndReplace
        } else {
            JournalRepairEffect::Replace
        };
        let mut candidate = self.clone();
        candidate.version_two = true;
        candidate.in_flight = None;
        candidate.reconciled = (!normalized).then(|| selected.clone());
        candidate.provenance = Some(Provenance {
            resolved,
            retired,
            source_finalized: false,
        });
        // Upgrade also validates retained v1 history, without changing v1 restore semantics.
        candidate.validate_v2()?;
        *self = candidate;
        Ok(effect)
    }

    /// Record actual completion of the exact still-authorized decision. The store must recheck
    /// authority and durable source/custody fences first, and persist again even on an inert retry.
    pub fn mark_published(&mut self, receipt_hash: Hash32) -> Result<(), ReplError> {
        if self
            .high_water
            .as_ref()
            .is_some_and(|r| r.hash() == receipt_hash)
            && !self
                .in_flight
                .as_ref()
                .is_some_and(|r| r.hash() == receipt_hash)
        {
            // A repaired baseline can later reselect the exact historical publication as an
            // adjacent pending decision. That active obligation must complete below; only a
            // genuinely historical retry is inert while a different repaired choice stands.
            return Ok(());
        }
        if self.provenance.is_some() && self.reconciled.is_none() {
            return Err(ReplError::ReceiptConflict);
        }
        // With a pending successor, a stale reconciliation callback cannot overtake it.
        let decision = self
            .in_flight
            .as_ref()
            .or(self.reconciled.as_ref())
            .filter(|r| r.hash() == receipt_hash)
            .ok_or(ReplError::ReceiptConflict)?;
        self.high_water = Some(decision.clone());
        self.in_flight = None;
        self.reconciled = None;
        self.cleanup_finalized();
        Ok(())
    }

    /// Acknowledge completion of the matching durable source transaction AND every required
    /// losing-evidence/recovery barrier. The caller supplies those external IO facts; this core
    /// cannot validate them. No runtime caller may infer them from publication or installation.
    ///
    /// Exact retries succeed while proof is retained. After cleanup, an absent proof refuses
    /// rather than certifying an arbitrary hash; the store may still reflush its unchanged record.
    pub fn mark_repair_source_finalized(&mut self, repair_hash: Hash32) -> Result<(), ReplError> {
        let proof = self
            .provenance
            .as_mut()
            .filter(|p| p.resolved.repair.hash() == repair_hash)
            .ok_or(ReplError::ReceiptConflict)?;
        proof.source_finalized = true;
        self.cleanup_finalized();
        Ok(())
    }

    fn cleanup_finalized(&mut self) {
        if self.reconciled.is_none() && self.provenance.as_ref().is_some_and(|p| p.source_finalized)
        {
            self.provenance = None;
        }
    }

    /// Canonical plaintext journal bytes. Seal in the enclosing atomic owner record.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u8(if self.version_two { 2 } else { 1 });
        put_receipt(&mut e, self.high_water.as_ref());
        put_receipt(&mut e, self.in_flight.as_ref());
        if !self.version_two {
            let selection = self
                .high_water
                .as_ref()
                .or(self.in_flight.as_ref())
                .map(TenureSelection::from);
            put_tenure(&mut e, selection.as_ref());
        } else {
            put_receipt(&mut e, self.reconciled.as_ref());
            match &self.tenure {
                None => {
                    e.put_u8(0);
                }
                Some(t) => {
                    e.put_u8(1);
                    put_hash(&mut e, &t.id);
                    e.put_u64(t.start);
                    e.put_bytes(&t.key).expect("fixed owner key fits");
                }
            }
            match &self.provenance {
                None => {
                    e.put_u8(0);
                }
                Some(p) => {
                    // Canonical and evidence-only roles are explicit, not guessed on restore.
                    e.put_u8(if self.reconciled.is_some() { 1 } else { 2 });
                    p.resolved.encode_into(&mut e);
                    e.put_u8(u8::from(p.source_finalized));
                    put_receipt(&mut e, p.retired.as_ref().map(|(r, _)| r));
                    if let Some((_, close)) = &p.retired {
                        e.put_bytes(&close.encode()).expect("bounded close fits");
                    }
                }
            }
        }
        e.finish()
    }

    /// Restore authenticated owner-local state. V1 retains its original signature policy; V2
    /// additionally checks historical signatures and every repair role. Neither grants live
    /// authority, admission of remote historical evidence, nor a source completion certificate.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_OWNER_RECEIPT_JOURNAL_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        let version = d.get_u8().map_err(|_| ReplError::Malformed)?;
        if version == 1 && bytes.len() > MAX_V1_BYTES {
            return Err(ReplError::EpochBound);
        }
        if version != 1 && version != 2 {
            return Err(ReplError::Malformed);
        }
        let high_water = get_receipt(&mut d)?;
        let in_flight = get_receipt(&mut d)?;
        if version == 1 {
            let selection = get_tenure(&mut d)?;
            d.finish().map_err(|_| ReplError::Malformed)?;
            let mut journal = Self {
                high_water,
                in_flight,
                ..Self::default()
            };
            journal.document = journal.effective_choice().map(|r| r.document.clone());
            journal.tenure = journal.effective_choice().map(OwnerTenure::from);
            if journal.document.is_some() != selection.is_some() {
                return Err(ReplError::Malformed);
            }
            for r in [&journal.high_water, &journal.in_flight]
                .into_iter()
                .flatten()
            {
                if journal.document.as_ref() != Some(&r.document)
                    || selection.as_ref() != Some(&TenureSelection::from(r))
                {
                    return Err(ReplError::Malformed);
                }
            }
            journal.validate_adjacency()?;
            return Ok(journal);
        }
        let reconciled = get_receipt(&mut d)?;
        let tenure = match d.get_u8().map_err(|_| ReplError::Malformed)? {
            0 => None,
            1 => Some(OwnerTenure {
                id: get_fixed(&mut d)?,
                start: d.get_u64().map_err(|_| ReplError::Malformed)?,
                key: d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec(),
            }),
            _ => return Err(ReplError::Malformed),
        };
        let provenance = match d.get_u8().map_err(|_| ReplError::Malformed)? {
            0 => None,
            role @ (1 | 2) => {
                if (role == 1) != reconciled.is_some() {
                    return Err(ReplError::Malformed);
                }
                let resolved = ResolvedRepair::decode_from(&mut d)?;
                let source_finalized = match d.get_u8().map_err(|_| ReplError::Malformed)? {
                    0 => false,
                    1 => true,
                    _ => return Err(ReplError::Malformed),
                };
                let retired = match get_receipt(&mut d)? {
                    None => None,
                    Some(r) => Some((
                        r,
                        CloseRecord::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?,
                    )),
                };
                Some(Provenance {
                    resolved,
                    retired,
                    source_finalized,
                })
            }
            _ => return Err(ReplError::Malformed),
        };
        d.finish().map_err(|_| ReplError::Malformed)?;
        let mut journal = Self {
            document: None,
            tenure,
            high_water,
            in_flight,
            reconciled,
            provenance,
            version_two: true,
        };
        journal.document = journal.effective_choice().map(|r| r.document.clone());
        journal.validate_v2()?;
        Ok(journal)
    }

    fn validate_adjacency(&self) -> Result<(), ReplError> {
        if let Some(pending) = &self.in_flight {
            let valid = match self.canonical_head() {
                Some(base) => adjacent(base, pending),
                None => pending.closed_epoch == pending.inherited.epoch(),
            };
            if !valid {
                return Err(ReplError::Malformed);
            }
        }
        Ok(())
    }

    fn validate_v2(&self) -> Result<(), ReplError> {
        if self.document.is_some() != self.tenure.is_some() {
            return Err(ReplError::Malformed);
        }
        for r in [&self.high_water, &self.in_flight, &self.reconciled]
            .into_iter()
            .flatten()
        {
            self.validate_receipt(r)?;
        }
        self.validate_adjacency()?;
        match &self.provenance {
            None if self.reconciled.is_some() => return Err(ReplError::Malformed),
            None => {}
            Some(p) => {
                p.resolved
                    .verify(self.document.as_ref(), p.resolved.repair.repair_sequence)?;
                self.validate_receipt(&p.resolved.selected)?;
                self.validate_receipt(&p.resolved.losing)?;
                if let Some((r, close)) = &p.retired {
                    self.validate_receipt(r)?;
                    validate_retired(&p.resolved, r, close)?;
                }
                if let Some(reconciled) = &self.reconciled {
                    if reconciled != &p.resolved.selected
                        || self.high_water.as_ref() == Some(reconciled)
                    {
                        return Err(ReplError::Malformed);
                    }
                } else {
                    // This one-step witness is why evidence-only state holds further writes.
                    if p.source_finalized
                        || self.in_flight.is_some()
                        || !self.high_water.as_ref().is_some_and(|r| {
                            r == &p.resolved.selected || adjacent(&p.resolved.selected, r)
                        })
                    {
                        return Err(ReplError::Malformed);
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_receipt(&self, receipt: &Receipt) -> Result<(), ReplError> {
        receipt.verify_signature_only()?;
        if Receipt::decode(&receipt.encode())? != *receipt {
            return Err(ReplError::Malformed);
        }
        if self.document.as_ref() != Some(&receipt.document)
            || self.tenure.as_ref() != Some(&OwnerTenure::from(receipt))
        {
            return Err(ReplError::EpochScope);
        }
        Ok(())
    }
}

fn validate_retired(
    resolved: &ResolvedRepair,
    receipt: &Receipt,
    close: &CloseRecord,
) -> Result<(), ReplError> {
    // Close fields are public. Bound before hashing/encoding; its hash excludes the signature,
    // so canonical decoding and hash binding alone cannot authenticate the retained evidence.
    if close.server_id.len() > MAX_SERVER_ID_BYTES
        || close.heads.len() > MAX_HEADS
        || close.author_public_key.len() != 32
    {
        return Err(ReplError::EpochBound);
    }
    if CloseRecord::decode(&close.encode())? != *close {
        return Err(ReplError::Malformed);
    }
    if !verify_with_public_bytes(
        &close.author_public_key,
        &close.signature_hash(),
        &close.signature,
    ) {
        return Err(ReplError::EpochAuthority);
    }
    if !resolved.covers(receipt)
        || close.server_id != receipt.document.server_id
        || close.doc_type != receipt.document.doc_type
        || close.closed_epoch != receipt.closed_epoch
        || close.hash() != receipt.close_record_hash
    {
        return Err(ReplError::ReceiptConflict);
    }
    // The signed hash binds concrete doc_id; historical authors need not remain current members.
    Ok(())
}

#[cfg(test)]
mod tests;
