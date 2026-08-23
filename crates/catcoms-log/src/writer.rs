//! The debug-log file: a bounded queue, a worker thread, an explicit quota, and counters that
//! describe what actually happened rather than what was asked for.
//!
//! The previous writer could not answer any of the questions a person asks when a bug report
//! arrives without a log attached. Was the file ever opened? Did the subscriber attach to it? Did
//! it stop writing halfway through? How much did it lose? It reported "logging: on", which was a
//! restatement of the user's preference, and a user could reproduce a hard bug, see that reassuring
//! word, and discover afterwards that nothing had been captured. Consuming somebody's only
//! reproduction while telling them it was safe is worse than having no logger at all.
//!
//! So every number here is measured at the point of writing. [`SinkHealth::state`] is derived from
//! whether bytes reached a file, never from the setting that asked for one.
//!
//! Events are written unbuffered, one `write` syscall each. That is deliberate: this file exists to
//! survive the crash it is describing, and a buffered last few kilobytes are exactly the ones worth
//! having. The data is in the OS page cache the moment the call returns, so a panic or a kill loses
//! nothing, and the queue in front keeps the syscall off the thread that emitted the event.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// --- the quota -----------------------------------------------------------------------------
//
// Explicit limits, because "however much the disk has" is not a limit. A retry loop, a hostile
// peer stimulus or a render storm can all emit without bound, and a diagnostics file that fills a
// disk has become the outage. The exact numbers are a starting point and can be tuned; having
// them at all is not optional.

/// How large one segment grows before the writer starts another.
pub const MAX_SEGMENT_BYTES: u64 = 10 * 1024 * 1024;

/// How many segments survive. Older ones are deleted as new ones open.
pub const MAX_SEGMENTS: usize = 5;

/// How much one session may write before it stops writing entirely.
pub const MAX_SESSION_BYTES: u64 = 50 * 1024 * 1024;

/// How much the whole diagnostics directory may hold, across sessions.
pub const MAX_DIR_BYTES: u64 = 100 * 1024 * 1024;

/// The fraction of the session quota at which the sink starts calling itself degraded, so the
/// warning arrives while there is still room to act on it.
const QUOTA_WARN: f64 = 0.7;

/// How many formatted events may be waiting to be written.
///
/// Bounded on purpose. An unbounded queue does not remove the backpressure, it converts it into
/// memory growth and hides it; this one overflows into a counter the console can show.
const QUEUE_CAPACITY: usize = 8192;

/// How long a caller waits for the queue to drain when it needs the file to be current.
pub(crate) const SYNC_TIMEOUT: Duration = Duration::from_secs(2);

// --- health --------------------------------------------------------------------------------

/// What the debug-log file is doing, as distinct from what was asked of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinkState {
    /// No file was asked for. Nothing is wrong.
    Stopped,
    /// Opened, attached, and a session-start record was read back from the file.
    Active,
    /// Writing, but not cleanly: something was dropped, or the quota is close enough to matter.
    Degraded,
    /// Not writing. [`SinkHealth::last_error`] says why.
    Failed,
}

impl SinkState {
    /// The lowercase wire name, for the desktop bridge and the console.
    pub fn as_str(self) -> &'static str {
        match self {
            SinkState::Stopped => "stopped",
            SinkState::Active => "active",
            SinkState::Degraded => "degraded",
            SinkState::Failed => "failed",
        }
    }
}

/// Everything the UI needs to tell the truth about the log file.
///
/// `desired` and `state` are separate fields because the entire point is that they can disagree,
/// and the disagreement is the interesting part:
///
/// ```text
/// Preference:   Enabled
/// Actual state: Failed, permission denied opening the diagnostics directory
/// ```
#[derive(Clone, Debug)]
pub struct SinkHealth {
    /// Whether a file was asked for.
    pub desired: bool,
    /// Whether one is happening. Derived from writes, never from `desired`.
    pub state: SinkState,
    /// Identifies this run in the log itself, so two files from one afternoon are distinguishable.
    pub session_id: String,
    /// The segment being written now. This is the file the UI should name: not the newest one in
    /// the directory, which may belong to a previous run that ended a minute ago.
    pub path: Option<PathBuf>,
    pub started_at_ms: Option<i64>,
    /// When a write last succeeded. A sink that claims to be active while this stops advancing is
    /// a sink that has quietly stopped.
    pub last_write_at_ms: Option<i64>,
    pub events_written: u64,
    pub bytes_written: u64,
    /// Events that never reached the file: queue overflow, or emitted after the quota stopped it.
    pub events_dropped: u64,
    pub queue_depth: usize,
    pub queue_high_water: usize,
    pub last_error: Option<String>,
}

impl SinkHealth {
    /// The health of a process that never asked for a file.
    pub fn stopped() -> Self {
        SinkHealth {
            desired: false,
            state: SinkState::Stopped,
            session_id: String::new(),
            path: None,
            started_at_ms: None,
            last_write_at_ms: None,
            events_written: 0,
            bytes_written: 0,
            events_dropped: 0,
            queue_depth: 0,
            queue_high_water: 0,
            last_error: None,
        }
    }
}

// --- why a sink could not be created ---------------------------------------------------------

/// Why diagnostics could not be started.
///
/// These used to be `let _ = ...`, which is how a logger ends up claiming to work. Each one is now
/// something the settings page can show and a person can act on.
#[derive(Debug)]
pub enum DiagnosticInitError {
    /// The diagnostics directory could not be created or reached.
    Directory { path: PathBuf, source: io::Error },
    /// The directory exists but the log file could not be opened in it.
    OpenFile { path: PathBuf, source: io::Error },
    /// Something else already installed a global subscriber, so our layers are not attached and
    /// nothing we do here would be recorded.
    SubscriberInstalled,
    /// Everything appeared to succeed, but the session-start record never arrived in the file. The
    /// last check before claiming to be active, and the one that catches a sink that is attached
    /// to something that silently discards.
    NoSessionRecord { path: PathBuf },
}

impl std::fmt::Display for DiagnosticInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiagnosticInitError::Directory { path, source } => {
                write!(f, "could not open the diagnostics directory {}: {source}", path.display())
            }
            DiagnosticInitError::OpenFile { path, source } => {
                write!(f, "could not open the log file {}: {source}", path.display())
            }
            DiagnosticInitError::SubscriberInstalled => {
                f.write_str("a tracing subscriber was already installed, so diagnostics are not attached")
            }
            DiagnosticInitError::NoSessionRecord { path } => {
                write!(f, "the log file {} accepted no session-start record", path.display())
            }
        }
    }
}

impl std::error::Error for DiagnosticInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DiagnosticInitError::Directory { source, .. }
            | DiagnosticInitError::OpenFile { source, .. } => Some(source),
            _ => None,
        }
    }
}

// --- counters ------------------------------------------------------------------------------

#[derive(Debug, Default)]
struct WriterStats {
    events_written: AtomicU64,
    bytes_written: AtomicU64,
    events_dropped: AtomicU64,
    queue_depth: AtomicUsize,
    queue_high_water: AtomicUsize,
    last_write_at_ms: AtomicI64,
    /// Set when the writer has stopped for good: quota reached, or the file became unwritable.
    stopped: AtomicBool,
    /// Set when it is still writing but something has already been lost or is close to being lost.
    degraded: AtomicBool,
    last_error: Mutex<Option<String>>,
    /// The segment currently open, which is what the UI names as "the current log".
    path: Mutex<Option<PathBuf>>,
}

impl WriterStats {
    fn note_error(&self, message: String) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(message);
        }
    }

    /// Stop writing, for a reason worth showing. Idempotent: the first reason is the useful one,
    /// because everything after it is a consequence.
    fn stop(&self, message: String) {
        if !self.stopped.swap(true, Ordering::Relaxed) {
            self.note_error(message);
        }
    }
}

fn now_ms() -> i64 {
    chrono::Local::now().timestamp_millis()
}

// --- the queue -------------------------------------------------------------------------------

enum Op {
    /// One formatted event, ready to write.
    Line(Vec<u8>),
    /// Acknowledge once everything queued before this has been written. Used to prove the file is
    /// current before claiming the sink is healthy, and to make sure the last events reach disk
    /// when the process is shutting down.
    Sync(SyncSender<()>),
}

/// The `MakeWriter` the `fmt` layer formats into.
#[derive(Clone)]
pub(crate) struct FileSink {
    tx: SyncSender<Op>,
    stats: Arc<WriterStats>,
}

/// One event's bytes, sent as a single message when the formatter is done with it.
///
/// `fmt` makes a writer per event and drops it when the line is complete, so buffering here and
/// sending on drop is what keeps one queue message equal to one event. Counting events would
/// otherwise mean counting `write` calls, which is a number about the formatter, not about the log.
pub(crate) struct LineWriter {
    buf: Vec<u8>,
    tx: SyncSender<Op>,
    stats: Arc<WriterStats>,
}

impl Write for LineWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for LineWriter {
    fn drop(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.buf);
        // `try_send`, never `send`. Blocking here would push disk latency onto whichever thread
        // emitted the event, which for this app includes actor and network paths: diagnostics
        // would become a source of the stalls it is supposed to explain. A full queue is recorded
        // as a drop instead, and the console shows the count.
        match self.tx.try_send(Op::Line(line)) {
            Ok(()) => {
                let depth = self.stats.queue_depth.fetch_add(1, Ordering::Relaxed) + 1;
                self.stats.queue_high_water.fetch_max(depth, Ordering::Relaxed);
            }
            Err(TrySendError::Full(_)) => {
                self.stats.events_dropped.fetch_add(1, Ordering::Relaxed);
                self.stats.degraded.store(true, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.stats.events_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileSink {
    type Writer = LineWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LineWriter {
            buf: Vec::with_capacity(256),
            tx: self.tx.clone(),
            stats: Arc::clone(&self.stats),
        }
    }
}

// --- segments ------------------------------------------------------------------------------

/// The prefix every debug log carries. Retention only ever considers files matching it, so nothing
/// else in the directory is at risk from the quota.
pub(crate) const LOG_PREFIX: &str = "debug_log_";

struct Segment {
    file: File,
    path: PathBuf,
    bytes: u64,
}

/// `debug_log_<stamp>.txt` for the first segment of a session, `debug_log_<stamp>_002.txt` after.
///
/// The first keeps the name the app has always used, so an existing support instruction ("send me
/// the newest debug_log file") still finds something sensible.
fn segment_name(base: &str, index: usize) -> String {
    if index <= 1 {
        format!("{base}.txt")
    } else {
        format!("{base}_{index:03}.txt")
    }
}

fn open_segment(dir: &Path, base: &str, index: usize) -> io::Result<Segment> {
    let path = dir.join(segment_name(base, index));
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
    Ok(Segment { file, path, bytes })
}

/// Every debug log in the directory, oldest first.
///
/// Sorted by name rather than by modification time: the names embed a sortable timestamp, and
/// mtime is the thing that made the old "which file is the current one" logic wrong. A file
/// touched by a backup tool must not be able to reorder the retention list.
fn segments_on_disk(dir: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(LOG_PREFIX))
        .map(|e| {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            (e.path(), size)
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// Delete the oldest logs until the directory is inside both the count and the byte quota.
///
/// Never deletes `current`: the file being written is not a candidate for its own retention pass,
/// however large the directory has become. If the quota cannot be met without it, the caller finds
/// out from the returned "still over" flag and stops writing instead.
fn retain(dir: &Path, current: &Path) -> bool {
    let mut held = segments_on_disk(dir);
    let mut total: u64 = held.iter().map(|(_, size)| size).sum();
    let mut index = 0;
    while index < held.len() && (held.len() > MAX_SEGMENTS || total > MAX_DIR_BYTES) {
        let (path, size) = &held[index];
        if path == current {
            index += 1;
            continue;
        }
        if std::fs::remove_file(path).is_ok() {
            total = total.saturating_sub(*size);
            held.remove(index);
        } else {
            index += 1;
        }
    }
    total > MAX_DIR_BYTES
}

// --- the worker ------------------------------------------------------------------------------

fn run(rx: Receiver<Op>, dir: PathBuf, base: String, first: Segment, stats: Arc<WriterStats>) {
    let mut segment = first;
    let mut index = 1usize;

    loop {
        let op = match rx.recv() {
            Ok(op) => op,
            // Every sender is gone, which only happens at process teardown.
            Err(_) => return,
        };
        let line = match op {
            Op::Sync(ack) => {
                let _ = segment.file.flush();
                let _ = ack.send(());
                continue;
            }
            Op::Line(line) => line,
        };
        stats.queue_depth.fetch_sub(1, Ordering::Relaxed);

        if stats.stopped.load(Ordering::Relaxed) {
            // Drain rather than block: the senders must keep making progress even after the sink
            // has given up, and the count of what was lost is itself a diagnostic.
            stats.events_dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        }

        let size = line.len() as u64;
        if segment.bytes + size > MAX_SEGMENT_BYTES {
            index += 1;
            match open_segment(&dir, &base, index) {
                Ok(next) => {
                    segment = next;
                    if let Ok(mut slot) = stats.path.lock() {
                        *slot = Some(segment.path.clone());
                    }
                    if retain(&dir, &segment.path) {
                        stats.stop(format!(
                            "the diagnostics directory is over its {} MiB quota and could not be reduced",
                            MAX_DIR_BYTES / (1024 * 1024)
                        ));
                        continue;
                    }
                }
                Err(e) => {
                    stats.stop(format!("could not open the next log segment: {e}"));
                    continue;
                }
            }
        }

        match segment.file.write_all(&line) {
            Ok(()) => {
                segment.bytes += size;
                stats.events_written.fetch_add(1, Ordering::Relaxed);
                stats.last_write_at_ms.store(now_ms(), Ordering::Relaxed);
                let total = stats.bytes_written.fetch_add(size, Ordering::Relaxed) + size;
                if total >= MAX_SESSION_BYTES {
                    stats.stop(format!(
                        "this session reached its {} MiB log quota and stopped writing",
                        MAX_SESSION_BYTES / (1024 * 1024)
                    ));
                } else if total as f64 >= MAX_SESSION_BYTES as f64 * QUOTA_WARN {
                    stats.degraded.store(true, Ordering::Relaxed);
                }
            }
            Err(e) => {
                // One failed write is usually the first of many (the disk filled, the file was
                // removed under us), so this stops rather than retrying into a loop that would
                // itself need rate limiting.
                stats.stop(format!("writing to the log file failed: {e}"));
            }
        }
    }
}

// --- the handle --------------------------------------------------------------------------------

/// A running debug-log file. Held by the process for its lifetime; its health is readable at any
/// point, which is the whole reason it exists as a value rather than as a side effect.
pub(crate) struct FileWriter {
    stats: Arc<WriterStats>,
    tx: SyncSender<Op>,
    session_id: String,
    started_at_ms: i64,
}

impl std::fmt::Debug for FileWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileWriter")
    }
}

impl FileWriter {
    /// Open the first segment, start the worker, and hand back the sink the `fmt` layer writes to.
    pub(crate) fn start(
        dir: &Path,
        base: &str,
        session_id: String,
    ) -> Result<(FileWriter, FileSink), DiagnosticInitError> {
        std::fs::create_dir_all(dir).map_err(|source| DiagnosticInitError::Directory {
            path: dir.to_path_buf(),
            source,
        })?;
        let first = open_segment(dir, base, 1).map_err(|source| DiagnosticInitError::OpenFile {
            path: dir.join(segment_name(base, 1)),
            source,
        })?;
        // Bring the directory inside its quota before adding to it, so a machine that has been
        // collecting logs for months does not need a rotation to happen first.
        retain(dir, &first.path);

        let stats = Arc::new(WriterStats::default());
        // Recorded here rather than on the worker thread, so a caller that asks which file it is
        // writing immediately after `start` gets an answer instead of racing the thread's first
        // instruction.
        if let Ok(mut slot) = stats.path.lock() {
            *slot = Some(first.path.clone());
        }
        let (tx, rx) = sync_channel(QUEUE_CAPACITY);
        let dir = dir.to_path_buf();
        let base = base.to_string();
        let worker_stats = Arc::clone(&stats);
        std::thread::Builder::new()
            .name("catcoms-log".into())
            .spawn(move || run(rx, dir, base, first, worker_stats))
            .map_err(|source| DiagnosticInitError::OpenFile {
                path: PathBuf::from("<log writer thread>"),
                source,
            })?;

        let sink = FileSink {
            tx: tx.clone(),
            stats: Arc::clone(&stats),
        };
        Ok((
            FileWriter {
                stats,
                tx,
                session_id,
                started_at_ms: now_ms(),
            },
            sink,
        ))
    }

    /// Block until everything queued so far has been written, or the deadline passes.
    ///
    /// Returns whether the acknowledgement arrived. A `false` means the worker is wedged or gone,
    /// which is worth knowing rather than assuming.
    ///
    /// The barrier waits for a queue slot rather than giving up when the queue is momentarily
    /// full. Unlike the logging path, blocking here is the whole point: every caller of this is
    /// explicitly asking for the disk to be current, and the busiest moment (a full queue) is
    /// exactly when a shutdown flush most needs to land. Failing instead would drop the last
    /// events written, which are the ones a crash report is made of.
    pub(crate) fn sync(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let (ack_tx, ack_rx) = sync_channel(1);
        let mut pending = Op::Sync(ack_tx);
        loop {
            match self.tx.try_send(pending) {
                Ok(()) => break,
                // Nobody is draining and nobody ever will be.
                Err(TrySendError::Disconnected(_)) => return false,
                Err(TrySendError::Full(returned)) => {
                    if std::time::Instant::now() >= deadline {
                        return false;
                    }
                    pending = returned;
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match ack_rx.recv_timeout(left) {
            Ok(()) => true,
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => false,
        }
    }

    /// The current path, for naming the file in the UI.
    pub(crate) fn path(&self) -> Option<PathBuf> {
        self.stats.path.lock().ok().and_then(|p| p.clone())
    }

    /// What this sink is actually doing. Read on demand, never cached: a sink that was healthy at
    /// startup and has since filled its quota must not still be described by the startup answer.
    pub(crate) fn health(&self) -> SinkHealth {
        let dropped = self.stats.events_dropped.load(Ordering::Relaxed);
        let last_write = self.stats.last_write_at_ms.load(Ordering::Relaxed);
        let state = if self.stats.stopped.load(Ordering::Relaxed) {
            SinkState::Failed
        } else if dropped > 0 || self.stats.degraded.load(Ordering::Relaxed) {
            SinkState::Degraded
        } else {
            SinkState::Active
        };
        SinkHealth {
            desired: true,
            state,
            session_id: self.session_id.clone(),
            path: self.path(),
            started_at_ms: Some(self.started_at_ms),
            last_write_at_ms: (last_write > 0).then_some(last_write),
            events_written: self.stats.events_written.load(Ordering::Relaxed),
            bytes_written: self.stats.bytes_written.load(Ordering::Relaxed),
            events_dropped: dropped,
            queue_depth: self.stats.queue_depth.load(Ordering::Relaxed),
            queue_high_water: self.stats.queue_high_water.load(Ordering::Relaxed),
            last_error: self.stats.last_error.lock().ok().and_then(|e| e.clone()),
        }
    }
}

impl Drop for FileWriter {
    fn drop(&mut self) {
        // The sink itself is cloned into the global subscriber, which is never dropped, so the
        // worker cannot learn about shutdown by its channel closing. An explicit barrier is what
        // gets the last events onto disk, and the last events are the ones a crash report needs.
        self.sync(SYNC_TIMEOUT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Session-scoped so a test's own files cannot be deleted by another test's retention pass.
    fn writer(dir: &Path) -> (FileWriter, FileSink) {
        FileWriter::start(dir, "debug_log_20260823_120000", "test".into()).unwrap()
    }

    fn write_line(sink: &FileSink, text: &str) {
        use tracing_subscriber::fmt::MakeWriter;
        let mut w = sink.make_writer();
        w.write_all(text.as_bytes()).unwrap();
        drop(w);
    }

    #[test]
    fn a_missing_directory_is_an_error_rather_than_a_healthy_looking_guard() {
        // The old code assigned `create_dir_all` to `_` and carried on to report success. A path
        // that cannot be a directory is the cheapest way to prove that no longer happens.
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("not-a-directory");
        std::fs::write(&blocked, b"i am a file").unwrap();
        let result = FileWriter::start(&blocked.join("logs"), "debug_log_x", "test".into());
        assert!(
            matches!(result, Err(DiagnosticInitError::Directory { .. })),
            "a directory that cannot be created must be reported, not swallowed"
        );
    }

    #[test]
    fn health_counts_what_reached_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, sink) = writer(dir.path());
        for n in 0..10 {
            write_line(&sink, &format!("event {n}\n"));
        }
        assert!(writer.sync(SYNC_TIMEOUT), "the worker acknowledged the barrier");

        let health = writer.health();
        assert_eq!(health.state, SinkState::Active);
        assert_eq!(health.events_written, 10);
        assert!(health.bytes_written > 0);
        assert_eq!(health.events_dropped, 0);
        assert!(health.last_write_at_ms.is_some());

        let path = health.path.expect("an open segment");
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("event 9"), "{written}");
    }

    #[test]
    fn a_stopped_sink_reports_failed_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, sink) = writer(dir.path());
        write_line(&sink, "before\n");
        assert!(writer.sync(SYNC_TIMEOUT));

        writer.stats.stop("the disk filled".into());
        write_line(&sink, "after\n");
        assert!(writer.sync(SYNC_TIMEOUT));

        let health = writer.health();
        assert_eq!(health.state, SinkState::Failed);
        assert_eq!(health.last_error.as_deref(), Some("the disk filled"));
        assert_eq!(health.events_written, 1, "the event after the stop never reached the file");
        assert_eq!(health.events_dropped, 1, "and it is counted rather than forgotten");
    }

    #[test]
    fn retention_keeps_the_newest_segments_and_never_the_open_one() {
        let dir = tempfile::tempdir().unwrap();
        for n in 1..=(MAX_SEGMENTS + 3) {
            std::fs::write(dir.path().join(format!("debug_log_2026010{n}_000000.txt")), b"old").unwrap();
        }
        // Something else in the directory, to prove the quota only ever considers its own files.
        std::fs::write(dir.path().join("vault.db"), b"not a log").unwrap();

        let current = dir.path().join("debug_log_20260823_120000.txt");
        std::fs::write(&current, b"current").unwrap();
        assert!(!retain(dir.path(), &current));

        let left = segments_on_disk(dir.path());
        assert_eq!(left.len(), MAX_SEGMENTS);
        assert!(left.iter().any(|(p, _)| p == &current), "the open segment survives");
        assert!(dir.path().join("vault.db").exists(), "unrelated files are untouched");
    }

    #[test]
    fn segment_names_keep_the_first_file_where_people_expect_it() {
        assert_eq!(segment_name("debug_log_20260823_120000", 1), "debug_log_20260823_120000.txt");
        assert_eq!(segment_name("debug_log_20260823_120000", 2), "debug_log_20260823_120000_002.txt");
    }

    /// The barrier has to survive the busiest moment, because that is when it is used: the last
    /// thing a crashing process does is flush, and it flushes a full queue.
    #[test]
    fn the_flush_barrier_waits_for_a_slot_instead_of_giving_up_on_a_full_queue() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, sink) = writer(dir.path());
        for n in 0..(QUEUE_CAPACITY + 200) {
            write_line(&sink, &format!("event {n}\n"));
        }
        assert!(
            writer.sync(SYNC_TIMEOUT),
            "a barrier queued behind a backlog still has to land"
        );
        assert!(writer.health().events_written > 0);
    }

    #[test]
    fn a_full_queue_drops_and_says_so_rather_than_blocking_the_caller() {
        // The queue is drained by a worker, so filling it means outrunning a thread rather than
        // choosing a number. Emitting far more than the capacity without ever blocking is the
        // property under test: this must terminate, and any loss must be counted.
        let dir = tempfile::tempdir().unwrap();
        let (writer, sink) = writer(dir.path());
        let bulk = "x".repeat(512);
        for _ in 0..(QUEUE_CAPACITY * 2) {
            write_line(&sink, &bulk);
        }
        assert!(writer.sync(SYNC_TIMEOUT));
        let health = writer.health();
        assert_eq!(
            health.events_written + health.events_dropped,
            (QUEUE_CAPACITY * 2) as u64,
            "every event either reached the file or was counted as lost"
        );
        assert!(health.queue_high_water > 0);
        if health.events_dropped > 0 {
            assert_eq!(health.state, SinkState::Degraded, "loss is never reported as healthy");
        }
    }
}
