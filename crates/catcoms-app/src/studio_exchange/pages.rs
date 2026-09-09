//! Receiver-owned progress, not a ciphertext outbox or finality protocol. Network waits never
//! borrow the vault. A pending page remains here until one accounted store transaction succeeds.
use super::*;
use crate::store::{EpochStudioBudget, StudioPageAdmission};
use catcoms_replication::epoch::MAX_EPOCH_OPERATIONS;
use catcoms_replication::studio::catchup::{StudioFrontier, StudioOpPage, StudioPageCursor};
use catcoms_replication::studio::catchup::{StudioPageOutcome, StudioPageProvider};
use catcoms_rt::PeerId;
use catcoms_sync::registry_catchup::{
    CompletedStudioPage, PendingStudioPage, StudioPageQuery, StudioReceivePermit,
};

const PASS_LIFETIME_MS: u64 = 600_000;
const REQUEST_INTERVAL_MS: u64 = 1_000;
const MAX_RECEIVED_BYTES: usize = 16 * 1024 * 1024;

/// All completion labels describe one provider's captured prefix only, never owner finality or
/// global currency. Held states require discovery/a new pass, not legacy catch-up fallback.
pub use crate::registry_catchup::RegistryReceiveState as StudioReceiveState;

/// Attempt/page counters are bounded work and durability evidence, not delivery acknowledgements.
pub use crate::registry_catchup::RegistryReceiveProgress as StudioReceiveProgress;

/// One of four accounted receiver passes shared with Registry per sync runtime. It pins provider AND requester full
/// identities, watch generation, concrete epoch and physical vault mount. At most one 512-KiB
/// page is retained. Drop loses only traversal: saved pages remain durable and a new pass derives
/// its frontier from them. The provider's opaque cursor is never persisted as security state.
pub struct ServerStudioReceive {
    permit: StudioReceivePermit,
    mount: Arc<()>,
    server: u64,
    target: StudioTarget,
    attempt: Arc<()>,
    requester: crate::DeviceId,
    peer: PeerId,
    provider: crate::DeviceId,
    frontier: StudioFrontier,
    empty_fallback_used: bool,
    cursor: Option<StudioPageCursor>,
    pending: Option<StudioOpPage>,
    pending_epoch: u64,
    state: StudioReceiveState,
    progress: StudioReceiveProgress,
    now: u64,
    expires: u64,
    retry_at: u64,
    write_at: u64,
}
impl std::fmt::Debug for ServerStudioReceive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerStudioReceive")
            .field("state", &self.state)
            .field("progress", &self.progress)
            .finish_non_exhaustive()
    }
}
impl ServerStudioReceive {
    pub fn state(&self) -> StudioReceiveState {
        self.state
    }
    pub fn progress(&self) -> StudioReceiveProgress {
        self.progress
    }
    /// Scheduling hint on the receiver's injected monotonic clock, not the provider cursor clock.
    pub fn retry_at_ms(&self) -> u64 {
        if self.pending.is_some() {
            self.write_at
        } else {
            self.retry_at
        }
    }

    /// Explicitly retry a failed/cancelled step without forgiving attempts, pacing or lifetime.
    /// A held page retries persistence, not network. Reconciliation of uncertain storage remains
    /// the caller's responsibility; retry is not permission to ignore an invalid budget.
    pub fn retry(&mut self) {
        if self.state == StudioReceiveState::Paused {
            self.state = if self.pending.is_some() {
                StudioReceiveState::PageReady
            } else {
                StudioReceiveState::Ready
            };
        }
    }

    /// Independent edits commonly give the requester a head this provider has never seen.
    /// Ask for the whole prefix once, retaining the verified seed and every charged limit.
    /// Never do this after accepting a page: changing the frontier invalidates its cursor.
    fn receive_restart(&mut self) {
        if !self.empty_fallback_used
            && !self.frontier.heads.is_empty()
            && self.cursor.is_none()
            && self.progress.received_pages == 0
        {
            self.frontier.heads.clear();
            self.empty_fallback_used = true;
            self.state = StudioReceiveState::Ready;
        } else {
            self.state = StudioReceiveState::RestartRequired;
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Reserve bounded ownership, verify and flush the checked starting state, then capture its
    /// frontier. No file is created when epoch zero is absent. The watch must already be installed
    /// and the provider endpoint proven. No vault borrow survives into a network request.
    pub fn begin_studio_receive(
        &mut self,
        store: &mut ServerStore,
        watch: &ServerStudioWatch,
        peer: PeerId,
        budget: &mut EpochStudioBudget,
    ) -> Result<ServerStudioReceive, AppError> {
        self.check_studio_page_watch(store, watch)?;
        let provider = self
            .sync
            .registry_page_peer_device(peer)
            .ok_or_else(|| AppError::Invalid("studio provider endpoint is not proven".into()))?;
        let permit = self.sync.begin_studio_receive(&watch.inner)?;
        let now = self.runtime_clock().monotonic_ms();
        let expires = now
            .checked_add(PASS_LIFETIME_MS)
            .ok_or_else(|| AppError::Invalid("studio receive clock exhausted".into()))?;
        let (requester, frontier) = self.sync.with_registry_context(|group, device, _, rng| {
            let admission = store.ingest_studio_page(
                watch.server,
                group,
                watch.target,
                permit.doc_id(),
                device,
                &[],
                rng,
                budget,
            )?;
            Ok::<_, AppError>((device.device_id(), admission.frontier))
        })?;
        Ok(ServerStudioReceive {
            permit,
            mount: store.registry_mount(),
            server: watch.server,
            target: watch.target,
            attempt: Arc::new(()),
            requester,
            peer,
            provider,
            frontier,
            empty_fallback_used: false,
            cursor: None,
            pending: None,
            pending_epoch: 0,
            state: StudioReceiveState::Ready,
            progress: StudioReceiveProgress::default(),
            now,
            expires,
            retry_at: now,
            write_at: now,
        })
    }

    fn check_studio_receive(&mut self, pass: &mut ServerStudioReceive) -> Result<(), AppError> {
        pass.now = pass.now.max(self.runtime_clock().monotonic_ms());
        let current = self.sync.with_registry_context(|group, device, _, _| {
            device.device_id() == pass.requester
                && group.member_signature_key(&device.device_id()).as_deref()
                    == Some(device.public_key_bytes().as_slice())
        });
        if !current
            || self.check_studio_channel(pass.target).is_err()
            || !self.sync.studio_receive_is_current(&pass.permit)
            || self.sync.registry_page_peer_device(pass.peer) != Some(pass.provider)
        {
            pass.pending = None;
            pass.state = StudioReceiveState::Stopped;
            return Err(AppError::Invalid(
                "studio receiver authority was replaced".into(),
            ));
        }
        if pass.now >= pass.expires {
            pass.pending = None;
            pass.state = StudioReceiveState::RestartRequired;
        }
        Ok(())
    }

    /// Prepare one bounded attempt while holding the Server briefly. Run its fetch after
    /// releasing Server/store/native custody; re-enter this exact pass for completion.
    pub fn prepare_studio_receive_step(
        &mut self,
        pass: &mut ServerStudioReceive,
    ) -> Result<Option<StudioPageAttempt<T>>, AppError> {
        self.check_studio_receive(pass)?;
        if pass.state != StudioReceiveState::Ready || pass.now < pass.retry_at {
            return Ok(None);
        }
        if pass.progress.attempts > MAX_EPOCH_OPERATIONS {
            pass.state = StudioReceiveState::RestartRequired;
            return Ok(None);
        }
        pass.progress.attempts += 1;
        pass.retry_at = pass.now.saturating_add(REQUEST_INTERVAL_MS);
        pass.state = StudioReceiveState::Paused;
        pass.pending_epoch = self
            .sync
            .with_registry_context(|group, _, _, _| group.epoch());
        pass.attempt = Arc::new(());
        let request = self.sync.prepare_studio_receive_page(
            &pass.permit,
            pass.peer,
            StudioPageQuery {
                target: pass.target,
                doc_id: pass.permit.doc_id(),
                heads: &pass.frontier.heads,
                seed: pass.frontier.seed,
                cursor: pass.cursor.as_ref().map(StudioPageCursor::as_bytes),
            },
        )?;
        Ok(Some(StudioPageAttempt {
            request,
            attempt: pass.attempt.clone(),
        }))
    }

    pub fn complete_studio_receive_step(
        &mut self,
        pass: &mut ServerStudioReceive,
        completed: StudioPageCompletion,
    ) -> Result<StudioReceiveState, AppError> {
        self.check_studio_receive(pass)?;
        if !Arc::ptr_eq(&pass.attempt, &completed.attempt)
            || pass.state != StudioReceiveState::Paused
        {
            return Err(invalid("page attempt was superseded"));
        }
        let outcome = self.sync.complete_studio_page(completed.completed)?;
        match outcome {
            None => {} // unsupported/refused is retryable, never prefix completion
            Some(StudioPageOutcome::Restart) => pass.receive_restart(),
            Some(StudioPageOutcome::CheckpointRequired) => {
                pass.state = StudioReceiveState::CheckpointRequired
            }
            Some(StudioPageOutcome::HistoricalAuthorizationRequired) => {
                pass.state = StudioReceiveState::HistoricalAuthorizationRequired
            }
            Some(StudioPageOutcome::Page(page)) => {
                let bytes: usize = page
                    .operations
                    .iter()
                    .map(|op| op.blob.ciphertext.len() + 62)
                    .sum();
                pass.progress.received_pages += 1;
                pass.progress.received_operations += page.operations.len();
                pass.progress.received_bytes += bytes;
                if pass.progress.received_pages > MAX_EPOCH_OPERATIONS + 1
                    || pass.progress.received_operations > MAX_EPOCH_OPERATIONS
                    || pass.progress.received_bytes > MAX_RECEIVED_BYTES
                    || page
                        .next
                        .as_ref()
                        .zip(pass.cursor.as_ref())
                        .is_some_and(|(next, old)| next.as_bytes() == old.as_bytes())
                {
                    pass.state = StudioReceiveState::RestartRequired;
                    return Ok(pass.state);
                }
                pass.pending = Some(page);
                pass.state = StudioReceiveState::PageReady;
            }
        }
        Ok(pass.state)
    }

    /// Save the pending page atomically, then and only then select its continuation. A failed
    /// save retains the SAME page and old cursor; retries do not spend more network work.
    /// Empty terminal pages pass all target/inventory/flush checks without creating epoch zero.
    pub fn persist_studio_receive_step(
        &mut self,
        store: &mut ServerStore,
        pass: &mut ServerStudioReceive,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioReceiveState, AppError> {
        if !Arc::ptr_eq(&pass.mount, &store.registry_mount()) {
            pass.pending = None;
            pass.state = StudioReceiveState::Stopped;
            return Err(AppError::Invalid(
                "studio receiver vault mount was replaced".into(),
            ));
        }
        self.check_studio_receive(pass)?;
        if pass.state != StudioReceiveState::PageReady {
            return Ok(pass.state);
        }
        if pass.now < pass.write_at {
            return Ok(pass.state);
        }
        if pass.progress.persist_attempts > MAX_EPOCH_OPERATIONS {
            pass.pending = None;
            pass.state = StudioReceiveState::RestartRequired;
            return Ok(pass.state);
        }
        pass.progress.persist_attempts += 1;
        pass.write_at = pass.now.saturating_add(REQUEST_INTERVAL_MS);
        pass.state = StudioReceiveState::Paused;
        if self
            .sync
            .with_registry_context(|group, _, _, _| group.epoch())
            != pass.pending_epoch
        {
            // These ciphertexts can never pass current-MLS admission now. Do not advertise a
            // persistence retry that will fail forever, or advance past a page we did not save.
            pass.pending = None;
            pass.state = StudioReceiveState::RestartRequired;
            return Err(AppError::Invalid(
                "studio page MLS epoch changed; start a fresh pass".into(),
            ));
        }
        let page = pass.pending.as_ref().expect("PageReady retains its page");
        let counts = self.sync.with_registry_context(|group, device, _, rng| {
            store.ingest_studio_page(
                pass.server,
                group,
                pass.target,
                pass.permit.doc_id(),
                device,
                &page.operations,
                rng,
                budget,
            )
        })?;
        let StudioPageAdmission {
            accepted,
            duplicates,
            ..
        } = counts;
        pass.progress.saved_pages += 1;
        pass.progress.accepted += accepted;
        pass.progress.duplicates += duplicates;
        pass.cursor = pass.pending.take().expect("saved pending page").next;
        pass.state = if pass.cursor.is_some() {
            StudioReceiveState::Ready
        } else {
            StudioReceiveState::PrefixComplete
        };
        Ok(pass.state)
    }
}

pub struct StudioPageAttempt<T: MeshTransport> {
    request: PendingStudioPage<T>,
    attempt: Arc<()>,
}
pub struct StudioPageCompletion {
    completed: CompletedStudioPage,
    attempt: Arc<()>,
}
impl<T: MeshTransport> std::fmt::Debug for StudioPageAttempt<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioPageAttempt { .. }")
    }
}
impl std::fmt::Debug for StudioPageCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioPageCompletion { .. }")
    }
}
impl<T: MeshTransport> StudioPageAttempt<T> {
    pub async fn fetch(self) -> StudioPageCompletion {
        StudioPageCompletion {
            completed: self.request.fetch().await,
            attempt: self.attempt,
        }
    }
}

/// One cursor secret per runtime/mount; no second source graph or persistence owner is retained.
pub struct ServerStudioPageProvider {
    inner: StudioPageProvider,
    instance: catcoms_sync::RegistrySyncInstance,
    mount: Arc<()>,
    server: u64,
}
impl std::fmt::Debug for ServerStudioPageProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerStudioPageProvider { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    fn check_studio_page_watch(
        &self,
        store: &ServerStore,
        watch: &ServerStudioWatch,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&watch.mount, &store.registry_mount())
            || !self.sync.studio_watch_is_current(&watch.inner)
        {
            return Err(invalid("page watch was replaced"));
        }
        self.check_studio_channel(watch.target)
    }
    pub fn studio_page_provider(
        &mut self,
        store: &ServerStore,
        server: u64,
    ) -> ServerStudioPageProvider {
        let instance = self.sync.registry_instance();
        let clock = self.runtime_clock();
        let inner = self.sync.with_registry_context(|_, device, _, rng| {
            StudioPageProvider::new(device.device_id(), clock, rng)
        });
        ServerStudioPageProvider {
            inner,
            instance,
            mount: store.registry_mount(),
            server,
        }
    }
    pub(crate) fn studio_page_provider_is_current(
        &self,
        store: &ServerStore,
        server: u64,
        provider: &ServerStudioPageProvider,
    ) -> bool {
        self.sync.matches_registry_instance(&provider.instance)
            && Arc::ptr_eq(&provider.mount, &store.registry_mount())
            && provider.server == server
    }
    pub fn serve_studio_request_step(
        &mut self,
        store: &mut ServerStore,
        provider: &mut ServerStudioPageProvider,
        watch: &ServerStudioWatch,
    ) -> Result<Option<()>, AppError> {
        self.check_studio_page_watch(store, watch)?;
        if !self.sync.matches_registry_instance(&provider.instance)
            || !Arc::ptr_eq(&provider.mount, &store.registry_mount())
            || provider.server != watch.server
        {
            return Err(invalid("page provider was replaced"));
        }
        self.sync
            .serve_studio_request(&watch.inner, |group, device, rng, request| {
                store.serve_studio_page(
                    watch.server,
                    group,
                    watch.target,
                    device,
                    &mut provider.inner,
                    request,
                    rng,
                )
            })?
            .transpose()
    }
}
