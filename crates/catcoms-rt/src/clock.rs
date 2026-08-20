//! The [`Clock`] seam: logical time, injectable everywhere.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

/// A source of wall-clock time in milliseconds since the Unix epoch.
///
/// Inject a `Clock` wherever time is needed (expiry horizons, MLS lifetimes,
/// retry backoff, CRDT timestamps) instead of reading the OS clock, so behaviour
/// is fully deterministic under test.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;

    /// Wait for a duration using this clock's notion of time.
    ///
    /// Runtime retries must use this seam as well as `now_ms`; otherwise a test can control expiry
    /// calculations but still has to wait on wall-clock backoff. The boxed future keeps the trait
    /// object-safe for the relay/rendezvous nodes that store `Arc<dyn Clock>`.
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

impl<T: Clock + ?Sized> Clock for Arc<T> {
    fn now_ms(&self) -> u64 {
        (**self).now_ms()
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        (**self).sleep(duration)
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

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// A deterministic, manually-advanced clock for tests. Cloneable; all clones
/// share the same underlying time.
#[derive(Debug, Default, Clone)]
pub struct ManualClock {
    now_ms: Arc<AtomicU64>,
    wake: Arc<Notify>,
}

impl ManualClock {
    /// A manual clock starting at `start_ms`.
    pub fn new(start_ms: u64) -> Self {
        Self {
            now_ms: Arc::new(AtomicU64::new(start_ms)),
            wake: Arc::new(Notify::new()),
        }
    }

    /// Advance time by `delta_ms` and return the new value.
    pub fn advance_ms(&self, delta_ms: u64) -> u64 {
        let now = self.now_ms.fetch_add(delta_ms, Ordering::SeqCst) + delta_ms;
        self.wake.notify_waiters();
        now
    }

    /// Set the absolute time.
    pub fn set_ms(&self, value_ms: u64) {
        self.now_ms.store(value_ms, Ordering::SeqCst);
        self.wake.notify_waiters();
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let target = self
            .now_ms()
            .saturating_add(duration.as_millis().try_into().unwrap_or(u64::MAX));
        Box::pin(async move {
            loop {
                // Enable registration before the second read so an advance between that read and
                // `await` cannot be lost. `notify_waiters` intentionally stores no spare permit.
                let advanced = self.wake.notified();
                tokio::pin!(advanced);
                advanced.as_mut().enable();
                if self.now_ms() >= target {
                    return;
                }
                advanced.await;
            }
        })
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

    #[tokio::test]
    async fn manual_sleep_completes_only_after_logical_time_advances() {
        let clock = ManualClock::new(1_000);
        let sleeper = clock.clone();
        let task = tokio::spawn(async move {
            sleeper.sleep(Duration::from_millis(60)).await;
            sleeper.now_ms()
        });
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        clock.advance_ms(59);
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        clock.advance_ms(1);
        assert_eq!(task.await.unwrap(), 1_060);
    }
}
