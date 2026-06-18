//! Diagnostics setup for CatComs binaries and tests.
//!
//! Library crates emit structured diagnostics with the `tracing` facade (no
//! global state). A binary (the dev CLI, the Tauri app) or a test installs a
//! subscriber once via [`init`] / [`init_test`]. Filtering is controlled by the
//! `RUST_LOG` environment variable (e.g. `RUST_LOG=catcoms_net=debug,info`).

use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

/// Default filter when `RUST_LOG` is unset: info everywhere, debug for our crates.
const DEFAULT_FILTER: &str = "info,catcoms_net=debug,catcoms_sync=debug";

fn filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER))
}

/// Install a human-readable tracing subscriber for a binary. Idempotent: a second
/// call is a no-op, so it is safe to call from multiple entry points.
pub fn init() {
    let _ = tracing_subscriber::registry()
        .with(filter())
        .with(fmt::layer())
        .try_init();
}

/// Install a subscriber suitable for tests (writes through the test harness so
/// output is captured per-test). Idempotent; honors `RUST_LOG`, otherwise traces
/// our crates verbosely.
pub fn init_test() {
    let env = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("catcoms_sync=trace,catcoms_net=trace"));
    let _ = tracing_subscriber::registry()
        .with(env)
        .with(fmt::layer().with_test_writer())
        .try_init();
}
