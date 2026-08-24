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

use tracing_subscriber::filter::FilterExt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter, Layer};

mod writer;

pub use writer::{
    DiagnosticInitError, SinkHealth, SinkState, MAX_DIR_BYTES, MAX_EVENT_BYTES, MAX_SEGMENTS,
    MAX_SEGMENT_BYTES, MAX_SESSION_BYTES, TRUNCATION_MARKER,
};
use writer::{FileWriter, SYNC_TIMEOUT};

// --- in-memory ring, for the in-app debug console ----------------------------------------
//
// This crate installs subscribers and owns the process-wide hub. It deliberately no longer owns a
// *shape* for the console to read: it used to expose a `LogRing` whose events were flattened
// `tracing` lines with the section, phase, span, references and capture mode dropped, and the
// console read that instead of the canonical record. Most of what the app took the trouble to
// instrument was discarded one layer before the only tool anyone looks at. The console reads
// `catcoms_diagnostics::event_view` off the hub now, and the flattening is gone rather than
// deprecated, so nothing can quietly reach for it again. See the Part 3 review, finding P3-005.

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

/// The process-wide diagnostic hub.
static HUB: OnceLock<catcoms_diagnostics::DiagnosticHub> = OnceLock::new();

/// The hub this process records into.
///
/// Created on first use rather than at subscriber installation, so a binary or test that never
/// asked for diagnostics still gets a working handle instead of a panic.
///
/// # Why it starts Safe
///
/// It used to start [`Enhanced`](catcoms_diagnostics::CaptureMode::Enhanced), on the reasoning that
/// this store never touches the disk: it lives in memory and is only shown in the app's own
/// console. That reasoning was already wrong when it was written. The console has a Copy button and
/// a Save button, so its contents reach a clipboard and a file on the very first occasion anybody
/// has a reason to look at them, which is the same occasion they are about to send them to someone.
///
/// Safe is therefore the honest default: identifiers are per-session references and an address
/// keeps only its family and transport, so what a user copies without thinking about it is
/// publishable. That is not a reduced diagnosis. `ip6/quic-v1` is exactly what says "every address
/// this member advertises is IPv6 and this device has no IPv6 route", which is the failure that
/// stranded a node for an hour.
///
/// What Safe does cost is transport *debug*: dial attempts and connection churn are not captured at
/// all, so raising the mode afterwards will not bring back what was never recorded. That is a real
/// trade and it is made visible rather than hidden, in the console's capture panel and in the count
/// of events the settings excluded. Enhanced is one labelled click away and needs no restart, which
/// is the entire reason capture mode and section level are separate axes.
pub fn hub() -> catcoms_diagnostics::DiagnosticHub {
    HUB.get_or_init(|| {
        catcoms_diagnostics::DiagnosticHub::with_capacity(
            std::sync::Arc::new(catcoms_rt::SystemClock),
            catcoms_diagnostics::SessionSalt::random(&mut catcoms_rt::OsCryptoRng),
            catcoms_diagnostics::CaptureMode::Safe,
            LOG_RING_CAPACITY,
            &mut catcoms_rt::OsCryptoRng,
        )
    })
    .clone()
}

/// The `tracing` layer that feeds [`hub`].
struct RingLayer;

/// The filtered ring layer, ready to stack onto a subscriber.
///
/// A generic function rather than a local closure: a `Layer` is typed by the subscriber it is
/// layered onto, the two `init_debug_with` branches stack onto different ones, and a closure would
/// be monomorphised once against whichever it saw first.
///
/// # Two filters, and why the second one matters
///
/// [`CONSOLE_RING_FILTER`] is static: it decides, once, which crates may reach the ring at all,
/// and it is what keeps a dependency's own chatter out.
///
/// The hub's capture gate is the live one, and it has to be consulted *here* rather than inside
/// `record`. The bridge formats every field of every event into a `String` before the hub sees it,
/// so a capture setting that only stopped events at the store still paid the whole cost of building
/// them and then threw them away. Turning capture off is supposed to mean the app stops paying to
/// be watched, and until this it did not: it meant the app kept paying and stopped keeping the
/// results. See the first review, section 7, "Runtime toggling".
fn ring_layer<S>() -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    let live = tracing_subscriber::filter::FilterFn::new(|metadata: &tracing::Metadata<'_>| {
        hub().admits(
            catcoms_diagnostics::Section::from_target(metadata.target()),
            catcoms_diagnostics::Level::from_tracing(metadata.level().as_str()),
        )
    });
    RingLayer.with_filter(EnvFilter::new(CONSOLE_RING_FILTER).and(live))
}

/// The code a `tracing` event that has not been converted to a structured one carries.
///
/// A single constant rather than a per-event guess, so counting them measures exactly how much of
/// the app is still emitting prose instead of stable codes. See
/// [`BridgedMessage`](catcoms_diagnostics::redact::BridgedMessage).
const BRIDGED_CODE: &str = "LOG.TRACING.EVENT";

/// The field name a `tracing` call site uses to say which operation its work belongs to.
///
/// A convention rather than a type, because a library crate emits through the facade and holds no
/// diagnostic state. `tracing::debug!(trace = %trace.as_hex(), "ACTOR.COMMAND.RECEIVED")` in
/// `catcoms-app` and a canonical `DiagnosticEvent::trace` set natively are then the same thing to a
/// reader, which is what lets one `hub.trace(id)` query gather both.
const TRACE_FIELD: &str = "trace";

/// A trace as a `tracing` field renders it, or `None` if it is not one.
///
/// Sixteen hex characters, matching `TraceId::as_hex`. Anything else is left alone as an ordinary
/// field: a crate is free to have a field called `trace` that means something else, and silently
/// reinterpreting it would be worse than not lifting it.
fn parse_trace(value: &str) -> Option<catcoms_diagnostics::TraceId> {
    if value.len() != 16 {
        return None;
    }
    u64::from_str_radix(value, 16)
        .ok()
        .map(catcoms_diagnostics::TraceId)
        .filter(|t| t.is_set())
}

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
            // A library crate cannot construct a canonical event: it emits through the `tracing`
            // facade precisely so it does not depend on whichever binary is observing it. What it
            // *can* do is state which operation its work belongs to, and lifting that here is what
            // joins the actor's stages to the command that caused them. Without it the trace stops
            // at the bridge, and "did the actor ever handle my send" stays unanswerable.
            if name == TRACE_FIELD {
                if let Some(trace) = parse_trace(&value) {
                    recorded = recorded.trace(trace);
                    // Not also kept as a field. It is structure now, and a duplicate would render
                    // on every line and be one more thing that could disagree with itself.
                    continue;
                }
            }
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

    /// What a user copies without thinking about it has to be publishable.
    ///
    /// The console has a Copy button and a Save button, so this store reaches a clipboard and a
    /// file on the first occasion anybody looks at it, which is the same occasion they are about to
    /// send it to somebody. Starting Enhanced meant the default report carried literal addresses.
    #[test]
    fn capture_starts_in_the_mode_whose_output_can_be_shared() {
        use catcoms_diagnostics::CaptureMode;
        let _held = capture_lock();
        assert_eq!(hub().mode(), CaptureMode::Safe);
        assert!(!hub().mode().allows_raw_addresses());
    }

    /// The bridge's code is mirrored in `apps/desktop/src/debug-console.ts`, which renders a
    /// bridged event's prose as the headline where a migrated one shows its code. A rename here
    /// with no rename there would turn every un-migrated line in the console into
    /// `LOG.TRACING.EVENT message=...`, so the literal is pinned rather than left to a comment.
    #[test]
    fn the_bridged_code_is_the_literal_the_console_matches_on() {
        assert_eq!(BRIDGED_CODE, "LOG.TRACING.EVENT");
    }

    /// An un-migrated `tracing` event still has to land somewhere a person would look for it, and
    /// still has to be tellable apart from a converted one.
    ///
    /// The bridge is a compatibility layer, not a destination: every event that arrives through it
    /// carries [`BRIDGED_CODE`] and its prose in a `message` field rather than a stable code of its
    /// own. Counting them is how the migration's progress is measured, so the two must stay
    /// distinguishable.
    #[test]
    fn a_bridged_event_keeps_its_prose_and_says_that_is_what_it_is() {
        use catcoms_diagnostics::{
            event_view, BridgedMessage, CaptureMode, DiagnosticEvent, Level, Section,
        };
        let mut event = DiagnosticEvent::new(Section::Transport, Level::Warn, BRIDGED_CODE)
            .target("catcoms_net")
            .field("message", BridgedMessage::new("dial failed"))
            .field("peer", BridgedMessage::new("2b5df389"));
        event.seq = 12;
        event.at_ms = 1_787_000_000_000;

        let view = event_view(&event, CaptureMode::Safe);
        assert_eq!(view.seq, 12);
        assert_eq!(view.level, "WARN");
        assert_eq!(view.target, "catcoms_net");
        assert_eq!(view.code, BRIDGED_CODE, "still prose, and it admits it");
        assert_eq!(view.section, "transport");
        assert_eq!(view.view, "network", "a crate lands in a console section");
        let fields: Vec<(&str, &str)> = view
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.value.as_str()))
            .collect();
        assert_eq!(fields, [("message", "dial failed"), ("peer", "2b5df389")]);
    }

    /// A library crate says which operation its work belongs to, and the bridge makes that the
    /// event's real trace.
    ///
    /// This is what carries a trace across a boundary the canonical model cannot reach: the actor
    /// crate emits through the `tracing` facade on purpose, so its stages could otherwise never
    /// join the command that caused them. A `hub.trace(id)` query has to return both.
    #[test]
    fn a_tracing_call_site_can_say_which_operation_its_work_belongs_to() {
        assert_eq!(
            parse_trace("7f2c000000000001"),
            Some(catcoms_diagnostics::TraceId(0x7f2c_0000_0000_0001))
        );
        // Anything that is not a trace stays an ordinary field. A crate is entitled to a field
        // called `trace` that means something else, and reinterpreting it silently would be worse
        // than not lifting it at all.
        assert_eq!(parse_trace("7f2c"), None, "the short form is not the id");
        assert_eq!(parse_trace("the quick brown "), None);
        assert_eq!(parse_trace(""), None);
        assert_eq!(
            parse_trace("0000000000000000"),
            None,
            "an all-zero trace is what unset renders as"
        );
    }

    /// The hub is process-wide, so a test that changes what is being captured changes it for
    /// whichever other test happens to be running beside it.
    ///
    /// Held by every test below that either moves the capture config or depends on it. Found the
    /// obvious way: two of them did this concurrently and one failed on an assertion about the
    /// other's setting.
    static CAPTURE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn capture_lock() -> std::sync::MutexGuard<'static, ()> {
        CAPTURE.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Turning capture off has to stop the app *paying*, not just stop it keeping the results.
    ///
    /// The bridge renders every field into a `String` before the hub sees the event, so a setting
    /// that only rejected events at the store left the whole cost in place and discarded the
    /// output. This measures the thing itself rather than a proxy: the field's `Debug` counts its
    /// own invocations, so the assertion is literally "nothing was formatted".
    #[test]
    fn capture_that_is_turned_down_costs_nothing_to_format() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static RENDERED: AtomicUsize = AtomicUsize::new(0);

        struct Counted;
        impl std::fmt::Debug for Counted {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                RENDERED.fetch_add(1, Ordering::Relaxed);
                f.write_str("counted")
            }
        }

        let _held = capture_lock();
        let hub = hub();
        let restore = hub.mode();
        let subscriber = tracing_subscriber::registry().with(ring_layer());
        tracing::subscriber::with_default(subscriber, || {
            hub.set_mode(catcoms_diagnostics::CaptureMode::Safe);
            tracing::debug!(target: "catcoms_app", payload = ?Counted, "ON");
            assert_eq!(
                RENDERED.load(Ordering::Relaxed),
                1,
                "with capture on, the field is rendered exactly once"
            );

            hub.set_mode(catcoms_diagnostics::CaptureMode::Off);
            for _ in 0..100 {
                tracing::debug!(target: "catcoms_app", payload = ?Counted, "OFF");
            }
            assert_eq!(
                RENDERED.load(Ordering::Relaxed),
                1,
                "with capture off, not one field is rendered"
            );

            // And it comes back without a restart, which is the other half of the same promise.
            hub.set_mode(catcoms_diagnostics::CaptureMode::Safe);
            tracing::debug!(target: "catcoms_app", payload = ?Counted, "ON AGAIN");
            assert_eq!(RENDERED.load(Ordering::Relaxed), 2);

            // The same has to hold for one section turned down, which is the setting somebody
            // actually reaches for: not "record nothing" but "I do not need the transport
            // narrating every connection right now". If that only stopped events being kept, the
            // section a user turned down would still be the most expensive one in the process.
            hub.set_mode(catcoms_diagnostics::CaptureMode::Enhanced);
            tracing::debug!(target: "catcoms_net", payload = ?Counted, "DIAL");
            assert_eq!(RENDERED.load(Ordering::Relaxed), 3);

            hub.set_section_level(catcoms_diagnostics::Section::Transport, None);
            for _ in 0..50 {
                tracing::debug!(target: "catcoms_net", payload = ?Counted, "DIAL");
            }
            assert_eq!(RENDERED.load(Ordering::Relaxed), 3, "and it stops costing");

            // The others are untouched, so this is a section control and not a mode change.
            tracing::debug!(target: "catcoms_sync", payload = ?Counted, "POST");
            assert_eq!(RENDERED.load(Ordering::Relaxed), 4);
        });
        hub.set_mode(restore);
    }

    /// One query has to recover the whole operation, across the bridge.
    ///
    /// This is the property P3-004 is about. A command's stages are recorded natively as canonical
    /// events; the actor's are emitted through the `tracing` facade, because a library crate holds
    /// no diagnostic state. If those two do not land under the same trace, "which of the ten stages
    /// failed" is still unanswerable, and every stage added before this worked only made the record
    /// longer rather than more useful.
    #[test]
    fn one_trace_gathers_stages_from_both_sides_of_the_bridge() {
        use catcoms_diagnostics::{DiagnosticEvent, Level, Phase, Section, TraceId};

        let _held = capture_lock();
        let trace = TraceId(0x7f2c_0000_0000_00a1);
        // Scoped to this thread rather than installed globally. A `tracing` subscriber can be
        // installed once per process, and claiming it here would make whichever other test wanted
        // it fail depending on the order they happened to run in.
        let subscriber = tracing_subscriber::registry().with(ring_layer());
        tracing::subscriber::with_default(subscriber, || {
            // The bridge's side: what a library crate can say about its own work.
            tracing::debug!(
                target: "catcoms_app",
                trace = %trace.as_hex(),
                "ACTOR.COMMAND.RECEIVED"
            );
            // And an unrelated operation, so "gathers everything" cannot pass by gathering
            // everything.
            tracing::debug!(
                target: "catcoms_app",
                trace = %TraceId(0x64aa_0000_0000_0009).as_hex(),
                "ACTOR.COMMAND.RECEIVED"
            );
        });
        // The canonical side: what the binary records directly.
        catcoms_diagnostics::DiagnosticHub::record(
            &hub(),
            DiagnosticEvent::new(Section::Ipc, Level::Debug, "IPC.EVENT.EMITTED")
                .phase(Phase::Success)
                .trace(trace)
                .target("catcoms_app"),
        );

        let stages = hub().trace(trace);
        assert_eq!(
            stages.len(),
            2,
            "one operation, both its sides, and nothing else: {stages:?}"
        );
        assert!(
            stages
                .iter()
                .any(|e| e.code == BRIDGED_CODE && e.target == "catcoms_app"),
            "the actor's stage, which could only arrive through the facade"
        );
        assert!(
            stages.iter().any(|e| e.code == "IPC.EVENT.EMITTED"),
            "and the bridge's own, recorded canonically"
        );
        // The lifted field is structure now, not a field that renders on every line.
        let bridged = stages.iter().find(|e| e.code == BRIDGED_CODE).unwrap();
        assert!(
            !bridged
                .fields
                .iter()
                .any(|(name, _)| name.as_str() == "trace"),
            "the trace is the event's own, not a duplicate alongside it: {:?}",
            bridged.fields
        );
    }

    /// A converted call site is the other half of the same comparison: a stable code, a phase and a
    /// trace, none of which the bridge can produce.
    #[test]
    fn a_converted_call_site_carries_structure_the_bridge_cannot() {
        use catcoms_diagnostics::{
            event_view, CaptureMode, DiagnosticEvent, Level, Phase, Section, TraceId,
        };
        let event = DiagnosticEvent::new(Section::Join, Level::Warn, "JOIN.ROUTES.EXHAUSTED")
            .target("catcoms_sync")
            .phase(Phase::Failure)
            .trace(TraceId(0x7f2c_0000_0000_0001))
            .field("direct_candidates", 4u64);

        let view = event_view(&event, CaptureMode::Safe);
        assert_eq!(view.code, "JOIN.ROUTES.EXHAUSTED");
        assert_eq!(view.phase, "failure");
        assert_eq!(
            view.trace, "7f2c000000000001",
            "the whole trace reaches the console, not a four-character summary of it"
        );
        assert_eq!(view.fields[0].name, "direct_candidates");
        assert_eq!(view.fields[0].value, "4");
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
        // Reads the hub, so it must not run while another test has capture turned off.
        let _held = capture_lock();
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
            let captured = hub().since(0, 100);
            let mine = captured
                .iter()
                .map(|e| catcoms_diagnostics::event_view(e, catcoms_diagnostics::CaptureMode::Safe))
                .find(|e| {
                    e.fields
                        .iter()
                        .any(|f| f.name == "message" && f.value == "hello from the debug log test")
                })
                .expect("the event reached the console record too");
            assert_eq!(mine.level, "INFO");
            assert_eq!(mine.target, "catcoms_log::tests");
            assert!(
                mine.fields
                    .iter()
                    .any(|f| f.name == "test_marker" && f.value == "1"),
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
