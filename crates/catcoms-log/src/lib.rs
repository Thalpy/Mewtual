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

/// Install console logging and, when `debug` is true, also a verbose debug log at
/// `<dir>/debug_log_<timestamp>.txt`. Returns a guard that must be kept alive
/// (dropping it flushes the file).
pub fn init_debug(debug: bool, dir: impl AsRef<Path>) -> LogGuard {
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
        .with_filter(EnvFilter::new("debug"));

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
