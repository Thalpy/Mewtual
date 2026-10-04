//! The repair step (design 5.7, 10.3). Only this runtime holds the durable owner snapshot, so the
//! explicit decision reaches issuance here, and the owner resumes a persisted decision after a
//! crash in the same slot as rotation, after discovery, seed and page work. One job per turn,
//! round-robin over watched targets, with a 5 s cadence that backs off to 60 s on any hold; a
//! persistent hold on an owed replacement also stops that target's seed refetches for 60 s. A
//! hold is always a per-target wait: nothing here returns an error that would pause catch-up.
use super::*;
use crate::store::{OfferedRepairEvidence, StudioRepairOutcome, StudioRepairRequest};
use crate::studio::StudioFaultScope;
use catcoms_replication::{Receipt, ReceiptRepair};

/// Remembered terminal Registry repairs, bounded; forgetting one only costs a reload.
const MAX_REMEMBERED_REGISTRY_REPAIRS: usize = 64;
/// How long a persistent repair hold suppresses refetching that target's selected seed. The same
/// 60 s the ordinary installer waits after a recovery warning.
const REPAIR_HOLD_BACKOFF_MS: u64 = 60_000;

impl CatchupRuntime {
    /// Fail-closed integration gate for repair transactions that still run synchronously while
    /// the receiver owns Server/store custody. The store/core implementation remains available
    /// for bounded tests, but live discovery, owner resume and repaired-seed installation must not
    /// enter it until a shared-pool job owns capture, detached execution, result custody and the
    /// mount/source/generation/authority revalidation at commit.
    ///
    /// Keep this as a function rather than a public/configurable flag: unfinished repair is not a
    /// user option and must not be enabled accidentally by configuration or a renderer command.
    pub(super) fn automatic_repair_execution_ready() -> bool {
        false
    }

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

    /// A bucket pass may belong to no Studio target; its failure is then only held, not shown.
    fn note_repair_failure_for(&mut self, target: Option<StudioTarget>, error: &AppError) {
        if let Some(target) = target {
            self.note_repair_failure(target, error);
        }
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
        failure_target: Option<StudioTarget>,
        repair: &ReceiptRepair,
        pair: &[Receipt; 2],
    ) {
        let now = server.runtime_clock().monotonic_ms();
        if self.checkpoint.is_some() || self.in_flight || self.discovery_plan.is_some() {
            // One checkpoint pass at a time, and never under a pending discovery, whose
            // completion would drop it; the next resume or discovery turn asks again.
            return;
        }
        if self
            .repair_backoff
            .get(&target)
            .is_some_and(|until| now < *until)
        {
            // A persistent hold: refetching the seed would fail the same way.
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
                self.repair_failure_target = failure_target;
            }
            Err(error) => {
                self.note_repair_failure_for(failure_target, &error);
                // CORE-007 or an unobserved tenure: neither clears within a discovery turn.
                self.hold_repair(target, now);
            }
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
        if !Self::automatic_repair_execution_ready() {
            return Err(invalid(
                "fault repair awaits detached admitted runtime execution",
            ));
        }
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
                    Some(target),
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
        if !Self::automatic_repair_execution_ready() {
            return Err(invalid(
                "Registry fault repair awaits detached admitted runtime execution",
            ));
        }
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
                    Some(target),
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
        if !Self::automatic_repair_execution_ready() {
            return Ok(None);
        }
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
        if !Self::automatic_repair_execution_ready() {
            return Ok(None);
        }
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
        if !Self::automatic_repair_execution_ready() {
            return Ok(false);
        }
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
                        Some(target),
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
        if !Self::automatic_repair_execution_ready() {
            return Ok(None);
        }
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
                        Some(target),
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

    /// Stop refetching `target`'s repaired seed for a while; expired holds are dropped here, so
    /// the map stays as small as the set of targets currently held.
    fn hold_repair(&mut self, target: CheckpointTarget, now: u64) {
        self.repair_backoff.retain(|_, until| now < *until);
        self.repair_backoff
            .insert(target, now.saturating_add(REPAIR_HOLD_BACKOFF_MS));
    }

    /// Route a checkpoint pass before the ordinary installer, so a repair is never an installer
    /// error that pauses all catch-up:
    /// - the source owes a repair and this pass has fetched exactly its selected seed: install
    ///   through the repair transaction itself, with typed RecoveryPending/StorageRefused and
    ///   owner recycling (the owner only through a current durable snapshot);
    /// - the source owes a repair and this pass names another receipt, or has no seed yet: drop
    ///   it and fetch the repair's own selected seed instead, from the source's committed
    ///   evidence (a peer self-heals here);
    /// - a held owner decision owns the target: defer this target only;
    /// - otherwise the ordinary installer proceeds (`Ok(None)`).
    ///
    /// `failure_target` is where diagnostics go; a bucket pass may have none.
    pub(super) fn route_checkpoint_install<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        failure_target: Option<StudioTarget>,
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
                self.note_repair_failure_for(failure_target, &error);
                self.retry_discovery(now);
                return Ok(Some(None));
            }
        };
        if let Some((repair, pair)) = owed {
            if !Self::automatic_repair_execution_ready() {
                // Do not let the ordinary installer consume the decision's selected checkpoint.
                // The fetched pass is network-derived and disposable; durable repair/source state
                // remains untouched for the future detached job.
                self.checkpoint = None;
                self.retry_discovery(now);
                return Ok(Some(None));
            }
            let fetched = self
                .checkpoint
                .as_ref()
                .is_some_and(|pass| pass.inner.is_fetched());
            if selected.hash() != repair.selected_receipt_hash || !fetched {
                // A different head, or a proof pass with no seed yet: fetch the repair's own
                // selected seed through a repaired pass instead. That selection is made under
                // this device's authoring tenure, never under a proof's own claim (6.3).
                self.checkpoint = None;
                self.await_repaired_seed(server, store, id, target, failure_target, &repair, &pair);
                if self.checkpoint.is_none() {
                    // No seed pass (a persistent hold, CORE-007, or no reachable peer): hold
                    // this target on the ordinary rotating cadence.
                    self.retry_discovery(now);
                }
                return Ok(Some(None));
            }
            let owner = server.sync.with_registry_context(|g, d, _, _| {
                g.designated_committer() == Some(d.device_id())
            });
            // The owner installs only as a resume would: through a current durable snapshot.
            let snapshot = match self.owner_snapshot.clone() {
                Some(snapshot)
                    if owner && server.owner_head_snapshot_is_current(store, id, &snapshot) =>
                {
                    Some(snapshot)
                }
                _ if owner => {
                    let stale = invalid("the durable owner snapshot is stale; retry");
                    self.note_repair_failure_for(failure_target, &stale);
                    self.retry_discovery(now);
                    return Ok(Some(None));
                }
                _ => None,
            };
            let mut budget = Self::budget(server, store, id)?;
            let pass = self.checkpoint.take().expect("pass");
            if let Some(failure_target) = failure_target {
                self.settlement
                    .note(failure_target, StudioSettlementState::RefreshRequired);
            }
            let installed = match target {
                CheckpointTarget::Studio(studio) => server
                    .install_repaired_studio_seed(
                        store,
                        id,
                        &pass,
                        &repair,
                        &pair,
                        snapshot.as_ref(),
                        &mut budget,
                    )
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
                            snapshot.as_ref(),
                            &mut budget,
                        )
                        .map(|(outcome, _)| outcome)
                }
            };
            return Ok(Some(match installed {
                Ok(outcome) if outcome.is_terminal() => {
                    self.repair_backoff.remove(&target);
                    match target {
                        CheckpointTarget::Registry(_) => {
                            self.reset_registry_tail(server, now);
                            self.discovery_plan = self.after_registry.take();
                            None
                        }
                        CheckpointTarget::Studio(studio) => {
                            self.discovery_needed = None;
                            self.discovery_watch = None;
                            self.next_at = now;
                            Some(studio)
                        }
                    }
                }
                Ok(_) => {
                    // Recovery warning, storage refusal or a hold: everything is retained. They
                    // need the user or the owner, so the seed is not refetched for a while.
                    self.hold_repair(target, now);
                    self.retry_discovery(now);
                    None
                }
                Err(error) => {
                    self.note_repair_failure_for(failure_target, &error);
                    self.hold_repair(target, now);
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
                self.note_repair_failure_for(failure_target, &error);
                self.retry_discovery(now);
                Ok(Some(None))
            }
        }
    }
}
