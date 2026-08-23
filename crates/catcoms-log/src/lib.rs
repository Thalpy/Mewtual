//! Diagnostics setup for Mewtual binaries and tests.
//!
//! Library crates emit structured diagnostics with the `tracing` facade (no
//! global state). A binary (the dev CLI, the Tauri app) or a test installs a
//! subscriber once. Console filtering is controlled by `RUST_LOG`
//! (e.g. `RUST_LOG=catcoms_net=debug,info`).
//!
//! [`init_debug`] adds an optional, toggleable **debug log file**: when enabled it
//! writes a verbose log to `<dir>/debug_log_<YYYYmmdd_HHMMSS>.txt` in addition to
//! the console. Hold the returned [`LogGuard`] for the lifetime of the program so
//! the file is flushed on exit.
//!
//! # What a debug log may contain
//!
//! This matters because the file exists to be *shared* with someone diagnosing a problem, and
//! this is a privacy tool. A debug log **can** contain:
//!
//! * network addresses: this device's own LAN and public IP addresses and ports, the addresses
//!   of peers it dialled, and relay/rendezvous infrastructure addresses;
//! * stable identifiers: libp2p peer ids, device-id fingerprints, group ids, invite nonce
//!   prefixes, content addresses (CIDs) of files and avatars;
//! * activity metadata: when this device connected, to whom, how many messages/ops/blobs moved
//!   and when, which documents were opened, and which channels exist by internal id.
//!
//! It does **not** contain message text, file contents, wiki bodies, display names, passphrases
//! or any key material: the payloads are sealed before they reach any code that logs, and no
//! subscriber here reaches inside them. At [`APP_FILE_FILTER`] the transport crates are held at
//! `info`, which is what keeps per-connection address churn out of an ordinary log.
//!
//! Treat a debug log as "who I talked to and when", and share it accordingly.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter, Layer};

// --- in-memory ring, for the in-app debug console ----------------------------------------

/// How many events the console's ring holds. Enough to cover the run-up to a failure that a user
/// notices and then goes looking for, small enough to be irrelevant against the app's footprint.
pub const LOG_RING_CAPACITY: usize = 4096;

/// How much of one rendered event survives. A single event should not be able to push the rest of
/// the ring out on its own.
const MAX_EVENT_CHARS: usize = 4000;

/// The filter the **ring** captures at.
///
/// Deliberately wider than [`APP_FILE_FILTER`], and that is the point of having two. The file
/// exists to be pasted to someone else, so it holds the transport crates at `info` to keep
/// per-connection address churn out of something a user is about to share. The ring never touches
/// the disk: it lives in memory, is shown only in this app's own debug console, and leaves only if
/// the user presses copy. So it can afford `catcoms_net` at `debug`, which is where dial attempts,
/// dial failures and connection churn are explained, and those turned out to be exactly the events
/// missing when a node sat unable to connect and nothing anywhere said why.
pub const CONSOLE_RING_FILTER: &str = "info,catcoms_app=debug,catcoms_sync=debug,\
     catcoms_mls=debug,catcoms_discovery=debug,catcoms_ui=debug,catcoms_net=debug,\
     catcoms_storage=debug,catcoms_replication=debug";

/// One captured diagnostic event, as the debug console renders it.
///
/// Deliberately plain data with no `serde` derive: this crate installs subscribers and should not
/// acquire a serialization dependency to do it. The desktop bridge maps this into its own IPC type.
#[derive(Clone, Debug)]
pub struct LogEvent {
    /// Monotonic within the session. The console polls with the last id it has seen, so this is
    /// what makes "give me what is new" cheap and gap-free.
    pub seq: u64,
    /// Wall-clock milliseconds, for display next to the user's own sense of when it happened.
    pub at_ms: i64,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG` or `TRACE`.
    pub level: &'static str,
    /// The emitting module path, e.g. `catcoms_net`. `catcoms_ui` is the webview's own.
    pub target: String,
    /// The event's message field.
    pub message: String,
    /// Its remaining structured fields, in declaration order.
    pub fields: Vec<(String, String)>,
}

/// What the console needs besides the events themselves.
#[derive(Clone, Copy, Debug, Default)]
pub struct LogRingStats {
    /// Errors seen this session, counted **before** the ring evicts anything. A roll-up that said
    /// "no errors" because the error had aged out would be worse than no roll-up.
    pub errors: u64,
    /// Warnings seen this session, on the same basis.
    pub warnings: u64,
    /// Events the ring dropped to stay inside its capacity. Surfaced so the console can say it
    /// lost some rather than presenting a gap as a quiet period.
    pub dropped: u64,
    /// The highest sequence number issued so far.
    pub latest_seq: u64,
}

#[derive(Default)]
struct RingInner {
    events: VecDeque<LogEvent>,
    next_seq: u64,
    stats: LogRingStats,
}

/// A bounded, in-memory view of this session's diagnostics, shared with the debug console.
#[derive(Clone, Default)]
pub struct LogRing(Arc<Mutex<RingInner>>);

impl std::fmt::Debug for LogRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LogRing")
    }
}

impl LogRing {
    /// Events issued after `after_seq`, oldest first, capped at `limit`.
    ///
    /// A caller that has fallen further behind than the ring is deep gets the oldest events the
    /// ring still holds, and learns from [`LogRingStats::dropped`] that it missed some.
    pub fn since(&self, after_seq: u64, limit: usize) -> Vec<LogEvent> {
        let inner = self.lock();
        inner
            .events
            .iter()
            .filter(|e| e.seq > after_seq)
            .take(limit)
            .cloned()
            .collect()
    }

    /// The session counters. See [`LogRingStats`].
    pub fn stats(&self) -> LogRingStats {
        self.lock().stats
    }

    /// Forget every held event, keeping the session counters and the sequence. Used by the
    /// console's clear button, which is about the view, not about rewriting what happened.
    pub fn clear(&self) {
        self.lock().events.clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RingInner> {
        // A panic in some other thread's short critical section must not take diagnostics down
        // with it: the data behind a poisoned lock here is still perfectly readable.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push(&self, level: &'static str, target: String, mut fields: Vec<(String, String)>) {
        let mut inner = self.lock();
        inner.next_seq += 1;
        let seq = inner.next_seq;
        inner.stats.latest_seq = seq;
        match level {
            "ERROR" => inner.stats.errors += 1,
            "WARN" => inner.stats.warnings += 1,
            _ => {}
        }
        // `message` is just another field to `tracing`; the console wants it as the headline.
        let message = fields
            .iter()
            .position(|(k, _)| k == "message")
            .map(|i| fields.remove(i).1)
            .unwrap_or_default();
        inner.events.push_back(LogEvent {
            seq,
            at_ms: chrono::Local::now().timestamp_millis(),
            level,
            target,
            message: truncate(message),
            fields,
        });
        while inner.events.len() > LOG_RING_CAPACITY {
            inner.events.pop_front();
            inner.stats.dropped += 1;
        }
    }
}

fn truncate(mut s: String) -> String {
    if s.chars().count() > MAX_EVENT_CHARS {
        s = s.chars().take(MAX_EVENT_CHARS).collect::<String>() + " [truncated]";
    }
    s
}

/// The process-wide ring, so the desktop bridge can reach the one the subscriber writes into.
static RING: OnceLock<LogRing> = OnceLock::new();

/// The ring this process is capturing into. Empty and inert until a subscriber installs one, so a
/// binary or test that never asked for a console still gets a working (if silent) handle.
pub fn ring() -> LogRing {
    RING.get_or_init(LogRing::default).clone()
}

/// The `tracing` layer that fills [`ring`].
struct RingLayer {
    ring: LogRing,
}

/// The filtered ring layer, ready to stack onto a subscriber.
///
/// A generic function rather than a local closure: a `Layer` is typed by the subscriber it is
/// layered onto, the two `init_debug_with` branches stack onto different ones, and a closure would
/// be monomorphised once against whichever it saw first.
fn ring_layer<S>() -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    RingLayer { ring: ring() }.with_filter(EnvFilter::new(CONSOLE_RING_FILTER))
}

/// Renders every field to a string. `tracing` gives typed values; the console shows text, and a
/// debug rendering is the one representation every value type has.
struct FieldCollector(Vec<(String, String)>);

impl tracing::field::Visit for FieldCollector {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_string(), format!("{value:?}")));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        // Strings would otherwise arrive quoted and escaped by the `Debug` path above, which is
        // noise in a message field that is already a sentence.
        self.0.push((field.name().to_string(), value.to_string()));
    }
    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
}

impl<S: tracing::Subscriber> Layer<S> for RingLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut fields = FieldCollector(Vec::new());
        event.record(&mut fields);
        self.ring.push(
            event.metadata().level().as_str(),
            event.metadata().target().to_string(),
            fields
                .0
                .into_iter()
                .map(|(k, v)| (k, truncate(v)))
                .collect(),
        );
    }
}

/// Default console filter when `RUST_LOG` is unset.
const DEFAULT_FILTER: &str = "info,catcomsctl=debug,catcoms_net=debug,catcoms_sync=debug";

fn console_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER))
}

/// Install a human-readable console subscriber for a binary. Idempotent.
pub fn init() {
    let _ = tracing_subscriber::registry()
        .with(fmt::layer().with_filter(console_filter()))
        .try_init();
}

/// Install a subscriber suitable for tests (captured per-test). Idempotent.
pub fn init_test() {
    let env = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("catcoms_sync=trace,catcoms_net=trace"));
    let _ = tracing_subscriber::registry()
        .with(fmt::layer().with_test_writer().with_filter(env))
        .try_init();
}

/// Keeps the debug-log file writer alive. Hold it until the program exits; on
/// drop the buffered file output is flushed.
pub struct LogGuard {
    _file: Option<tracing_appender::non_blocking::WorkerGuard>,
}

impl std::fmt::Debug for LogGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LogGuard")
    }
}

/// The debug-file filter for a long-running **GUI** session.
///
/// Deliberately not a blanket `debug`. The CLI's log is read by whoever ran the command, seconds
/// later; the app's is read by whoever the user pastes it to, so the conservative default is the
/// right one. Our own product and protocol layers log at `debug` (which is where a join,
/// admission or catch-up failure is explained); the transport crates stay at `info`, because
/// `catcoms_net`/`libp2p` at `debug` narrate every connection, stream and address the node ever
/// sees, which is both the bulk of the volume and the most identifying part of it. Someone
/// chasing a transport bug can still set `RUST_LOG`.
/// `catcoms_ui` is the webview's own target (the desktop bridge's `log_ui` command). It sits at
/// `debug` with our other product layers, because half the app runs in the webview and a log that
/// cannot see its errors is a log of half the app.
pub const APP_FILE_FILTER: &str = "info,catcoms_app=debug,catcoms_sync=debug,catcoms_mls=debug,\
     catcoms_discovery=debug,catcoms_ui=debug";

/// Install console logging and, when `debug` is true, also a verbose debug log at
/// `<dir>/debug_log_<timestamp>.txt`. Returns a guard that must be kept alive
/// (dropping it flushes the file).
///
/// The file captures everything at `debug` and above, for every crate; see
/// [`init_debug_with`] for a caller that wants a narrower one.
pub fn init_debug(debug: bool, dir: impl AsRef<Path>) -> LogGuard {
    init_debug_with(debug, dir, "debug")
}

/// [`init_debug`] with an explicit filter for the **file** layer (the console layer keeps
/// obeying `RUST_LOG`). See [`APP_FILE_FILTER`] for the GUI's choice and why it is narrower
/// than the CLI's.
pub fn init_debug_with(debug: bool, dir: impl AsRef<Path>, file_filter: &str) -> LogGuard {
    let console = fmt::layer().with_filter(console_filter());
    // Always on, and independent of the debug-log setting. The in-app console is the thing a user
    // can actually look at while a problem is happening, so making it depend on having enabled a
    // file beforehand and restarted would put it out of reach in exactly the situation it exists
    // for. It costs a bounded amount of memory and writes nothing anywhere.
    //
    if !debug {
        let _ = tracing_subscriber::registry()
            .with(console)
            .with(ring_layer())
            .try_init();
        return LogGuard { _file: None };
    }

    let dir = dir.as_ref();
    let _ = std::fs::create_dir_all(dir);
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let name = format!("debug_log_{stamp}.txt");
    let appender = tracing_appender::rolling::never(dir, &name);
    let (non_blocking, guard) = tracing_appender::non_blocking(appender);

    // The file captures verbose detail regardless of the console filter.
    let file = fmt::layer()
        .with_ansi(false)
        .with_writer(non_blocking)
        .with_filter(EnvFilter::new(file_filter));

    let _ = tracing_subscriber::registry()
        .with(console)
        .with(file)
        .with(ring_layer())
        .try_init();

    eprintln!("[catcoms] debug log -> {}", dir.join(&name).display());
    LogGuard { _file: Some(guard) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app filter is a privacy decision, so it is pinned rather than left to drift: the
    /// crates that narrate addresses and connections must not be at `debug` in a file a user is
    /// about to paste into a chat.
    #[test]
    fn the_app_file_filter_keeps_the_transport_crates_quiet() {
        let f = EnvFilter::new(APP_FILE_FILTER);
        let dump = f.to_string();
        assert!(dump.contains("catcoms_sync=debug"), "{dump}");
        assert!(dump.contains("catcoms_app=debug"), "{dump}");
        assert!(
            !dump.contains("catcoms_net=debug") && !dump.contains("libp2p=debug"),
            "the transport crates must stay at the default level: {dump}"
        );
    }

    /// The ring exists to show what the shared file deliberately leaves out. If the two filters
    /// ever converge, the console stops being able to answer the question it was built for
    /// ("why can this node not connect?") and the file starts carrying address churn a user is
    /// about to paste somewhere. Both halves are pinned.
    #[test]
    fn the_console_ring_sees_the_transport_crate_the_shared_file_does_not() {
        let ring = EnvFilter::new(CONSOLE_RING_FILTER).to_string();
        let file = EnvFilter::new(APP_FILE_FILTER).to_string();
        assert!(ring.contains("catcoms_net=debug"), "{ring}");
        assert!(!file.contains("catcoms_net=debug"), "{file}");
    }

    #[test]
    fn the_ring_keeps_its_counters_when_events_age_out() {
        // A roll-up that reported "no errors" because the error had been evicted would be worse
        // than having no roll-up at all, so the counters are taken before the bound is applied.
        let ring = LogRing::default();
        ring.push("ERROR", "catcoms_net".into(), vec![
            ("message".into(), "the first failure".into()),
        ]);
        for n in 0..LOG_RING_CAPACITY {
            ring.push(
                "INFO",
                "catcoms_sync".into(),
                vec![("message".into(), format!("filler {n}"))],
            );
        }
        let stats = ring.stats();
        assert_eq!(stats.errors, 1, "the error is still counted");
        assert!(stats.dropped >= 1, "and the ring admits it dropped events");
        assert!(
            !ring
                .since(0, LOG_RING_CAPACITY)
                .iter()
                .any(|e| e.message == "the first failure"),
            "even though the event itself has aged out"
        );
    }

    #[test]
    fn the_ring_serves_only_what_a_caller_has_not_seen() {
        let ring = LogRing::default();
        for n in 0..5 {
            ring.push(
                "INFO",
                "catcoms_app".into(),
                vec![("message".into(), format!("event {n}"))],
            );
        }
        let first = ring.since(0, 10);
        assert_eq!(first.len(), 5);
        assert_eq!(first[0].message, "event 0");
        let newest = first.last().unwrap().seq;
        assert!(
            ring.since(newest, 10).is_empty(),
            "a caller that is up to date is told nothing twice"
        );
        ring.push(
            "WARN",
            "catcoms_ui".into(),
            vec![("message".into(), "later".into())],
        );
        let next = ring.since(newest, 10);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].message, "later");
    }

    #[test]
    fn debug_mode_writes_a_timestamped_file() {
        let dir = tempfile::tempdir().unwrap();
        {
            let _guard = init_debug(true, dir.path());
            tracing::info!(test_marker = 1, "hello from the debug log test");
            // guard drops here -> flush
        }
        let entry = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().starts_with("debug_log_"))
            .expect("a debug_log_*.txt file was created");
        let contents = std::fs::read_to_string(entry.path()).unwrap();
        assert!(contents.contains("hello from the debug log test"));
    }
}
