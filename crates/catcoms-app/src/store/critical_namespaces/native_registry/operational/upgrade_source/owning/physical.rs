//! Finite original physical3 custody, separate from affine admission. No raw
//! Current/Store, owned index, SDK entry or native command credential escapes.
use super::*;
use crate::store::critical_namespaces::native_registry::lifecycle::{
    cell, founder, refresh, rename,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

pub(super) struct Physical {
    // Actual poisoned cell and its pending files/buffers retire before Current.
    cell: Mutex<cell::Cell>,
    // None exists ONLY during private preallocation. Every allocation/check
    // precedes extraction of the original Current; the closer is then armed
    // before an infallible unique move fills Some. No method exposes None.
    current: Option<V2CurrentLayoutStore>,
    slot: Arc<Slot>,
    ui_slot: Arc<Slot>,
    // LAST: actual original payment after every file/slot control allocation.
    handler: lifecycle_admission::Handler,
}
struct Slot {
    taken: AtomicBool,
}

/// Exactly one prepaid fixed capsule. It is not Clone and contains no affine
/// close owner. Destroy the WHOLE capsule outside native/cell/catalog guards:
/// its final physical Arc can run real file and Current destructors.
pub(in crate::store::critical_namespaces::native_registry) struct MetadataWork {
    // FIRST: actual last-Arc physical destruction finishes before slot release.
    physical: Arc<Physical>,
    _ack: SlotAck,
}
struct SlotAck {
    original: Arc<Slot>,
    _payment: lifecycle_admission::PaymentTail,
}
impl Drop for SlotAck {
    fn drop(&mut self) {
        self.original.taken.store(false, Ordering::SeqCst);
    }
}

pub(super) const METADATA: usize = 8
    * (std::mem::size_of::<Physical>() + std::mem::size_of::<MetadataWork>()
    + std::mem::size_of::<SlotAck>() + std::mem::size_of::<Slot>()
    // Physical and both slot Arc controls, plus Option control metadata.
    + 6 * std::mem::size_of::<usize>())
    + 512;

impl Physical {
    /// Allocate BEFORE original Current extraction, with the original upgrade
    /// still wholly armed. No physical allocation is exposed until install().
    pub(super) fn allocate(handler: lifecycle_admission::Handler) -> Arc<Self> {
        Arc::new(Self {
            cell: Mutex::default(),
            current: None,
            slot: Arc::new(Slot {
                taken: AtomicBool::new(false),
            }),
            ui_slot: Arc::new(Slot {
                taken: AtomicBool::new(false),
            }),
            handler,
        })
    }
    /// The caller owns the sole unexposed Arc and has armed its destination's
    /// closer. This is a plain move: no callback, IO, allocation or loan.
    pub(super) fn install(&mut self, current: V2CurrentLayoutStore) {
        debug_assert!(self.current.is_none());
        self.current = Some(current);
    }
    fn current(&self) -> Result<&V2CurrentLayoutStore, AppError> {
        self.current.as_ref().ok_or_else(refused)
    }
    pub(super) fn finish_cold(&self) -> Result<(), AppError> {
        self.handler.finish_cold(self.current()?.checked_store()?)
    }
    #[cfg(test)]
    pub(super) fn handler_for_test(&self) -> lifecycle_admission::Handler {
        self.handler.clone()
    }
    #[cfg(test)]
    pub(super) fn actual_store_for_test(&self) -> Result<&ServerStore, AppError> {
        self.current()?.checked_store()
    }
    fn open_for_custody(&self) -> bool {
        self.handler.is_open_cached()
            && self
                .current
                .as_ref()
                .is_some_and(|current| current.physical_mount_is_live_cached())
    }
    /// Only cached atomics and infallible Arc clones. The one fixed slot pays
    /// this capsule before capture; no account/observer/cell lock is needed.
    /// An active claim may be lent for exact retry, never mistaken for Idle.
    pub(super) fn capture(self: &Arc<Self>) -> Option<MetadataWork> {
        if !self.open_for_custody()
            || self
                .slot
                .taken
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            return None;
        }
        // Arm the ack immediately after the slot CAS. All construction here is
        // plain moves/Arc clones; there is no fallible gap with an orphan slot.
        let ack = SlotAck {
            original: self.slot.clone(),
            _payment: self.handler.payment_tail(),
        };
        let value = MetadataWork {
            physical: self.clone(),
            _ack: ack,
        };
        if !value.physical.open_for_custody() {
            return None;
        }
        Some(value)
    }
    /// The UI sidecar is independent of index WAL/claim state. Reusing the
    /// index slot would let an uncertain metadata transaction starve drafts.
    pub(super) fn capture_ui(self: &Arc<Self>) -> Option<UiWork> {
        if !self.open_for_custody()
            || self
                .ui_slot
                .taken
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            return None;
        }
        let ack = SlotAck {
            original: self.ui_slot.clone(),
            _payment: self.handler.payment_tail(),
        };
        let work = UiWork {
            physical: self.clone(),
            _ack: ack,
        };
        if !work.is_live_cached() {
            return None;
        }
        Some(work)
    }
    pub(super) fn verify_native_passphrase(&self, passphrase: &[u8]) -> Result<(), AppError> {
        if !self.open_for_custody() {
            return Err(refused());
        }
        self.current()?
            .cached_store()?
            .verify_native_critical_passphrase(passphrase)?;
        if !self.open_for_custody() {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn close_admission(&self) {
        self.handler.close();
    }
    /// Exact bounded immutable receipt lookup under the SAME cell/claim. A
    /// matched unsupported backend refuses; it must never become absence.
    pub(super) fn lookup_creation_data(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<Option<crate::store::NativeCreationStatusData>, AppError> {
        let cell = self.cell.try_lock().map_err(|_| refused())?;
        cell.require_idle()?;
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        let current = self.current()?;
        let store = current.checked_store()?;
        let inventory =
            UpgradeInventory::capture(store, self.handler.debit(), self.handler.pool())?;
        let result =
            super::super::super::super::lifecycle::preparation::lookup_selected_creation_data(
                store, &inventory, invocation, request,
            )?;
        let guard = store
            .pending_join_catalog
            .try_lock()
            .map_err(|_| refused())?;
        self.handler.require_idle_cached()?;
        if guard.as_ref().ok_or_else(refused)?.pending.is_some() {
            return Err(refused());
        }
        inventory.require(store)?;
        drop(guard);
        drop(inventory);
        drop(cell);
        Ok(result)
    }
    pub(super) fn row_count_data(&self) -> Result<usize, AppError> {
        let cell = self.cell.try_lock().map_err(|_| refused())?;
        cell.require_idle()?;
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        let current = self.current()?;
        let store = current.checked_store()?;
        let inventory =
            UpgradeInventory::capture(store, self.handler.debit(), self.handler.pool())?;
        let (count, _) = super::super::super::super::lifecycle::preparation::authenticate_selected(
            store, &inventory,
        )?;
        let guard = store
            .pending_join_catalog
            .try_lock()
            .map_err(|_| refused())?;
        self.handler.require_idle_cached()?;
        if guard.as_ref().ok_or_else(refused)?.pending.is_some() {
            return Err(refused());
        }
        inventory.require(store)?;
        drop(guard);
        drop(inventory); // actual buffers drain before cell unlock
        Ok(count)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn birth_founder_data(
        &self,
        request: &founder::Pending,
        rng: &mut impl CryptoRngCore,
    ) -> Result<
        (
            super::super::super::mount::NativeCreationStatusData,
            Option<cell::CompletedDebt>,
        ),
        AppError,
    > {
        cell::birth_founder(&self.cell, self.current()?, &self.handler, request, rng)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn activate_founder_data(
        &self,
        request: &founder::Activate,
        rng: &mut impl CryptoRngCore,
    ) -> Result<
        (
            super::super::super::mount::NativeCreationStatusData,
            Option<cell::CompletedDebt>,
        ),
        AppError,
    > {
        cell::activate_founder(&self.cell, self.current()?, &self.handler, request, rng)
    }
    pub(super) fn refresh_legacy_invite_data(
        &self,
        request: &refresh::Request,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(rename::Metadata, Option<cell::CompletedDebt>), AppError> {
        cell::refresh_legacy_invite(&self.cell, self.current()?, &self.handler, request, rng)
    }
    pub(super) fn rename_metadata_data(
        &self,
        request: &rename::Request,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(rename::Metadata, Option<cell::CompletedDebt>), AppError> {
        cell::rename_metadata(&self.cell, self.current()?, &self.handler, request, rng)
    }
}
/// One dedicated original UI-sidecar slot. Neither Clone nor a close owner.
/// Actual last Physical/Current destruction precedes its acknowledgement.
pub(in crate::store::critical_namespaces::native_registry) struct UiWork {
    physical: Arc<Physical>,
    _ack: SlotAck,
}
impl UiWork {
    pub(in crate::store::critical_namespaces::native_registry) fn is_live_cached(&self) -> bool {
        self.physical.open_for_custody()
    }
    pub(in crate::store::critical_namespaces::native_registry) fn read(
        &self,
    ) -> Result<Option<Vec<u8>>, AppError> {
        if !self.is_live_cached() {
            return Err(refused());
        }
        let result = self
            .physical
            .current()?
            .cached_store()?
            .load_ui_state_record()?;
        if !self.is_live_cached() {
            return Err(refused());
        }
        Ok(result)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn save(
        &self,
        bytes: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if !self.is_live_cached() {
            return Err(refused());
        }
        self.physical
            .current()?
            .cached_store()?
            .save_ui_state(bytes, rng)
    }
}

/// Cached original negatives only, with no Physical/Current/Store/slot/cold
/// evidence. Its caller owns an independent finite metadata invoice.
pub(in crate::store::critical_namespaces::native_registry) struct AuthenticationNegative {
    gate: lifecycle_admission::OpenNegative,
    mount: crate::store::chat_admission::CachedPhysicalMountNegative,
}
impl AuthenticationNegative {
    pub(in crate::store::critical_namespaces::native_registry) fn is_live_cached(&self) -> bool {
        self.gate.is_live_cached() && self.mount.is_live_cached()
    }
}
impl MetadataWork {
    /// Outside every guard, after original-pool validation. Capture the SAME
    /// mount while this work still retains the physical Store. No file reads.
    pub(in crate::store::critical_namespaces::native_registry) fn authentication_negative(
        &self,
    ) -> Result<AuthenticationNegative, AppError> {
        Ok(AuthenticationNegative {
            gate: self.physical.handler.observe_open(),
            mount: self.physical.current()?.capture_cached_physical_negative(),
        })
    }
    pub(in crate::store::critical_namespaces::native_registry) fn is_live_cached(&self) -> bool {
        self.physical.open_for_custody()
    }
    pub(in crate::store::critical_namespaces::native_registry) fn verify_native_passphrase(
        &self,
        secret: &[u8],
    ) -> Result<(), AppError> {
        self.physical.verify_native_passphrase(secret)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn row_count_data(
        &mut self,
    ) -> Result<usize, AppError> {
        self.physical.row_count_data()
    }
    pub(in crate::store::critical_namespaces::native_registry) fn refresh_legacy_invite_data(
        &mut self,
        request: &refresh::Request,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(rename::Metadata, Option<cell::CompletedDebt>), AppError> {
        self.physical.refresh_legacy_invite_data(request, rng)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn rename_metadata_data(
        &mut self,
        request: &rename::Request,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(rename::Metadata, Option<cell::CompletedDebt>), AppError> {
        self.physical.rename_metadata_data(request, rng)
    }
}

#[cfg(test)]
mod tests;
