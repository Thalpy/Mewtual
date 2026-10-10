//! Keep uninterruptible image workers charged until the blocking closure actually exits.
use std::sync::Arc;
use std::time::Duration;

use catcoms_rt::Clock;
use tokio::sync::Semaphore;

use crate::media_decode::DecodeRefusal;

pub(super) async fn run<R: Send + 'static>(
    permits: Arc<Semaphore>,
    timeout: Duration,
    clock: &impl Clock,
    work: impl FnOnce() -> R + Send + 'static,
) -> Result<R, DecodeRefusal> {
    let permit = permits
        .acquire_owned()
        .await
        .map_err(|_| DecodeRefusal::DeadlineExceeded)?;
    let handle = tauri::async_runtime::spawn_blocking(move || {
        // The waiter can disappear on timeout, lock or request cancellation. Its JoinHandle
        // cannot interrupt a running decoder, so capacity must belong to this closure instead.
        let _worker_permit = permit;
        work()
    });
    // As before, the response deadline starts once capacity is acquired. It is not a promise
    // that the decoder can be stopped, and cannot make another worker slot available early.
    tokio::select! {
        result = handle => result.map_err(|_| DecodeRefusal::DecoderPanicked),
        _ = clock.sleep(timeout) => Err(DecodeRefusal::DeadlineExceeded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::{ManualClock, SystemClock};
    use std::future::{poll_fn, Future};
    use std::pin::Pin;
    use std::sync::mpsc;
    use std::task::Poll;
    use tokio::sync::oneshot;

    async fn bounded<T>(future: impl Future<Output = T>) -> T {
        tokio::select! {
            result = future => result,
            _ = SystemClock.sleep(Duration::from_secs(5)) => panic!("controlled worker did not make progress"),
        }
    }

    async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
        poll_fn(|context| Poll::Ready(future.as_mut().poll(context))).await
    }

    fn paused_worker() -> (
        impl FnOnce() + Send + 'static,
        mpsc::Sender<()>,
        oneshot::Receiver<()>,
    ) {
        let (release, receiver) = mpsc::channel();
        let (started, observed) = oneshot::channel();
        (
            move || {
                let _ = started.send(());
                // Dropping the sender also releases this worker if any test assertion unwinds.
                // An adversarial mutant must fail instead of hanging runtime shutdown.
                let _ = receiver.recv();
            },
            release,
            observed,
        )
    }

    #[tokio::test]
    async fn media_worker_keeps_capacity_after_waiter_cancellation_and_timeout() {
        let permits = Arc::new(Semaphore::new(crate::MEDIA_TRANSCODE_PERMITS));
        assert_eq!(crate::MEDIA_TRANSCODE_PERMITS, 2);
        let clock = ManualClock::new(0);
        let (first_work, first_release, first_started) = paused_worker();
        let mut cancelled = Box::pin(run(
            Arc::clone(&permits),
            crate::MEDIA_TRANSCODE_TIMEOUT,
            &clock,
            first_work,
        ));
        assert!(poll_once(cancelled.as_mut()).await.is_pending());
        bounded(first_started).await.unwrap();
        drop(cancelled);
        assert_eq!(
            permits.available_permits(),
            1,
            "a cancelled waiter does not finish its decoder"
        );

        let (second_work, second_release, second_started) = paused_worker();
        let mut timed_out = Box::pin(run(
            Arc::clone(&permits),
            crate::MEDIA_TRANSCODE_TIMEOUT,
            &clock,
            second_work,
        ));
        assert!(poll_once(timed_out.as_mut()).await.is_pending());
        bounded(second_started).await.unwrap();
        clock.advance_ms(crate::MEDIA_TRANSCODE_TIMEOUT.as_millis() as u64);
        assert!(matches!(
            bounded(timed_out).await,
            Err(DecodeRefusal::DeadlineExceeded)
        ));
        assert_eq!(
            permits.available_permits(),
            0,
            "timeout still charges the second live decoder"
        );

        let (third_started, mut third_observed) = oneshot::channel();
        let mut third = Box::pin(run(
            Arc::clone(&permits),
            crate::MEDIA_TRANSCODE_TIMEOUT,
            &clock,
            move || {
                let _ = third_started.send(());
                7
            },
        ));
        assert!(poll_once(third.as_mut()).await.is_pending());
        assert!(
            matches!(
                third_observed.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "the shared cap forbids a third blocking worker while both predecessors run"
        );
        drop(first_release);
        assert_eq!(bounded(third).await.unwrap(), 7);
        bounded(third_observed).await.unwrap();
        assert_eq!(permits.available_permits(), 1);
        drop(second_release);
        let first = bounded(Arc::clone(&permits).acquire_owned()).await.unwrap();
        let second = bounded(Arc::clone(&permits).acquire_owned()).await.unwrap();
        drop((first, second));
        assert_eq!(
            permits.available_permits(),
            2,
            "worker exit restores all capacity"
        );
    }

    #[tokio::test]
    async fn media_worker_panic_releases_its_owned_capacity() {
        let permits = Arc::new(Semaphore::new(1));
        let result = bounded(run(
            Arc::clone(&permits),
            Duration::from_secs(10),
            &SystemClock,
            || panic!("controlled blocking worker panic"),
        ))
        .await;
        assert!(matches!(result, Err(DecodeRefusal::DecoderPanicked)));
        assert_eq!(permits.available_permits(), 1);
    }
}
