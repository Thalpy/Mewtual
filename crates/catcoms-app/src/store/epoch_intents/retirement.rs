//! Monotonic included-only retirement. The registry coordinator has already flushed the exact
//! Closing source and saved recovery. Until its final replacement that source remains the
//! restart proof, so retirement can safely precede checkpoint selection without a new journal.

use super::*;
use catcoms_replication::registry_epoch::RegistrySettlementPlan;

#[test]
fn intent_retirement_shrink_still_needs_physical_replacement_headroom() {
    let generation = Arc::new(());
    let mut budget = EpochIntentBudget {
        generation: generation.clone(),
        records: BTreeMap::from([([1; 32], 1024)]),
        record_slots: super::super::epoch_budget::MAX_ACCOUNTED_RECORDS,
        bytes: MAX_VAULT_INTENT_BYTES,
        // This case is about the class total at its cap; no archive is involved.
        archive_bytes: 0,
        ready: true,
    };
    assert!(budget
        .preflight(&generation, [1; 32], Some(1024), 128, false)
        .is_err());
    assert_eq!(budget.bytes, MAX_VAULT_INTENT_BYTES);
    assert!(budget.ready, "known quota refusal needs no rescan");
    assert!(budget
        .preflight(&generation, [1; 32], Some(1024), 1024, true)
        .is_ok());
    assert!(
        budget
            .preflight(&generation, [2; 32], None, 0, true)
            .is_ok(),
        "absent ledger creates no slot"
    );
}

/// The archive sub-cap's arithmetic. The wiring, that the real writer actually consults this
/// rather than the ordinary class preflight, is anchored separately by
/// `the_archive_writer_refuses_at_the_sub_cap_not_the_class_ceiling`; neither test substitutes
/// for the other.
///
/// The sub-cap models the PHYSICAL peak, not the resulting logical occupancy. An earlier version
/// of this test asserted that replacing an archive "releases its bytes first" so a near-cap
/// same-size rewrite would fit. That was wrong: a non-sync write stages its replacement beside
/// the record it replaces, so both exist at once, and a crash at that moment leaves the
/// temporary charged against the same cap.
#[test]
fn draft_archive_sub_cap_covers_the_physical_replacement_peak() {
    let generation = Arc::new(());
    let near = MAX_VAULT_DRAFT_ARCHIVE_BYTES - 1024;
    let mut budget = EpochIntentBudget {
        generation: generation.clone(),
        records: BTreeMap::from([([1; 32], near)]),
        record_slots: 1,
        // Plenty of class headroom: only the archive tally is near its limit, so every refusal
        // below is attributable to the sub-cap.
        bytes: near,
        archive_bytes: near,
        ready: true,
    };
    assert!(
        budget.bytes < MAX_VAULT_INTENT_BYTES,
        "the class ceiling must not be what refuses these"
    );
    assert!(
        budget
            .preflight_draft_archive(&generation, [2; 32], None, 4096, false)
            .is_err(),
        "a new archive over the sub-cap must refuse while the class total still has room"
    );
    assert!(
        budget.ready,
        "a known quota refusal needs no rescan, exactly as the class cap behaves"
    );
    assert!(
        budget
            .preflight_draft_archive(&generation, [1; 32], Some(near), near, false)
            .is_err(),
        "a non-sync replacement stages beside the record it replaces, so the peak is old + new \
         and must refuse near the cap even at the same size"
    );
    // A sync-only retry claims no new bytes and stages nothing, so it is never refused here.
    // This is what keeps an uncertain write repeatable when the vault is at its cap.
    assert!(budget
        .preflight_draft_archive(&generation, [1; 32], Some(near), near, true)
        .is_ok());

    // Commit does the logical `old -> next` move, after the write has actually landed.
    budget.commit_draft_archive([2; 32], None, 4096);
    assert_eq!(budget.archive_bytes, near + 4096);
    assert_eq!(budget.bytes, near + 4096);
}

/// Existing archive occupancy above the policy sub-cap must be grandfathered, not fatal.
///
/// The class ceiling and the sub-cap differ in kind. A vault over the class ceiling is in a
/// state this code cannot safely account for. A vault holding more archive bytes than the policy
/// currently admits is still perfectly accountable, and refusing to construct its budget would
/// be self-locking: every accounted write needs one, so the vault would lose unrelated intent
/// writes and also lose the archive release that is the only way back under the cap.
#[test]
fn an_over_cap_archive_inventory_still_yields_a_usable_budget() {
    let generation = Arc::new(());
    let over = MAX_VAULT_DRAFT_ARCHIVE_BYTES + 4096;
    let mut budget = EpochIntentBudget {
        generation: generation.clone(),
        records: BTreeMap::from([([1; 32], over), ([2; 32], 2048)]),
        record_slots: 2,
        bytes: over + 2048,
        archive_bytes: over,
        ready: true,
    };
    // The premise: over the archive policy, comfortably under the class rail.
    assert!(budget.archive_bytes > MAX_VAULT_DRAFT_ARCHIVE_BYTES);
    assert!(budget.bytes < MAX_VAULT_INTENT_BYTES);

    // Unrelated ordinary intent work is unaffected.
    assert!(
        budget
            .preflight(&generation, [2; 32], Some(2048), 4096, false)
            .is_ok(),
        "an over-cap archive tally must not block ordinary accounted intent writes"
    );
    // The exact archive retry stays available, so an uncertain write is still repeatable.
    assert!(
        budget
            .preflight_draft_archive(&generation, [1; 32], Some(over), over, true)
            .is_ok(),
        "a sync-only archive retry must remain possible over the cap"
    );
    // Growth is what is refused.
    assert!(
        budget
            .preflight_draft_archive(&generation, [3; 32], None, 1024, false)
            .is_err(),
        "a new archive must be refused while the tally is over the cap"
    );
    // Reduction remains possible, which is the route back under the policy: releasing an
    // archive subtracts its bytes rather than claiming any.
    budget.commit_draft_archive([1; 32], Some(over), 0);
    assert_eq!(budget.archive_bytes, 0);
    assert!(
        budget
            .preflight_draft_archive(&generation, [3; 32], None, 1024, false)
            .is_ok(),
        "once reduced under the cap, a new archive must be admitted again"
    );
}

impl ServerStore {
    /// Test the shared writer's two dispositions with the same actual recorded envelopes.
    /// This bypasses only plan selection; production callers remain the typed coordinators.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn retire_overlay_mixture_for_test(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        included: &BTreeMap<[u8; 32], LocalIntent>,
        manual: bool,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<(), AppError> {
        self.retire_included_with_io(
            server,
            document,
            included,
            manual,
            rng,
            budget,
            intents,
            |m: &EpochMutation<'_>, p: &Path, b: &[u8]| m.write(p, b),
            |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_intent(p, b),
        )
    }
    /// Store-internal ordering seam, not an arbitrary-id removal API. The caller must keep the
    /// exclusive store borrow from source verification through successor selection. A retry of
    /// an already installed successor MUST skip this step: its intents belong to newer work.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn retire_registry_intents_with_io(
        &mut self,
        server: u64,
        plan: &RegistrySettlementPlan,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&EpochMutation<'_>, &Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&EpochMutation<'_>, &Path, u64) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        self.retire_included_with_io(
            server,
            &plan.receipt().document,
            plan.included_operations(),
            false,
            rng,
            budget,
            intents,
            writer,
            sync,
        )
    }

    /// Typed Studio counterpart of Registry retirement. Only a verified settlement plan may
    /// select envelopes; callers cannot supply arbitrary ids to the common private writer.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn retire_studio_intents_with_io(
        &mut self,
        server: u64,
        plan: &catcoms_replication::studio::StudioSettlementPlan,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&EpochMutation<'_>, &Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&EpochMutation<'_>, &Path, u64) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        self.retire_included_with_io(
            server,
            &plan.receipt().document,
            plan.included_operations(),
            false,
            rng,
            budget,
            intents,
            writer,
            sync,
        )
    }

    /// Only Studio's recovery-first disposition transaction may supply these exact checked
    /// envelopes. This is separate from receipt-covered retirement and never reports finality.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn write_studio_manual_recovery_disposition_with_io(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        recovered: &BTreeMap<[u8; 32], catcoms_replication::LocalIntent>,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&EpochMutation<'_>, &Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&EpochMutation<'_>, &Path, u64) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        self.retire_included_with_io(
            server, document, recovered, true, rng, budget, intents, writer, sync,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn retire_included_with_io(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        included: &BTreeMap<[u8; 32], catcoms_replication::LocalIntent>,
        manual_recovery: bool,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&EpochMutation<'_>, &Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&EpochMutation<'_>, &Path, u64) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let scope = scope_bytes(server, document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        // Retirement needs the ledger and overlay id membership, never a projection.
        let (mut state, old) = match self.read_epoch_intent_record_structural(&scope, document) {
            Ok(value) => value,
            Err(error) => {
                budget.invalidate();
                intents.ready = false;
                return Err(error);
            }
        };
        let id = *blake3::hash(&scope).as_bytes();
        if state.handoff_prepared() {
            return Err(invalid("overlay handoff must resolve before retirement"));
        }
        let observed = old
            .map(|n| storage_record(server, document, &scope, n))
            .transpose()?;
        if let Err(error) = budget.verify_record(&storage_scope, id, observed) {
            intents.ready = false;
            return Err(invalid(error));
        }
        // A derived id binds author + nonce, NOT body. A held conflicting envelope is not
        // proof that this local intent was receipted; fail without deleting it or the source.
        for (id, held) in state.pending() {
            if included.get(id).is_some_and(|included| included != held) {
                return Err(invalid(if manual_recovery {
                    "recovery intent envelope conflicts with local replay data"
                } else {
                    "receipted intent envelope conflicts with local replay data"
                }));
            }
        }
        // Foundation hold: only ordinary entries may retire, even in a mixed disposition.
        // Retain complete annotated envelopes and their ordering/base evidence unconditionally.
        let ids = included
            .keys()
            .filter(|id| !state.is_overlay(id))
            .copied()
            .collect();
        let removed = if manual_recovery {
            state.ledger.remove_to_manual_recovery(&ids)
        } else {
            state.ledger.remove_receipted(&ids)
        };
        if removed == 0 {
            intents.preflight(&self.intent_generation, id, old, old.unwrap_or(0), true)?;
            if let Some(record) = observed {
                // Exact retries may see a successful rename whose directory sync failed.
                // Flush the actual file, including excluded/unaccepted intents; never reseed it.
                let reservation = budget
                    .reserve_sync(&storage_scope, record)
                    .map_err(invalid)?;
                intents.ready = false;
                self.intent_generation = Arc::new(());
                // I-4: the zero-removal exact retry is a mutation for inventory purposes.
                let path = self.epoch_intent_path(&scope);
                let bytes = old.expect("observed file");
                let mutation = self.epoch_mutation_guard();
                sync(&mutation, &path, bytes)?;
                reservation.commit();
                intents.generation = self.intent_generation.clone();
                intents.ready = true;
            }
            return Ok(());
        }
        let plain = state.encode(&scope)?;
        let next = plain.len() as u64 + 40;
        // Even a shrinking ledger requires a full physical replacement copy. Refuse safely at
        // the vault cap; do not spend deletion credit before the filesystem barrier succeeds.
        let final_bytes = intents.preflight(&self.intent_generation, id, old, next, false)?;
        let record = storage_record(server, document, &scope, next)?;
        let reservation = budget
            .reserve(
                &storage_scope,
                Replacement {
                    record,
                    scratch_bytes: 0,
                    purpose: WritePurpose::Settlement,
                },
            )
            .map_err(invalid)?;
        let sealed = match self.keys.db_key().and_then(|key| seal(&key, &plain, rng)) {
            Ok(sealed) => sealed,
            Err(error) => {
                reservation.cancel_before_write();
                return Err(error.into());
            }
        };
        // Poison both inventories BEFORE I/O, including unwinds. A later scan must observe the
        // actual old/new file and every temporary sibling rather than trusting early credit.
        intents.ready = false;
        self.intent_generation = Arc::new(());
        // I-4: path first, then rotate, then touch disk.
        let path = self.epoch_intent_path(&scope);
        let framed = frame(&sealed);
        let mutation = self.epoch_mutation_guard();
        writer(&mutation, &path, &framed)?;
        reservation.commit();
        intents.records.insert(id, next);
        intents.bytes = final_bytes;
        intents.generation = self.intent_generation.clone();
        intents.ready = true;
        Ok(())
    }
}
