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

use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter, Layer};

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
pub const APP_FILE_FILTER: &str =
    "info,catcoms_app=debug,catcoms_sync=debug,catcoms_mls=debug,catcoms_discovery=debug";

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

    if !debug {
        let _ = tracing_subscriber::registry().with(console).try_init();
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
