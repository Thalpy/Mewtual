//! Finite original physical3 custody, separate from affine admission. No raw
//! Current/Store, owned index, SDK entry or native command credential escapes.
use super::*;
mod work;
use crate::store::critical_namespaces::native_registry::lifecycle::{
    cell, founder, refresh, rename,
};
use std::sync::Mutex;
pub(in crate::store::critical_namespaces::native_registry) use work::WorkDrain;
use work::{Slot, SlotAck};

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

/// Exactly one prepaid fixed capsule. It is not Clone and contains no affine
/// close owner. Destroy the WHOLE capsule outside native/cell/catalog guards:
/// its final physical Arc can run real file and Current destructors.
pub(in crate::store::critical_namespaces::native_registry) struct MetadataWork {
    // FIRST: actual last-Arc physical destruction finishes before slot release.
    physical: Arc<Physical>,
    _ack: SlotAck,
}

/// Long-lived original source custody, independent of the single metadata
/// slot. Its caller separately prepays and bounds the source collection and
/// each runtime. This is not a second affine close owner or a Current getter.
/// Keep the entire value outside native/registry/operation guards on drop.
pub(in crate::store::critical_namespaces::native_registry) struct SourceWork {
    physical: Arc<Physical>,
}

/// Opaque prepared AEAD key; never exposes its key bytes. All fallible
/// original Current/file proofs precede construction, so a local refusal is
/// retryable before the owning job marks the actual seal attempt entered.
pub(in crate::store::critical_namespaces::native_registry) struct NetworkSeal {
    key: Zeroizing<[u8; 32]>,
}
#[cfg(test)]
thread_local! { static NETWORK_SEAL_REFUSAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
impl NetworkSeal {
    pub(in crate::store::critical_namespaces::native_registry) fn seal(
        &self,
        plain: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        Ok(Zeroizing::new(crate::store::frame(&catcoms_crypto::seal(
            &self.key, plain, rng,
        )?)))
    }
}

pub(super) const METADATA: usize = 8
    * (std::mem::size_of::<Physical>() + std::mem::size_of::<MetadataWork>()
    + std::mem::size_of::<SlotAck>() + std::mem::size_of::<Slot>()
    // Physical and both slot Arc controls, plus Option control metadata.
    + 6 * std::mem::size_of::<usize>())
    + work::METADATA
    + 512;

impl Physical {
    /// Allocate BEFORE original Current extraction, with the original upgrade
    /// still wholly armed. No physical allocation is exposed until install().
    pub(super) fn allocate(handler: lifecycle_admission::Handler) -> Arc<Self> {
        Arc::new(Self {
            cell: Mutex::default(),
            current: None,
            slot: Arc::new(Slot::new()),
            ui_slot: Arc::new(Slot::new()),
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
        if !self.open_for_custody() {
            return None;
        }
        #[cfg(test)]
        tests::before_work_slot(false);
        if !self.slot.enter() {
            return None;
        }
        #[cfg(test)]
        tests::work_slot_claimed();
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
        #[cfg(test)]
        tests::after_work_slot(false);
        // After winning ACTIVE, return WHOLE custody even if closure raced.
        // The caller rejects stale negatives outside its native guard. A late
        // None here would drop SlotAck and synchronously wake close under it.
        Some(value)
    }
    /// The UI sidecar is independent of index WAL/claim state. Reusing the
    /// index slot would let an uncertain metadata transaction starve drafts.
    pub(super) fn capture_ui(self: &Arc<Self>) -> Option<UiWork> {
        if !self.open_for_custody() {
            return None;
        }
        #[cfg(test)]
        tests::before_work_slot(true);
        if !self.ui_slot.enter() {
            return None;
        }
        #[cfg(test)]
        tests::work_slot_claimed();
        let ack = SlotAck {
            original: self.ui_slot.clone(),
            _payment: self.handler.payment_tail(),
        };
        let work = UiWork {
            physical: self.clone(),
            _ack: ack,
        };
        #[cfg(test)]
        tests::after_work_slot(true);
        // Same owning late-refusal rule as MetadataWork above: read/save
        // recheck this original outside guards before performing any IO.
        Some(work)
    }

    pub(super) fn capture_source(self: &Arc<Self>) -> Option<SourceWork> {
        if !self.open_for_custody() {
            return None;
        }
        let work = SourceWork {
            physical: self.clone(),
        };
        work.is_live_cached().then_some(work)
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
        self.slot.close();
        self.ui_slot.close();
    }
    /// Original metadata/UI controls only. The affine owning caller has
    /// already closed its index; no Physical or cold evidence escapes.
    pub(super) fn observe_closed_work(&self) -> WorkDrain {
        self.close_admission();
        WorkDrain::new(self)
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

impl SourceWork {
    /// Configuration is negative only, but an incompatible DATA-only mount
    /// must refuse BEFORE retaining an authoring entry or generating any key.
    pub(in crate::store::critical_namespaces::native_registry) fn require_native_founder_configuration(
        &self,
    ) -> Result<(), AppError> {
        let store = self.physical.current()?.checked_store()?;
        let configured = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?;
        if configured.pending.execution.is_none() || store.member_outbox_policy().is_none() {
            return Err(refused());
        }
        store.validate_mounted_member_outbox_layout()
    }
    /// Actual original mounted context, never a renderer/ciphertext's claim.
    pub(in crate::store::critical_namespaces::native_registry) fn native_network_context(
        &self,
    ) -> Result<([u8; 32], [u8; 32]), AppError> {
        let store = self.physical.current()?.checked_store()?;
        let guard = store.current_layout.lock().map_err(|_| refused())?;
        let layout = guard.as_ref().ok_or_else(refused)?.layout;
        Ok((layout.vault, layout.root))
    }
    pub(in crate::store::critical_namespaces::native_registry) fn prepare_native_network_seal(
        &self,
    ) -> Result<NetworkSeal, AppError> {
        #[cfg(test)]
        if NETWORK_SEAL_REFUSAL.with(|slot| slot.replace(false)) {
            return Err(AppError::Io(
                "injected local network seal preparation refusal".into(),
            ));
        }
        let store = self.physical.current()?.checked_store()?;
        Ok(NetworkSeal {
            key: Zeroizing::new(store.keys.db_key()?),
        })
    }
    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn refuse_next_network_seal_for_test(
        &self,
    ) {
        NETWORK_SEAL_REFUSAL.with(|slot| {
            assert!(!slot.get());
            slot.set(true);
        });
    }
    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn native_network_plain_for_test(
        &self,
        frame: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let store = self.physical.current()?.checked_store()?;
        let key = Zeroizing::new(store.keys.db_key()?);
        Ok(Zeroizing::new(catcoms_crypto::unseal(
            &key,
            &crate::store::unframe(frame)?,
        )?))
    }
    pub(in crate::store::critical_namespaces::native_registry) fn birth_native_network(
        &self,
        pending: &founder::Pending,
        rng: &mut impl CryptoRngCore,
    ) -> Result<crate::store::NativeCreationStatusData, AppError> {
        let work = self.metadata().ok_or_else(refused)?;
        let (status, debt) = work.physical.birth_founder_data(pending, rng)?;
        drop(debt); // Actual IO/claim retirement OUTSIDE all cell/catalog writers.
        Ok(status)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn activate_native_founder(
        &self,
        request: &founder::Activate,
        network: &Arc<crate::store::persistence::native_registry::network::NetworkDependency>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<crate::store::NativeCreationStatusData, AppError> {
        let work = self.metadata().ok_or_else(refused)?;
        let (status, debt) = cell::activate_native_founder(
            &work.physical.cell,
            work.physical.current()?,
            &work.physical.handler,
            request,
            network,
            rng,
        )?;
        drop(debt);
        Ok(status)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn prepare_native_network_dependency(
        &self,
        io: &mut crate::store::persistence::native_registry::network::NetworkIo,
        expected: crate::store::persistence::native_registry::network::NetworkExpected,
        frame: &[u8],
        payment: Arc<crate::store::critical_namespaces::native_registry::operational::sources::RuntimeMetadata>,
    ) -> Result<Arc<crate::store::persistence::native_registry::network::NetworkDependency>, AppError>
    {
        io.dependency(
            self.physical.current()?.checked_store()?,
            expected,
            frame,
            payment,
        )
    }
    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn activate_data_only_for_test(
        &self,
        request: &founder::Activate,
        rng: &mut impl CryptoRngCore,
    ) -> Result<crate::store::NativeCreationStatusData, AppError> {
        let work = self.metadata().ok_or_else(refused)?;
        let (status, debt) = work.physical.activate_founder_data(request, rng)?;
        drop(debt);
        Ok(status)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn prepare_native_network_io(
        &self,
        row: u64,
        frame: &[u8],
    ) -> Result<crate::store::persistence::native_registry::network::NetworkIo, AppError> {
        crate::store::persistence::native_registry::network::NetworkIo::prepare(
            self.physical.current()?.checked_store()?,
            row,
            frame,
        )
    }
    pub(in crate::store::critical_namespaces::native_registry) fn cover_native_network_io(
        &self,
        io: &mut crate::store::persistence::native_registry::network::NetworkIo,
        frame: &[u8],
    ) -> Result<(), AppError> {
        io.cover(self.physical.current()?.checked_store()?, frame)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn lookup_creation(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<Option<crate::store::NativeCreationStatusData>, AppError> {
        let work = self.metadata().ok_or_else(refused)?;
        work.physical.lookup_creation_data(invocation, request)
    }
    /// The concrete source factory reserves its runtime invoice BEFORE asking
    /// for this additional original pin. There is no public Clone/getter and
    /// no second physical mount or affine admission owner is constructed.
    pub(in crate::store::critical_namespaces::native_registry) fn for_paid_runtime(
        &self,
        _metadata: &crate::store::critical_namespaces::native_registry::operational::sources::RuntimeMetadata,
    ) -> Option<Self> {
        self.physical.capture_source()
    }
    /// Exercise the actual whole Host's independently retained member End.
    /// This is deliberately absent from production Source/Current surfaces.
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store::critical_namespaces::native_registry) fn hold_native_member_end_for_test<
        T: catcoms_rt::MeshTransport,
        R: CryptoRngCore,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
    ) -> Result<Box<dyn Send>, AppError> {
        driver.hold_member_save_source_for_test(self.physical.current()?)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn is_live_cached(&self) -> bool {
        self.physical.open_for_custody()
    }

    /// Capture short-lived index work only while actually advancing Birth or
    /// Activate. A retained founder never holds this slot across its lifetime.
    pub(in crate::store::critical_namespaces::native_registry) fn metadata(
        &self,
    ) -> Option<MetadataWork> {
        self.physical.capture()
    }

    /// Concrete original-founder route only. Inputs are already owned by the
    /// checked Root producer; its device cut must precede device generation.
    /// Failure returns those same inputs. No library-origin fallback exists.
    pub(in crate::store::critical_namespaces::native_registry) fn prepare_native_founder<T, R>(
        &self,
        invocation: [u8; 32],
        name: &str,
        policy: crate::store::FreshV2TextFounderPolicy,
        inputs: crate::store::ArchiveFounderInputs<T, R>,
        original: catcoms_rt::NativeChatSourceObservation,
    ) -> Result<
        crate::store::FreshV2TextFounderBegin<T, R>,
        crate::store::ReturnedArchiveFounderInputs<T, R>,
    >
    where
        T: catcoms_rt::MeshTransport,
        R: CryptoRngCore,
    {
        // The retained Physical always contains this same Current. On closure
        // the existing concrete factory refuses and returns untouched inputs;
        // it does not reopen files or choose a different source grammar.
        self.physical.current.as_ref().expect("installed original physical").prepare_fresh_v2_text_founder_with_origin(
            invocation, name, policy, inputs,
            crate::store::critical_namespaces::archive_bootstrap::founder::FounderOrigin::NativeRequired(original),
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn retry_native_founder<T, R>(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        rng: &mut impl CryptoRngCore,
        keys: &mut crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
    ) -> Result<(), AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: CryptoRngCore,
    {
        driver.retry_with_access(
            self.physical.current()?,
            rng,
            crate::store::critical_namespaces::archive_bootstrap::founder::FounderAccess::Native(
                keys,
            ),
        )
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
