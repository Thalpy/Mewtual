//! Preserve the sampled actor deadline across selector cancellation and delayed first polls.
use catcoms_rt::Clock;
use std::future::poll_fn;
use std::task::Poll;
use std::time::Duration;

pub(super) async fn wait(clock: impl Clock, due_ms: Option<u64>) {
    let Some(due_ms) = due_ms else {
        return std::future::pending().await;
    };
    let now = clock.monotonic_ms();
    if now >= due_ms {
        return;
    }
    let mut sleep = clock.sleep(Duration::from_millis(due_ms - now));
    poll_fn(|cx| {
        // A ManualClock can advance between the remaining-time calculation and sleep arming.
        // Its notification wakes this wrapper even when the relative sleep's own target shifted.
        if clock.monotonic_ms() >= due_ms {
            return Poll::Ready(());
        }
        let ready = sleep.as_mut().poll(cx);
        if clock.monotonic_ms() >= due_ms {
            Poll::Ready(())
        } else {
            ready
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::ManualClock;
    use std::future::Future;
    use std::pin::Pin;

    async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
        poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
    }

    #[tokio::test]
    async fn actor_deadline_elapsed_before_first_poll_does_not_restart_the_throttle() {
        let clock = ManualClock::new(1_000);
        let now = clock.monotonic_ms();
        let delivery = std::collections::HashMap::from([(1, (now, Vec::new()))]);
        let dirty = std::collections::HashSet::from([1]);
        let delay = super::super::next_delivery_delay(now, &delivery, &dirty).unwrap();
        let mut wake = Box::pin(wait(clock.clone(), Some(now + delay)));
        clock.advance_ms(delay);
        assert!(
            poll_once(wake.as_mut()).await.is_ready(),
            "the original deadline is already due"
        );
    }

    #[derive(Debug)]
    struct AdvanceWhileArming {
        clock: ManualClock,
        advance_ms: u64,
    }

    impl Clock for AdvanceWhileArming {
        fn now_ms(&self) -> u64 {
            panic!("actor deadlines never consult wall time")
        }
        fn monotonic_ms(&self) -> u64 {
            self.clock.monotonic_ms()
        }
        fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.clock.advance_ms(self.advance_ms);
            self.clock.sleep(duration)
        }
    }

    #[tokio::test]
    async fn actor_deadline_observes_full_or_partial_clock_advance_while_arming() {
        for advance_ms in [400, 1_000] {
            let clock = ManualClock::new(1_000);
            let mut wake = Box::pin(wait(
                AdvanceWhileArming {
                    clock: clock.clone(),
                    advance_ms,
                },
                Some(2_000),
            ));
            let first = poll_once(wake.as_mut()).await;
            if advance_ms == 1_000 {
                assert!(first.is_ready(), "arming crossed the absolute deadline");
            } else {
                assert!(first.is_pending());
                clock.advance_ms(600);
                assert!(
                    poll_once(wake.as_mut()).await.is_ready(),
                    "a shifted relative target cannot postpone the original deadline"
                );
            }
        }
    }

    #[tokio::test]
    async fn actor_deadline_stays_pending_before_monotonic_due_despite_wall_change() {
        let clock = ManualClock::new(1_000);
        let mut wake = Box::pin(wait(clock.clone(), Some(2_000)));
        assert!(poll_once(wake.as_mut()).await.is_pending());
        clock.set_wall_ms(u64::MAX - 1_000);
        clock.advance_ms(999);
        assert!(poll_once(wake.as_mut()).await.is_pending());
        clock.set_wall_ms(0);
        clock.advance_ms(1);
        assert!(poll_once(wake.as_mut()).await.is_ready());
        let mut absent = Box::pin(wait(clock, None));
        assert!(poll_once(absent.as_mut()).await.is_pending());
    }
}
