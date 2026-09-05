//! Local P1 storage admission from a complete, trusted disk inventory.
//!
//! This is accounting, not authority to delete history. Callers must inventory all managed files,
//! including orphan temporaries, before constructing/reconciling a budget. Neither peer claims nor
//! a persisted counter prove disk usage. Inventory discovery and multi-record settlement remain
//! separate integration work; this module deliberately does not scan, delete or guess.

use std::collections::BTreeMap;

use thiserror::Error;

/// Total P1 bytes per server, including both reserves and temporary files.
pub const MAX_STORAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Space ordinary edits cannot consume, for one serialized settlement's copies and staged data.
pub const SETTLEMENT_RESERVE_BYTES: u64 = 48 * 1024 * 1024;
/// Space reserved for protocol records, not ordinary user content.
pub const PROTOCOL_ALLOWANCE_BYTES: u64 = 16 * 1024 * 1024;
/// Permanent content ceiling; the reserves are inside, not above, the total.
pub const CONTENT_ALLOWANCE_BYTES: u64 =
    MAX_STORAGE_BYTES - SETTLEMENT_RESERVE_BYTES - PROTOCOL_ALLOWANCE_BYTES;
/// Local metadata rail. This bounds the accounting map even for empty/tiny hostile records; it
/// is a storage-admission limit, not a replicated materialization or registry-admission rule.
/// Crash-created temporary siblings can exceed it; bounded cleanup must precede reconciliation
/// in that case. Never truncate an inventory to fit the rail and then admit new writes.
pub const MAX_ACCOUNTED_RECORDS: usize = 65_536;

/// Local mount id and full group id. Both are needed when an app-local id is later reused.
#[derive(Clone, PartialEq, Eq)]
pub struct StorageScope {
    server: u64,
    group: Vec<u8>,
}

impl StorageScope {
    /// Validate a scope before it can name an inventory or budget.
    pub fn new(server: u64, group: &[u8]) -> Result<Self, BudgetError> {
        if group.is_empty() || group.len() > 256 {
            return Err(BudgetError::Scope);
        }
        Ok(Self {
            server,
            group: group.to_vec(),
        })
    }
}

impl std::fmt::Debug for StorageScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageScope").finish_non_exhaustive()
    }
}

/// Physical encoded bytes, including vault framing and authentication tags. The caller classifies
/// them from verified local record types; this classification must never come from a peer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Footprint {
    /// Retained user content and recovery versions.
    pub content: u64,
    /// Receipts, closes, registry state and other protocol allowance consumers.
    pub protocol: u64,
    /// Staged recovery and unsettled temporary bytes pinning one document's reserve.
    pub settlement: u64,
}

impl Footprint {
    /// Whole physical length, rejecting arithmetic overflow rather than wrapping to spare space.
    pub fn total(self) -> Result<u64, BudgetError> {
        self.content
            .checked_add(self.protocol)
            .and_then(|v| v.checked_add(self.settlement))
            .ok_or(BudgetError::Limit)
    }

    fn plus(self, rhs: Self) -> Result<Self, BudgetError> {
        Ok(Self {
            content: self
                .content
                .checked_add(rhs.content)
                .ok_or(BudgetError::Limit)?,
            protocol: self
                .protocol
                .checked_add(rhs.protocol)
                .ok_or(BudgetError::Limit)?,
            settlement: self
                .settlement
                .checked_add(rhs.settlement)
                .ok_or(BudgetError::Limit)?,
        })
    }

    fn minus(self, rhs: Self) -> Self {
        // Only exact entries previously charged to this inventory may be removed.
        Self {
            content: self.content - rhs.content,
            protocol: self.protocol - rhs.protocol,
            settlement: self.settlement - rhs.settlement,
        }
    }

    fn check(self) -> Result<(), BudgetError> {
        if self.content > CONTENT_ALLOWANCE_BYTES
            || self.protocol > PROTOCOL_ALLOWANCE_BYTES
            || self.settlement > SETTLEMENT_RESERVE_BYTES
            || self.total()? > MAX_STORAGE_BYTES
        {
            return Err(BudgetError::Limit);
        }
        Ok(())
    }
}

/// One inventory entry. Ids are fixed local storage keys, not filenames supplied by peers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageRecord {
    /// Unique physical record, including a distinct id for any retained temporary sibling.
    pub id: [u8; 32],
    /// Logical document owning this record and any pinned settlement bytes.
    pub document: [u8; 32],
    /// The observed, classified physical length.
    pub footprint: Footprint,
}

/// Which pool may carry the extra physical copy needed before atomic replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePurpose {
    /// Both permanent bytes and temporary copies fit outside the settlement reserve.
    Ordinary,
    /// Temporary copies use the one document-bound settlement reserve. This conveys no receipt
    /// or deletion authority; the caller's settlement coordinator must supply that separately.
    Settlement,
}

/// A single-file replacement, not a speculative batch with credit for future deletions.
#[derive(Debug, Clone, Copy)]
pub struct Replacement {
    /// Complete final record. Its stable id may replace an existing entry belonging to this doc.
    pub record: StorageRecord,
    /// Additional scratch bytes beyond the complete replacement copy, all charged at peak.
    pub scratch_bytes: u64,
    /// Whether temporary copies may borrow the settlement reserve.
    pub purpose: WritePurpose,
}

/// Distinct refusals let future diagnostics distinguish busy recovery from genuine exhaustion.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum BudgetError {
    /// Wrong or malformed local server/group scope.
    #[error("storage inventory has the wrong scope")]
    Scope,
    /// Counters may no longer describe disk, including an abandoned/forgotten reservation.
    #[error("storage inventory must be reconciled before another write")]
    Reconcile,
    /// Permanent or peak bytes exceed a pool's hard cap, or arithmetic overflowed.
    #[error("studio storage limit reached")]
    Limit,
    /// Inventory cardinality exceeded its local metadata rail.
    #[error("studio storage inventory has too many records")]
    RecordLimit,
    /// Duplicate ids, changed owner, or a mismatched observed old record.
    #[error("storage inventory does not match the record")]
    Inventory,
    /// Another logical document already holds the settlement reserve.
    #[error("another document holds the settlement reserve")]
    SettlementBusy,
}

/// One non-cloneable budget. A ServerStore/coordinator must own one per server and serialize its
/// use with file writes. A second fabricated budget is not independent authority to spend space.
pub struct EpochStorageBudget {
    scope: StorageScope,
    records: BTreeMap<[u8; 32], StorageRecord>,
    usage: Footprint,
    pinned: Option<[u8; 32]>,
    ready: bool,
}

impl std::fmt::Debug for EpochStorageBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Diagnostics need counters/readiness, not a potentially huge list of private record ids.
        f.debug_struct("EpochStorageBudget")
            .field("records", &self.records.len())
            .field("usage", &self.usage)
            .field("settlement_pinned", &self.pinned.is_some())
            .field("ready", &self.ready)
            .finish_non_exhaustive()
    }
}

impl EpochStorageBudget {
    /// Construct only from a complete verified inventory, not a partial directory page. Reject
    /// duplicate ids instead of silently reducing usage. Unknown/orphan bytes must be included.
    /// Empty orphan files still consume a metadata slot (a crash can precede the first write).
    pub fn from_inventory(
        scope: StorageScope,
        records: impl IntoIterator<Item = StorageRecord>,
    ) -> Result<Self, BudgetError> {
        let mut budget = Self {
            scope,
            records: BTreeMap::new(),
            usage: Footprint::default(),
            pinned: None,
            ready: true,
        };
        for record in records {
            if budget.records.len() == MAX_ACCOUNTED_RECORDS {
                return Err(BudgetError::RecordLimit);
            }
            if budget.records.contains_key(&record.id) {
                return Err(BudgetError::Inventory);
            }
            if record.footprint.settlement != 0 {
                if budget.pinned.is_some_and(|doc| doc != record.document) {
                    return Err(BudgetError::SettlementBusy);
                }
                budget.pinned = Some(record.document);
            }
            budget.usage = budget.usage.plus(record.footprint)?;
            budget.usage.check()?;
            budget.records.insert(record.id, record);
        }
        Ok(budget)
    }

    /// Reload all actual files after startup or an uncertain write. A failed reconciliation
    /// leaves the budget closed; stale counters must not survive a failed disk inventory.
    pub fn reconcile(
        &mut self,
        scope: &StorageScope,
        records: impl IntoIterator<Item = StorageRecord>,
    ) -> Result<(), BudgetError> {
        self.ready = false;
        if &self.scope != scope {
            return Err(BudgetError::Scope);
        }
        let next = Self::from_inventory(scope.clone(), records)?;
        *self = next;
        Ok(())
    }

    /// Current counters. When reconciliation is required these are only the last known view.
    pub fn usage(&self) -> Footprint {
        self.usage
    }

    /// Whether any subsequent write must wait for a complete disk inventory.
    pub fn requires_reconciliation(&self) -> bool {
        !self.ready
    }

    /// Close admission after an unaccounted mutation or failed/incomplete disk observation.
    pub fn invalidate(&mut self) {
        self.ready = false;
    }

    /// Verify an observed old record under the same scope before reserving its replacement.
    /// A mismatch invalidates the whole inventory rather than granting accidental free bytes.
    pub fn verify_record(
        &mut self,
        scope: &StorageScope,
        id: [u8; 32],
        observed: Option<StorageRecord>,
    ) -> Result<(), BudgetError> {
        if !self.ready {
            return Err(BudgetError::Reconcile);
        }
        if &self.scope != scope {
            return Err(BudgetError::Scope);
        }
        if observed.is_some_and(|record| record.id != id)
            || self.records.get(&id).copied() != observed
        {
            self.ready = false;
            return Err(BudgetError::Inventory);
        }
        Ok(())
    }

    /// Preflight both final occupancy and peak old+new+scratch bytes. No old bytes are credited
    /// at peak: they cannot disappear until the replacement has actually committed.
    pub fn reserve(
        &mut self,
        scope: &StorageScope,
        replacement: Replacement,
    ) -> Result<StorageReservation<'_>, BudgetError> {
        if !self.ready {
            return Err(BudgetError::Reconcile);
        }
        if &self.scope != scope {
            return Err(BudgetError::Scope);
        }
        let record = replacement.record;
        if record.footprint.total()? == 0 {
            return Err(BudgetError::Inventory);
        }
        let old = self.records.get(&record.id).copied();
        if old.is_some_and(|old| old.document != record.document) {
            return Err(BudgetError::Inventory);
        }
        if old.is_none() && self.records.len() == MAX_ACCOUNTED_RECORDS {
            return Err(BudgetError::RecordLimit);
        }
        let final_usage = self
            .usage
            .minus(old.map_or(Footprint::default(), |record| record.footprint))
            .plus(record.footprint)?;
        final_usage.check()?;
        match replacement.purpose {
            WritePurpose::Ordinary => {
                if record.footprint.settlement != 0
                    || old.is_some_and(|r| r.footprint.settlement != 0)
                {
                    return Err(BudgetError::SettlementBusy);
                }
                self.usage
                    .plus(record.footprint)?
                    .plus(Footprint {
                        content: replacement.scratch_bytes,
                        ..Footprint::default()
                    })?
                    .check()?;
            }
            WritePurpose::Settlement => {
                if self.pinned.is_some_and(|doc| doc != record.document) {
                    return Err(BudgetError::SettlementBusy);
                }
                let peak = self
                    .usage
                    .settlement
                    .checked_add(record.footprint.total()?)
                    .and_then(|v| v.checked_add(replacement.scratch_bytes))
                    .ok_or(BudgetError::Limit)?;
                if peak > SETTLEMENT_RESERVE_BYTES {
                    return Err(BudgetError::Limit);
                }
            }
        }
        // Set the fail-closed bit BEFORE giving the guard to the caller. Drop alone is not enough:
        // mem::forget ends the mutable borrow without running Drop, but must never restore credit.
        self.ready = false;
        Ok(StorageReservation {
            budget: self,
            record,
            final_usage,
        })
    }
}

/// A reservation keeps exclusive access through I/O. Dropping/forgetting it leaves the budget
/// blocked, because a failed write may have left bytes on disk. Reconcile only after inventorying
/// or durably removing those bytes. There is deliberately no automatic refund in Drop.
#[derive(Debug)]
pub struct StorageReservation<'a> {
    budget: &'a mut EpochStorageBudget,
    record: StorageRecord,
    final_usage: Footprint,
}

impl Drop for StorageReservation<'_> {
    fn drop(&mut self) {
        // No refund: the budget was marked unready before this guard existed. Only commit or a
        // proven pre-I/O cancellation reopens it, even if the guard is leaked instead of dropped.
    }
}

impl StorageReservation<'_> {
    /// Complete only after atomic write AND required flush/temporary cleanup succeeded. All
    /// arithmetic/limits were prevalidated, so committing the counters cannot fail after disk I/O.
    pub fn commit(self) {
        self.budget.records.insert(self.record.id, self.record);
        self.budget.usage = self.final_usage;
        self.budget.pinned = self
            .budget
            .records
            .values()
            .find(|r| r.footprint.settlement != 0)
            .map(|r| r.document);
        self.budget.ready = true;
    }

    /// Cancel only while no disk I/O has occurred. A write/flush/cleanup error must not use this
    /// shortcut; its possibly materialized bytes require reconciliation instead.
    pub fn cancel_before_write(self) {
        self.budget.ready = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;
    fn scope() -> StorageScope {
        StorageScope::new(7, b"group").unwrap()
    }
    fn record(id: u8, content: u64) -> StorageRecord {
        StorageRecord {
            id: [id; 32],
            document: [1; 32],
            footprint: Footprint {
                content,
                ..Footprint::default()
            },
        }
    }
    fn replace(record: StorageRecord, purpose: WritePurpose) -> Replacement {
        Replacement {
            record,
            purpose,
            scratch_bytes: 0,
        }
    }
    fn full() -> EpochStorageBudget {
        let mut protocol = record(3, 0);
        protocol.footprint.protocol = PROTOCOL_ALLOWANCE_BYTES;
        EpochStorageBudget::from_inventory(
            scope(),
            [
                record(1, 16 * MIB),
                record(2, CONTENT_ALLOWANCE_BYTES - 16 * MIB),
                protocol,
            ],
        )
        .unwrap()
    }

    #[test]
    fn pools_and_reserves_fit_inside_the_total_not_above_it() {
        assert_eq!(
            CONTENT_ALLOWANCE_BYTES + PROTOCOL_ALLOWANCE_BYTES + SETTLEMENT_RESERVE_BYTES,
            MAX_STORAGE_BYTES
        );
        assert_eq!(CONTENT_ALLOWANCE_BYTES, 1984 * MIB);
        let mut staged = record(4, 0);
        staged.footprint.settlement = SETTLEMENT_RESERVE_BYTES;
        let mut records: Vec<_> = full().records.values().copied().collect();
        records.push(staged);
        assert_eq!(
            EpochStorageBudget::from_inventory(scope(), records)
                .unwrap()
                .usage()
                .total()
                .unwrap(),
            MAX_STORAGE_BYTES
        );
        let mut budget = full();
        assert_eq!(
            budget
                .reserve(&scope(), replace(record(5, 1), WritePurpose::Ordinary))
                .unwrap_err(),
            BudgetError::Limit
        );
        assert!(!budget.requires_reconciliation());
    }

    #[test]
    fn old_bytes_are_not_refunded_while_the_replacement_copy_is_written() {
        let mut budget = full();
        let before = budget.usage();
        assert_eq!(
            budget
                .reserve(
                    &scope(),
                    replace(record(1, 8 * MIB), WritePurpose::Ordinary)
                )
                .unwrap_err(),
            BudgetError::Limit
        );
        let reservation = budget
            .reserve(
                &scope(),
                replace(record(1, 8 * MIB), WritePurpose::Settlement),
            )
            .unwrap();
        reservation.commit();
        assert_eq!(budget.usage().content, before.content - 8 * MIB);
        assert_eq!(budget.usage().protocol, before.protocol);
        // A new recovery version gets no credit for a source epoch that might be deleted later.
        let mut budget = full();
        assert_eq!(
            budget
                .reserve(&scope(), replace(record(5, 1), WritePurpose::Settlement))
                .unwrap_err(),
            BudgetError::Limit
        );
    }

    #[test]
    fn peak_includes_staged_bytes_replacement_and_all_scratch() {
        let mut budget = full();
        let mut staged = record(4, 0);
        staged.footprint.settlement = 6 * MIB;
        let mut inventory: Vec<_> = budget.records.values().copied().collect();
        inventory.push(staged);
        budget.reconcile(&scope(), inventory).unwrap();
        let mut plan = replace(record(1, 8 * MIB), WritePurpose::Settlement);
        plan.scratch_bytes = 34 * MIB + 1;
        assert_eq!(
            budget.reserve(&scope(), plan).unwrap_err(),
            BudgetError::Limit
        );
        plan.scratch_bytes -= 1;
        budget
            .reserve(&scope(), plan)
            .unwrap()
            .cancel_before_write();
        assert_eq!(budget.usage().content, CONTENT_ALLOWANCE_BYTES);
    }

    #[test]
    fn staged_recovery_pins_reserve_but_does_not_block_unrelated_ordinary_edits() {
        let mut staged = record(1, 100);
        staged.footprint.settlement = 100;
        let mut budget = EpochStorageBudget::from_inventory(scope(), [staged]).unwrap();
        let mut other = record(2, 100);
        other.document = [2; 32];
        assert_eq!(
            budget
                .reserve(&scope(), replace(other, WritePurpose::Settlement))
                .unwrap_err(),
            BudgetError::SettlementBusy
        );
        budget
            .reserve(&scope(), replace(other, WritePurpose::Ordinary))
            .unwrap()
            .commit();
        budget
            .reserve(&scope(), replace(record(1, 200), WritePurpose::Settlement))
            .unwrap()
            .commit();
        budget
            .reserve(&scope(), replace(other, WritePurpose::Settlement))
            .unwrap()
            .cancel_before_write();
    }

    #[test]
    fn dropping_or_forgetting_a_reservation_never_refunds_unknown_disk_bytes() {
        for forget in [false, true] {
            let mut budget = EpochStorageBudget::from_inventory(scope(), []).unwrap();
            let reservation = budget
                .reserve(&scope(), replace(record(1, 10), WritePurpose::Settlement))
                .unwrap();
            if forget {
                std::mem::forget(reservation);
            } else {
                drop(reservation);
            }
            assert!(budget.requires_reconciliation());
            assert_eq!(
                budget
                    .reserve(&scope(), replace(record(1, 10), WritePurpose::Settlement))
                    .unwrap_err(),
                BudgetError::Reconcile
            );
            budget.reconcile(&scope(), [record(1, 10)]).unwrap();
            assert_eq!(budget.usage().content, 10);
            budget
                .reserve(&scope(), replace(record(1, 20), WritePurpose::Settlement))
                .unwrap()
                .commit();
            assert_eq!(budget.usage().content, 20);
        }
    }

    #[test]
    fn cancellation_before_io_and_refusals_leave_the_old_inventory_unchanged() {
        let mut budget = EpochStorageBudget::from_inventory(scope(), [record(1, 100)]).unwrap();
        budget
            .reserve(&scope(), replace(record(1, 20), WritePurpose::Ordinary))
            .unwrap()
            .cancel_before_write();
        assert_eq!(budget.usage().content, 100);
        assert!(!budget.requires_reconciliation());
        let wrong = StorageScope::new(7, b"other-group").unwrap();
        assert_eq!(
            budget
                .reserve(&wrong, replace(record(1, 20), WritePurpose::Ordinary))
                .unwrap_err(),
            BudgetError::Scope
        );
        let mut other = record(1, 20);
        other.document = [2; 32];
        assert_eq!(
            budget
                .reserve(&scope(), replace(other, WritePurpose::Settlement))
                .unwrap_err(),
            BudgetError::Inventory
        );
    }

    #[test]
    fn matching_total_length_does_not_hide_a_wrong_pool_or_owner() {
        for owner in [false, true] {
            let old = record(1, 100);
            let mut budget = EpochStorageBudget::from_inventory(scope(), [old]).unwrap();
            let mut observed = old;
            if owner {
                observed.document = [9; 32];
            } else {
                observed.footprint.content = 0;
                observed.footprint.settlement = 100;
            }
            assert_eq!(
                budget.verify_record(&scope(), old.id, Some(observed)),
                Err(BudgetError::Inventory)
            );
            assert!(budget.requires_reconciliation());
        }
    }

    #[test]
    fn reconciliation_is_fail_closed_and_duplicates_are_not_deduplicated() {
        assert!(matches!(
            EpochStorageBudget::from_inventory(scope(), [record(1, 10), record(1, 10)]),
            Err(BudgetError::Inventory)
        ));
        let mut budget = EpochStorageBudget::from_inventory(scope(), []).unwrap();
        let wrong = StorageScope::new(8, b"group").unwrap();
        assert_eq!(budget.reconcile(&wrong, []), Err(BudgetError::Scope));
        assert!(budget.requires_reconciliation());
        assert_eq!(
            budget.reconcile(&scope(), [record(1, CONTENT_ALLOWANCE_BYTES + 1)]),
            Err(BudgetError::Limit)
        );
        assert!(budget.requires_reconciliation());
        budget.reconcile(&scope(), []).unwrap();
        assert!(!budget.requires_reconciliation());
    }

    #[test]
    fn empty_crash_orphans_are_inventory_entries_not_valid_replacements() {
        // File creation can succeed just before a crash prevents even the first byte of a write.
        // This changes inventory admission only: encoded recovery records must still be valid.
        let orphan = record(1, 0);
        let saved = record(2, 100);
        let mut budget = EpochStorageBudget::from_inventory(scope(), [orphan, saved]).unwrap();
        assert_eq!(budget.records.len(), 2);
        assert_eq!(budget.usage().total().unwrap(), 100);
        budget
            .verify_record(&scope(), orphan.id, Some(orphan))
            .unwrap();
        assert_eq!(
            budget
                .reserve(&scope(), replace(orphan, WritePurpose::Settlement))
                .unwrap_err(),
            BudgetError::Inventory
        );
        assert!(!budget.requires_reconciliation());
        assert!(matches!(
            EpochStorageBudget::from_inventory(scope(), [orphan, orphan]),
            Err(BudgetError::Inventory)
        ));
    }

    #[test]
    fn hostile_counts_and_arithmetic_cannot_create_credit() {
        assert_eq!(
            Footprint {
                content: u64::MAX,
                protocol: 1,
                settlement: 0
            }
            .total(),
            Err(BudgetError::Limit)
        );
        let mut budget = EpochStorageBudget::from_inventory(scope(), []).unwrap();
        let mut plan = replace(record(1, 1), WritePurpose::Settlement);
        plan.scratch_bytes = u64::MAX;
        assert_eq!(
            budget.reserve(&scope(), plan).unwrap_err(),
            BudgetError::Limit
        );
        for bytes in [0, 1] {
            let mut visited = 0;
            let entries = (0..).map(|n: u64| {
                visited += 1;
                let mut r = record(1, bytes);
                r.id[..8].copy_from_slice(&n.to_be_bytes());
                r
            });
            assert!(matches!(
                EpochStorageBudget::from_inventory(scope(), entries),
                Err(BudgetError::RecordLimit)
            ));
            assert_eq!(visited, MAX_ACCOUNTED_RECORDS + 1);
        }
    }
}
