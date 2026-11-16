//! A small owned retry window for losing a previously authenticated member. The native minute
//! cadence remains the long-term fallback; an active read must not depend on its startup phase.
//! This schedules only the existing consented-route dial, never PEX, storage or file work.

const FIRST_MS: u64 = 5_000;
const RETRY_MS: u64 = 15_000;
const WINDOW_MS: u64 = 25_000;
const COOLDOWN_MS: u64 = 60_000;
// The shared endpoint scheduler allows at most two actual socket attempts per endpoint/minute.
// Spread these opportunities (+5s, +20s) instead of spending both while a member is restarting.
const ATTEMPTS: u8 = 2;
/// A full driver queue must not hold the sole server actor indefinitely. Queued driver-owned
/// scheduler permits remain charged after this wait ends; an ambiguous submission is not undone.
const SUBMISSION_WAIT_MS: u64 = 250;

pub(super) async fn submit_bounded(
    clock: impl catcoms_rt::Clock,
    submission: impl std::future::Future,
) {
    let due = clock.monotonic_ms().saturating_add(SUBMISSION_WAIT_MS);
    tokio::select! {
        biased;
        _ = super::deadline::wait(clock, Some(due)) => {},
        _ = submission => {},
    }
}

#[derive(Default)]
pub(super) struct RetryWindow {
    next: Option<u64>,
    until: u64,
    cooldown_until: u64,
    remaining: u8,
}
impl RetryWindow {
    pub(super) fn note_loss(&mut self, now: u64) {
        if now < self.cooldown_until {
            return;
        }
        // One original window owns all subsequent wakes. A flapping member cannot extend it,
        // replenish attempts, accelerate a retry or allocate a queue of per-event work.
        self.until = now.saturating_add(WINDOW_MS);
        self.cooldown_until = now.saturating_add(COOLDOWN_MS);
        self.remaining = ATTEMPTS;
        self.next = Some(now.saturating_add(FIRST_MS));
    }

    pub(super) fn next_due(&self) -> Option<u64> {
        self.next
    }

    /// Spend synchronously before any cancellable submission. Missed deadlines do not create
    /// a catch-up burst; the second opportunity stays fifteen seconds after the preceding one.
    pub(super) fn take_due(&mut self, now: u64) -> bool {
        let Some(next) = self.next else { return false };
        if now >= self.until || self.remaining == 0 {
            self.next = None;
            return false;
        }
        if now < next {
            return false;
        }
        self.remaining -= 1;
        self.next = (self.remaining != 0).then(|| now.saturating_add(RETRY_MS).min(self.until));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::{Clock, ManualClock};

    #[tokio::test]
    async fn a_stalled_driver_acknowledgement_cannot_hold_the_owner_past_its_original_deadline() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use std::task::Poll;
        let clock = ManualClock::new(0);
        let submissions = Arc::new(AtomicUsize::new(0));
        let observed = submissions.clone();
        let mut waiting = Box::pin(submit_bounded(clock.clone(), async move {
            observed.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
        }));
        async fn poll_once<F: std::future::Future>(
            future: std::pin::Pin<&mut F>,
        ) -> Poll<F::Output> {
            let mut future = future;
            std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
        }
        assert!(poll_once(waiting.as_mut()).await.is_pending());
        assert_eq!(submissions.load(Ordering::SeqCst), 1);
        clock.advance_ms(SUBMISSION_WAIT_MS - 1);
        assert!(poll_once(waiting.as_mut()).await.is_pending());
        clock.advance_ms(1);
        assert!(poll_once(waiting.as_mut()).await.is_ready());
        assert_eq!(
            submissions.load(Ordering::SeqCst),
            1,
            "the bounded wait cannot resubmit"
        );
    }

    #[test]
    fn loss_bursts_neither_extend_nor_refill_the_original_window() {
        let clock = ManualClock::new(0);
        let mut retry = RetryWindow::default();
        assert_eq!(retry.next_due(), None);
        retry.note_loss(clock.monotonic_ms());
        for delay in [FIRST_MS, RETRY_MS] {
            clock.advance_ms(delay - 1);
            retry.note_loss(clock.monotonic_ms());
            assert!(!retry.take_due(clock.monotonic_ms()));
            clock.advance_ms(1);
            assert!(retry.take_due(clock.monotonic_ms()));
            assert!(
                !retry.take_due(clock.monotonic_ms()),
                "a cancelled waiter cannot refund its turn"
            );
        }
        assert_eq!(retry.next_due(), None);
        retry.note_loss(clock.monotonic_ms());
        assert_eq!(retry.next_due(), None);
        clock.advance_ms(COOLDOWN_MS - clock.monotonic_ms());
        retry.note_loss(clock.monotonic_ms());
        assert_eq!(retry.next_due(), Some(COOLDOWN_MS + FIRST_MS));
    }

    #[test]
    fn late_owner_poll_keeps_spacing_and_expires_without_a_catchup_burst() {
        let clock = ManualClock::new(0);
        let mut retry = RetryWindow::default();
        retry.note_loss(clock.monotonic_ms());
        clock.advance_ms(19_000);
        assert!(retry.take_due(clock.monotonic_ms()));
        assert_eq!(retry.next_due(), Some(WINDOW_MS));
        assert!(!retry.take_due(clock.monotonic_ms()));
        clock.advance_ms(WINDOW_MS - clock.monotonic_ms());
        assert!(!retry.take_due(clock.monotonic_ms()));
        assert_eq!(retry.next_due(), None);
    }
}
