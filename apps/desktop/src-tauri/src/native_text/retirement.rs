//! Native ownership of Root's canonical UI-key retirement. The worker retains
//! only its original batch and fee-only completion control, never AppState or
//! the affine vault. Losing an IPC waiter cannot cancel already-started closure.
use super::*;
use catcoms_app::store::{
    CriticalCatalogReadPool, NativeTextUiRetirement, NativeTextUiRetirementBatch,
    NativeTextUiRetirementDrain, NativeTextUiRetirementState, NativeUiInvocationMetadata,
};
use std::sync::atomic::AtomicU8;
use tokio::sync::Notify;

const PENDING: u8 = 0;
const COMPLETE: u8 = 1;
const FAILED: u8 = 2;
const UNAVAILABLE: &str = "Chat is still locking. Try again.";
const FAILED_MESSAGE: &str =
    "Chat could not finish locking. Unlock is unavailable for this session.";

pub(super) struct Coalescer {
    active: StdMutex<Option<Arc<Job>>>,
    waiters: Arc<Semaphore>,
    // An independent account: retirement cannot depend on spare credit in the
    // very canonical-generation registry it must drain to release capacity.
    budget: NativeUiInvocationBudget,
    // LAST: control/slots retire before their independent metadata allowance.
    _metadata: NativeUiInvocationMetadata,
}
impl Default for Coalescer {
    fn default() -> Self {
        let budget = NativeUiInvocationBudget::new(
            CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity())
                .expect("fixed retirement policy"),
        )
        .expect("fixed retirement capacity");
        let metadata = budget.prepare().expect("fixed retirement control payment");
        Self {
            active: StdMutex::new(None),
            waiters: Arc::new(Semaphore::new(8)),
            budget,
            _metadata: metadata,
        }
    }
}

/// Counts native mutation preludes, not successful persistence. Begin before
/// publishing the lock generation; finish only after snapshot custody and the
/// early resumable=false assignment. A close holds its outer guard through
/// destruction, so no unlock may reopen between the final drain and destroy.
pub(crate) struct Prelude {
    active: std::sync::atomic::AtomicUsize,
    mutations: std::sync::atomic::AtomicUsize,
    changed: Notify,
    #[cfg(test)]
    waiting: Notify,
    _metadata: NativeUiInvocationMetadata,
}
pub(crate) struct PreludeGuard {
    original: Arc<Prelude>,
    mutation: bool,
}
impl Prelude {
    pub(crate) fn begin(self: &Arc<Self>) -> Result<PreludeGuard, &'static str> {
        self.begin_kind(true)
    }
    pub(crate) fn begin_close(self: &Arc<Self>) -> Result<PreludeGuard, &'static str> {
        self.begin_kind(false)
    }
    fn begin_kind(self: &Arc<Self>, mutation: bool) -> Result<PreludeGuard, &'static str> {
        self.active
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1))
            .map_err(|_| UNAVAILABLE)?;
        if mutation
            && self
                .mutations
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1))
                .is_err()
        {
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.changed.notify_waiters();
            return Err(UNAVAILABLE);
        }
        // A finite producer increments mutations BEFORE its SC registration.
        // If close overtakes the tiny active->mutations window, its later
        // authentication-incarnation check still rejects a superseded capture;
        // any capture sampling after close SC is already unauthorized.
        Ok(PreludeGuard {
            original: self.clone(),
            mutation,
        })
    }
    pub(crate) fn is_idle_cached(&self) -> bool {
        self.active.load(Ordering::SeqCst) == 0
    }
    #[cfg(test)]
    pub(super) async fn wait_blocked_for_test(&self) {
        self.waiting.notified().await;
    }
    pub(crate) async fn wait_idle(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.is_idle_cached() {
                return;
            }
            #[cfg(test)]
            self.waiting.notify_one();
            changed.await;
        }
    }
}
impl PreludeGuard {
    /// A close holds THIS guard through destroy. Older producers must finish
    /// snapshot registration before the final save/ack decision; later locked
    /// echoes cannot create fresh authorized payloads.
    pub(crate) async fn wait_for_other_mutations(&self) {
        loop {
            let changed = self.original.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.original.mutations.load(Ordering::SeqCst) == 0 {
                return;
            }
            changed.await;
        }
    }
}
impl Drop for PreludeGuard {
    fn drop(&mut self) {
        if self.mutation {
            self.original.mutations.fetch_sub(1, Ordering::SeqCst);
        }
        self.original.active.fetch_sub(1, Ordering::SeqCst);
        self.original.changed.notify_waiters();
    }
}
impl Coalescer {
    #[cfg(test)]
    pub(super) fn poison_for_test(&self) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = self.active.lock().unwrap();
            panic!("fixture poison of the actual coalescer mutex");
        }));
        assert!(result.is_err());
    }
    pub(super) fn prelude(&self) -> Arc<Prelude> {
        Arc::new(Prelude {
            active: std::sync::atomic::AtomicUsize::new(0),
            mutations: std::sync::atomic::AtomicUsize::new(0),
            changed: Notify::new(),
            #[cfg(test)]
            waiting: Notify::new(),
            _metadata: self
                .budget
                .prepare()
                .expect("fixed prelude control payment"),
        })
    }
}

struct Job {
    status: AtomicU8,
    // One original, negative-only counter observation. Install before publishing
    // PENDING; a failed task must never substitute a replacement Root registry.
    drain: std::sync::OnceLock<NativeTextUiRetirementDrain>,
    completed: Notify,
    _metadata: NativeUiInvocationMetadata,
}
impl Job {
    fn new(metadata: NativeUiInvocationMetadata) -> Arc<Self> {
        Arc::new(Self {
            status: AtomicU8::new(PENDING),
            drain: std::sync::OnceLock::new(),
            completed: Notify::new(),
            _metadata: metadata,
        })
    }
    fn is_finished_cached(&self) -> bool {
        match self.status.load(Ordering::Acquire) {
            COMPLETE => true,
            FAILED => self
                .drain
                .get()
                .is_some_and(|original| original.is_drained_cached()),
            _ => false,
        }
    }
    async fn wait(&self) -> Result<(), &'static str> {
        loop {
            let notified = self.completed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match self.status.load(Ordering::Acquire) {
                COMPLETE => return Ok(()),
                FAILED => {
                    // Task destruction closes affine owners, but admitted key
                    // Ends may still exist. Only THEIR original counters can
                    // establish recovery, even if the mounted vault changed.
                    self.drain.get().ok_or(FAILED_MESSAGE)?.wait().await;
                    return Ok(());
                }
                _ => notified.await,
            }
        }
    }
}

struct Completion {
    original: Arc<Job>,
    finished: bool,
}
impl Completion {
    fn finish(&mut self) {
        self.finished = true;
        self.original.status.store(COMPLETE, Ordering::Release);
        self.original.completed.notify_waiters();
    }
}
impl Drop for Completion {
    fn drop(&mut self) {
        if !self.finished {
            self.original.status.store(FAILED, Ordering::Release);
            self.original.completed.notify_waiters();
        }
    }
}

// Field order also covers a task dropped completely unpolled: actual affine
// close owners retire BEFORE failure is reported to coalesced waiters.
struct OwnedTask {
    batch: Option<NativeTextUiRetirementBatch>,
    completion: Completion,
}
impl OwnedTask {
    async fn run(mut self) {
        if let Some(batch) = self.batch.take() {
            batch.retire().await;
        }
        self.completion.finish();
    }
}

enum Step {
    Started(Arc<Job>),
    Waiting(Arc<Job>),
    Drained,
}

/// Called AFTER native publishes its SC lock generation, and BEFORE either
/// lock completion or unlock authorization can acquire ui_session_commit.
/// Root's bound is inclusive; only generations OLDER than this barrier close.
pub(crate) async fn older_than(state: &AppState, generation: u64) -> Result<(), String> {
    #[cfg(test)]
    {
        let queue = state.native_text.retirement_queue.lock().unwrap().clone();
        if let Some(queue) = queue {
            return older_with_spawn(state, generation, move |task| {
                queue.lock().unwrap().push(task);
                state.native_text.retirement_enqueued.notify_one();
            })
            .await;
        }
    }
    older_with_spawn(state, generation, |task| {
        drop(tauri::async_runtime::spawn(task));
    })
    .await
}

// Scheduling is the only test seam. It cannot replace a Root batch, native
// generation, actual mounted owner or the final original-generation checks.
pub(super) async fn older_with_spawn(
    state: &AppState,
    generation: u64,
    spawn: impl Fn(Task),
) -> Result<(), String> {
    let control = &state.native_text.retirement;
    // Preserve Legacy's existing lock path, including its concurrency behavior.
    // A pending old native drain still remains a GLOBAL resource barrier.
    let mut old = None;
    let bypass = {
        let store = state.store.lock().await;
        let mut active = control.active.lock().map_err(|_| UNAVAILABLE)?;
        if store.as_ref().and_then(MountedVault::as_text_v2).is_none()
            && active.as_ref().is_none_or(|job| job.is_finished_cached())
        {
            old = active.take();
            true
        } else {
            false
        }
    };
    drop(old);
    if bypass {
        return Ok(());
    }
    let _waiter = control
        .waiters
        .clone()
        .try_acquire_owned()
        .map_err(|_| UNAVAILABLE)?;
    // One live caller frame and, separately, one candidate task/control object.
    // Both reservations precede any native/Root guard or batch extraction.
    let _caller = control.budget.prepare().map_err(|_| UNAVAILABLE)?;
    loop {
        let mut candidate = Some(Job::new(control.budget.prepare().map_err(|_| UNAVAILABLE)?));
        // Arm failure BEFORE publishing a task or extracting any Root batch.
        // Declared before original: unwind drops actual close owners first,
        // then notifies failure, even in the publication-before-spawn gap.
        let mut completion = Some(Completion {
            original: candidate.as_ref().expect("paid candidate").clone(),
            finished: false,
        });
        // Declare all outgoing owners before guards. Every losing candidate,
        // old completion, batch and GC object is destroyed outside those guards.
        let mut original: Option<NativeTextUiRetirement> = None;
        let mut previous = None;
        let mut garbage = None;
        let step = {
            // This is the ONLY lock order: Store -> coalescer -> Root registry.
            // No await, callback, pool admission or owning Drop under coalescer.
            let store = state.store.lock().await;
            let mut active = control.active.lock().map_err(|_| UNAVAILABLE)?;
            // A failed task remains a barrier until its own CLOSED+zero
            // counters establish drainage. Missing original proof refuses in
            // wait(); another mount can never clear this barrier by being empty.
            if let Some(job) = active.as_ref().filter(|job| !job.is_finished_cached()) {
                Step::Waiting(job.clone())
            } else if let Some(vault) = store.as_ref().and_then(MountedVault::as_text_v2) {
                let through = generation.checked_sub(1).ok_or(UNAVAILABLE)?;
                original = Some(
                    vault
                        .begin_ui_retirement(through)
                        .map_err(|_| UNAVAILABLE)?,
                );
                match original.as_ref().expect("captured above").state() {
                    NativeTextUiRetirementState::Started => {
                        let job = candidate.take().expect("one independently paid candidate");
                        job.drain
                            .set(
                                original
                                    .as_ref()
                                    .expect("captured original")
                                    .observe_drain()
                                    .expect("Started carries the original batch"),
                            )
                            .expect("one publication");
                        previous = active.replace(job.clone());
                        Step::Started(job)
                    }
                    // No task here owns that batch. Never pretend it drained or
                    // refund/reconstruct a foreign/abandoned close capability.
                    NativeTextUiRetirementState::AlreadyRetiring => return Err(UNAVAILABLE.into()),
                    NativeTextUiRetirementState::KeyWorkDrained => {
                        garbage = Some(vault.collect_retired_ui().map_err(|_| UNAVAILABLE)?);
                        previous = active.take();
                        Step::Drained
                    }
                }
            } else {
                previous = active.take();
                Step::Drained
            }
        };
        drop(garbage);
        drop(previous);
        drop(candidate);
        match step {
            Step::Drained => return Ok(()),
            Step::Waiting(job) => {
                job.wait().await.map_err(str::to_string)?;
                // Re-probe the actual mounted registry. Completion of one old
                // batch cannot establish that another generation has drained.
            }
            Step::Started(job) => {
                #[cfg(test)]
                assert!(
                    !state
                        .native_text
                        .panic_retirement_publication
                        .swap(false, Ordering::SeqCst),
                    "fixture panic after original retirement publication"
                );
                let completion = completion.take().expect("armed before publication");
                let batch = original
                    .take()
                    .and_then(NativeTextUiRetirement::into_batch)
                    .ok_or(UNAVAILABLE)?;
                spawn(Box::pin(
                    OwnedTask {
                        batch: Some(batch),
                        completion,
                    }
                    .run(),
                ));
                job.wait().await.map_err(str::to_string)?;
            }
        }
    }
}

// A task/control quote is separate from the caller quote. Root already funds
// each actual batch/record; this pins the extra native wrappers to their4KiB.
const _: () = assert!(
    2 * std::mem::size_of::<OwnedTask>()
        + std::mem::size_of::<Job>()
        + std::mem::size_of::<Prelude>()
        + std::mem::size_of::<Coalescer>()
        + 64 * std::mem::size_of::<usize>()
        < 4096
);
