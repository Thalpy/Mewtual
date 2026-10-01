//! The repair step (design 5.7, 10.3). Only this runtime holds the durable owner snapshot, so the
//! explicit decision reaches issuance here, and the owner resumes a persisted decision after a
//! crash in the same slot as rotation, after discovery, seed and page work. One job per turn,
//! round-robin over watched targets, with a 5 s cadence that backs off to 60 s on any hold.
use super::*;
use crate::store::{StudioRepairOutcome, StudioRepairRequest};
use crate::studio::StudioFaultScope;

impl CatchupRuntime {
    /// The visible exit (Flow X): `Repairing` has exactly this producer, after the application
    /// barrier, while a replacement or hold is owed. Terminal outcomes report the saved phase.
    fn note_repair(
        &mut self,
        target: StudioTarget,
        outcome: StudioRepairOutcome,
        phase: EpochPhase,
    ) {
        match outcome {
            StudioRepairOutcome::AwaitingSeed | StudioRepairOutcome::Held(_) => {
                self.settlement
                    .note(target, StudioSettlementState::Repairing);
            }
            StudioRepairOutcome::RecoveryPending => {
                self.settlement
                    .note(target, StudioSettlementState::Repairing);
                self.settlement
                    .note(target, StudioSettlementState::RecoveryEvictionPending);
            }
            StudioRepairOutcome::StorageRefused => {
                self.settlement
                    .note(target, StudioSettlementState::StorageRefused);
            }
            StudioRepairOutcome::Repaired
            | StudioRepairOutcome::Screened
            | StudioRepairOutcome::Installed
            | StudioRepairOutcome::AlreadyRepaired => self.settlement.note(target, phase.into()),
        }
    }

    /// The owner's explicit `RepairFault`. Without a current durable snapshot this device is
    /// not provably the owner, and it refuses rather than signing on in-memory tenure.
    pub(in crate::studio::receiver) fn repair_fault<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        request: StudioRepairRequest,
    ) -> Result<StudioControlResponse, AppError> {
        let snapshot = self.owner_snapshot.clone().ok_or_else(|| {
            invalid("only the current owner, with a durable snapshot, may decide")
        })?;
        if !server.owner_head_snapshot_is_current(store, id, &snapshot) {
            return Err(invalid("the durable owner snapshot is stale; retry"));
        }
        let mut budget = Self::inventory_budget(server, store, id)?;
        // Even an error may follow B1 or B2: request a fresh view before any label.
        self.settlement
            .note(target, StudioSettlementState::RefreshRequired);
        let (_, outcome, state) = server.issue_studio_fault_repair(
            store,
            id,
            target,
            &snapshot,
            request,
            None,
            &mut budget,
        )?;
        let phase = state.phase();
        server
            .sync
            .with_registry_context(|g, d, _, _| store.retain_studio_source(g, d, state));
        self.note_repair(target, outcome, phase);
        Ok(StudioControlResponse::Repaired {
            target,
            scope: StudioFaultScope::Source,
            outcome,
        })
    }

    /// The owner's explicit decision for the target's Registry bucket. The bucket's prepared
    /// provider is dropped afterwards: any write invalidates a read-only prepared wrapper.
    pub(in crate::studio::receiver) fn repair_registry_fault<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        request: StudioRepairRequest,
    ) -> Result<StudioControlResponse, AppError> {
        let snapshot = self.owner_snapshot.clone().ok_or_else(|| {
            invalid("only the current owner, with a durable snapshot, may decide")
        })?;
        if !server.owner_head_snapshot_is_current(store, id, &snapshot) {
            return Err(invalid("the durable owner snapshot is stale; retry"));
        }
        let bucket = server.studio_registry_bucket(target)?;
        let mut budget = Self::inventory_budget(server, store, id)?;
        self.settlement
            .note(target, StudioSettlementState::RefreshRequired);
        self.registry_provider = None;
        let (_, outcome, _) = server.issue_registry_fault_repair(
            store,
            id,
            target,
            &snapshot,
            request,
            None,
            &mut budget,
        )?;
        Ok(StudioControlResponse::Repaired {
            target,
            scope: StudioFaultScope::RegistryBucket(bucket),
            outcome,
        })
    }

    /// Flow D: an authenticated answer carried a repair. A peer applies it whatever its own
    /// fault status; which case it lands in is the core's classification alone. The owner never
    /// re-applies a decision from an answer: its own are resumed by `repair_owner`. Failures are
    /// bounded diagnostics, never a reason to stop discovery for other targets.
    pub(super) fn apply_offered_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        repair: &catcoms_replication::ReceiptRepair,
        offered: Option<&catcoms_replication::Receipt>,
    ) -> Result<Option<StudioRepairOutcome>, AppError> {
        let owner = server
            .sync
            .with_registry_context(|g, d, _, _| g.designated_committer() == Some(d.device_id()));
        if owner {
            return Ok(None);
        }
        let pair = server.sync.with_registry_context(|g, d, _, _| {
            store.studio_repair_evidence(id, g, target, d, repair, offered)
        });
        let pair = match pair {
            Ok(Some(pair)) => pair,
            Ok(None) => return Ok(None),
            Err(error) => {
                self.repair_failure = Some((target, error.to_string().chars().take(256).collect()));
                return Ok(None);
            }
        };
        if !self.prepare(server, store, id, target)? {
            return Ok(None);
        }
        let mut budget = Self::inventory_budget(server, store, id)?;
        self.settlement
            .note(target, StudioSettlementState::RefreshRequired);
        match server.apply_studio_fault_repair(store, id, target, repair, &pair, None, &mut budget)
        {
            Ok((outcome, state)) => {
                let phase = state.phase();
                server
                    .sync
                    .with_registry_context(|g, d, _, _| store.retain_studio_source(g, d, state));
                self.note_repair(target, outcome, phase);
                self.repair_failure = None;
                Ok(Some(outcome))
            }
            Err(error) => {
                self.repair_failure = Some((target, error.to_string().chars().take(256).collect()));
                Ok(None)
            }
        }
    }

    /// Flow D for a Registry bucket: the same rules as a Studio source. A faulted bucket blocks
    /// discovery, so the repair an owner's answer carries is applied here before anything else.
    pub(super) fn apply_offered_registry_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        bucket: u8,
        repair: &catcoms_replication::ReceiptRepair,
        offered: Option<&catcoms_replication::Receipt>,
    ) -> Result<Option<StudioRepairOutcome>, AppError> {
        let owner = server
            .sync
            .with_registry_context(|g, d, _, _| g.designated_committer() == Some(d.device_id()));
        if owner {
            return Ok(None);
        }
        let pair = server.sync.with_registry_context(|g, d, _, _| {
            store.registry_repair_evidence(id, g, bucket, d, repair, offered)
        });
        let Ok(Some(pair)) = pair else {
            return Ok(None);
        };
        let mut budget = Self::inventory_budget(server, store, id)?;
        self.registry_provider = None;
        match server.apply_registry_bucket_repair(
            store,
            id,
            bucket,
            repair,
            &pair,
            None,
            &mut budget,
        ) {
            Ok((outcome, _)) => Ok(Some(outcome)),
            Err(error) => {
                if let Some(target) = self.target {
                    self.repair_failure =
                        Some((target, error.to_string().chars().take(256).collect()));
                }
                Ok(None)
            }
        }
    }

    /// The owner resumes a held Registry decision for the bucket behind `target`. The explicit
    /// decision itself is never made here: only one already persisted at B1 is continued.
    pub(super) fn resume_registry_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        bucket: u8,
    ) -> Result<bool, AppError> {
        let Some(snapshot) = self.owner_snapshot.clone() else {
            return Ok(false);
        };
        if !server.owner_head_snapshot_is_current(store, id, &snapshot) {
            return Ok(false);
        }
        let held = server
            .sync
            .with_registry_context(|g, d, _, _| store.held_registry_repair(id, g, bucket, d));
        let Ok(Some((repair, pair))) = held else {
            return Ok(false);
        };
        let mut budget = Self::inventory_budget(server, store, id)?;
        self.registry_provider = None;
        if let Err(error) = server.resume_registry_fault_repair(
            store,
            id,
            target,
            &snapshot,
            &repair,
            &pair,
            None,
            &mut budget,
        ) {
            self.repair_failure = Some((target, error.to_string().chars().take(256).collect()));
        }
        Ok(true)
    }

    /// Resume a persisted owner decision. A held B1 decision owns its target until terminal, so
    /// leaving it unresumed after a restart would strand the fault; this step is that resume.
    pub(super) fn repair_owner<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        watches: &VecDeque<(ServerStudioWatch, u128)>,
    ) -> Result<Option<StudioTarget>, AppError> {
        let now = server.runtime_clock().monotonic_ms();
        if now < self.repair_next_at || watches.is_empty() {
            return Ok(None);
        }
        let Some(snapshot) = self.owner_snapshot.clone() else {
            return Ok(None);
        };
        if !server.owner_head_snapshot_is_current(store, id, &snapshot) {
            return Ok(None);
        }
        let target = watches[self.repair_selection % watches.len()].0.target;
        self.repair_selection = self.repair_selection.wrapping_add(1);
        self.repair_next_at = now.saturating_add(5_000);
        let held = server
            .sync
            .with_registry_context(|g, d, _, _| store.held_studio_repair(id, g, target, d));
        let (repair, pair) = match held {
            Ok(Some(held)) => held,
            Ok(None) => return Ok(None),
            Err(error) => {
                self.repair_failure = Some((target, error.to_string().chars().take(256).collect()));
                self.repair_next_at = now.saturating_add(60_000);
                return Ok(None);
            }
        };
        if !self.prepare(server, store, id, target)? {
            return Ok(None);
        }
        let mut budget = Self::inventory_budget(server, store, id)?;
        self.settlement
            .note(target, StudioSettlementState::RefreshRequired);
        match server.resume_studio_fault_repair(
            store,
            id,
            target,
            &snapshot,
            &repair,
            &pair,
            None,
            &mut budget,
        ) {
            Ok((outcome, state)) => {
                let phase = state.phase();
                server
                    .sync
                    .with_registry_context(|g, d, _, _| store.retain_studio_source(g, d, state));
                self.note_repair(target, outcome, phase);
                self.repair_failure = None;
                if !outcome.is_terminal() {
                    self.repair_next_at = now.saturating_add(60_000);
                }
                Ok(Some(target))
            }
            Err(error) => {
                self.repair_failure = Some((target, error.to_string().chars().take(256).collect()));
                self.repair_next_at = now.saturating_add(60_000);
                Ok(None)
            }
        }
    }
}
