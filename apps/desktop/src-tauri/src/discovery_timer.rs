//! The discovery cadence shared by the native host and headless transport acceptance tests.
use std::future::Future;
use std::time::Duration;

use catcoms_rt::Clock;
use tokio::sync::watch;

use super::{DISCOVERY_INTERVAL_SECS, DISCOVERY_JITTER_MS, DISCOVERY_START_SPREAD_MS};

/// Run one recovery pass at a time. A network change expedites the next pass; changes that
/// arrive during a pass remain pending on the watch receiver and coalesce into one wake.
/// The caller subscribes before spawning and owns platform refresh and persistence effects.
pub(super) async fn run<C, J, F, P>(
    clock: &C,
    mut network_changes: watch::Receiver<u64>,
    mut jitter: J,
    mut pass: F,
) where
    C: Clock,
    J: FnMut(u64, u64) -> Duration,
    F: FnMut() -> P,
    P: Future<Output = bool>,
{
    let mut delay = jitter(0, DISCOVERY_START_SPREAD_MS);
    loop {
        tokio::select! {
            _ = clock.sleep(delay) => {}
            changed = network_changes.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
        if !pass().await {
            break;
        }
        delay = jitter(
            DISCOVERY_INTERVAL_SECS * 1_000 - DISCOVERY_JITTER_MS,
            DISCOVERY_JITTER_MS * 2,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::ManualClock;
    use std::future::poll_fn;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::Poll;

    async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
        poll_fn(|context| Poll::Ready(future.as_mut().poll(context))).await
    }

    #[tokio::test]
    async fn discovery_cadence_preserves_start_and_both_period_boundaries() {
        for high in [false, true] {
            let clock = ManualClock::new(0);
            let (_changes, receiver) = watch::channel(0);
            let calls = Arc::new(AtomicUsize::new(0));
            let ranges = Arc::new(Mutex::new(Vec::new()));
            let count = calls.clone();
            let seen = ranges.clone();
            let mut timer = Box::pin(run(
                &clock,
                receiver,
                move |base, spread| {
                    seen.lock().unwrap().push((base, spread));
                    Duration::from_millis(base + if high { spread - 1 } else { 0 })
                },
                move || {
                    let again = count.fetch_add(1, Ordering::SeqCst) == 0;
                    async move { again }
                },
            ));
            assert!(poll_once(timer.as_mut()).await.is_pending());
            if high {
                clock.advance_ms(DISCOVERY_START_SPREAD_MS - 2);
                assert!(poll_once(timer.as_mut()).await.is_pending());
                assert_eq!(calls.load(Ordering::SeqCst), 0);
                clock.advance_ms(1);
                assert!(poll_once(timer.as_mut()).await.is_pending());
            }
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            let period = DISCOVERY_INTERVAL_SECS * 1_000 - DISCOVERY_JITTER_MS
                + if high { DISCOVERY_JITTER_MS * 2 - 1 } else { 0 };
            clock.advance_ms(period - 1);
            assert!(poll_once(timer.as_mut()).await.is_pending());
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            clock.advance_ms(1);
            assert!(poll_once(timer.as_mut()).await.is_ready());
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            assert_eq!(
                *ranges.lock().unwrap(),
                vec![(0, 5_000), (45_000, 30_000)],
                "production cadence changed"
            );
        }
    }

    #[tokio::test]
    async fn discovery_cadence_keeps_changes_during_a_slow_pass_without_overlap() {
        let clock = ManualClock::new(0);
        let (changes, receiver) = watch::channel(0);
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (release, wait) = tokio::sync::oneshot::channel();
        let mut first_wait = Some(wait);
        let mut timer = Box::pin(run(
            &clock,
            receiver,
            |base, spread| Duration::from_millis(base + spread - 1),
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                let wait = first_wait.take();
                async move {
                    if let Some(wait) = wait {
                        wait.await.unwrap();
                        true
                    } else {
                        false
                    }
                }
            },
        ));
        assert!(poll_once(timer.as_mut()).await.is_pending());
        changes.send_replace(1);
        assert!(poll_once(timer.as_mut()).await.is_pending());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        changes.send_replace(2);
        changes.send_replace(3);
        clock.advance_ms(150_000);
        assert!(poll_once(timer.as_mut()).await.is_pending());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a pass cannot overlap itself"
        );
        release.send(()).unwrap();
        assert!(poll_once(timer.as_mut()).await.is_ready());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "changes coalesce but are not lost"
        );
    }

    #[tokio::test]
    async fn discovery_cadence_exits_when_its_network_signal_closes() {
        let clock = ManualClock::new(0);
        let (changes, receiver) = watch::channel(0);
        drop(changes);
        let mut timer = Box::pin(run(
            &clock,
            receiver,
            |base, spread| Duration::from_millis(base + spread - 1),
            || async { panic!("a retired cadence must not begin another pass") },
        ));
        assert!(poll_once(timer.as_mut()).await.is_ready());
    }
}
