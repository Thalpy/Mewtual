//! Bounded manual recovery is a disposition, not finality. Reuse the existing accounted
//! recovery record and intent replacement; do not introduce a second ledger or receipt rule.
use super::*;
use catcoms_replication::{studio::StudioRecovery, LocalIntent};
use std::collections::{BTreeMap, BTreeSet};

impl ServerStore {
    /// Internal post-replay batch. The caller names saved ids, never supplies operation bodies.
    /// Every selected own envelope must still occur in actual retained/staged recovery and
    /// must NOT occur in the current signed log. Holding the store borrow spans both barriers.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn move_studio_intents_to_recovery(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        expected_epoch: u128,
        ids: &BTreeSet<[u8; 32]>,
        clock: &dyn catcoms_rt::Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<usize, AppError> {
        self.move_studio_intents_to_recovery_with_io(
            server,
            group,
            target,
            device,
            expected_epoch,
            ids,
            clock,
            rng,
            budget,
            &mut |_, path, bytes| atomic_write(path, bytes),
            &mut super::super::epoch_intents::sync_intent,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn move_studio_intents_to_recovery_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        expected_epoch: u128,
        ids: &BTreeSet<[u8; 32]>,
        clock: &dyn catcoms_rt::Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        write: &mut impl FnMut(bool, &Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(&Path, u64) -> Result<(), AppError>,
    ) -> Result<usize, AppError> {
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        if ids.len() > catcoms_replication::epoch::MAX_INTENTS_PER_DOCUMENT {
            return Err(invalid("manual recovery batch exceeds intent bound"));
        }
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        let recovery = self.load_epoch_recovery(server, &logical)?;
        let typed = recovery
            .retained()
            .chain(recovery.staged())
            .map(|s| StudioRecovery::from_snapshot(s, &logical, target.channel()).map_err(invalid))
            .collect::<Result<Vec<_>, _>>()?;
        let pending = self.load_epoch_intents(server, &logical)?;
        let mut selected = BTreeMap::<[u8; 32], LocalIntent>::new();
        for (id, intent) in pending.pending().filter(|(id, _)| ids.contains(*id)) {
            if intent.author != device.device_id() {
                return Err(invalid("cannot dispose another device's intent"));
            }
            if !typed.iter().any(|r| r.operations().get(id) == Some(intent)) {
                return Err(invalid("manual recovery needs the complete saved envelope"));
            }
            selected.insert(*id, intent.clone());
        }
        // No current-log edit can be archived just because an older snapshot happens to hold
        // the same id/body. The ordinary Save/receipt path owns all newly replayed operations.
        self.with_studio_source(server, group, target, device, |state| {
            if state.doc_id() != expected_epoch || state.phase() != EpochPhase::Open {
                return Err(invalid("manual recovery requires the selected Open epoch"));
            }
            let current = state.current_operations()?;
            for id in selected.keys() {
                // A conflicting body at the same derived id is a refusal too, never a way
                // to erase local evidence while a different signed envelope stays current.
                if current.contains_key(id) {
                    return Err(invalid(
                        "current signed-log intents cannot move to manual recovery",
                    ));
                }
            }
            let scope = StorageScope::new(server, &logical.server_id).map_err(invalid)?;
            let record = state
                .source
                .as_ref()
                .ok_or_else(|| invalid("missing current source record"))?
                .record();
            budget
                .storage
                .verify_record(&scope, record.id, Some(record))
                .map_err(invalid)
        })?
        .ok_or_else(|| invalid("manual recovery needs an installed source"))?;
        if selected.is_empty() {
            // An earlier ledger rename may have succeeded but failed its directory flush.
            // Flush the actual ledger on an exact empty retry; never manufacture a new file
            // or report this as receipt finality. No recovery evidence authorizes new removal.
            self.write_studio_manual_recovery_disposition_with_io(
                server,
                &logical,
                &selected,
                rng,
                &mut budget.storage,
                &mut budget.intents,
                |p, b| write(false, p, b),
                sync,
            )?;
            return Ok(0);
        }
        // Re-stage an EXACT existing snapshot: this rewrites/flushes the complete recovery
        // record without changing its slots, warning pair or deadline. One flush covers every
        // selected envelope across the at-most-three typed slots, including restart retries.
        let saved = recovery
            .retained()
            .next()
            .or(recovery.staged())
            .ok_or_else(|| invalid("manual recovery is absent"))?
            .clone();
        self.update_epoch_recovery_accounted_with_writer(
            server,
            &logical,
            EpochRecoveryAction::Stage(saved),
            clock,
            rng,
            &mut budget.storage,
            |p, b| write(true, p, b),
        )?;
        self.write_studio_manual_recovery_disposition_with_io(
            server,
            &logical,
            &selected,
            rng,
            &mut budget.storage,
            &mut budget.intents,
            |p, b| write(false, p, b),
            sync,
        )?;
        Ok(selected.len())
    }
}
