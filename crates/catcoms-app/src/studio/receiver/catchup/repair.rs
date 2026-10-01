//! The repair step (design 5.7, 10.3). Only this runtime holds the durable owner snapshot, so the
//! explicit decision reaches issuance here, and the owner resumes a persisted decision after a
//! crash in the same slot as rotation, after discovery, seed and page work. One job per turn,
//! round-robin over watched targets, with a 5 s cadence that backs off to 60 s on any hold. A
//! hold is always a per-target wait: nothing here returns an error that would pause catch-up.
use super::*;
use crate::store::{OfferedRepairEvidence, StudioRepairOutcome, StudioRepairRequest};
use crate::studio::StudioFaultScope;
use catcoms_replication::{Receipt, ReceiptRepair};

/// Remembered terminal Registry repairs, bounded; forgetting one only costs a reload.
const MAX_REMEMBERED_REGISTRY_REPAIRS: usize = 64;

impl CatchupRuntime {
    /// The visible exit (Flow X): `Repairing` has exactly this producer, after the application
    /// barrier, while a replacement is owed. A hold changed nothing, so it reports the saved
    /// phase like a terminal outcome does; before B2 a Fault therefore stays Fault.
    fn note_repair(
        &mut self,
        target: StudioTarget,
        outcome: StudioRepairOutcome,
        phase: EpochPhase,
    ) {
        match outcome {
            StudioRepairOutcome::AwaitingSeed => {
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
            StudioRepairOutcome::Held(_)
            | StudioRepairOutcome::Repaired
            | StudioRepairOutcome::Screened
            | StudioRepairOutcome::Installed
            | StudioRepairOutcome::AlreadyRepaired => self.settlement.note(target, phase.into()),
        }
    }

    fn note_repair_failure(&mut self, target: StudioTarget, error: &AppError) {
        // The same bounded, surfaced diagnostic slot as owner rotation.
        self.owner_failure = Some((target, error.to_string().chars().take(256).collect()));
    }

    fn remember_registry_repair(&mut self, bucket: u8, repair: &ReceiptRepair) {
        if self.registry_repairs_seen.len() >= MAX_REMEMBERED_REGISTRY_REPAIRS {
            self.registry_repairs_seen.clear();
        }
        self.registry_repairs_seen.insert((bucket, repair.hash()));
    }

    /// A repair that owes its replacement needs the selected checkpoint's seed. No fresh owner
    /// proof will name that receipt while a decision is held or after the owner moved on, so
    /// mint the seed pass from the locally verified repair and let the existing checkpoint
    /// machinery fetch it. `route_checkpoint_install` then installs it through the repair itself.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn await_repaired_seed<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
        target: CheckpointTarget,
        failure_target: StudioTarget,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
    ) {
        if self.checkpoint.is_some() {
            // One checkpoint pass at a time; the next resume turn asks again.
            return;
        }
        let Some(selected) = pair
            .iter()
            .find(|r| r.hash() == repair.selected_receipt_hash)
        else {
            return;
        };
        // Any member that installed the selected checkpoint can serve its seed, the owner
        // included; rotate so one peer without it cannot be asked forever.
        let peers = server.sync.studio_page_peers();
        if peers.is_empty() {
            return;
        }
        let peer = peers[self.repair_seed_peer % peers.len()];
        self.repair_seed_peer = self.repair_seed_peer.wrapping_add(1);
        match server.select_repaired_checkpoint(store, id, target, repair, selected) {
            Ok(pass) => {
                self.pass = None;
                self.checkpoint = Some(pass);
                self.checkpoint_sealed = true;
                self.checkpoint_peer = Some(peer);
                self.checkpoint_retry = 0;
            }
            Err(error) => self.note_repair_failure(failure_target, &error),
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
        let (repair, outcome, state) = server.issue_studio_fault_repair(
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
        if outcome == StudioRepairOutcome::AwaitingSeed {
            let held = server
                .sync
                .with_registry_context(|g, d, _, _| store.held_studio_repair(id, g, target, d));
            if let Ok(Some((_, pair))) = held {
                self.await_repaired_seed(
                    server,
                    store,
                    id,
                    CheckpointTarget::Studio(target),
                    target,
                    &repair,
                    &pair,
                );
            }
        }
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
        let (repair, outcome, _) = server.issue_registry_fault_repair(
            store,
            id,
            target,
            &snapshot,
            request,
            None,
            &mut budget,
        )?;
        if outcome == StudioRepairOutcome::AwaitingSeed {
            let held = server
                .sync
                .with_registry_context(|g, d, _, _| store.held_registry_repair(id, g, bucket, d));
            if let Ok(Some((_, pair))) = held {
                self.await_repaired_seed(
                    server,
                    store,
                    id,
                    CheckpointTarget::Registry(bucket),
                    target,
                    &repair,
                    &pair,
                );
            }
        }
        Ok(StudioControlResponse::Repaired {
            target,
            scope: StudioFaultScope::RegistryBucket(bucket),
            outcome,
        })
    }

    /// Flow D: an authenticated answer carried a repair. A peer applies it whatever its own
    /// fault status; which case it lands in is the core's classification alone. The owner never
    /// re-applies a decision from an answer: its own are resumed by `repair_owner`. The source is
    /// prepared through the detached pool first, so evidence is read warm, never rebuilt here.
    /// The caller decides what an `AwaitingSeed` needs: a proof pass for the selected receipt
    /// already supplies its seed, so a repaired pass is minted only when none does.
    pub(super) fn apply_offered_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        repair: &ReceiptRepair,
        offered: Option<&Receipt>,
    ) -> Result<Option<(StudioRepairOutcome, [Receipt; 2])>, AppError> {
        let owner = server
            .sync
            .with_registry_context(|g, d, _, _| g.designated_committer() == Some(d.device_id()));
        if owner || !self.prepare(server, store, id, target)? {
            return Ok(None);
        }
        let Some(pair) = server.sync.with_registry_context(|g, d, _, _| {
            store.studio_repair_evidence(id, g, target, d, repair, offered)
        }) else {
            return Ok(None);
        };
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
                Ok(Some((outcome, pair)))
            }
            Err(error) => {
                self.note_repair_failure(target, &error);
                Ok(None)
            }
        }
    }

    /// Flow D for a Registry bucket: the same rules as a Studio source. A faulted bucket blocks
    /// discovery, so the repair an owner's answer carries is applied here before anything else.
    /// The bucket's own detached preparation runs first when its source is large.
    pub(super) fn apply_offered_registry_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        bucket: u8,
        repair: &ReceiptRepair,
        offered: Option<&Receipt>,
    ) -> Result<Option<(StudioRepairOutcome, [Receipt; 2])>, AppError> {
        let owner = server
            .sync
            .with_registry_context(|g, d, _, _| g.designated_committer() == Some(d.device_id()));
        if owner
            || self
                .registry_repairs_seen
                .contains(&(bucket, repair.hash()))
            || (!store.registry_receive_source_fits(id, &server.group_id(), bucket)?
                && !self.prepare_registry_inventory(server, store, id, bucket)?)
        {
            return Ok(None);
        }
        let pair = server.sync.with_registry_context(|g, d, _, _| {
            store.registry_repair_evidence(id, g, bucket, d, repair, offered)
        });
        let pair = match pair {
            Ok(OfferedRepairEvidence::Pair(pair)) => *pair,
            // Terminal here: remember it so later answers carrying the same repair cost no
            // further Registry restores on the actor. Unverifiable may change, so it is not.
            Ok(OfferedRepairEvidence::Terminal) => {
                self.remember_registry_repair(bucket, repair);
                return Ok(None);
            }
            Ok(OfferedRepairEvidence::Unverifiable) => return Ok(None),
            Err(error) => {
                if let Some(target) = self.target {
                    self.note_repair_failure(target, &error);
                }
                return Ok(None);
            }
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
            Ok((outcome, _)) => {
                if outcome.is_terminal() {
                    self.remember_registry_repair(bucket, repair);
                }
                Ok(Some((outcome, pair)))
            }
            Err(error) => {
                if let Some(target) = self.target {
                    self.note_repair_failure(target, &error);
                }
                Ok(None)
            }
        }
    }

    /// The owner resumes a held Registry decision for the bucket behind `target`. The explicit
    /// decision itself is never made here: only one already persisted at B1 is continued. Paced
    /// like the Studio step: a hold or failure backs off to 60 s instead of every Registry turn.
    pub(super) fn resume_registry_repair<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
        bucket: u8,
    ) -> Result<bool, AppError> {
        let now = server.runtime_clock().monotonic_ms();
        if now < self.registry_repair_next_at {
            return Ok(false);
        }
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
        match server.resume_registry_fault_repair(
            store,
            id,
            target,
            &snapshot,
            &repair,
            &pair,
            None,
            &mut budget,
        ) {
            Ok((outcome, _)) if outcome.is_terminal() => {}
            Ok((outcome, _)) => {
                self.registry_repair_next_at = now.saturating_add(60_000);
                if outcome == StudioRepairOutcome::AwaitingSeed {
                    self.await_repaired_seed(
                        server,
                        store,
                        id,
                        CheckpointTarget::Registry(bucket),
                        target,
                        &repair,
                        &pair,
                    );
                }
            }
            Err(error) => {
                self.registry_repair_next_at = now.saturating_add(60_000);
                self.note_repair_failure(target, &error);
            }
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
                self.note_repair_failure(target, &error);
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
                if !outcome.is_terminal() {
                    self.repair_next_at = now.saturating_add(60_000);
                }
                if outcome == StudioRepairOutcome::AwaitingSeed {
                    self.await_repaired_seed(
                        server,
                        store,
                        id,
                        CheckpointTarget::Studio(target),
                        target,
                        &repair,
                        &pair,
                    );
                }
                Ok(Some(target))
            }
            Err(error) => {
                self.note_repair_failure(target, &error);
                self.repair_next_at = now.saturating_add(60_000);
                Ok(None)
            }
        }
    }

    /// Route a fetched checkpoint pass before the ordinary installer, so a repair is never an
    /// installer error that pauses all catch-up:
    /// - the source owes a repair whose selected receipt is this pass's: install through the
    ///   repair transaction itself, with typed RecoveryPending/StorageRefused and owner recycling;
    /// - the source owes a different replacement: drop this pass and fetch that repair's own
    ///   selected seed instead, from the source's committed evidence (a peer self-heals here);
    /// - a held owner decision owns the target: defer this target only;
    /// - otherwise the ordinary installer proceeds (`Ok(None)`).
    pub(super) fn route_checkpoint_install<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        failure_target: StudioTarget,
    ) -> Result<Option<Option<StudioTarget>>, AppError> {
        let Some(pass) = self.checkpoint.as_ref() else {
            return Ok(None);
        };
        let target = pass.inner.target();
        let selected = pass.inner.selected_receipt().clone();
        let now = server.runtime_clock().monotonic_ms();
        let owed = server
            .sync
            .with_registry_context(|g, d, _, _| match target {
                CheckpointTarget::Studio(studio) => Ok(store.owed_studio_repair(id, g, studio, d)),
                CheckpointTarget::Registry(bucket) => store.owed_registry_repair(id, g, bucket, d),
            });
        let owed = match owed {
            Ok(owed) => owed,
            Err(error) => {
                self.note_repair_failure(failure_target, &error);
                self.retry_discovery(now);
                return Ok(Some(None));
            }
        };
        if let Some((repair, pair)) = owed {
            if selected.hash() != repair.selected_receipt_hash {
                self.checkpoint = None;
                self.await_repaired_seed(server, store, id, target, failure_target, &repair, &pair);
                if self.checkpoint.is_none() {
                    // No seed pass (CORE-007: a previous owner's repair no longer verifies, or no
                    // peer is reachable): hold this target on the ordinary rotating cadence.
                    self.retry_discovery(now);
                }
                return Ok(Some(None));
            }
            let pass = self.checkpoint.take().expect("pass");
            let mut budget = Self::budget(server, store, id)?;
            self.settlement
                .note(failure_target, StudioSettlementState::RefreshRequired);
            let installed = match target {
                CheckpointTarget::Studio(studio) => server
                    .install_repaired_studio_seed(store, id, &pass, &repair, &pair, &mut budget)
                    .map(|(outcome, state)| {
                        let (phase, doc_id) = (state.phase(), state.doc_id());
                        server.sync.with_registry_context(|g, d, _, _| {
                            store.retain_received_studio_source(g, d, state)
                        });
                        self.note_repair(studio, outcome, phase);
                        if outcome.is_terminal() {
                            self.binding = Some((studio, doc_id));
                        }
                        outcome
                    }),
                CheckpointTarget::Registry(_) => {
                    self.registry_provider = None;
                    server
                        .install_repaired_registry_seed(
                            store,
                            id,
                            &pass,
                            &repair,
                            &pair,
                            &mut budget,
                        )
                        .map(|(outcome, _)| outcome)
                }
            };
            return Ok(Some(match installed {
                Ok(outcome) if outcome.is_terminal() => {
                    if let CheckpointTarget::Registry(_) = target {
                        self.discovery_plan = self.after_registry.take();
                        None
                    } else {
                        self.discovery_needed = None;
                        self.discovery_watch = None;
                        self.next_at = now;
                        Some(failure_target)
                    }
                }
                Ok(_) => {
                    // Recovery warning, storage refusal or a hold: everything is retained and
                    // this target retries on the ordinary discovery cadence.
                    self.retry_discovery(now);
                    None
                }
                Err(error) => {
                    self.note_repair_failure(failure_target, &error);
                    self.retry_discovery(now);
                    None
                }
            }));
        }
        let deferred = server
            .sync
            .with_registry_context(|g, d, _, _| match target {
                CheckpointTarget::Studio(studio) => {
                    store.studio_install_deferred_by_repair(id, g, studio, d, &selected)
                }
                // The bucket owes nothing (checked above), so only a held decision can defer it.
                CheckpointTarget::Registry(bucket) => {
                    store.registry_install_deferred_by_repair(id, g, bucket, None, &selected)
                }
            });
        match deferred {
            Ok(false) => Ok(None),
            Ok(true) => {
                self.retry_discovery(now);
                Ok(Some(None))
            }
            Err(error) => {
                // An unreadable owner record is not a reason to install, and not silent either.
                self.note_repair_failure(failure_target, &error);
                self.retry_discovery(now);
                Ok(Some(None))
            }
        }
    }
}
