//! One FIFO process admission per Actor. A broadcast wake followed by try_acquire lets
//! already-busy Actors repeatedly beat cold groups. Retain the actual semaphore future
//! across Actor select turns instead; cancellation of an observer must not lose its place.
use super::*;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

type Waiting = Pin<Box<dyn Future<Output = OwnedSemaphorePermit> + Send>>;

#[derive(Default)]
pub(super) enum Admission {
    #[default]
    Idle,
    Waiting(Waiting),
    Ready(OwnedSemaphorePermit),
}
impl Admission {
    /// Nonblocking registration/consumption. No peer, local permit, Server, signed context
    /// or request is retained while waiting. The one fixed future owns only the semaphore.
    /// Poll once here so FIFO registration precedes another hot Actor's next pump, even if
    /// this Actor handles a command before polling its asynchronous maintenance branch.
    pub(super) fn try_take(&mut self, capacity: &Arc<Capacity>) -> Option<OwnedSemaphorePermit> {
        if matches!(self, Self::Idle) {
            let slots = capacity.slots.clone();
            *self = Self::Waiting(Box::pin(async move {
                // This internal process owner is never closed. No peer or caller can close
                // it; destruction/cancellation instead drops this original pending future.
                slots
                    .acquire_owned()
                    .await
                    .expect("process maintenance semaphore stays open")
            }));
        }
        if matches!(self, Self::Ready(_)) {
            let Self::Ready(permit) = std::mem::take(self) else {
                unreachable!()
            };
            return Some(permit);
        }
        let Self::Waiting(waiting) = self else {
            unreachable!()
        };
        let mut cx = Context::from_waker(Waker::noop());
        match waiting.as_mut().poll(&mut cx) {
            Poll::Pending => None,
            Poll::Ready(permit) => {
                *self = Self::Idle;
                Some(permit)
            }
        }
    }

    /// Borrow the SAME stored acquisition. Dropping Maintenance::next's select branch
    /// leaves Pending in place. Store Ready before returning so even an enclosing cancelled
    /// observer cannot drop the grant output or silently re-register at the FIFO tail.
    pub(super) async fn wait_ready(&mut self) {
        let permit = match self {
            Self::Waiting(waiting) => waiting.as_mut().await,
            Self::Ready(_) => return,
            Self::Idle => std::future::pending().await,
        };
        *self = Self::Ready(permit);
    }

    /// There is no eligible local work. A Pending future may ALREADY own an assigned
    /// semaphore grant before its next poll, so drop it as well as an observed Ready grant.
    /// Actual Flight/Lower custody is independent and is never refunded here.
    pub(super) fn clear(&mut self, capacity: &Arc<Capacity>) {
        let old = std::mem::take(self);
        let changed = !matches!(old, Self::Idle);
        drop(old);
        if changed {
            capacity.signal();
        }
    }
}
