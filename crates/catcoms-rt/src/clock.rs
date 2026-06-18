//! The [`Clock`] seam: logical time, injectable everywhere.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A source of wall-clock time in milliseconds since the Unix epoch.
///
/// Inject a `Clock` wherever time is needed (expiry horizons, MLS lifetimes,
/// retry backoff, CRDT timestamps) instead of reading the OS clock, so behaviour
/// is fully deterministic under test.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
}

impl<T: Clock + ?Sized> Clock for Arc<T> {
    fn now_ms(&self) -> u64 {
        (**self).now_ms()
    }
}

/// The real OS clock. This is the **only** type permitted to read the system
/// time anywhere in the codebase (enforced by the CI ambient-dependency gate).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// A deterministic, manually-advanced clock for tests. Cloneable; all clones
/// share the same underlying time.
#[derive(Debug, Default, Clone)]
pub struct ManualClock {
    now_ms: Arc<AtomicU64>,
}

impl ManualClock {
    /// A manual clock starting at `start_ms`.
    pub fn new(start_ms: u64) -> Self {
        Self {
            now_ms: Arc::new(AtomicU64::new(start_ms)),
        }
    }

    /// Advance time by `delta_ms` and return the new value.
    pub fn advance_ms(&self, delta_ms: u64) -> u64 {
        self.now_ms.fetch_add(delta_ms, Ordering::SeqCst) + delta_ms
    }

    /// Set the absolute time.
    pub fn set_ms(&self, value_ms: u64) {
        self.now_ms.store(value_ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_advances_and_sets() {
        let c = ManualClock::new(1_000);
        assert_eq!(c.now_ms(), 1_000);
        assert_eq!(c.advance_ms(500), 1_500);
        assert_eq!(c.now_ms(), 1_500);
        c.set_ms(42);
        assert_eq!(c.now_ms(), 42);
    }

    #[test]
    fn manual_clock_clones_share_time() {
        let a = ManualClock::new(0);
        let b = a.clone();
        a.advance_ms(10);
        assert_eq!(b.now_ms(), 10);
    }

    #[test]
    fn clock_is_object_safe() {
        let c: Box<dyn Clock> = Box::new(ManualClock::new(7));
        assert_eq!(c.now_ms(), 7);
    }
}
