//! Bounded work selection for the existing native receiver. Disk barriers use its sole lease;
//! expensive vault verification and network waits leave the actor free to serve the other peer.
use super::*;
use crate::registry_catchup::{
    ServerPreparedRegistryPageSource, ServerRegistryPagePreparation, ServerRegistryPageProvider,
};
use crate::registry_head::ServerOwnerSnapshot;
use crate::store::{PreparedStudioSource, StudioSourceCapture};
use crate::studio_exchange::discovery::{
    CheckpointDiscoveryAttempt, CheckpointDiscoveryCompletion, ServerCheckpointFetch,
};
use crate::studio_exchange::{
    ServerStudioPageProvider, ServerStudioReceive, StudioPageAttempt, StudioPageCompletion,
    StudioReceiveState,
};
use catcoms_rt::{PeerId, RequestCancellation};
use catcoms_sync::checkpoint_exchange::CheckpointTarget;
use catcoms_sync::epoch_service::{EpochServiceInterest, EpochServiceKind};
use catcoms_sync::registry_seed::{CompletedCheckpointSeed, PendingCheckpointSeed};
use tokio::sync::OwnedSemaphorePermit;
mod discovery;
mod registry;
mod registry_runtime;
mod rotation;
use discovery::DiscoveryPlan;

/// Keep failure classification across detached work. A peer's bad service key must never
/// turn into the local receiver's explicit-access-only storage pause when its worker fails.
#[derive(Clone)]
pub(crate) struct PreparationContext {
    target: StudioTarget,
    service: Option<Arc<()>>,
}
struct ServiceWork {
    interest: EpochServiceInterest,
    mount: Arc<()>,
    server: u64,
    generation: Arc<()>,
    captured: bool,
}
type PreparedStudioResult = Result<(Box<PreparedStudioSource>, OwnedSemaphorePermit), AppError>;
type PreparedRegistryResult = Result<Box<ServerPreparedRegistryPageSource>, AppError>;

/// One tracked network attempt and preparation waiter per actor. Cancellation may leave a
/// blocking worker running: it retains its shared process permit until actual completion.
/// Thus all running, queued and result-holding preparations together still occupy at most
/// four slots process-wide. A ready result retains its permit until native custody.
pub(crate) enum StudioBackgroundJob<T: MeshTransport> {
    RegistryPage(crate::registry_catchup::RegistryPageAttempt<T>),
    Page(StudioPageAttempt<T>),
    Head(CheckpointDiscoveryAttempt<T>),
    Seed(PendingCheckpointSeed<T>),
    Prepare(
        StudioSourceCapture,
        OwnedSemaphorePermit,
        PreparationContext,
    ),
    PrepareRegistry(ServerRegistryPagePreparation, Option<Arc<()>>),
}
pub(crate) enum StudioBackgroundResult {
    RegistryPage(Box<crate::registry_catchup::RegistryPageCompletion>),
    Page(Box<StudioPageCompletion>),
    Head(Box<CheckpointDiscoveryCompletion>),
    Seed(Box<CompletedCheckpointSeed>),
    Prepared(PreparationContext, PreparedStudioResult),
    PreparedRegistry(Option<Arc<()>>, PreparedRegistryResult),
    CancelledRegistry(Option<Arc<()>>),
    Cancelled {
        preparation: Option<PreparationContext>,
    },
}
impl<T: MeshTransport + 'static> StudioBackgroundJob<T> {
    #[cfg(test)]
    pub(crate) fn is_preparation_for_test(&self) -> bool {
        matches!(self, Self::Prepare(..) | Self::PrepareRegistry(..))
    }
    pub(crate) async fn run(
        self,
        mut cancellation: Option<RequestCancellation>,
    ) -> StudioBackgroundResult {
        let cancelled = match &self {
            Self::Prepare(_, _, context) => StudioBackgroundResult::Cancelled {
                preparation: Some(context.clone()),
            },
            Self::PrepareRegistry(_, generation) => {
                StudioBackgroundResult::CancelledRegistry(generation.clone())
            }
            _ => StudioBackgroundResult::Cancelled { preparation: None },
        };
        let work = async move {
            match self {
                Self::RegistryPage(attempt) => {
                    StudioBackgroundResult::RegistryPage(Box::new(attempt.fetch().await))
                }
                Self::PrepareRegistry(job, generation) => {
                    let result = job.rebuild().await.map(Box::new);
                    StudioBackgroundResult::PreparedRegistry(generation, result)
                }
                Self::Head(attempt) => {
                    StudioBackgroundResult::Head(Box::new(attempt.fetch().await))
                }
                Self::Seed(attempt) => {
                    StudioBackgroundResult::Seed(Box::new(attempt.fetch().await))
                }
                Self::Page(attempt) => {
                    StudioBackgroundResult::Page(Box::new(attempt.fetch().await))
                }
                Self::Prepare(capture, permit, context) => {
                    let result = tokio::task::spawn_blocking(move || {
                        capture.rebuild().map(|p| (Box::new(p), permit))
                    })
                    .await;
                    StudioBackgroundResult::Prepared(
                        context,
                        result.unwrap_or_else(|_| Err(invalid("Studio preparation worker failed"))),
                    )
                }
            }
        };
        tokio::select! {
            biased;
            _ = async { match cancellation.as_mut() { Some(c) => c.cancelled().await, None => std::future::pending::<()>().await } } => cancelled,
            result = work => result,
        }
    }
}

#[derive(Default)]
pub(super) struct CatchupRuntime {
    pub(super) settlement: SettlementNotices,
    provider: Option<ServerStudioPageProvider>,
    pass: Option<ServerStudioReceive>,
    target: Option<StudioTarget>,
    preparation: Option<(
        StudioSourceCapture,
        OwnedSemaphorePermit,
        PreparationContext,
    )>,
    prepared: Option<(PreparationContext, PreparedStudioResult)>,
    in_flight: bool,
    preparing: bool,
    next_at: u64,
    selection: usize,
    peers: Vec<PeerId>,
    client_turn: bool,
    service: Option<ServiceWork>,
    owner_snapshot: Option<ServerOwnerSnapshot>,
    lifecycle: Option<(Arc<()>, u64, u64)>,
    lifecycle_retry_at: u64,
    discovery_needed: Option<StudioTarget>,
    discovery_plan: Option<DiscoveryPlan>,
    after_registry: Option<DiscoveryPlan>,
    registry_attempts: u8,
    head_result: Option<Box<CheckpointDiscoveryCompletion>>,
    checkpoint: Option<ServerCheckpointFetch>,
    checkpoint_peer: Option<PeerId>,
    checkpoint_sealed: bool,
    checkpoint_retry: u64,
    binding: Option<(StudioTarget, u128)>,
    discovery_watch: Option<catcoms_sync::StudioWatch>,
    registry_provider: Option<ServerRegistryPageProvider>,
    registry_retained_until: u64,
    registry_preparation: Option<(ServerRegistryPagePreparation, Option<Arc<()>>)>,
    registry_prepared: Option<(Option<Arc<()>>, PreparedRegistryResult)>,
    owner_next_at: u64,
    owner_selection: usize,
    owner_target: Option<StudioTarget>,
    // Bounded one-at-a-time failure state, surfaced by the settlement view/event adapter.
    owner_failure: Option<(StudioTarget, String)>,
    registry_watch: Option<crate::registry_ingress::ServerRegistryWatch>,
    registry_watch_id: Option<u128>,
    registry_pass: Option<crate::registry_catchup::ServerRegistryReceive>,
    registry_target: Option<StudioTarget>,
    registry_next_at: u64,
    registry_selection: usize,
}
impl CatchupRuntime {
    /// Never evict the source of a ready/active page or checkpoint just to start replay.
    pub(super) fn replay_ready(&self) -> bool {
        !self.in_flight
            && !self.preparing
            && self.preparation.is_none()
            && self.prepared.is_none()
            && self.registry_preparation.is_none()
            && self.registry_prepared.is_none()
            && self.checkpoint.is_none()
            && self.discovery_plan.is_none()
            && self.pass.is_none()
            && self.registry_pass.is_none()
    }
    #[cfg(test)]
    pub(in crate::studio::receiver) fn hold_registry_page_for_test(
        &mut self,
        target: StudioTarget,
        pass: crate::registry_catchup::ServerRegistryReceive,
    ) {
        assert_eq!(
            pass.state(),
            crate::registry_catchup::RegistryReceiveState::PageReady
        );
        self.registry_target = Some(target);
        self.registry_pass = Some(pass);
    }
    #[cfg(test)]
    pub(in crate::studio::receiver) fn has_registry_page_for_test(&self) -> bool {
        self.registry_pass.is_some()
    }
    #[cfg(test)]
    pub(in crate::studio::receiver) fn hold_page_for_test(
        &mut self,
        target: StudioTarget,
        pass: ServerStudioReceive,
    ) {
        assert_eq!(pass.state(), StudioReceiveState::PageReady);
        self.target = Some(target);
        self.pass = Some(pass);
    }
    fn drop_service_preparation(&mut self, context: &PreparationContext) {
        if self.service.as_ref().is_some_and(|work| {
            context
                .service
                .as_ref()
                .is_some_and(|g| Arc::ptr_eq(g, &work.generation))
        }) {
            self.service = None;
        }
    }
    /// Native's existing idle pass runs even without UI watches. Save owner/MLS evidence once
    /// per local mount/epoch observation, BEFORE enabling remote interests; never serialize
    /// whole-server state once per query. A failed lifecycle save yields hints only.
    pub(super) fn lifecycle<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
    ) {
        self.expire_registry_source(server.runtime_clock().monotonic_ms());
        self.lifecycle_with(server, store, id, |server, store, id| {
            server.prepare_owner_head_snapshot(store, id)
        });
    }
    fn lifecycle_with<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
        prepare: impl FnOnce(
            &mut Server<T, R>,
            &ServerStore,
            u64,
        ) -> Result<ServerOwnerSnapshot, AppError>,
    ) {
        let epoch = server.sync.with_registry_context(|g, _, _, _| g.epoch());
        let mount = store.registry_mount();
        let same = self
            .lifecycle
            .as_ref()
            .is_some_and(|(m, s, e)| Arc::ptr_eq(m, &mount) && *s == id && *e == epoch);
        let now = server.runtime_clock().monotonic_ms();
        if same && (self.owner_snapshot.is_some() || now < self.lifecycle_retry_at) {
            return;
        }
        if !same {
            server.sync.disable_epoch_service();
            self.service = None;
            self.owner_snapshot = None;
        }
        let owner = server
            .sync
            .with_registry_context(|g, d, _, _| g.designated_committer() == Some(d.device_id()));
        if owner {
            self.owner_snapshot = prepare(server, store, id).ok();
        }
        // A transient failed flush must recover on local idle cadence. Query frequency cannot
        // accelerate this retry or mint proof before a real successful save.
        self.lifecycle_retry_at = if owner {
            now.saturating_add(30_000)
        } else {
            u64::MAX
        };
        self.lifecycle = Some((mount, id, epoch));
        server.sync.enable_epoch_service();
    }
    /// Metadata is selected/charged by sync before a disk lookup. Service errors are refusal,
    /// not corruption of a local edit; they cannot globally pause unrelated watched receive.
    fn serve<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> bool {
        let work = self.service.take().or_else(|| {
            server
                .sync
                .reserve_epoch_service_interest()
                .map(|interest| ServiceWork {
                    interest,
                    mount: store.registry_mount(),
                    server: id,
                    generation: Arc::new(()),
                    captured: false,
                })
        });
        let Some(mut work) = work else {
            return false;
        };
        if work.server != id
            || !Arc::ptr_eq(&work.mount, &store.registry_mount())
            || !server
                .sync
                .epoch_service_interest_is_current(&work.interest)
        {
            return true;
        }
        let CheckpointTarget::Studio(target) = work.interest.target() else {
            self.serve_registry(server, store, id, work);
            return true;
        };
        if !server
            .channels()
            .iter()
            .any(|c| c.id == u128::from_be_bytes(target.channel()))
        {
            return true;
        }
        let warm = server
            .sync
            .with_registry_context(|g, d, _, _| store.studio_source_is_warm(id, g, target, d));
        if work.captured
            && !warm
            && !self.preparing
            && self.preparation.is_none()
            && self.prepared.is_none()
        {
            return true;
        }
        match self.prepare_for(
            server,
            store,
            id,
            PreparationContext {
                target,
                service: Some(work.generation.clone()),
            },
        ) {
            Ok(false) => {
                if self.preparation.as_ref().is_some_and(|(_, _, c)| {
                    c.service
                        .as_ref()
                        .is_some_and(|g| Arc::ptr_eq(g, &work.generation))
                }) {
                    work.captured = true;
                }
                self.service = Some(work);
                return true;
            }
            Err(_) => return true,
            Ok(true) => {}
        }
        // Source preparation may have consumed the request lifetime. Its checked warm cache
        // can survive, but this old request cannot sign a response for a later same-key query.
        if !server
            .sync
            .epoch_service_interest_is_current(&work.interest)
        {
            return true;
        }
        if work.interest.kind() == EpochServiceKind::Page {
            if self
                .provider
                .as_ref()
                .is_none_or(|p| !server.studio_page_provider_is_current(store, id, p))
            {
                self.provider = Some(server.studio_page_provider(store, id));
            }
            let _ = server.serve_studio_page_interest(
                store,
                id,
                self.provider.as_mut().expect("provider"),
                &work.interest,
            );
        } else if let Ok(mut budget) = Self::inventory_budget(server, store, id) {
            if self
                .owner_snapshot
                .as_ref()
                .is_some_and(|s| !server.owner_head_snapshot_is_current(store, id, s))
            {
                self.owner_snapshot = None;
            }
            let _ = server.serve_studio_checkpoint_interest(
                store,
                id,
                &work.interest,
                self.owner_snapshot.as_ref(),
                &mut budget,
            );
        }
        true
    }
    pub(super) fn explicit_retry(&mut self, now: u64) {
        if !self.in_flight {
            if let Some(pass) = &mut self.pass {
                pass.retry();
            }
        }
        self.next_at = now;
        // Registry publication is paced idle maintenance, not part of the Read/Save result.
        // Do not turn the first local watch into immediate work on every quiet native pass.
        if self.registry_next_at == 0 {
            self.registry_next_at = now.saturating_add(5_000);
        }
    }
    pub(super) fn pending<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
        watches: &VecDeque<(ServerStudioWatch, u128)>,
    ) -> bool {
        if self.prepared.is_some() || self.registry_prepared.is_some() {
            return true;
        }
        if self.head_result.is_some() {
            return true;
        }
        if server.sync.has_epoch_service_interest()
            || self
                .service
                .as_ref()
                .is_some_and(|s| server.sync.epoch_service_interest_is_current(&s.interest))
        {
            return true;
        }
        if !watches
            .iter()
            .any(|(w, _)| server.sync.studio_watch_is_current(&w.inner))
        {
            return false;
        }
        if self.in_flight || self.preparing || self.preparation.is_some() {
            return false;
        }
        let now = server.runtime_clock().monotonic_ms();
        if self.checkpoint.is_some() || self.discovery_plan.is_some() {
            return now >= self.checkpoint_retry;
        }
        if let Some(pass) = &self.pass {
            return now >= pass.retry_at_ms();
        }
        if let Some(pass) = &self.registry_pass {
            return now >= pass.retry_at_ms();
        }
        if (self.owner_snapshot.is_some() && now >= self.owner_next_at)
            || now >= self.registry_next_at
        {
            return true;
        }
        let peers = server.sync.studio_page_peers();
        !peers.is_empty() && (now >= self.next_at || peers != self.peers)
    }
    /// Local capture only, never history replay. The scheduler coalesces one target at a time.
    pub(super) fn prepare<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        target: StudioTarget,
    ) -> Result<bool, AppError> {
        self.prepare_for(
            server,
            store,
            id,
            PreparationContext {
                target,
                service: None,
            },
        )
    }
    fn prepare_for<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        context: PreparationContext,
    ) -> Result<bool, AppError> {
        let target = context.target;
        let warm = server
            .sync
            .with_registry_context(|g, d, _, _| store.studio_source_is_warm(id, g, target, d));
        if warm {
            return Ok(true);
        }
        if self.preparing
            || self.preparation.is_some()
            || self.prepared.is_some()
            || self.registry_preparation.is_some()
            || self.registry_prepared.is_some()
        {
            return Ok(false);
        }
        let Ok(permit) = crate::registry_catchup::preparation_pool()
            .clone()
            .try_acquire_owned()
        else {
            // A retained Registry graph must not permanently occupy this actor's preparation
            // capacity while a foreground/receive Studio graph needs the shared worker pool.
            self.registry_provider = None;
            return Ok(false);
        };
        let capture = server
            .sync
            .with_registry_context(|g, d, _, _| store.capture_studio_source(id, g, target, d))?;
        if let Some(capture) = capture {
            self.preparation = Some((capture, permit, context));
            return Ok(false);
        }
        // Actual absence needs no reconstruction. The normal inventory gate still decides
        // whether it is legal to receive into absent epoch zero, never an Index pointer alone.
        Ok(true)
    }
    fn budget<T: MeshTransport, R: CryptoRngCore>(
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> Result<crate::store::EpochStudioBudget, AppError> {
        let snapshot = server.snapshot()?;
        server
            .sync
            .with_registry_context(|_, _, _, rng| store.save_server(id, &snapshot, rng))?;
        Self::inventory_budget(server, store, id)
    }
    fn inventory_budget<T: MeshTransport, R: CryptoRngCore>(
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> Result<crate::store::EpochStudioBudget, AppError> {
        let mut scan = store.scan_studio_receive_inventory()?;
        while !scan.step()?.complete {}
        let inventory = scan.finish()?;
        server
            .sync
            .with_registry_context(|g, _, _, _| store.studio_storage_budget(id, g, &inventory))
    }
    pub(super) fn run<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        watches: &VecDeque<(ServerStudioWatch, u128)>,
    ) -> Result<Option<StudioTarget>, AppError> {
        let now = server.runtime_clock().monotonic_ms();
        self.complete_registry_preparation(server, store)?;
        if let Some((context, prepared)) = self.prepared.take() {
            let result = prepared.and_then(|(source, _permit)| {
                server.sync.with_registry_context(|g, d, _, _| {
                    store.install_prepared_studio_source(g, d, *source)
                })
            });
            let installed = match result {
                Ok(installed) => installed,
                Err(_) if context.service.is_some() => {
                    self.drop_service_preparation(&context);
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            if !installed {
                self.drop_service_preparation(&context);
                // A concurrent healthy edit/MLS transition is not a storage fault. Discard the
                // old result and its process permit; future bounded selection captures anew.
                self.next_at = now.saturating_add(5_000);
                return Ok(None);
            }
        }
        if let Some(updated) = self.advance_checkpoint(server, store, id)? {
            return Ok(Some(updated));
        }
        // A held page wins its next native pass over further source requests. Otherwise a
        // steady stream of page requests could continuously evict the very graph it needs.
        if self
            .pass
            .as_ref()
            .is_some_and(|p| p.state() == StudioReceiveState::PageReady)
        {
            if !self.prepare(server, store, id, self.target.expect("pass target"))? {
                return Ok(None);
            }
            let mut budget = Self::budget(server, store, id)?;
            let pass = self.pass.as_mut().expect("page pass");
            let before = pass.progress().accepted;
            if let Err(error) = server.persist_studio_receive_step(store, pass, &mut budget) {
                if matches!(
                    pass.state(),
                    StudioReceiveState::RestartRequired | StudioReceiveState::Stopped
                ) {
                    // The adapter discarded a page whose authority/context expired. Nothing
                    // was saved; bounded retry, not an explicit-access-only storage pause, is
                    // the correct response to normal membership or subscription churn.
                    self.pass = None;
                    self.next_at = now.saturating_add(5_000);
                    return Ok(None);
                }
                return Err(error);
            }
            return Ok((pass.progress().accepted > before)
                .then_some(self.target)
                .flatten());
        }
        if self.persist_registry_page(server, store, id)? {
            // A Registry hint is not a Studio edit. Only a subsequent actual source update
            // may emit StudioUpdated; otherwise timeline/acceptance observers see false work.
            return Ok(None);
        }
        // Owner work also uses the sole prepared source. It must never displace a fetched
        // page before that page has crossed its save-before-cursor barrier.
        if !self.in_flight
            && !self.preparing
            && self.checkpoint.is_none()
            && self.discovery_plan.is_none()
        {
            if let Some(updated) = self.rotate_owner(server, store, id, watches)? {
                return Ok(Some(updated));
            }
        }
        // Answer requests even during our own detached fetch; symmetric reconnect must not
        // wait for one side's client pass to finish before serving its counterpart.
        let serve_now = self.in_flight
            || self.preparing
            || watches.is_empty()
            || !self.pending(server, watches)
            || !self.client_turn;
        self.client_turn = !self.client_turn;
        if serve_now && self.serve(server, store, id) {
            return Ok(None);
        }
        if self.in_flight || self.preparing || self.preparation.is_some() {
            return Ok(None);
        }
        if self.checkpoint.is_some() || self.discovery_plan.is_some() {
            return Ok(None);
        }
        if let Some(pass) = &mut self.pass {
            match pass.state() {
                StudioReceiveState::Ready => return Ok(None),
                // Network errors use a paced fresh pass. Storage failure returns above and
                // leaves the SAME pending page Paused until an explicit successful access.
                state => {
                    if matches!(
                        state,
                        StudioReceiveState::CheckpointRequired
                            | StudioReceiveState::RestartRequired
                    ) {
                        self.discovery_needed = self.target;
                    }
                    self.pass = None;
                    self.next_at = now.saturating_add(5_000);
                }
            }
        }
        let peers = server.sync.studio_page_peers();
        if watches.is_empty() || peers.is_empty() || (now < self.next_at && peers == self.peers) {
            // Registry maintenance uses the intentional gap between Studio page passes.
            // It cannot consume the client turn promised by the existing service alternation.
            self.work_registry(server, store, id, watches)?;
            return Ok(None);
        }
        self.peers = peers;
        if let Some(target) = self.discovery_needed.take() {
            if let Some((watch, _)) = watches.iter().find(|(w, _)| w.target == target) {
                self.schedule_discovery(store, id, watch, self.peers[0]);
                return Ok(None);
            }
        }
        let index = self.selection % watches.len();
        let peer = self.peers[(self.selection / watches.len()) % self.peers.len()];
        let watch = &watches[index].0;
        if watch.server != id || !Arc::ptr_eq(&watch.mount, &store.registry_mount()) {
            return Err(invalid("catch-up mount changed"));
        }
        if !self.prepare(server, store, id, watch.target)? {
            return Ok(None);
        }
        self.selection = self.selection.wrapping_add(1);
        self.target = Some(watch.target);
        // The first page itself needs all-family accounting. Warm the associated saved
        // Registry footprint before that scan, not only after a later checkpoint response.
        // Other unrelated cold records still obey the existing inventory work rail.
        let logical = watch.target.document(&server.group_id()).map_err(invalid)?;
        let bucket =
            catcoms_replication::registry::PointerKey::new(logical.doc_type, logical.logical_key)
                .map_err(invalid)?
                .bucket();
        if !store.registry_receive_source_fits(id, &server.group_id(), bucket)?
            && !self.prepare_registry_inventory(server, store, id, bucket)?
        {
            return Ok(None);
        }
        let mut budget = Self::budget(server, store, id)?;
        let status = server.sync.with_registry_context(|g, d, _, _| {
            store.prepared_studio_status(id, g, watch.target, d, &mut budget)
        })?;
        if let Some((doc_id, phase)) = status {
            if doc_id != watches[index].1 {
                self.binding = Some((watch.target, doc_id));
                return Ok(Some(watch.target));
            }
            if phase != catcoms_replication::EpochPhase::Open {
                // A persisted Closing epoch survives expiry/restart. It needs a fresh private
                // head selection, not an Open-only page pass that globally pauses the receiver.
                if phase == catcoms_replication::EpochPhase::Closing {
                    self.schedule_discovery(store, id, watch, self.peers[0]);
                }
                self.next_at = now.saturating_add(5_000);
                return Ok(None);
            }
        }
        self.pass = Some(server.begin_studio_receive(store, watch, peer, &mut budget)?);
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
impl StudioReceiver {
    #[cfg(test)]
    pub(crate) fn preparing_for_test(&self) -> bool {
        self.catchup.preparing
    }

    /// Called under the successful native custody window, then run after releasing that lease.
    pub(crate) fn detach<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
    ) -> Option<StudioBackgroundJob<T>> {
        if self.paused {
            return None;
        }
        let work = if let Some((capture, permit, context)) = self.catchup.preparation.take() {
            Some(StudioBackgroundJob::Prepare(capture, permit, context))
        } else if let Some((job, generation)) = self.catchup.registry_preparation.take() {
            Some(StudioBackgroundJob::PrepareRegistry(job, generation))
        } else if self.catchup.in_flight
            || (self.catchup.discovery_plan.is_some()
                && server.runtime_clock().monotonic_ms() < self.catchup.checkpoint_retry)
        {
            None
        } else if let Some(plan) = self.catchup.discovery_plan.take() {
            match server.prepare_checkpoint_discovery_at_mount(
                plan.mount,
                plan.server,
                plan.peer,
                plan.target,
            ) {
                Ok(attempt) => Some(StudioBackgroundJob::Head(attempt)),
                Err(_) => {
                    self.catchup
                        .retry_discovery(server.runtime_clock().monotonic_ms());
                    None
                }
            }
        } else if let Some(pass) = &mut self.catchup.checkpoint {
            if !self.catchup.checkpoint_sealed
                || server.runtime_clock().monotonic_ms() < self.catchup.checkpoint_retry
            {
                None
            } else {
                match server.prepare_checkpoint_seed_fetch(
                    pass,
                    self.catchup.checkpoint_peer.expect("checkpoint peer"),
                ) {
                    Ok(Some(attempt)) => {
                        self.catchup.checkpoint_retry =
                            server.runtime_clock().monotonic_ms().saturating_add(1_000);
                        Some(StudioBackgroundJob::Seed(attempt))
                    }
                    Ok(None) => None,
                    Err(_) => {
                        self.catchup
                            .retry_discovery(server.runtime_clock().monotonic_ms());
                        None
                    }
                }
            }
        } else if self.catchup.registry_pass.as_ref().is_some_and(|p| {
            p.state() == crate::registry_catchup::RegistryReceiveState::Ready
                && server.runtime_clock().monotonic_ms() >= p.retry_at_ms()
        }) {
            let pass = self
                .catchup
                .registry_pass
                .as_mut()
                .expect("ready Registry pass");
            match server.prepare_registry_receive_step(pass) {
                Ok(Some(attempt)) => Some(StudioBackgroundJob::RegistryPage(attempt)),
                Ok(None) => None,
                Err(_) => {
                    self.catchup.registry_pass = None;
                    self.catchup.registry_next_at =
                        server.runtime_clock().monotonic_ms().saturating_add(5_000);
                    None
                }
            }
        } else if let Some(pass) = &mut self.catchup.pass {
            match server.prepare_studio_receive_step(pass) {
                Ok(Some(attempt)) => Some(StudioBackgroundJob::Page(attempt)),
                Ok(None) => None,
                Err(_) => {
                    self.catchup.pass = None;
                    self.catchup.next_at =
                        server.runtime_clock().monotonic_ms().saturating_add(5_000);
                    None
                }
            }
        } else {
            None
        };
        match &work {
            Some(StudioBackgroundJob::Prepare(..)) => self.catchup.preparing = true,
            Some(StudioBackgroundJob::PrepareRegistry(..)) => self.catchup.preparing = true,
            Some(StudioBackgroundJob::Page(..) | StudioBackgroundJob::RegistryPage(..)) => {
                self.catchup.in_flight = true
            }
            Some(StudioBackgroundJob::Head(..) | StudioBackgroundJob::Seed(..)) => {
                self.catchup.in_flight = true
            }
            None => {}
        }
        work
    }
    pub(crate) fn complete<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        result: StudioBackgroundResult,
    ) {
        match result {
            StudioBackgroundResult::RegistryPage(completed) => {
                self.catchup.in_flight = false;
                if let Some(pass) = &mut self.catchup.registry_pass {
                    if server
                        .complete_registry_receive_step(pass, *completed)
                        .is_err()
                    {
                        self.catchup.registry_pass = None;
                        self.catchup.registry_next_at =
                            server.runtime_clock().monotonic_ms().saturating_add(5_000);
                    }
                }
            }
            StudioBackgroundResult::PreparedRegistry(generation, result) => {
                self.catchup.preparing = false;
                self.catchup.registry_prepared = Some((generation, result));
            }
            StudioBackgroundResult::CancelledRegistry(generation) => {
                self.catchup.preparing = false;
                if let Some(generation) = generation {
                    self.catchup.drop_registry_service(&generation);
                } else {
                    self.catchup
                        .retry_discovery(server.runtime_clock().monotonic_ms());
                }
            }
            StudioBackgroundResult::Head(completed) => {
                self.catchup.in_flight = false;
                self.catchup.head_result = Some(completed);
            }
            StudioBackgroundResult::Seed(completed) => {
                self.catchup.in_flight = false;
                if let Some(pass) = &mut self.catchup.checkpoint {
                    if server
                        .complete_checkpoint_seed_fetch(pass, *completed)
                        .is_err()
                    {
                        self.catchup
                            .retry_discovery(server.runtime_clock().monotonic_ms());
                    }
                }
            }
            StudioBackgroundResult::Prepared(context, result) => {
                self.catchup.preparing = false;
                self.catchup.prepared = Some((context, result));
            }
            StudioBackgroundResult::Page(completed) => {
                self.catchup.in_flight = false;
                if let Some(pass) = &mut self.catchup.pass {
                    if server
                        .complete_studio_receive_step(pass, *completed)
                        .is_err()
                    {
                        // This is a network/context failure, not permission to clear storage pause.
                        self.catchup.pass = None;
                        self.catchup.next_at =
                            server.runtime_clock().monotonic_ms().saturating_add(5_000);
                    }
                }
            }
            StudioBackgroundResult::Cancelled { preparation } => {
                if let Some(context) = preparation {
                    self.catchup.preparing = false;
                    self.catchup.drop_service_preparation(&context);
                } else {
                    self.catchup.in_flight = false;
                    self.catchup.pass = None;
                    self.catchup.registry_pass = None;
                    self.catchup
                        .retry_discovery(server.runtime_clock().monotonic_ms());
                }
                self.catchup.next_at = server.runtime_clock().monotonic_ms().saturating_add(5_000);
            }
        }
    }
}
