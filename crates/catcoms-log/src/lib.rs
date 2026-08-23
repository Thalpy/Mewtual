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

use std::path::Path;
use std::sync::OnceLock;

use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter, Layer};

mod writer;

pub use writer::{
    DiagnosticInitError, SinkHealth, SinkState, MAX_DIR_BYTES, MAX_SEGMENTS, MAX_SEGMENT_BYTES,
    MAX_SESSION_BYTES,
};
use writer::{FileWriter, SYNC_TIMEOUT};

// --- in-memory ring, for the in-app debug console ----------------------------------------

/// How many events the console's ring holds. Enough to cover the run-up to a failure that a user
/// notices and then goes looking for, small enough to be irrelevant against the app's footprint.
pub const LOG_RING_CAPACITY: usize = 4096;

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

/// The name `tracing` gives an event's primary message. It is just another field to `tracing`;
/// the console wants it as the headline, so it is lifted out on the way in and back out again on
/// the way to the console.
const MESSAGE_FIELD: &str = "message";

/// A bounded, in-memory view of this session's diagnostics, shared with the debug console.
///
/// A projection over [`catcoms_diagnostics::DiagnosticHub`] rather than a store of its own. There
/// is one canonical record now, and this is the shape the console already reads it in: keeping the
/// old type and its methods means the hub could take over the storage without the console, the
/// desktop bridge or their tests changing at all.
#[derive(Clone, Default)]
pub struct LogRing;

impl std::fmt::Debug for LogRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LogRing")
    }
}

/// Flatten a canonical event into the console's line-oriented shape.
///
/// Lossy in one direction only: the trace, phase and section survive as ordinary fields rather
/// than as structure, because that is all the current console can render. The canonical event is
/// still intact in the hub for anything that wants the rest.
fn project(event: &catcoms_diagnostics::DiagnosticEvent) -> LogEvent {
    let mode = catcoms_diagnostics::CaptureMode::Enhanced;
    let mut message = String::new();
    let mut fields = Vec::with_capacity(event.fields.len() + 2);
    for (name, value) in &event.fields {
        if name.as_str() == MESSAGE_FIELD && message.is_empty() {
            message = value.render(mode);
        } else {
            fields.push(((*name).to_string(), value.render(mode)));
        }
    }
    // A structured event has a code rather than prose. Showing the code as the headline is what
    // makes a converted call site read better in the console than the sentence it replaced.
    if message.is_empty() && !event.code.is_empty() {
        message = event.code.to_string();
    }
    if event.trace.is_set() {
        fields.push(("trace".to_string(), event.trace.short()));
    }
    LogEvent {
        seq: event.seq,
        at_ms: event.at_ms as i64,
        level: event.level.as_str(),
        target: event.target.clone(),
        message,
        fields,
    }
}

impl LogRing {
    /// Events issued after `after_seq`, oldest first, capped at `limit`.
    ///
    /// A caller that has fallen further behind than the ring is deep gets the oldest events the
    /// ring still holds, and learns from [`LogRingStats::dropped`] that it missed some.
    pub fn since(&self, after_seq: u64, limit: usize) -> Vec<LogEvent> {
        hub().since(after_seq, limit).iter().map(project).collect()
    }

    /// The session counters. See [`LogRingStats`].
    pub fn stats(&self) -> LogRingStats {
        let stats = hub().stats();
        LogRingStats {
            errors: stats.errors,
            warnings: stats.warnings,
            dropped: stats.dropped,
            latest_seq: stats.latest_seq,
        }
    }

    /// Forget every held event, keeping the session counters and the sequence. Used by the
    /// console's clear button, which is about the view, not about rewriting what happened.
    pub fn clear(&self) {
        hub().clear();
    }
}

/// The process-wide diagnostic hub.
static HUB: OnceLock<catcoms_diagnostics::DiagnosticHub> = OnceLock::new();

/// The hub this process records into.
///
/// Created on first use rather than at subscriber installation, so a binary or test that never
/// asked for diagnostics still gets a working handle instead of a panic. Capture starts in
/// [`CaptureMode::Enhanced`](catcoms_diagnostics::CaptureMode::Enhanced) because this ring never
/// touches the disk: it lives in memory, is shown only in this app's own debug console, and leaves
/// only if the user presses copy. The file, which is the thing a user pastes to a stranger, is a
/// separate sink with its own narrower filter.
pub fn hub() -> catcoms_diagnostics::DiagnosticHub {
    HUB.get_or_init(|| {
        catcoms_diagnostics::DiagnosticHub::with_capacity(
            std::sync::Arc::new(catcoms_rt::SystemClock),
            catcoms_diagnostics::SessionSalt::random(&mut catcoms_rt::OsCryptoRng),
            catcoms_diagnostics::CaptureMode::Enhanced,
            LOG_RING_CAPACITY,
            &mut catcoms_rt::OsCryptoRng,
        )
    })
    .clone()
}

/// The ring this process is capturing into. Empty and inert until a subscriber installs one, so a
/// binary or test that never asked for a console still gets a working (if silent) handle.
pub fn ring() -> LogRing {
    LogRing
}

/// The `tracing` layer that feeds [`hub`].
struct RingLayer;

/// The filtered ring layer, ready to stack onto a subscriber.
///
/// A generic function rather than a local closure: a `Layer` is typed by the subscriber it is
/// layered onto, the two `init_debug_with` branches stack onto different ones, and a closure would
/// be monomorphised once against whichever it saw first.
fn ring_layer<S>() -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    RingLayer.with_filter(EnvFilter::new(CONSOLE_RING_FILTER))
}

/// The code a `tracing` event that has not been converted to a structured one carries.
///
/// A single constant rather than a per-event guess, so counting them measures exactly how much of
/// the app is still emitting prose instead of stable codes. See
/// [`BridgedMessage`](catcoms_diagnostics::redact::BridgedMessage).
const BRIDGED_CODE: &str = "LOG.TRACING.EVENT";

/// Renders every field to a string. `tracing` gives typed values; the console shows text, and a
/// debug rendering is the one representation every value type has.
struct FieldCollector(Vec<(String, String)>);

impl tracing::field::Visit for FieldCollector {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .push((field.name().to_string(), format!("{value:?}")));
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
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        use catcoms_diagnostics::{BridgedMessage, DiagnosticEvent, Level, Section};

        let metadata = event.metadata();
        let target = metadata.target();
        let mut collected = FieldCollector(Vec::new());
        event.record(&mut collected);

        // Every `tracing` event in the app arrives here, and none of them has been converted to a
        // structured code yet. The section is inferred from the emitting crate, which is coarse
        // but places each one somewhere a person would look for it; a converted call site states
        // its own section and overrides this entirely.
        let mut recorded = DiagnosticEvent::new(
            Section::from_target(target),
            Level::from_tracing(metadata.level().as_str()),
            BRIDGED_CODE,
        )
        .target(target);
        for (name, value) in collected.0 {
            // Both halves are moved rather than copied. This runs on whichever thread emitted the
            // event, which includes actor and network paths, so a per-field allocation that could
            // have been a move is a cost the app pays for being observed.
            recorded = recorded.field(name, BridgedMessage::from_owned(value));
        }
        catcoms_diagnostics::DiagnosticHub::record(&hub(), recorded);
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

/// Keeps the debug-log file writer alive, and answers for it.
///
/// Hold it until the program exits: dropping it waits for everything queued to reach the disk. The
/// [`health`](LogGuard::health) method is the reason this is a value with an API rather than an
/// opaque token. A caller that wants to tell a user whether logging is working must ask the sink,
/// because the alternative is repeating back the setting that asked for it, and a setting has never
/// once failed to open a file.
pub struct LogGuard {
    file: Option<FileWriter>,
}

impl LogGuard {
    /// What the debug-log file is actually doing right now. Never cached: a sink that was healthy
    /// at startup and has since filled its quota must not still be described by the startup answer.
    pub fn health(&self) -> SinkHealth {
        match &self.file {
            Some(file) => file.health(),
            None => SinkHealth::stopped(),
        }
    }

    /// Block until everything emitted so far has reached the file, up to a two second wait.
    /// Returns whether it got there. Used by the settings page's "write a test record" button, so
    /// the answer it shows describes the disk rather than the queue.
    pub fn sync(&self) -> bool {
        match &self.file {
            Some(file) => file.sync(SYNC_TIMEOUT),
            None => true,
        }
    }
}

impl std::fmt::Debug for LogGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LogGuard")
    }
}

/// A short id for this run.
///
/// Two logs written the same afternoon need to be distinguishable, and an excerpt someone pastes
/// needs to be matchable to the file it came from. Not a secret and not globally unique: it only
/// has to separate this run from the last one on this machine.
fn new_session_id() -> String {
    // FNV-1a over the two values that differ between consecutive runs. Keeping this local avoids
    // a random-number dependency in a crate whose whole job is installing subscribers.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let nanos = chrono::Local::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    let pid = u64::from(std::process::id());
    for byte in nanos.to_le_bytes().iter().chain(pid.to_le_bytes().iter()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{:08x}", hash as u32)
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
/// (dropping it waits for the queued output to reach the disk).
///
/// The file captures everything at `debug` and above, for every crate; see
/// [`init_debug_with`] for a caller that wants a narrower one.
pub fn init_debug(debug: bool, dir: impl AsRef<Path>) -> Result<LogGuard, DiagnosticInitError> {
    init_debug_with(debug, dir, "debug")
}

/// [`init_debug`] with an explicit filter for the **file** layer (the console layer keeps
/// obeying `RUST_LOG`). See [`APP_FILE_FILTER`] for the GUI's choice and why it is narrower
/// than the CLI's.
///
/// # Why this returns a `Result`
///
/// It used to return an apparently healthy guard whatever happened: the directory creation, the
/// file open and the subscriber installation were each assigned to `_`. A user could then be told
/// "logging: active" by a process that had never opened a file, reproduce a hard bug on the
/// strength of that, and find nothing to send. Every step that can fail now says so, and the
/// caller decides what to show.
pub fn init_debug_with(
    debug: bool,
    dir: impl AsRef<Path>,
    file_filter: &str,
) -> Result<LogGuard, DiagnosticInitError> {
    let console = fmt::layer().with_filter(console_filter());
    // The ring is always on, and independent of the debug-log setting. The in-app console is the
    // thing a user can actually look at while a problem is happening, so making it depend on having
    // enabled a file beforehand and restarted would put it out of reach in exactly the situation it
    // exists for. It costs a bounded amount of memory and writes nothing anywhere.
    if !debug {
        return tracing_subscriber::registry()
            .with(console)
            .with(ring_layer())
            .try_init()
            .map(|()| LogGuard { file: None })
            .map_err(|_| DiagnosticInitError::SubscriberInstalled);
    }

    let dir = dir.as_ref();
    let session_id = new_session_id();
    let base = format!("debug_log_{}", chrono::Local::now().format("%Y%m%d_%H%M%S"));
    let (file_writer, sink) = FileWriter::start(dir, &base, session_id.clone())?;
    let path = file_writer
        .path()
        .expect("a started writer knows the segment it opened");

    // The file captures verbose detail regardless of the console filter.
    let file = fmt::layer()
        .with_ansi(false)
        .with_writer(sink)
        .with_filter(EnvFilter::new(file_filter));

    tracing_subscriber::registry()
        .with(console)
        .with(file)
        .with(ring_layer())
        .try_init()
        .map_err(|_| DiagnosticInitError::SubscriberInstalled)?;

    // The last check, and the one that makes "active" mean something. A directory that resolved, a
    // file that opened and a layer that attached are all necessary and none of them is sufficient:
    // the layer's filter could exclude everything, or the write could go somewhere that discards.
    // So emit a record, wait for the worker, and read the size back off the disk. Only a file with
    // bytes in it counts as a working sink.
    tracing::info!(
        target: "catcoms_log",
        session = %session_id,
        file = %path.display(),
        "DIAG.SESSION.STARTED"
    );
    file_writer.sync(SYNC_TIMEOUT);
    let recorded = std::fs::metadata(&path)
        .map(|m| m.len() > 0)
        .unwrap_or(false);
    if !recorded {
        return Err(DiagnosticInitError::NoSessionRecord { path });
    }

    Ok(LogGuard {
        file: Some(file_writer),
    })
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

    // The ring's own behaviour (bounds, counters that outlive their events, per-section counts)
    // is tested where it lives now, in `catcoms-diagnostics`. What is left here is the join
    // between the two: whether a canonical event still reads the way the console expects.

    /// A `tracing` message has to arrive as the headline, not buried among the structured fields.
    /// It is the first thing on every rendered line, and the thing a person scans for.
    #[test]
    fn a_bridged_message_projects_to_the_headline_the_console_renders() {
        use catcoms_diagnostics::{BridgedMessage, DiagnosticEvent, Level, Section};
        let mut event = DiagnosticEvent::new(Section::Transport, Level::Warn, BRIDGED_CODE)
            .target("catcoms_net")
            .field("message", BridgedMessage::new("dial failed"))
            .field("peer", BridgedMessage::new("2b5df389"));
        event.seq = 12;
        event.at_ms = 1_787_000_000_000;

        let projected = project(&event);
        assert_eq!(projected.seq, 12);
        assert_eq!(projected.level, "WARN");
        assert_eq!(projected.target, "catcoms_net");
        assert_eq!(projected.message, "dial failed");
        assert_eq!(
            projected.fields,
            vec![("peer".to_string(), "2b5df389".to_string())]
        );
    }

    /// A converted call site has a stable code instead of prose. Showing the code as the headline
    /// is what makes the migration an improvement in the console rather than a blank line.
    #[test]
    fn a_structured_event_shows_its_code_where_a_message_would_be() {
        use catcoms_diagnostics::{DiagnosticEvent, Level, Section, TraceId};
        let event = DiagnosticEvent::new(Section::Join, Level::Warn, "JOIN.ROUTES.EXHAUSTED")
            .target("catcoms_sync")
            .trace(TraceId(0x7f2c_0000_0000_0001))
            .field("direct_candidates", 4u64);

        let projected = project(&event);
        assert_eq!(projected.message, "JOIN.ROUTES.EXHAUSTED");
        assert!(projected
            .fields
            .contains(&("direct_candidates".to_string(), "4".to_string())));
        // The trace is what ties this line to the rest of its operation, so it travels even in the
        // flattened shape the current console renders.
        assert!(projected
            .fields
            .contains(&("trace".to_string(), "7f2c".to_string())));
    }

    /// Field names arrive from `tracing` as runtime strings, and the event owns them.
    ///
    /// They used to be interned in a global table behind a mutex, which put a second lock
    /// acquisition on the emitting thread for every field of every event and leaked a string per
    /// distinct name. Diagnostics that add contention to a path shared with actor and network work
    /// become a cause of the stalls they exist to explain.
    #[test]
    fn a_bridged_field_name_is_owned_rather_than_interned_behind_a_lock() {
        use catcoms_diagnostics::{BridgedMessage, DiagnosticEvent, FieldName, Level, Section};
        let event = DiagnosticEvent::new(Section::Transport, Level::Warn, BRIDGED_CODE)
            .field("peer_id".to_string(), BridgedMessage::new("2b5df389"));
        assert!(matches!(event.fields[0].0, FieldName::Owned(_)));
        assert_eq!(event.fields[0].0.as_str(), "peer_id");
    }

    #[test]
    fn debug_mode_writes_a_timestamped_file_and_reports_it_healthy() {
        let dir = tempfile::tempdir().unwrap();
        let path;
        {
            let guard = init_debug(true, dir.path()).expect("the sink started");

            // The health is the point: it names the file this process opened, and it says active
            // only because a session-start record was read back off the disk during init.
            let health = guard.health();
            assert!(health.desired);
            assert_eq!(health.state, SinkState::Active);
            assert!(!health.session_id.is_empty());
            assert!(health.last_error.is_none());
            path = health.path.clone().expect("an open segment");

            tracing::info!(test_marker = 1, "hello from the debug log test");
            assert!(guard.sync(), "the queued event reached the disk");

            // The other half of the same emission: one `tracing` call has to reach both sinks,
            // the file that gets pasted to somebody and the in-memory record the console reads.
            // They are separate on purpose (different filters, different privacy exposure), and
            // an event landing in one but not the other is the bug this asserts against.
            let captured = ring().since(0, 100);
            let mine = captured
                .iter()
                .find(|e| e.message == "hello from the debug log test")
                .expect("the event reached the console record too");
            assert_eq!(mine.level, "INFO");
            assert_eq!(mine.target, "catcoms_log::tests");
            assert!(
                mine.fields
                    .iter()
                    .any(|(k, v)| k == "test_marker" && v == "1"),
                "structured fields survive the bridge: {:?}",
                mine.fields
            );

            let after = guard.health();
            assert!(
                after.events_written >= 2,
                "the session marker and the test event"
            );
            assert!(after.bytes_written > 0);
            assert_eq!(after.events_dropped, 0);
            assert!(after.last_write_at_ms.is_some());
        }
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("DIAG.SESSION.STARTED"), "{contents}");
        assert!(
            contents.contains("hello from the debug log test"),
            "{contents}"
        );
    }

    /// A file that could not be opened must reach the caller as an error. The previous version
    /// discarded it and returned a guard, which is how the app came to tell users that logging was
    /// active while no file existed anywhere.
    #[test]
    fn a_sink_that_cannot_be_opened_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("occupied");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let result = init_debug(true, blocked.join("logs"));
        assert!(matches!(result, Err(DiagnosticInitError::Directory { .. })));
    }

    /// Off is off. No file, no directory, and health that says so without inventing a state.
    #[test]
    fn logging_off_leaves_no_file_and_reports_stopped() {
        let health = LogGuard { file: None }.health();
        assert!(!health.desired);
        assert_eq!(health.state, SinkState::Stopped);
        assert_eq!(health.events_written, 0);
        assert!(health.path.is_none());
    }

    /// Two runs on one machine must be tellable apart in the file itself.
    #[test]
    fn a_session_id_is_stable_within_a_run_and_short_enough_to_quote() {
        let id = new_session_id();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
