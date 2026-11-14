//! Final-screen transactions. Cells retain DATA only; an independently owned
//! blocking job retains Root IO/output until its recorder runs, then releases
//! the shared lane LAST. Neither IPC observer loss nor panic loses retry bytes.
use super::*;
use tokio::sync::OwnedMutexGuard;

type Target = Arc<NativeTextVaultAuthenticationCheck>;
#[derive(Clone)]
enum Failure {
    Busy,
    Unavailable,
    Interrupted,
    Foreign,
    Invalid(String),
    Io(String),
}
impl Failure {
    fn message(&self) -> String {
        match self {
            Self::Busy => "Screen-state saving is busy. Try again.".into(),
            Self::Unavailable => "The vault is unavailable for saving screen state.".into(),
            Self::Interrupted => "Screen-state saving was interrupted. Try again.".into(),
            Self::Foreign => "The pending screen state belongs to a different vault.".into(),
            Self::Invalid(error) => format!("Screen state could not be saved: {error}"),
            Self::Io(error) => format!("Screen state could not be saved: {error}"),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Policy {
    Retry,
    Loss,
}
struct Revision {
    serial: Option<u64>,
}
#[derive(Clone)]
enum Outcome {
    Waiting,
    Saved,
    Failed(Arc<Failure>, Policy),
}
struct Attempt {
    generation: u64,
    outcome: Outcome,
    revision: Arc<Revision>,
    source: Arc<Revision>,
    target: Option<Target>,
    auth_warned: bool,
    close_shown: bool,
}
#[derive(Clone, Copy)]
enum NoticeKind {
    Warn,
    Acknowledge,
    Close,
}
/// Correlation metadata only; created after real authentication/close checks.
/// Updating it happens only after the matching actual native response returns.
pub(crate) struct Notice {
    generation: u64,
    revision: Arc<Revision>,
    target: Option<Target>,
    kind: NoticeKind,
}

struct Payload {
    generation: u64,
    source: Arc<Revision>,
    json: String,
    target: Target,
    // LAST: all payload bytes/metadata retire before returning buffer capacity.
    _permit: OwnedSemaphorePermit,
}
#[derive(Default)]
struct State {
    pending: Option<Arc<Payload>>,
    latest: Option<Attempt>,
    active: Option<Arc<()>>,
    pending_failure: Option<Arc<Revision>>,
    control_busy: Option<u64>,
    saved: Option<SavedOrder>,
}
struct SavedOrder {
    generation: u64,
    target: Target,
    order: Arc<Revision>,
}
/// Opaque capture from the actual native main-window router, before scheduling.
/// Authorization is sampled before the lock; registration checks the immutable
/// authentication incarnation and original target. DATA is published before
/// scheduling, so a later poll cannot rebrand or reinsert an old snapshot.
pub(crate) struct Capture {
    generation: u64,
    authorized_generation: u64,
    authorized: bool,
    source: Option<Arc<Revision>>,
    target: Option<Target>,
}
enum Captured {
    None,
    Payload(String, Target, OwnedSemaphorePermit),
    Failed(Arc<Failure>),
}
pub(crate) struct Snapshots {
    state: Arc<StdMutex<State>>,
    payloads: Arc<Semaphore>,
    controls: Arc<Semaphore>,
    io: NativeUiStateBudget,
    sequence: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    pub(in crate::native_text) before_io: StdMutex<Option<Arc<tests::BlockingIoBarrier>>>,
    #[cfg(test)]
    pub(in crate::native_text) before_publish: StdMutex<Option<Arc<tests::BlockingIoBarrier>>>,
    #[cfg(test)]
    pub(in crate::native_text) panic_after_write: Arc<AtomicBool>,
    #[cfg(test)]
    pub(in crate::native_text) drop_job: AtomicBool,
}
impl Default for Snapshots {
    fn default() -> Self {
        Self {
            state: Arc::new(StdMutex::new(State::default())),
            sequence: std::sync::atomic::AtomicU64::new(1),
            payloads: Arc::new(Semaphore::new(3)),
            controls: Arc::new(Semaphore::new(8)),
            // Independent from the four ordinary requests. Only the lane owner
            // can capture one snapshot IO credit; it is never Arc-shared.
            io: NativeUiStateBudget::new(
                CriticalCatalogReadPool::new(NativeUiStateBudget::required_capacity())
                    .expect("fixed snapshot IO pool"),
            )
            .expect("fixed snapshot IO policy"),
            #[cfg(test)]
            before_io: StdMutex::new(None),
            #[cfg(test)]
            before_publish: StdMutex::new(None),
            #[cfg(test)]
            panic_after_write: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            drop_job: AtomicBool::new(false),
        }
    }
}
impl Snapshots {
    fn revision(&self) -> Arc<Revision> {
        Arc::new(Revision {
            serial: self
                .sequence
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                    value.checked_add(1)
                })
                .ok(),
        })
    }
    pub(crate) fn capture(&self, app: &AppState, json: Option<&str>) -> Capture {
        // Count synchronous receipt/copy/publication, never the unpolled task.
        // Close/auth therefore sees all earlier owned bytes without waiting on
        // another long-lived close fence or preventing a later reauthentication.
        let _copy = app.native_text.preludes.begin();
        let generation = app.ui_session_generation.load(Ordering::SeqCst);
        let authorized_generation = app.ui_authorized_generation.load(Ordering::SeqCst);
        let authorized = authorized_generation == generation
            && !app.session_lock_requested.load(Ordering::SeqCst)
            && _copy.is_ok();
        let target = if authorized {
            app.native_text.ui_state.original_target()
        } else {
            None
        };
        let source = (authorized && json.is_some()).then(|| self.revision());
        let value = match (authorized, json) {
            (true, Some(json)) if json.len() > MAX_UI_STATE_BYTES => Captured::Failed(Arc::new(
                Failure::Invalid("Screen state is too large.".into()),
            )),
            (true, Some(json)) => match target.clone() {
                Some(target) => match self.payloads.clone().try_acquire_owned() {
                    Ok(permit) => Captured::Payload(json.to_owned(), target, permit),
                    Err(_) => Captured::Failed(Arc::new(Failure::Busy)),
                },
                None => Captured::Failed(Arc::new(Failure::Unavailable)),
            },
            _ => Captured::None,
        };
        let capture = Capture {
            generation,
            authorized_generation,
            authorized,
            source,
            target,
        };
        #[cfg(test)]
        if capture.source.is_some() {
            let edge = self.before_publish.lock().unwrap().clone();
            if let Some(edge) = edge {
                edge.wait_once();
            }
        }
        self.publish_capture(&capture, value);
        capture
    }
    fn malformed(&self, app: &AppState) -> Capture {
        let _copy = app.native_text.preludes.begin();
        let mut capture = self.capture(app, None);
        if capture.authorized {
            capture.source = Some(self.revision());
            self.publish_capture(
                &capture,
                Captured::Failed(Arc::new(Failure::Invalid(
                    "Invalid screen-state request.".into(),
                ))),
            );
        }
        capture
    }
    /// Publish DATA at synchronous ingress. No command/key/IO authority is
    /// published here. The original target and a real SC/auth edge are checked
    /// again before disk; a dropped unpolled task cannot lose its only copy.
    fn publish_capture(&self, capture: &Capture, value: Captured) {
        let Some(source) = capture.source.as_ref() else {
            return;
        };
        let Some(order) = source.serial else { return };
        let (pending, outcome) = match value {
            Captured::None => return,
            Captured::Payload(json, target, permit) => (
                Some(Arc::new(Payload {
                    generation: capture.generation,
                    source: source.clone(),
                    json,
                    target,
                    _permit: permit,
                })),
                Outcome::Waiting,
            ),
            Captured::Failed(error) => (None, Outcome::Failed(error, Policy::Loss)),
        };
        let retired = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            // An earlier producer can finish copying after a newer ordinary
            // save completed. Keep its durable watermark even when there were
            // no pending bytes at the time of that save.
            if state.saved.as_ref().is_some_and(|saved| {
                saved.generation == capture.generation
                    && capture
                        .target
                        .as_ref()
                        .is_some_and(|t| t.same_original(&saved.target))
                    && saved.order.serial.is_some_and(|newer| newer > order)
            }) {
                return;
            }
            let replace_pending = pending.as_ref().is_some_and(|_| {
                state
                    .pending
                    .as_ref()
                    .is_none_or(|p| p.source.serial.is_some_and(|old| old < order))
            });
            let old_failure = if replace_pending {
                state.pending_failure.take()
            } else {
                None
            };
            let old_pending = if replace_pending {
                pending.map(|p| state.pending.replace(p))
            } else {
                None
            };
            let old_latest = if state
                .latest
                .as_ref()
                .is_none_or(|a| a.source.serial.is_some_and(|old| old < order))
            {
                state.latest.replace(Attempt {
                    generation: capture.generation,
                    outcome,
                    revision: source.clone(),
                    source: source.clone(),
                    target: capture.target.clone(),
                    auth_warned: false,
                    close_shown: false,
                })
            } else {
                None
            };
            (old_pending, old_latest, old_failure)
        };
        drop(retired);
    }
    /// Registration validates the original UI incarnation, not task poll order.
    /// It NEVER reinserts an already saved ingress payload. Same-session locks
    /// may coalesce; a different reauthentication remains superseded, even on
    /// the same Host or an unchanged numeric lock generation.
    pub(crate) fn register(&self, app: &AppState, capture: Capture, generation: u64) -> bool {
        let same_auth =
            app.ui_authorized_generation.load(Ordering::SeqCst) == capture.authorized_generation;
        let target_matches = capture.target.as_ref().is_none_or(|target| {
            target.is_live_cached()
                && app
                    .native_text
                    .ui_state
                    .original_target()
                    .is_some_and(|current| current.same_original(target))
        });
        generation != 0 && same_auth && (!capture.authorized || target_matches)
    }
    fn busy(&self, generation: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.control_busy = Some(state.control_busy.unwrap_or(0).max(generation));
    }
    fn overflow_capture(&self, app: &AppState, json_present: bool) -> Capture {
        let _copy = app.native_text.preludes.begin();
        let mut capture = self.capture(app, None);
        if capture.authorized && json_present {
            capture.source = Some(self.revision());
            self.publish_capture(&capture, Captured::Failed(Arc::new(Failure::Busy)));
        }
        capture
    }
    pub(crate) fn ordinary_completion(
        &self,
        generation: u64,
        target: Target,
    ) -> OrdinaryCompletion {
        OrdinaryCompletion {
            state: self.state.clone(),
            generation,
            target,
            order: self.revision(),
        }
    }
    /// Read only: a stale auth attempt must never consume a warning. Receipts
    /// commit only at the actual matching native response edge below.
    fn result(&self, authentication: bool) -> (Result<(), String>, Option<Notice>) {
        let (outcome, notice) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.control_busy.is_some() {
                return (Err(Failure::Busy.message()), None);
            }
            let outcome = state.latest.as_ref().map(|a| a.outcome.clone());
            let notice = state.latest.as_ref().and_then(|a| {
                if authentication
                    && state.pending.is_none()
                    && state.active.is_none()
                    && matches!(a.outcome, Outcome::Failed(_, Policy::Loss))
                {
                    Some(Notice {
                        generation: a.generation,
                        revision: a.revision.clone(),
                        target: a.target.clone(),
                        kind: if a.auth_warned {
                            NoticeKind::Acknowledge
                        } else {
                            NoticeKind::Warn
                        },
                    })
                } else {
                    None
                }
            });
            (outcome, notice)
        };
        let result = match outcome {
            Some(Outcome::Waiting) => Err("Screen state is waiting to be saved. Try again.".into()),
            Some(Outcome::Failed(error, _)) => {
                if matches!(
                    notice.as_ref().map(|n| n.kind),
                    Some(NoticeKind::Acknowledge)
                ) {
                    Ok(())
                } else if notice.is_some() {
                    Err(format!(
                        "{} Unlock again to continue without that screen state.",
                        error.message()
                    ))
                } else {
                    Err(error.message())
                }
            }
            _ => Ok(()),
        };
        (result, notice)
    }
    /// Null/control-only refusal may resolve after a later actual key drain;
    /// it is never converted into a discardable loss notice.
    fn settle_control(&self) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .control_busy = None;
    }
    fn discard_matches(state: &State, serial: Option<u64>) -> bool {
        let Some(attempt) = state.latest.as_ref() else {
            return false;
        };
        state.control_busy.is_none()
            && state.active.is_none()
            && attempt.close_shown
            && matches!(
                attempt.outcome,
                Outcome::Failed(_, Policy::Loss | Policy::Retry)
            )
            && state.pending.as_ref().is_none_or(|p| {
                Arc::ptr_eq(&p.source, &attempt.source)
                    || state
                        .pending_failure
                        .as_ref()
                        .is_some_and(|failed| Arc::ptr_eq(failed, &p.source))
            })
            && attempt.revision.serial.is_some()
            && attempt.revision.serial == serial
    }
    pub(crate) fn can_discard(&self, token: Option<&str>) -> bool {
        let serial = token
            .filter(|t| {
                !t.is_empty()
                    && t.len() <= 20
                    && !(t.len() > 1 && t.starts_with('0'))
                    && t.bytes().all(|b| b.is_ascii_digit())
            })
            .and_then(|t| t.parse::<u64>().ok());
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::discard_matches(&state, serial)
    }
    pub(crate) fn close_decision(&self, token: Option<&str>) -> CloseDecision {
        let serial = token
            .filter(|t| {
                !t.is_empty()
                    && t.len() <= 20
                    && !(t.len() > 1 && t.starts_with('0'))
                    && t.bytes().all(|b| b.is_ascii_digit())
            })
            .and_then(|t| t.parse::<u64>().ok());
        let (busy, outcome, acknowledged, notice) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let busy = state.control_busy.is_some();
            let outcome = state.latest.as_ref().map(|a| a.outcome.clone());
            let notice = if state.control_busy.is_none() && state.active.is_none() {
                state
                    .latest
                    .as_ref()
                    .filter(|a| {
                        matches!(a.outcome, Outcome::Failed(_, Policy::Loss | Policy::Retry))
                    })
                    .map(|a| Notice {
                        generation: a.generation,
                        revision: a.revision.clone(),
                        target: a.target.clone(),
                        kind: NoticeKind::Close,
                    })
            } else {
                None
            };
            (busy, outcome, Self::discard_matches(&state, serial), notice)
        };
        let result = if busy {
            Err(Failure::Busy.message())
        } else {
            match outcome {
                Some(Outcome::Failed(error, _)) => Err(error.message()),
                Some(Outcome::Waiting) => {
                    Err("Screen state is waiting to be saved. Try closing again.".into())
                }
                _ => Ok(()),
            }
        };
        let revision = notice
            .as_ref()
            .and_then(|n| n.revision.serial)
            .map(|s| s.to_string());
        CloseDecision {
            result,
            acknowledged,
            revision,
            notice,
        }
    }
    pub(crate) async fn notice_is_current(&self, state: &AppState, notice: &Notice) -> bool {
        let matches = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .latest
            .as_ref()
            .is_some_and(|a| Arc::ptr_eq(&a.revision, &notice.revision));
        if notice.generation > state.ui_session_generation.load(Ordering::SeqCst) {
            return false;
        }
        if !matches {
            return false;
        }
        if matches!(notice.kind, NoticeKind::Close) {
            return true;
        }
        if let Some(target) = &notice.target {
            let store = state.store.lock().await;
            store
                .as_ref()
                .and_then(MountedVault::as_text_v2)
                .is_some_and(|vault| target.is_for_original_vault(vault))
        } else {
            false
        }
    }
    /// Invoke only AFTER the actual responder returned under its checked UI
    /// commit. Returning the retired value keeps owning drops outside commit.
    pub(crate) fn publish_notice(&self, notice: &Notice) -> Option<RetiredNotice> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state
            .latest
            .as_ref()
            .is_some_and(|a| Arc::ptr_eq(&a.revision, &notice.revision))
        {
            return None;
        }
        match notice.kind {
            NoticeKind::Warn => {
                state.latest.as_mut().unwrap().auth_warned = true;
                None
            }
            NoticeKind::Close => {
                state.latest.as_mut().unwrap().close_shown = true;
                None
            }
            NoticeKind::Acknowledge
                if state.pending.is_none()
                    && state.active.is_none()
                    && state.latest.as_ref().is_some_and(|a| a.auth_warned) =>
            {
                state
                    .latest
                    .take()
                    .map(|attempt| RetiredNotice { _attempt: attempt })
            }
            _ => None,
        }
    }
}
/// A completed ordinary save supersedes only older DATA for the same original
/// target/authentication. Mark it before releasing the lane so a queued older
/// lock cannot overwrite a newer durable screen state.
pub(crate) struct OrdinaryCompletion {
    state: Arc<StdMutex<State>>,
    generation: u64,
    target: Target,
    order: Arc<Revision>,
}
impl OrdinaryCompletion {
    /// Called only by the actual IO lane owner before a write. Request poll
    /// order is not ingress order; an old ordinary writer must not regress a
    /// later durable save after that save already superseded a snapshot.
    pub(crate) fn may_write(&self) -> bool {
        let Some(order) = self.order.serial else {
            return false;
        };
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        !state.saved.as_ref().is_some_and(|saved| {
            saved.generation == self.generation
                && saved.target.same_original(&self.target)
                && saved.order.serial.is_some_and(|newer| newer > order)
        })
    }
    pub(crate) fn saved(&self) {
        let Some(order) = self.order.serial else {
            return;
        };
        let retired = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let saved = if state
                .saved
                .as_ref()
                .is_none_or(|saved| saved.order.serial.is_some_and(|old| old < order))
            {
                state.saved.replace(SavedOrder {
                    generation: self.generation,
                    target: self.target.clone(),
                    order: self.order.clone(),
                })
            } else {
                None
            };
            let pending = if state.pending.as_ref().is_some_and(|p| {
                p.generation == self.generation
                    && p.target.same_original(&self.target)
                    && p.source.serial.is_some_and(|old| old < order)
            }) {
                state.pending.take()
            } else {
                None
            };
            let failure = if pending.is_some() {
                state.pending_failure.take()
            } else {
                None
            };
            let latest = if state.latest.as_ref().is_some_and(|a| {
                a.generation == self.generation
                    && a.target
                        .as_ref()
                        .is_some_and(|t| t.same_original(&self.target))
                    && a.source.serial.is_some_and(|old| old < order)
            }) {
                state.latest.take()
            } else {
                None
            };
            (pending, latest, failure, saved)
        };
        drop(retired);
    }
}

pub(crate) struct CloseDecision {
    pub(crate) result: Result<(), String>,
    pub(crate) acknowledged: bool,
    pub(crate) revision: Option<String>,
    pub(crate) notice: Option<Notice>,
}
pub(crate) struct RetiredNotice {
    _attempt: Attempt,
}
struct Recorder {
    state: Arc<StdMutex<State>>,
    identity: Arc<()>,
    payload: Arc<Payload>,
    interrupted: Arc<Failure>,
    completed_revision: Arc<Revision>,
    interrupted_revision: Arc<Revision>,
    finished: bool,
}
impl Recorder {
    fn finish(&mut self, outcome: Outcome, revision: Arc<Revision>) {
        let retired = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if !state
                .active
                .as_ref()
                .is_some_and(|id| Arc::ptr_eq(id, &self.identity))
            {
                return;
            }
            let active = state.active.take();
            let old_pending_failure = if state
                .pending
                .as_ref()
                .is_some_and(|p| Arc::ptr_eq(p, &self.payload))
            {
                if matches!(outcome, Outcome::Failed(_, Policy::Retry)) {
                    state.pending_failure.replace(self.payload.source.clone())
                } else {
                    state.pending_failure.take()
                }
            } else {
                None
            };
            let pending = if state
                .pending
                .as_ref()
                .is_some_and(|p| Arc::ptr_eq(p, &self.payload))
                && matches!(outcome, Outcome::Saved | Outcome::Failed(_, Policy::Loss))
            {
                state.pending.take()
            } else {
                None
            };
            let previous = if state
                .latest
                .as_ref()
                .is_some_and(|a| Arc::ptr_eq(&a.source, &self.payload.source))
            {
                state.latest.as_mut().map(|a| {
                    let previous = std::mem::replace(&mut a.outcome, outcome);
                    let old_revision = std::mem::replace(&mut a.revision, revision);
                    a.auth_warned = false;
                    a.close_shown = false;
                    (previous, old_revision)
                })
            } else {
                None
            };
            (active, pending, previous, old_pending_failure)
        };
        self.finished = true;
        drop(retired);
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(
                Outcome::Failed(self.interrupted.clone(), Policy::Retry),
                self.interrupted_revision.clone(),
            );
        }
    }
}
/// Field order is the failure contract: all actual Root IO/output destruction
/// precedes interruption recording; the lane is released last, even unpolled.
struct Job {
    preparation: Option<NativeUiStatePreparation>,
    saved: Option<NativeUiStateSaved>,
    payload: Arc<Payload>,
    recorder: Recorder,
    lane: Option<OwnedMutexGuard<()>>,
    #[cfg(test)]
    before_io: Option<Arc<tests::BlockingIoBarrier>>,
    #[cfg(test)]
    panic_after_write: Arc<AtomicBool>,
}
impl Job {
    fn run(mut self) -> OwnedMutexGuard<()> {
        #[cfg(test)]
        if let Some(edge) = &self.before_io {
            edge.wait_once();
        }
        let outcome = match validate_ui_state_json(&self.payload.json) {
            Err(error) => Outcome::Failed(Arc::new(Failure::Invalid(error)), Policy::Loss),
            Ok(()) => match self
                .preparation
                .take()
                .expect("one original IO capsule")
                .save(self.payload.json.as_bytes(), &mut OsCryptoRng)
            {
                Ok(saved) => {
                    self.saved = Some(saved);
                    Outcome::Saved
                }
                Err(error) => {
                    Outcome::Failed(Arc::new(Failure::Io(error.to_string())), Policy::Retry)
                }
            },
        };
        #[cfg(test)]
        if self.saved.is_some() && self.panic_after_write.swap(false, Ordering::SeqCst) {
            panic!("fixture panic after real screen-state write");
        }
        drop(self.preparation.take());
        drop(self.saved.take());
        self.recorder
            .finish(outcome, self.recorder.completed_revision.clone());
        self.lane
            .take()
            .expect("lane remains through result recording")
    }
}
pub(crate) enum Purpose<'a> {
    Lock {
        keys_drained: bool,
        discard_token: Option<&'a str>,
    },
    Authentication {
        generation: u64,
        target: &'a Target,
    },
}
/// Caller already owns the ONE IO lane. No writer is held over disk or result
/// recording. At most three coalesced full snapshots per invocation; a sustained
/// newer capture leaves truthful pending state for the next bounded caller.
pub(crate) async fn flush(
    state: &AppState,
    mut lane: OwnedMutexGuard<()>,
    purpose: Purpose<'_>,
) -> (OwnedMutexGuard<()>, Result<(), String>, Option<Notice>) {
    let control = &state.native_text.ui_state.snapshots;
    if matches!(
        purpose,
        Purpose::Authentication { .. }
            | Purpose::Lock {
                keys_drained: true,
                ..
            }
    ) {
        control.settle_control();
    }
    for _ in 0..3 {
        if let Purpose::Lock { discard_token, .. } = &purpose {
            if control.can_discard(*discard_token) {
                let (result, _) = control.result(false);
                return (lane, result, None);
            }
        }
        let payload = control
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .clone();
        let Some(payload) = payload else { break };
        if let Purpose::Authentication { target, .. } = &purpose {
            if !target.same_original(&payload.target) || !payload.target.is_live_cached() {
                return (lane, Err(Failure::Foreign.message()), None);
            }
        }
        let credit = match control.io.prepare() {
            Ok(credit) => credit,
            Err(_) => return (lane, Err(Failure::Busy.message()), None),
        };
        let mut preparation = None;
        let admitted = {
            let _commit = state.ui_session_commit.lock().await;
            if payload.generation > state.ui_session_generation.load(Ordering::SeqCst) {
                false
            } else {
                match &purpose {
                    Purpose::Authentication { generation, .. }
                        if !state.native_text.preludes.is_idle_cached()
                            || state.ui_session_generation.load(Ordering::SeqCst)
                                != *generation =>
                    {
                        false
                    }
                    Purpose::Lock { .. }
                        if state.ui_authorized_generation.load(Ordering::SeqCst)
                            == state.ui_session_generation.load(Ordering::SeqCst)
                            && !state.session_lock_requested.load(Ordering::SeqCst) =>
                    {
                        false
                    }
                    _ => {
                        let store = state.store.lock().await;
                        if let Some(vault) = store.as_ref().and_then(MountedVault::as_text_v2) {
                            preparation = Some(
                                vault.prepare_ui_state_for_original(credit, payload.target.clone()),
                            );
                            true
                        } else {
                            false
                        }
                    }
                }
            }
        };
        if !admitted {
            return (lane, Err(Failure::Unavailable.message()), None);
        }
        let identity = Arc::new(());
        let recorder = Recorder {
            state: control.state.clone(),
            identity: identity.clone(),
            payload: payload.clone(),
            interrupted: Arc::new(Failure::Interrupted),
            completed_revision: control.revision(),
            interrupted_revision: control.revision(),
            finished: false,
        };
        // Recorder is armed BEFORE active publication or spawning. Cells contain
        // neither this Root preparation nor a future/guard/back-reference cycle.
        let job = Job {
            preparation,
            saved: None,
            payload: payload.clone(),
            recorder,
            lane: Some(lane),
            #[cfg(test)]
            before_io: control.before_io.lock().unwrap().clone(),
            #[cfg(test)]
            panic_after_write: control.panic_after_write.clone(),
        };
        let previous = control
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .replace(identity);
        drop(previous);
        #[cfg(test)]
        if control.drop_job.swap(false, Ordering::SeqCst) {
            drop(job); // WHOLE real owner, including its lane last
            lane = state.native_text.ui_state.lock_lane().await;
            let (result, _) = control.result(false);
            return (lane, result, None);
        }
        let returned = tauri::async_runtime::spawn_blocking(move || job.run()).await;
        lane = match returned {
            Ok(lane) => lane,
            Err(_) => {
                // The panic owner already destroyed Root output, recorded
                // Interrupted and released the original lane in that order.
                let lane = state.native_text.ui_state.lock_lane().await;
                let (result, _) = control.result(false);
                return (lane, result, None);
            }
        };
        let same_pending = control
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(p, &payload));
        if same_pending {
            break;
        } // retry IO only on a later caller, never spin
    }
    let (result, notice) = control.result(matches!(purpose, Purpose::Authentication { .. }));
    (lane, result, notice)
}

/// Native-mode lock/close entry, selected by trusted Rust startup policy only.
/// ACL and all ingress/payload bounds precede any copy or task publication.
pub(crate) fn route<R: tauri::Runtime>(invoke: Invoke<R>, spawn: &impl Fn(Task)) -> bool {
    if invoke.acl.is_none()
        || invoke.message.webview_ref().label() != "main"
        || invoke.message.webview_ref().window().label() != "main"
    {
        invoke
            .resolver
            .reject("Lock is not available from this window.");
        return true;
    }
    let close = invoke.message.command() == "close_vault_window";
    let app = invoke.message.webview_ref().app_handle().clone();
    let state = app.state::<AppState>();
    let control = &state.native_text.ui_state.snapshots;
    let permit = match control.controls.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            let json_present = matches!(invoke.message.payload(), InvokeBody::Json(v)
                if v.get("uiStateJson").is_some_and(serde_json::Value::is_string));
            let capture = control.overflow_capture(&state, json_present);
            let has_loss = capture.source.is_some();
            let prelude = state.native_text.preludes.begin();
            let generation = register_ui_lock_request(&state);
            revoke_ui_lock_owners(&state);
            if has_loss {
                control.register(&state, capture, generation);
            } else {
                control.busy(generation);
            }
            drop(prelude);
            if close {
                // A rejected bridge response triggers the frontend's emergency
                // destroy fallback. This is a known incomplete native lock, so
                // resolve deferred/non-discardable instead of authorizing exit.
                invoke.resolver.resolve(CloseVaultWindowOutcome {
                    continuity_error: None,
                    continuity_revision: None,
                    continuity_discardable: Some(false),
                    history_error: Some("Lock cleanup is busy. Try closing again.".into()),
                    deferred: true,
                    destroy_error: None,
                });
            } else {
                invoke.resolver.resolve(RegisteredLockSessionOutcome {
                    continuity_error: Some("Lock cleanup is busy. Try again.".into()),
                    generation: generation.to_string(),
                });
            }

            return true; // no task, drainage receipt or destroy permission
        }
    };
    let object = match invoke.message.payload() {
        InvokeBody::Json(v) => v.as_object(),
        _ => None,
    };
    let parsed = object
        .filter(|o| {
            o.keys().all(|k| {
                k == "uiStateJson"
                    || (close
                        && (k == "discardContinuityError" || k == "discardContinuityRevision"))
            })
        })
        .and_then(|o| {
            let json = match o.get("uiStateJson") {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::String(json)) => Some(json.as_str()),
                _ => return None,
            };
            let discard = match o.get("discardContinuityError") {
                None => false,
                Some(serde_json::Value::Bool(value)) => *value,
                _ => return None,
            };
            let revision = match o.get("discardContinuityRevision") {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::String(value)) if value.len() <= 20 => Some(value.clone()),
                _ => return None,
            };
            Some((json, discard, revision))
        });
    let (capture, discard, revision) = match parsed {
        Some((json, discard, revision)) => (control.capture(&state, json), discard, revision),
        None => (control.malformed(&state), false, None),
    };
    let resolver = invoke.resolver;
    drop(invoke.message);
    spawn(Box::pin(async move {
        let _permit = permit;
        let state = app.state::<AppState>();
        if close {
            let mut notice = None;
            let result = close_vault_window_with_capture(
                &app,
                &state,
                None,
                discard,
                revision,
                Some(capture),
                &mut notice,
            )
            .await;
            let mut _retired_notice = None;
            let _commit = state.ui_session_commit.lock().await;
            let shown = result.as_ref().is_ok_and(|r| {
                r.deferred && r.continuity_error.is_some() && r.history_error.is_none()
            });
            let current = if let Some(n) = notice.as_ref().filter(|_| shown) {
                control_notice_current(&state, n).await
            } else {
                false
            };
            match result {
                Ok(outcome) => resolver.resolve(outcome),
                Err(error) => resolver.reject(error),
            }
            if current {
                _retired_notice = state
                    .native_text
                    .ui_state
                    .snapshots
                    .publish_notice(notice.as_ref().unwrap());
            }
        } else {
            let (generation, snapshot, keys) =
                lock_session_with_capture(&state, None, Some(capture), None).await;
            resolver.resolve(RegisteredLockSessionOutcome {
                continuity_error: snapshot.err().or_else(|| keys.err()),
                generation: generation.to_string(),
            });
        }
    }));
    true
}

async fn control_notice_current(state: &AppState, notice: &Notice) -> bool {
    state
        .native_text
        .ui_state
        .snapshots
        .notice_is_current(state, notice)
        .await
}
