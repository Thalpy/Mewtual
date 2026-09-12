//! Quiet/solo owner work shares the existing native idle worker and detached source pool.
//! A peer request is not needed to rotate or to make an installed checkpoint discoverable.
use super::*;
use crate::store::StudioRotationOutcome;

impl CatchupRuntime {
    pub(super) fn rotate_owner<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        watches: &VecDeque<(ServerStudioWatch, u128)>,
    ) -> Result<Option<StudioTarget>, AppError> {
        let now = server.runtime_clock().monotonic_ms();
        if now < self.owner_next_at || watches.is_empty() {
            return Ok(None);
        }
        let Some(snapshot) = self.owner_snapshot.clone() else {
            return Ok(None);
        };
        if !server.owner_head_snapshot_is_current(store, id, &snapshot) {
            return Ok(None);
        }
        let target = self
            .owner_target
            .filter(|t| watches.iter().any(|(w, _)| w.target == *t))
            .unwrap_or(watches[self.owner_selection % watches.len()].0.target);
        self.owner_target = Some(target);
        if !self.prepare(server, store, id, target)? {
            // Captured, busy or superseded work must all pay the same local cadence. The
            // detached result may attach immediately, but another attempt waits this deadline.
            self.owner_next_at = now.saturating_add(5_000);
            return Ok(None);
        }
        // This target's Registry bucket participates in the same all-family inventory. After
        // restart it can be large and cold even though the Studio source itself is warm. Use
        // the existing detached Registry preparation before attempting owner accounting, just
        // as the ordinary Studio tail path does; unrelated cold records keep their work rail.
        let logical = target.document(&server.group_id()).map_err(invalid)?;
        let bucket =
            catcoms_replication::registry::PointerKey::new(logical.doc_type, logical.logical_key)
                .map_err(invalid)?
                .bucket();
        if !store.registry_receive_source_fits(id, &server.group_id(), bucket)?
            && !self.prepare_registry_inventory(server, store, id, bucket)?
        {
            self.owner_next_at = now.saturating_add(5_000);
            return Ok(None);
        }
        self.owner_target = None;
        self.owner_selection = self.owner_selection.wrapping_add(1);
        // A single local pacing rail, independent of inbound traffic, errors or membership
        // churn. A close is constructed only after cheap admitted counters say it may qualify.
        self.owner_next_at = now.saturating_add(5_000);
        let mut budget = Self::inventory_budget(server, store, id)?;
        let needed = server.sync.with_registry_context(|g, d, _, _| {
            store.studio_owner_rotation_needed(id, g, target, d, &mut budget)
        })?;
        if !needed {
            return Ok(None);
        }
        let result = server.rotate_studio_owner_step(store, id, target, &snapshot, &mut budget);
        // A failed later write can leave a durably sealed source. Do not infer its phase or
        // label every refusal as disk-full; request a fresh view even on the error path.
        self.settlement
            .note(target, StudioSettlementState::RefreshRequired);
        let (outcome, state) = match result {
            Ok(value) => value,
            Err(error) => {
                // Known capacity/closure/succession refusals must not pause unrelated receive.
                // Retain only bounded diagnostics; retry uses fresh source and accounting.
                self.owner_failure = Some((target, error.to_string().chars().take(256).collect()));
                return Ok(None);
            }
        };
        let epoch = state.doc_id();
        self.settlement.note(target, state.phase().into());
        if outcome == StudioRotationOutcome::RecoveryPending {
            self.settlement
                .note(target, StudioSettlementState::RecoveryEvictionPending);
        }
        server
            .sync
            .with_registry_context(|g, d, _, _| store.retain_studio_source(g, d, state));
        self.binding = Some((target, epoch));
        if matches!(
            outcome,
            StudioRotationOutcome::Installed { .. }
                | StudioRotationOutcome::AlreadyInstalled { .. }
        ) {
            if let Err(error) =
                server.complete_studio_owner_availability(store, id, target, &snapshot, &mut budget)
            {
                self.owner_failure = Some((target, error.to_string().chars().take(256).collect()));
            } else {
                self.owner_failure = None;
            }
        }
        // The saved phase/epoch changes even if availability completion fails. Rebind only to
        // this actually installed source; the old packet/page generations are then revoked.
        Ok(Some(target))
    }
}
