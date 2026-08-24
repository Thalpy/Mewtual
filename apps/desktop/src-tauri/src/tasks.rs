//! What the background tasks are doing, and what became of the ones that stopped.
//!
//! # Why a registry rather than a log line
//!
//! A panic in a background task used to produce one `tracing` line and nothing else. That line ages
//! out of the ring, and after it does the task's state is not "dead", it is *unknown*: the console
//! shows a healthy-looking app and the only evidence that half of it stopped working has scrolled
//! away. State that has to stay in a bounded buffer to be true is not state.
//!
//! # Why it matters more than it sounds
//!
//! The event forwarder is the clearest case. It can die while the server actor is perfectly
//! healthy: the protocol keeps running, membership keeps changing, messages keep arriving, and the
//! webview is told none of it. What a user sees is a stale unread badge, a stale jukebox queue and
//! stale presence, which is exactly the class of symptom this whole suite exists to explain, and
//! the app's own answer would have been that everything was fine.
//!
//! Only the server actor was supervised. Six other long-lived tasks had their handles dropped on
//! the floor. Found by adversarial review (P3-009).

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// What a supervised task is doing now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Running,
    /// Returned on its own, which is ordinary at shutdown.
    Exited,
    Cancelled,
    Panicked,
    /// Still running, but has not reported progress within the interval it declared.
    ///
    /// Only ever reported for a task that said how often it expects to do something. A task with
    /// nothing to say for an hour may be perfectly healthy, and inventing a stall for it would make
    /// the panel cry wolf until nobody reads it.
    Stalled,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskState::Running => "running",
            TaskState::Exited => "exited",
            TaskState::Cancelled => "cancelled",
            TaskState::Panicked => "panicked",
            TaskState::Stalled => "stalled",
        }
    }

    /// Whether this is a state somebody should be told about.
    ///
    /// `Exited` is not: it is what every task does at shutdown, and colouring it as a fault would
    /// paint an ordinary close as a crash.
    pub fn is_fault(self) -> bool {
        matches!(
            self,
            TaskState::Cancelled | TaskState::Panicked | TaskState::Stalled
        )
    }
}

/// One supervised task.
#[derive(Debug)]
struct TaskRecord {
    id: u64,
    kind: &'static str,
    server: Option<u64>,
    started_ms: i64,
    /// Last time it reported progress, for tasks that report any.
    beat_ms: AtomicI64,
    /// How often it expects to, if it said. `None` means it makes no promise and cannot stall.
    expect_ms: Option<u64>,
    state: TaskState,
    /// Why it stopped, when it stopped for a reason worth reading.
    cause: Option<String>,
}

/// A task's slot in the registry, handed to the task so it can report progress.
#[derive(Clone, Copy, Debug)]
pub struct TaskHandle(u64);

impl TaskHandle {
    /// This task's id, for finding it again in a snapshot.
    pub fn id(self) -> u64 {
        self.0
    }

    /// Note that the task is still doing its job.
    ///
    /// Cheap enough for a loop: one lock on a map with a dozen entries, taken as often as the task
    /// declared it would. Not for a hot path.
    pub fn beat(self, now_ms: i64) {
        with_registry(|tasks| {
            if let Some(task) = tasks.iter_mut().find(|t| t.id == self.0) {
                task.beat_ms.store(now_ms, Ordering::Relaxed);
            }
        });
    }
}

/// A task's health, as the console reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskHealth {
    pub id: u64,
    pub kind: &'static str,
    pub server: Option<u64>,
    pub started_ms: i64,
    pub last_beat_ms: Option<i64>,
    pub state: &'static str,
    /// Whether this is a state somebody should be told about.
    ///
    /// Decided here rather than by whoever renders it, so the rule that an ordinary shutdown is not
    /// a crash lives in one place and is testable. A console that re-derived it from the state name
    /// would be a second opinion waiting to disagree.
    pub fault: bool,
    pub cause: Option<String>,
}

static TASKS: OnceLock<Mutex<Vec<TaskRecord>>> = OnceLock::new();
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(0);

fn with_registry<T>(f: impl FnOnce(&mut Vec<TaskRecord>) -> T) -> T {
    let cell = TASKS.get_or_init(|| Mutex::new(Vec::new()));
    // A poisoned registry must not take supervision down with it. What is behind the lock is a list
    // of task states, all of it perfectly readable, and refusing to read it would mean losing the
    // record of whatever caused the panic.
    let mut tasks = cell.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut tasks)
}

/// How many finished tasks are kept.
///
/// Finished ones are the interesting ones, so they are kept rather than removed; this bounds a
/// long session that opens and closes many servers. Running tasks are never evicted.
const MAX_FINISHED: usize = 64;

/// Register a task that is starting.
pub fn register(
    kind: &'static str,
    server: Option<u64>,
    now_ms: i64,
    expect_ms: Option<u64>,
) -> TaskHandle {
    let id = NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed) + 1;
    with_registry(|tasks| {
        tasks.push(TaskRecord {
            id,
            kind,
            server,
            started_ms: now_ms,
            beat_ms: AtomicI64::new(now_ms),
            expect_ms,
            state: TaskState::Running,
            cause: None,
        });
        // Evict oldest finished first, so a fault is the last thing to be forgotten.
        while tasks.len() > MAX_FINISHED {
            let Some(at) = tasks.iter().position(|t| t.state != TaskState::Running) else {
                break;
            };
            tasks.remove(at);
        }
    });
    TaskHandle(id)
}

/// Record how a task ended.
pub fn finished(handle: TaskHandle, state: TaskState, cause: Option<String>) {
    with_registry(|tasks| {
        if let Some(task) = tasks.iter_mut().find(|t| t.id == handle.0) {
            task.state = state;
            task.cause = cause;
        }
    });
}

/// Every task this process has supervised, newest last.
///
/// `now_ms` is passed in rather than read, both because this crate has no business reading a clock
/// on its own and because a stall is a judgement about elapsed time that the caller's clock should
/// decide.
pub fn snapshot(now_ms: i64) -> Vec<TaskHealth> {
    with_registry(|tasks| {
        tasks
            .iter()
            .map(|task| {
                let beat = task.beat_ms.load(Ordering::Relaxed);
                // A task that promised to do something regularly and has not is the one case where
                // "running" is a worse answer than the truth. Three intervals, so an ordinary late
                // tick is not a fault.
                let stalled = task.state == TaskState::Running
                    && task
                        .expect_ms
                        .is_some_and(|expect| now_ms.saturating_sub(beat) > (expect as i64) * 3);
                let state = if stalled {
                    TaskState::Stalled
                } else {
                    task.state
                };
                TaskHealth {
                    id: task.id,
                    kind: task.kind,
                    server: task.server,
                    started_ms: task.started_ms,
                    last_beat_ms: task.expect_ms.map(|_| beat),
                    state: state.as_str(),
                    fault: state.is_fault(),
                    cause: task.cause.clone(),
                }
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is process-wide, so tests that read it must not run beside each other.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn a_task_that_panics_is_still_known_to_have_panicked_much_later() {
        // The whole point. A `tracing` line ages out of a bounded ring, and after it does the
        // task's state was not "dead" but "unknown": a healthy-looking app whose only evidence of
        // a stopped half had scrolled away.
        let _held = serial();
        let handle = register("event_forwarder", Some(7), 1_000, None);
        finished(
            handle,
            TaskState::Panicked,
            Some("index out of bounds".into()),
        );

        let found = snapshot(9_999_999)
            .into_iter()
            .find(|t| t.id == handle.0)
            .expect("a registered task is in the snapshot");
        assert_eq!(found.state, "panicked");
        assert_eq!(found.kind, "event_forwarder");
        assert_eq!(found.server, Some(7));
        assert_eq!(found.cause.as_deref(), Some("index out of bounds"));
        assert!(
            found.fault,
            "and it is a state somebody should be told about"
        );
    }

    /// An ordinary shutdown must not read as a crash, or the panel cries wolf and stops being read.
    #[test]
    fn a_task_that_simply_finished_is_not_a_fault() {
        let _held = serial();
        assert!(!TaskState::Exited.is_fault());
        assert!(!TaskState::Running.is_fault());
        assert!(TaskState::Panicked.is_fault());
        assert!(TaskState::Cancelled.is_fault());
        assert!(TaskState::Stalled.is_fault());

        // And the classification travels with the snapshot rather than being re-derived by whoever
        // renders it, so there is only one opinion about what counts as a fault.
        let handle = register("relay_fold", Some(3), 0, None);
        finished(handle, TaskState::Exited, None);
        let found = snapshot(1_000)
            .into_iter()
            .find(|t| t.id == handle.0)
            .unwrap();
        assert_eq!(found.state, "exited");
        assert!(!found.fault);
    }

    /// A stall is only ever claimed about a task that said how often it expects to do something.
    #[test]
    fn only_a_task_that_promised_a_rhythm_can_be_late_for_it() {
        let _held = serial();
        let quiet = register("event_forwarder", None, 0, None);
        let ticking = register("discovery_timer", Some(1), 0, Some(60_000));

        // An hour later. The forwarder may have had nothing to forward, which is not a fault; the
        // timer said it ticks every minute, and it has not.
        let later = snapshot(3_600_000);
        let quiet = later.iter().find(|t| t.id == quiet.0).unwrap();
        let ticking = later.iter().find(|t| t.id == ticking.0).unwrap();
        assert_eq!(quiet.state, "running", "silence is not evidence of a stall");
        assert_eq!(ticking.state, "stalled");
        assert_eq!(
            quiet.last_beat_ms, None,
            "a task with no rhythm reports no beat, rather than one a reader would judge it by"
        );

        // And a tick clears it, without a restart or anything else happening.
        TaskHandle(ticking.id).beat(3_600_000);
        let now = snapshot(3_600_001);
        assert_eq!(
            now.iter().find(|t| t.id == ticking.id).unwrap().state,
            "running"
        );
    }

    /// One late tick is not a stall: a timer that fires a second after its interval is a timer.
    #[test]
    fn an_ordinary_late_tick_is_not_a_fault() {
        let _held = serial();
        let handle = register("discovery_timer", Some(2), 0, Some(60_000));
        let soon = snapshot(120_000);
        assert_eq!(
            soon.iter().find(|t| t.id == handle.0).unwrap().state,
            "running"
        );
    }

    #[test]
    fn the_registry_stays_bounded_and_forgets_finished_tasks_first() {
        let _held = serial();
        let survivor = register("server_actor", Some(1), 0, None);
        for _ in 0..MAX_FINISHED * 2 {
            let handle = register("relay_fold", Some(2), 0, None);
            finished(handle, TaskState::Exited, None);
        }
        let all = snapshot(1_000);
        assert!(all.len() <= MAX_FINISHED + 1, "held {} records", all.len());
        assert!(
            all.iter().any(|t| t.id == survivor.0),
            "a running task is never evicted to make room"
        );
    }
}
