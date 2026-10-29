//! Negative-only lifetime signals. Only the actual Store owns `Owner`; observers and revokers
//! keep Weak references. The atomic is checked at final handoff without the inventory writer.
//! Notify is separate from Owner so suspended observers never keep a revoking owner alive.
use catcoms_rt::{RequestAuthority, RequestCancellation};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use tokio::sync::{watch, Notify};

pub(super) struct Owner {
    identity: Arc<()>,
    revoked: AtomicBool,
    wake: Arc<Notify>,
    signal: watch::Sender<bool>,
}
impl Owner {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            identity: Arc::new(()),
            revoked: AtomicBool::new(false),
            wake: Arc::new(Notify::new()),
            signal: watch::channel(false).0,
        })
    }
    pub(super) fn identity(&self) -> Arc<()> {
        self.identity.clone()
    }
    pub(super) fn live(&self) -> bool {
        !self.revoked.load(Ordering::Acquire)
    }
    pub(super) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
    pub(super) fn handle(self: &Arc<Self>) -> Handle {
        Handle(Arc::downgrade(self))
    }
    pub(super) fn observer(self: &Arc<Self>) -> RequestCancellation {
        RequestCancellation::new(self.signal.subscribe(), None).with_authority(Arc::new(
            Authority {
                original: Arc::downgrade(self),
                wake: self.wake.clone(),
            },
        ))
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.revoke();
    }
}

/// No lookup by logical key and no retained publisher. A stale handle can affect only its
/// original owner, never whatever newer admission/mount occupies the same inventory identity.
#[derive(Clone)]
pub(super) struct Handle(Weak<Owner>);
impl Handle {
    pub(super) fn revoke(&self) {
        if let Some(owner) = self.0.upgrade() {
            owner.revoke();
        }
    }
}

struct Authority {
    original: Weak<Owner>,
    wake: Arc<Notify>,
}
impl std::fmt::Debug for Authority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatRevocationAuthority")
            .finish_non_exhaustive()
    }
}
#[async_trait::async_trait]
impl RequestAuthority for Authority {
    fn authorized(&self) -> bool {
        self.original.upgrade().is_some_and(|owner| owner.live())
    }
    async fn revoked(&self) {
        loop {
            // Register BEFORE checking the atomic, including notify_waiters' non-retained wake.
            // Only Notify is retained across await; Owner Drop must remain observable.
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.authorized() {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    #[tokio::test]
    async fn registered_waiter_and_weak_handle_do_not_retain_the_only_owner() {
        let owner = Owner::new();
        let identity = owner.identity();
        let handle = owner.handle();
        let mut observer = owner.observer();
        let mut waiting = std::pin::pin!(observer.cancelled());
        std::future::poll_fn(|cx| {
            assert!(waiting.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(owner);
        std::future::poll_fn(|cx| {
            assert!(waiting.as_mut().poll(cx).is_ready());
            std::task::Poll::Ready(())
        })
        .await;
        handle.revoke();
        assert_eq!(
            Arc::strong_count(&identity),
            1,
            "dummy identity cannot keep the owner alive"
        );
    }
}
