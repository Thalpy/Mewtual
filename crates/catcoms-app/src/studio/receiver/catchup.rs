//! Bounded work selection for the existing native receiver. Disk barriers use its sole lease;
//! expensive vault verification and network waits leave the actor free to serve the other peer.
use super::*;
use crate::store::{PreparedStudioSource, StudioSourceCapture};
use crate::studio_exchange::{
    ServerStudioPageProvider, ServerStudioReceive, StudioPageAttempt, StudioPageCompletion,
    StudioReceiveState,
};
use catcoms_rt::{PeerId, RequestCancellation};
use tokio::sync::OwnedSemaphorePermit;

/// One tracked network attempt and preparation waiter per actor. Cancellation may leave a
/// blocking worker running: it retains its shared process permit until actual completion.
/// Thus all running, queued and result-holding preparations together still occupy at most
/// four slots process-wide. A ready result retains its permit until native custody.
pub(crate) enum StudioBackgroundJob<T: MeshTransport> {
    Page(StudioPageAttempt<T>),
    Prepare(StudioSourceCapture, OwnedSemaphorePermit),
}
pub(crate) enum StudioBackgroundResult {
    Page(Box<StudioPageCompletion>),
    Prepared(Result<(Box<PreparedStudioSource>, OwnedSemaphorePermit), AppError>),
    Cancelled { preparation: bool },
}
impl<T: MeshTransport + 'static> StudioBackgroundJob<T> {
    pub(crate) async fn run(
        self,
        mut cancellation: Option<RequestCancellation>,
    ) -> StudioBackgroundResult {
        let preparation = matches!(&self, Self::Prepare(..));
        let work = async move {
            match self {
                Self::Page(attempt) => {
                    StudioBackgroundResult::Page(Box::new(attempt.fetch().await))
                }
                Self::Prepare(capture, permit) => {
                    let result = tokio::task::spawn_blocking(move || {
                        capture.rebuild().map(|p| (Box::new(p), permit))
                    })
                    .await;
                    StudioBackgroundResult::Prepared(
                        result.unwrap_or_else(|_| Err(invalid("Studio preparation worker failed"))),
                    )
                }
            }
        };
        tokio::select! {
            biased;
            _ = async { match cancellation.as_mut() { Some(c) => c.cancelled().await, None => std::future::pending::<()>().await } } => StudioBackgroundResult::Cancelled { preparation },
            result = work => result,
        }
    }
}

#[derive(Default)]
pub(super) struct CatchupRuntime {
    provider: Option<ServerStudioPageProvider>,
    pass: Option<ServerStudioReceive>,
    target: Option<StudioTarget>,
    preparation: Option<(StudioSourceCapture, OwnedSemaphorePermit)>,
    prepared: Option<Result<(Box<PreparedStudioSource>, OwnedSemaphorePermit), AppError>>,
    in_flight: bool,
    preparing: bool,
    next_at: u64,
    selection: usize,
    peers: Vec<PeerId>,
    client_turn: bool,
}
impl CatchupRuntime {
    pub(super) fn explicit_retry(&mut self, now: u64) {
        if !self.in_flight {
            if let Some(pass) = &mut self.pass {
                pass.retry();
            }
        }
        self.next_at = now;
    }
    pub(super) fn pending<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
        watches: &VecDeque<(ServerStudioWatch, u128)>,
    ) -> bool {
        if watches.is_empty() {
            return false;
        }
        if self.prepared.is_some() {
            return true;
        }
        if self.in_flight || self.preparing || self.preparation.is_some() {
            return false;
        }
        let now = server.runtime_clock().monotonic_ms();
        if let Some(pass) = &self.pass {
            return now >= pass.retry_at_ms();
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
        let warm = server
            .sync
            .with_registry_context(|g, d, _, _| store.studio_source_is_warm(id, g, target, d));
        if warm {
            return Ok(true);
        }
        if self.preparing || self.preparation.is_some() || self.prepared.is_some() {
            return Ok(false);
        }
        let Ok(permit) = crate::registry_catchup::preparation_pool()
            .clone()
            .try_acquire_owned()
        else {
            return Ok(false);
        };
        let capture = server
            .sync
            .with_registry_context(|g, d, _, _| store.capture_studio_source(id, g, target, d))?;
        if let Some(capture) = capture {
            self.preparation = Some((capture, permit));
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
        let mut scan = store.scan_studio_receive_inventory()?;
        while !scan.step()?.complete {}
        let inventory = scan.finish()?;
        let snapshot = server.snapshot()?;
        server.sync.with_registry_context(|g, _, _, rng| {
            store.save_server(id, &snapshot, rng)?;
            store.studio_storage_budget(id, g, &inventory)
        })
    }
    pub(super) fn run<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        watches: &VecDeque<(ServerStudioWatch, u128)>,
    ) -> Result<Option<StudioTarget>, AppError> {
        let now = server.runtime_clock().monotonic_ms();
        if let Some(prepared) = self.prepared.take() {
            let (source, _permit) = prepared?;
            let installed = server.sync.with_registry_context(|g, d, _, _| {
                store.install_prepared_studio_source(g, d, *source)
            })?;
            if !installed {
                // A concurrent healthy edit/MLS transition is not a storage fault. Discard the
                // old result and its process permit; future bounded selection captures anew.
                self.next_at = now.saturating_add(5_000);
                return Ok(None);
            }
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
        // Answer requests even during our own detached fetch; symmetric reconnect must not
        // wait for one side's client pass to finish before serving its counterpart.
        let serve_now =
            self.in_flight || self.preparing || !self.pending(server, watches) || !self.client_turn;
        self.client_turn = !self.client_turn;
        if let Some((watch, _)) = watches
            .iter()
            .find(|(w, _)| server.sync.studio_has_page_request(&w.inner))
            .filter(|_| serve_now)
        {
            if self.prepare(server, store, id, watch.target)? {
                if self
                    .provider
                    .as_ref()
                    .is_none_or(|p| !server.studio_page_provider_is_current(store, id, p))
                {
                    self.provider = Some(server.studio_page_provider(store, id));
                }
                // An absent source has no prepared graph to serve. Consume/refuse that request
                // through the checked provider instead of inventing an authoritative empty head.
                let _ =
                    server.serve_studio_request_step(store, self.provider.as_mut().unwrap(), watch);
            }
            return Ok(None);
        }
        if self.in_flight || self.preparing || self.preparation.is_some() {
            return Ok(None);
        }
        if let Some(pass) = &mut self.pass {
            match pass.state() {
                StudioReceiveState::Ready => return Ok(None),
                // Network errors use a paced fresh pass. Storage failure returns above and
                // leaves the SAME pending page Paused until an explicit successful access.
                _ => {
                    self.pass = None;
                    self.next_at = now.saturating_add(5_000);
                }
            }
        }
        let peers = server.sync.studio_page_peers();
        if watches.is_empty() || peers.is_empty() || (now < self.next_at && peers == self.peers) {
            return Ok(None);
        }
        self.peers = peers;
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
        let mut budget = Self::budget(server, store, id)?;
        self.pass = Some(server.begin_studio_receive(store, watch, peer, &mut budget)?);
        Ok(None)
    }
}
impl StudioReceiver {
    /// Called under the successful native custody window, then run after releasing that lease.
    pub(crate) fn detach<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
    ) -> Option<StudioBackgroundJob<T>> {
        if self.paused {
            return None;
        }
        let work = if let Some((capture, permit)) = self.catchup.preparation.take() {
            Some(StudioBackgroundJob::Prepare(capture, permit))
        } else if self.catchup.in_flight {
            None
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
            Some(StudioBackgroundJob::Page(..)) => self.catchup.in_flight = true,
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
            StudioBackgroundResult::Prepared(result) => {
                self.catchup.preparing = false;
                self.catchup.prepared = Some(result);
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
                if preparation {
                    self.catchup.preparing = false;
                } else {
                    self.catchup.in_flight = false;
                    self.catchup.pass = None;
                }
                self.catchup.next_at = server.runtime_clock().monotonic_ms().saturating_add(5_000);
            }
        }
    }
}
