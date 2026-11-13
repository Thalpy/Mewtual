//! Negative-only lifetime signals. Only the actual Store owns `Owner`; observers and revokers
//! keep Weak references. The atomic is checked at final handoff without the inventory writer.
//! Notify is separate from Owner so suspended observers never keep a revoking owner alive.
use catcoms_rt::{RequestAuthority, RequestCancellation, SharedRequestKeepalive};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use tokio::sync::{watch, Notify};

#[cfg(test)]
thread_local! {
    static OBSERVER_CONSTRUCTIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(super) fn take_observer_constructions() -> u64 {
    OBSERVER_CONSTRUCTIONS.with(|count| count.replace(0))
}

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
        self.handle_with_custody(None)
    }
    /// The actual AdmissionOwner supplies its SAME existing resource while it is borrowed.
    /// No weak upgrade may silently turn a funded owner into an unfunded revoker tail.
    pub(super) fn handle_with_custody(
        self: &Arc<Self>,
        custody: Option<SharedRequestKeepalive>,
    ) -> Handle {
        Handle {
            original: Arc::downgrade(self),
            _custody: custody,
        }
    }
    pub(super) fn observer(self: &Arc<Self>) -> RequestCancellation {
        self.observer_with_custody(None)
    }
    pub(super) fn observer_with_custody(
        self: &Arc<Self>,
        custody: Option<SharedRequestKeepalive>,
    ) -> RequestCancellation {
        #[cfg(test)]
        OBSERVER_CONSTRUCTIONS.with(|count| count.set(count.get() + 1));
        // Both chains retain the SAME payment: a lower layer may replace effect authority
        // while preserving the native receiver/keepalive; either original tail remains paid.
        RequestCancellation::new(self.signal.subscribe(), custody.clone())
            .with_authority(self.authority_with_custody(custody))
    }
    fn authority_with_custody(
        self: &Arc<Self>,
        custody: Option<SharedRequestKeepalive>,
    ) -> Arc<Authority> {
        Arc::new(Authority {
            original: Arc::downgrade(self),
            wake: self.wake.clone(),
            _custody: custody,
        })
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
pub(super) struct Handle {
    original: Weak<Owner>,
    // LAST after the possibly final weak original control block. This is resource custody,
    // not a strong owner, sender, renewable authority or lookup by replacement identity.
    _custody: Option<SharedRequestKeepalive>,
}
impl Handle {
    pub(super) fn revoke(&self) {
        if let Some(owner) = self.original.upgrade() {
            owner.revoke();
        }
    }
}

struct Authority {
    original: Weak<Owner>,
    wake: Arc<Notify>,
    // LAST: any transient strong Owner upgraded by authorized() ends before this refund.
    _custody: Option<SharedRequestKeepalive>,
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
    use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
    use crate::store::CriticalCatalogReadPool;
    use std::future::Future;

    struct PaidTail {
        _fee: CatalogReadReservation,
    }
    impl std::fmt::Debug for PaidTail {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("PaidTail").finish_non_exhaustive()
        }
    }
    struct AlwaysAllow;
    impl std::fmt::Debug for AlwaysAllow {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("AlwaysAllow")
        }
    }
    #[async_trait::async_trait]
    impl RequestAuthority for AlwaysAllow {
        fn authorized(&self) -> bool {
            true
        }
        async fn revoked(&self) {
            std::future::pending::<()>().await;
        }
    }

    #[test]
    fn original_receiver_closes_even_while_its_resource_custody_is_deferred() {
        let pool = CriticalCatalogReadPool::new(4096).unwrap();
        let paid = Arc::new(PaidTail {
            _fee: pool.try_reserve(1024).unwrap(),
        });
        let owner = Owner::new();
        let token = owner
            .observer_with_custody(Some(paid.clone()))
            .with_authority(Arc::new(AlwaysAllow));
        let deferred: Option<SharedRequestKeepalive> = Some(paid.clone());
        drop(paid);
        assert!(!token.is_cancelled());
        owner.revoke();
        drop(owner); // Exactly the old Sender lifetime; do not defer this strong owner.
        assert!(
            token.is_cancelled(),
            "native receiver closes despite replacement authority"
        );
        assert_eq!(pool.charged_for_test(), 1024);
        drop(deferred);
        assert_eq!(
            pool.charged_for_test(),
            1024,
            "lower keepalive retains same charge"
        );
        drop(token);
        assert_eq!(pool.charged_for_test(), 0);
    }

    #[test]
    fn authority_keepalive_and_revoker_only_tails_each_retain_the_actual_payment() {
        let pool = CriticalCatalogReadPool::new(4096).unwrap();
        for which in 0..3 {
            let paid = Arc::new(PaidTail {
                _fee: pool.try_reserve(1024).unwrap(),
            });
            let owner = Owner::new();
            let token = owner.observer_with_custody(Some(paid.clone()));
            let keepalive = token.keepalive();
            let authority = owner.authority_with_custody(Some(paid.clone()));
            let revoker = owner.handle_with_custody(Some(paid.clone()));
            drop(token);
            drop(paid);
            drop(owner);
            assert!(!authority.authorized());
            match which {
                0 => {
                    drop(authority);
                    drop(revoker);
                    assert_eq!(pool.charged_for_test(), 1024);
                    drop(keepalive);
                }
                1 => {
                    drop(keepalive);
                    drop(revoker);
                    assert_eq!(pool.charged_for_test(), 1024);
                    drop(authority);
                }
                _ => {
                    drop(authority);
                    drop(keepalive);
                    assert_eq!(pool.charged_for_test(), 1024);
                    revoker.revoke(); // A dead original can never select a replacement.
                    drop(revoker);
                }
            }
            assert_eq!(pool.charged_for_test(), 0);
        }
    }

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
