//! A bounded, volatile fallback behind authoritative checkpoint work. Every network step is
//! detached; only completion/current-scope checks run under native vault custody.
use super::*;
use crate::studio_exchange::provisional::*;
use std::{future::Future, pin::Pin, sync::OnceLock};
fn preview_preparation_pool() -> &'static Arc<tokio::sync::Semaphore> {
    static POOL: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    POOL.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(3)))
}

pub(crate) enum PreviewPreparation {
    Seed(Box<ServerProvisionalStudioSeedPreparation>),
    Tail(Box<ServerProvisionalStudioTailPreparation>),
    #[cfg(test)]
    Paused(
        Box<Self>,
        tokio::sync::oneshot::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    ),
}
impl PreviewPreparation {
    fn prepare(self) -> Result<ServerPreparedProvisionalStudioSeed, AppError> {
        match self {
            Self::Seed(p) => p.prepare(),
            Self::Tail(p) => p.prepare(),
            #[cfg(test)]
            Self::Paused(work, entered, release) => {
                let _ = entered.send(());
                let _ = release.recv();
                work.prepare()
            }
        }
    }
}
pub(crate) enum PreviewCompletion {
    Head(Box<ProvisionalStudioDiscoveryCompletion>),
    Seed(Box<ProvisionalStudioSeedCompletion>),
    Tail(Box<ProvisionalStudioTailCompletion>),
    Prepared(Result<Box<ServerPreparedProvisionalStudioSeed>, AppError>),
    Cancelled { preparation: bool },
}
pub(crate) enum PreviewJob {
    Network(Pin<Box<dyn Future<Output = PreviewCompletion> + Send>>),
    Prepare(
        PreviewPreparation,
        OwnedSemaphorePermit,
        OwnedSemaphorePermit,
    ),
}
impl PreviewJob {
    #[cfg(test)]
    pub(crate) fn pause_for_test(
        self,
    ) -> (
        Self,
        tokio::sync::oneshot::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let Self::Prepare(work, permit, preview_permit) = self else {
            panic!("preparation job");
        };
        let (entered, entry) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        (
            Self::Prepare(
                PreviewPreparation::Paused(Box::new(work), entered, released),
                permit,
                preview_permit,
            ),
            entry,
            release,
        )
    }
    pub(crate) fn is_preparation(&self) -> bool {
        matches!(self, Self::Prepare(..))
    }
    pub(crate) async fn run(self) -> PreviewCompletion {
        match self {
            Self::Network(work) => work.await,
            Self::Prepare(work, permit, preview_permit) => {
                let result = tokio::task::spawn_blocking(move || {
                    // The actual blocking worker owns this permit even if its waiter is
                    // cancelled. A ready preview retains seed capacity, never a parser slot.
                    let _permits = (permit, preview_permit);
                    work.prepare()
                })
                .await
                .unwrap_or_else(|_| Err(invalid("preview worker failed")));
                PreviewCompletion::Prepared(result.map(Box::new))
            }
        }
    }
}
struct Retry {
    watch: ServerStudioWatch,
    peer: PeerId,
    epoch: u64,
    expires: u64,
    head_after: u64,
}
struct Ready {
    target: StudioTarget,
    seed: Arc<ServerPreparedProvisionalStudioSeed>,
    refresh_at: u64,
}
enum Next {
    Hint(Box<ServerProvisionalStudioHint>),
    Prepare(PreviewPreparation),
    Tail(Box<ServerPreparedProvisionalStudioSeed>),
}
#[derive(Default)]
pub(in crate::studio::receiver) struct PreviewRuntime {
    generation: Arc<()>,
    #[cfg(test)]
    pause_parse: Option<(
        tokio::sync::oneshot::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    )>,
    #[cfg(test)]
    pools: Option<(Arc<tokio::sync::Semaphore>, Arc<tokio::sync::Semaphore>)>,
    queue: VecDeque<Retry>,
    active: Option<Retry>,
    next: Option<Next>,
    pub(in crate::studio::receiver) job: Option<PreviewJob>,
    pub(in crate::studio::receiver) completed: Option<PreviewCompletion>,
    ready: VecDeque<Ready>,
    notices: VecDeque<StudioTarget>,
}
impl PreviewRuntime {
    pub(super) fn generation(&self) -> Arc<()> {
        self.generation.clone()
    }
    pub(super) fn complete(&mut self, generation: Arc<()>, result: PreviewCompletion) {
        if self.active.is_some() && Arc::ptr_eq(&self.generation, &generation) {
            self.completed = Some(result);
        }
    }

    fn preparation_pools(&self) -> (Arc<tokio::sync::Semaphore>, Arc<tokio::sync::Semaphore>) {
        #[cfg(test)]
        if let Some(pools) = &self.pools {
            return pools.clone();
        }
        (
            crate::registry_catchup::preparation_pool().clone(),
            preview_preparation_pool().clone(),
        )
    }

    pub(in crate::studio::receiver) fn queue(
        &mut self,
        watch: &ServerStudioWatch,
        peer: PeerId,
        now: u64,
    ) {
        // Duplicate hints cannot reset a retry's position or displace another watched key.
        if self.queue.iter().any(|r| r.watch.target == watch.target)
            || self
                .active
                .as_ref()
                .is_some_and(|r| r.watch.target == watch.target)
        {
            return;
        }
        if self.queue.len() == 16 {
            self.queue.pop_front();
        }
        self.queue.push_back(Retry {
            watch: ServerStudioWatch {
                inner: watch.inner.copy_binding(),
                mount: watch.mount.clone(),
                server: watch.server,
                target: watch.target,
            },
            peer,
            epoch: 0,
            expires: 0,
            // Registry + Studio head discovery can spend the provider's two-request burst.
            // Its shared requester rail refills one token/second. Preserve this bounded retry
            // until then, without reserving a slot or resetting its place on duplicate Hints.
            head_after: now.saturating_add(1_000),
        });
    }
    pub(in crate::studio::receiver) fn pending(&self, now: u64) -> bool {
        self.completed.is_some()
            || self.next.is_some()
            || self.job.is_some()
            || self.queue.front().is_some_and(|r| now >= r.head_after)
    }
    pub(in crate::studio::receiver) fn defers(&self, target: StudioTarget, now: u64) -> bool {
        self.active
            .as_ref()
            .is_some_and(|r| r.watch.target == target)
            || self
                .ready
                .iter()
                .any(|r| r.target == target && now < r.refresh_at)
    }
    pub(in crate::studio::receiver) fn take_notice(&mut self) -> Option<StudioTarget> {
        self.notices.pop_front()
    }
    fn notice(&mut self, target: StudioTarget) {
        if !self.notices.contains(&target) {
            if self.notices.len() == 16 {
                self.notices.pop_front();
            }
            self.notices.push_back(target);
        }
    }
    pub(in crate::studio::receiver) fn maintain<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
    ) {
        let mut removed = Vec::new();
        self.ready.retain(|r| {
            let keep = server
                .with_provisional_studio_seed(store, id, &r.seed, |_| ())
                .is_ok();
            if !keep {
                removed.push(r.target);
            }
            keep
        });
        for target in removed {
            self.notice(target);
        }
        self.queue.retain(|r| {
            r.watch.server == id
                && Arc::ptr_eq(&r.watch.mount, &store.registry_mount())
                && server.sync.studio_watch_is_current(&r.watch.inner)
        });
        if self.active.as_ref().is_some_and(|r| {
            r.watch.server != id
                || r.epoch != server.sync.epoch()
                || server.runtime_clock().monotonic_ms() >= r.expires
                || !Arc::ptr_eq(&r.watch.mount, &store.registry_mount())
                || !server.sync.studio_watch_is_current(&r.watch.inner)
        }) {
            self.active = None;
            self.next = None;
            self.job = None;
        }
        let Some(completed) = self.completed.take() else {
            return;
        };
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let next = match completed {
            PreviewCompletion::Head(c) => server
                .complete_provisional_studio_discovery(store, id, *c)
                .ok()
                .flatten()
                .map(|hint| Next::Hint(Box::new(hint))),
            PreviewCompletion::Seed(c) => server
                .complete_provisional_studio_seed(store, id, *c)
                .ok()
                .flatten()
                .map(|p| Next::Prepare(PreviewPreparation::Seed(Box::new(p)))),
            PreviewCompletion::Tail(c) => server
                .complete_provisional_studio_tail(store, id, *c)
                .ok()
                .flatten()
                .map(|p| Next::Prepare(PreviewPreparation::Tail(Box::new(p)))),
            PreviewCompletion::Prepared(Ok(seed))
                if server
                    .with_provisional_studio_seed(store, id, &seed, |_| ())
                    .is_ok() =>
            {
                if seed.tail_complete() {
                    let target = active.watch.target;
                    self.ready.retain(|r| r.target != target);
                    self.ready.push_back(Ready {
                        target,
                        seed: Arc::from(seed),
                        refresh_at: server.runtime_clock().monotonic_ms().saturating_add(30_000),
                    });
                    self.notice(target);
                    None
                } else {
                    Some(Next::Tail(seed))
                }
            }
            _ => None,
        };
        self.next = next;
        if self.next.is_none() {
            self.active = None;
        }
    }
    pub(in crate::studio::receiver) fn read<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
        store: &ServerStore,
        id: u64,
        target: StudioTarget,
    ) -> Option<StudioPreview> {
        let ready = self.ready.iter().find(|r| r.target == target)?;
        server
            .with_provisional_studio_seed(store, id, &ready.seed, |_| ())
            .ok()?;
        Some(StudioPreview::new(ready.seed.clone()))
    }
    /// Called only after held authoritative pages/checkpoints/preparations have had their turn.
    pub(in crate::studio::receiver) fn schedule<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
    ) -> bool {
        if self.job.is_some() {
            return true;
        }
        if let Some(next) = self.next.take() {
            self.job = match next {
                Next::Hint(hint) => server
                    .prepare_provisional_studio_seed(store, id, *hint)
                    .ok()
                    .map(|a| {
                        PreviewJob::Network(Box::pin(async {
                            PreviewCompletion::Seed(Box::new(a.fetch().await))
                        }))
                    }),
                Next::Tail(seed) => server
                    .prepare_provisional_studio_tail(store, id, *seed)
                    .ok()
                    .map(|a| {
                        PreviewJob::Network(Box::pin(async {
                            PreviewCompletion::Tail(Box::new(a.fetch().await))
                        }))
                    }),
                Next::Prepare(work) => {
                    // try_acquire respects already queued authoritative waiters in the shared
                    // semaphore. Preview parsing never queues ahead of them or holds a permit
                    // while waiting for network/vault custody.
                    let (pool, preview_pool) = self.preparation_pools();
                    let Ok(preview_permit) = preview_pool.try_acquire_owned() else {
                        self.next = Some(Next::Prepare(work));
                        return true;
                    };
                    match pool.try_acquire_owned() {
                        Ok(permit) => {
                            #[cfg(test)]
                            let work = match self.pause_parse.take() {
                                Some((entered, release)) => {
                                    PreviewPreparation::Paused(Box::new(work), entered, release)
                                }
                                None => work,
                            };
                            Some(PreviewJob::Prepare(work, permit, preview_permit))
                        }
                        Err(_) => {
                            self.next = Some(Next::Prepare(work));
                            return true;
                        }
                    }
                }
            };
            if self.job.is_none() {
                self.active = None;
            }
            return true;
        }
        if self
            .queue
            .front()
            .is_some_and(|r| server.runtime_clock().monotonic_ms() < r.head_after)
        {
            return false;
        }
        let Some(mut retry) = self.queue.pop_front() else {
            return false;
        };
        // Eviction is independent of successful installation. Native delivery and cancelled
        // lower workers can still retain these slots; reservation below remains authoritative.
        if self.ready.len() == 3 {
            let old = self.ready.pop_front().expect("three ready previews");
            self.notice(old.target);
        }
        self.generation = Arc::new(());
        retry.epoch = server.sync.epoch();
        retry.expires = server.runtime_clock().monotonic_ms().saturating_add(60_000);
        self.job = server
            .prepare_provisional_studio_discovery(store, id, retry.peer, &retry.watch)
            .ok()
            .map(|a| {
                PreviewJob::Network(Box::pin(async {
                    PreviewCompletion::Head(Box::new(a.fetch().await))
                }))
            });
        if self.job.is_some() {
            self.active = Some(retry);
        }
        true
    }
}

#[cfg(test)]
impl StudioReceiver {
    /// Observe the actual actor cache and probe its own capacity allocator. The temporary
    /// reservations are released before returning; no source, queue, selection or clock changes.
    /// An optional barrier pauses the next real parser only after it owns both worker permits.
    pub(crate) fn scheduling_for_test<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        pause_parse: Option<(
            tokio::sync::oneshot::Sender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    ) -> (Vec<StudioTarget>, usize) {
        if pause_parse.is_some() {
            assert!(self.catchup.preview.pause_parse.is_none());
            self.catchup.preview.pause_parse = pause_parse;
        }
        let ready = self
            .catchup
            .preview
            .ready
            .iter()
            .filter(|r| r.seed.unconfirmed_is_unexpired())
            .map(|r| r.target)
            .collect();
        let mut free = Vec::new();
        while let Ok(slot) = server.sync.reserve_provisional_checkpoint_capacity() {
            free.push(slot);
        }
        (ready, free.len())
    }
}

/// The harness drives the production queue/jobs/current-scope completion, with real fixture
/// transports. It supplies no parsed seed, arbitrary ready-cache entry or capacity counter.
#[cfg(test)]
pub(crate) struct PreviewHarness(PreviewRuntime);
#[cfg(test)]
impl Default for PreviewHarness {
    fn default() -> Self {
        // The production pool is process-wide. Separate fixture pools keep lifetime assertions
        // deterministic beside parallel actor tests; scheduling and actual workers are shared.
        Self(PreviewRuntime {
            pools: Some((
                Arc::new(tokio::sync::Semaphore::new(4)),
                Arc::new(tokio::sync::Semaphore::new(3)),
            )),
            ..Default::default()
        })
    }
}
#[cfg(test)]
impl PreviewHarness {
    pub(crate) fn parser_pool(&self) -> Arc<tokio::sync::Semaphore> {
        self.0.preparation_pools().0
    }
    pub(crate) fn into_receiver(self, watches: Vec<(ServerStudioWatch, u128)>) -> StudioReceiver {
        let mut receiver = StudioReceiver {
            watches: watches.into(),
            ..Default::default()
        };
        receiver.catchup.preview = self.0;
        receiver
    }
    pub(crate) fn tail_preparation(
        &self,
        work: ServerProvisionalStudioTailPreparation,
    ) -> PreviewJob {
        PreviewJob::Prepare(
            PreviewPreparation::Tail(Box::new(work)),
            self.0.preparation_pools().0.try_acquire_owned().unwrap(),
            self.0.preparation_pools().1.try_acquire_owned().unwrap(),
        )
    }
    pub(crate) fn queue(&mut self, watch: &ServerStudioWatch, peer: PeerId, now: u64) {
        self.0.queue(watch, peer, now);
    }
    pub(crate) fn step<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
    ) {
        self.0.maintain(server, store, id);
        self.0.schedule(server, store, id);
    }
    pub(crate) fn take_job(&mut self) -> Option<PreviewJob> {
        self.0.job.take()
    }
    pub(crate) fn complete(&mut self, result: PreviewCompletion) {
        self.0.complete(self.0.generation(), result);
    }
    pub(crate) fn ready(&self) -> usize {
        self.0.ready.len()
    }
    pub(crate) fn read<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
        store: &ServerStore,
        id: u64,
        target: StudioTarget,
    ) -> Option<StudioPreview> {
        self.0.read(server, store, id, target)
    }
}
