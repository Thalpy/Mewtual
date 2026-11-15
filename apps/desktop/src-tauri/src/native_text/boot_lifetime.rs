//! Native bootstrap FILE-WORK lifetime, not the complete unlock/IPC task.
//! Never enter before waiting for the close prelude: that task may own
//! vault_mount while close itself retains the prelude through destruction.
//!
//! The owner of an End must keep it LAST through actual blocking work and its
//! returned Bootstrap/ReadyBundle, until canonical transfer or destruction.
//! Zero does not dispose a retained partial plan parked in the native slot.
use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::sync::Notify;

const CLOSED: usize = 1 << (usize::BITS - 1);
const MAX_WORK: usize = 1;

/// Independently paid, fee-only original stamp. No Bootstrap, Source, facade,
/// native registry or physical backlink may be added to this allocation.
pub(super) struct Control {
    state: AtomicUsize,
    changed: Notify,
    _metadata: catcoms_app::store::NativeUiInvocationMetadata,
}
impl Control {
    pub(super) fn new(metadata: catcoms_app::store::NativeUiInvocationMetadata) -> Arc<Self> {
        Arc::new(Self {
            state: AtomicUsize::new(0),
            changed: Notify::new(),
            _metadata: metadata,
        })
    }
    /// Exact allocation identity, independent of user labels or UI generation.
    /// This proves no current authority; actual UI/close checks remain separate.
    pub(super) fn same_original(self: &Arc<Self>, other: &Arc<Self>) -> bool {
        Arc::ptr_eq(self, other)
    }
    /// Admit immediately before real bootstrap/file work, after prelude-idle
    /// and original-generation checks. A delayed old task cannot enter CLOSED.
    pub(super) fn enter(self: &Arc<Self>) -> Result<End, &'static str> {
        self.state
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                (value & CLOSED == 0 && value < MAX_WORK).then_some(value + 1)
            })
            .map_err(|_| "Vault startup is unavailable for this session.")?;
        Ok(End(self.clone()))
    }
    /// Only irreversible close (after continuity/may_destroy admission) uses
    /// this. Tentative/continuity-deferred close must preserve later unlock.
    /// SC-only: no notification or destructor under native Store/controller.
    pub(super) fn close(&self) {
        self.state.fetch_or(CLOSED, Ordering::SeqCst);
    }
    pub(super) fn is_closed_cached(&self) -> bool {
        self.state.load(Ordering::SeqCst) & CLOSED != 0
    }
    pub(super) fn is_drained_cached(&self) -> bool {
        self.state.load(Ordering::SeqCst) == CLOSED
    }
    /// Issued only after close. Wait outside Store/UI/controller and WITHOUT
    /// joining vault_mount or the complete unlock finalizer/IPC task.
    pub(super) async fn wait(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.is_drained_cached() {
                return;
            }
            changed.await;
        }
    }
}

/// Actual work and returned owners precede this End in every job/envelope.
/// Retiring it is not permission to replace/drop an unknown original plan.
pub(super) struct End(Arc<Control>);
impl Drop for End {
    fn drop(&mut self) {
        let previous = self.0.state.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(previous & !CLOSED != 0);
        self.0.changed.notify_waiters();
    }
}

const _: () =
    assert!(2 * (std::mem::size_of::<Control>() + std::mem::size_of::<End>()) + 512 <= 4096);

#[cfg(test)]
mod tests {
    use super::*;
    fn control() -> Arc<Control> {
        let budget = NativeUiInvocationBudget::new(
            catcoms_app::store::CriticalCatalogReadPool::new(
                NativeUiInvocationBudget::required_capacity(),
            )
            .unwrap(),
        )
        .unwrap();
        Control::new(budget.prepare().unwrap())
    }
    #[test]
    fn native_text_boot_lifetime_close_before_entry_is_permanent_and_original() {
        let original = control();
        let foreign = control();
        original.close();
        assert!(original.is_drained_cached());
        assert!(original.enter().is_err());
        assert!(original.same_original(&original.clone()));
        assert!(!original.same_original(&foreign));
        assert!(!foreign.is_drained_cached());
        let work = foreign.enter().unwrap();
        assert!(
            foreign.enter().is_err(),
            "the one actual bootstrap position is bounded"
        );
        foreign.close();
        assert!(!foreign.is_drained_cached());
        drop(work);
        assert!(foreign.is_drained_cached());
        assert!(foreign.enter().is_err());
    }
    #[tokio::test]
    async fn native_text_boot_lifetime_returned_owner_and_unwind_retire_before_end() {
        struct Owner(Arc<Control>);
        impl Drop for Owner {
            fn drop(&mut self) {
                assert!(!self.0.is_drained_cached());
            }
        }
        struct Returned {
            _owner: Owner,
            _end: End,
        }
        let original = control();
        let output = Returned {
            _owner: Owner(original.clone()),
            _end: original.enter().unwrap(),
        };
        original.close();
        let mut wait = Box::pin(original.wait());
        std::future::poll_fn(|cx| {
            assert!(wait.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(output);
        wait.await;
        let second = control();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _output = Returned {
                _owner: Owner(second.clone()),
                _end: second.enter().unwrap(),
            };
            second.close();
            panic!("actual owning envelope unwind");
        }));
        assert!(result.is_err());
        assert!(second.is_drained_cached());
    }
}
