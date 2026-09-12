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

impl ServerStore {
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
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
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
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
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
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
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
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let scope = scope_bytes(server, document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let (mut state, old) = match self.read_epoch_intent_record(&scope, document) {
            Ok(value) => value,
            Err(error) => {
                budget.invalidate();
                intents.ready = false;
                return Err(error);
            }
        };
        let id = *blake3::hash(&scope).as_bytes();
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
        let ids = included.keys().copied().collect();
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
                sync(&self.epoch_intent_path(&scope), old.expect("observed file"))?;
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
        writer(&self.epoch_intent_path(&scope), &frame(&sealed))?;
        reservation.commit();
        intents.records.insert(id, next);
        intents.bytes = final_bytes;
        intents.generation = self.intent_generation.clone();
        intents.ready = true;
        Ok(())
    }
}
