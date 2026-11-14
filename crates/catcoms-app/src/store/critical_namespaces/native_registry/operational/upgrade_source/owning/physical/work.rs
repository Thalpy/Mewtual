//! Original metadata/UI capsule drainage. Source/SDK/TCP and completed debt
//! have independent custody; observing these slots cannot replace their proofs.
use super::*;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::sync::Notify;

const ACTIVE: u8 = 1;
const CLOSED: u8 = 2;

pub(super) struct Slot {
    state: AtomicU8,
    changed: Notify,
}
impl Slot {
    pub(super) fn new() -> Self {
        Self {
            state: AtomicU8::new(0),
            changed: Notify::new(),
        }
    }
    /// One CAS registers the actual capsule before its Physical clone. A
    /// delayed caller can never increment behind a CLOSED+zero observation.
    pub(super) fn enter(&self) -> bool {
        self.state
            .compare_exchange(0, ACTIVE, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
    /// SC-only, including idle closure. Observers are issued AFTER this step,
    /// so no notification/destructor can run under the native Store guard.
    pub(super) fn close(&self) {
        self.state.fetch_or(CLOSED, Ordering::SeqCst);
    }
    fn drained(&self) -> bool {
        self.state.load(Ordering::SeqCst) == CLOSED
    }
    #[cfg(test)]
    pub(super) fn active_for_test(&self) -> bool {
        self.state.load(Ordering::SeqCst) & ACTIVE != 0
    }
}

pub(super) struct SlotAck {
    pub(super) original: Arc<Slot>,
    pub(super) _payment: lifecycle_admission::PaymentTail,
}
impl Drop for SlotAck {
    fn drop(&mut self) {
        // Work declares Physical FIRST, Ack LAST. Do not move this acknowledge
        // before the actual work/outputs or clear CLOSED and reopen admission.
        let previous = self.original.state.fetch_and(!ACTIVE, Ordering::SeqCst);
        debug_assert!(previous & ACTIVE != 0);
        self.original.changed.notify_waiters();
    }
}

/// Fee-only ORIGINAL observation. No Handler/cold evidence, Physical, Current,
/// Source owner, weak upgrade or authority to open/change anything is retained.
pub(in crate::store::critical_namespaces::native_registry) struct WorkDrain {
    index: Arc<Slot>,
    ui: Arc<Slot>,
    _payment: lifecycle_admission::PaymentTail,
}
impl WorkDrain {
    pub(super) fn new(physical: &Physical) -> Self {
        // Idempotent closure precedes observation issuance. This is not an
        // already-waiting OPEN counter that would need an idle-close wake.
        physical.slot.close();
        physical.ui_slot.close();
        Self {
            index: physical.slot.clone(),
            ui: physical.ui_slot.clone(),
            _payment: physical.handler.payment_tail(),
        }
    }
    pub(in crate::store::critical_namespaces::native_registry) fn is_drained(&self) -> bool {
        self.index.drained() && self.ui.drained()
    }
    pub(in crate::store::critical_namespaces::native_registry) async fn wait(&self) {
        loop {
            let index = self.index.changed.notified();
            let ui = self.ui.changed.notified();
            tokio::pin!(index, ui);
            index.as_mut().enable();
            ui.as_mut().enable();
            if self.is_drained() {
                return;
            }
            tokio::select! {_=index => {}, _=ui => {}}
        }
    }
}
impl std::fmt::Debug for WorkDrain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OriginalMetadataUiWorkDrain { .. }")
    }
}

pub(super) const METADATA: usize = 2 * std::mem::size_of::<Slot>()
    + 2 * std::mem::size_of::<WorkDrain>()
    + 8 * std::mem::size_of::<usize>()
    + 256;
