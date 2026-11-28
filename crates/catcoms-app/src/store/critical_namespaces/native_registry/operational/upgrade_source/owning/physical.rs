//! Finite original physical3 custody, separate from affine admission. No raw
//! Current/Store, owned index, SDK entry or native command credential escapes.
use super::*;
mod work;
use crate::store::critical_namespaces::native_registry::lifecycle::{
    cell, founder, network, refresh, rename,
};
use crate::store::critical_namespaces::review_hold_namespace::declaration::ReviewDeclarationKind;
use std::sync::Mutex;
pub(in crate::store::critical_namespaces::native_registry) use work::WorkDrain;
use work::{Slot, SlotAck};

pub(super) struct Physical {
    // Actual poisoned cell and its pending files/buffers retire before Current.
    cell: Mutex<cell::Cell>,
    // Original files kept through the common cold opener until founder
    // recovery rechecks the authenticated selected row.
    cold_networks: Vec<Arc<crate::store::persistence::native_registry::network::NetworkDependency>>,
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
    pub(super) fn joined_pending_store(&mut self) -> Result<&ServerStore, AppError> {
        self.cell.get_mut().map_err(|_| refused())?.require_idle()?;
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        if !self.handler.uses_joined_pin_layout() {
            return Err(refused());
        }
        self.current()?.checked_store()
    }

    pub(super) fn joined_request_data<'a>(
        &self,
        pending: &'a crate::store::persistence::governance_layout::joined::JoinedPendingIo,
    ) -> Result<&'a [u8], AppError> {
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        pending.request_data(self.current()?.checked_store()?)
    }

    pub(super) fn attach_recovered_joined_node<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &mut self,
        inputs: &mut Option<
            crate::store::persistence::governance_layout::joined::JoinedRuntimeInputs<T, R>,
        >,
        output: &mut Option<catcoms_sync::ClosedV2JoinedNode<T, R>>,
    ) -> Result<Option<catcoms_sync::SelectedRejoinRemoval>, AppError> {
        self.cell.get_mut().map_err(|_| refused())?.require_idle()?;
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        if !self.handler.uses_joined_pin_layout() {
            return Err(refused());
        }
        self.current
            .as_mut()
            .ok_or_else(refused)?
            .attach_recovered_joined_node(inputs, output)
    }

    pub(super) fn cover_recovered_member_add<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &self,
        node: &mut catcoms_sync::ClosedV2JoinedNode<T, R>,
        delivery: &[u8],
        caller: &catcoms_rt::RequestCancellation,
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<(), AppError> {
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        if !self.handler.uses_joined_pin_layout() {
            return Err(refused());
        }
        let retired = self
            .current()?
            .checked_store()?
            .cover_recovered_member_add(node, delivery, caller, rng)?;
        drop(retired);
        self.handler.require()
    }

    pub(super) fn joined_exchange_resources(
        &self,
        byte_count: usize,
    ) -> Result<(catcoms_rt::RequestCancellation, CatalogReadReservation), AppError> {
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        let store = self.current()?.checked_store()?;
        let policy = store
            .governance_layout_policy
            .as_ref()
            .ok_or_else(refused)?;
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(byte_count)?;
        Ok((store.chat_mount.observer(), fee))
    }

    pub(super) fn warm_join_outcome_data(
        &self,
        pending: &crate::store::persistence::governance_layout::joined::JoinedPendingIo,
    ) -> Result<Option<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64)>, AppError>
    {
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        pending.warm_outcome_data(self.current()?.checked_store()?)
    }

    pub(super) fn recovered_join_outcome_data(
        &self,
    ) -> Result<Option<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64)>, AppError>
    {
        self.current()?
            .checked_store()?
            .recovered_join_outcome_data()
    }

    pub(super) fn recovered_join_transport_data(
        &self,
    ) -> Result<Option<(catcoms_rt::PeerId, u16)>, AppError> {
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        self.current()?
            .checked_store()?
            .recovered_join_transport_data()
    }

    pub(super) fn recovered_join_registration_data(
        &self,
    ) -> Result<Option<crate::store::NativeTextJoinedRegistrationData>, AppError> {
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        self.current()?
            .checked_store()?
            .recovered_join_registration_data()
    }

    pub(super) fn select_review_storage_before_founder_startup(
        &self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<bool, AppError> {
        if !self.handler.accepts_rejoin_startup() {
            return Err(refused());
        }
        let store = self.current()?.checked_store()?;
        if !store.has_mounted_shared_history()? || store.has_declared_review_decision()? {
            return Ok(false);
        }
        let kind = if store.has_mounted_review_hold()? {
            ReviewDeclarationKind::Decision
        } else {
            ReviewDeclarationKind::Hold
        };
        // The caller retains the original unpublished native owner. No
        // listener or native command can observe a half-selected catalog.
        let Some(founder) = self.recovered_cold_founder()? else {
            return Ok(false);
        };
        self.require_recovered_cold_founder(&founder)?;
        let mut declaration = store.prepare_founder_review_declaration(kind, rng)?;
        declaration.select(store)?;
        Ok(true)
    }

    pub(super) fn select_review_storage_before_joined_startup(
        &self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<bool, AppError> {
        if !self.handler.accepts_rejoin_startup() {
            return Err(refused());
        }
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        let store = self.current()?.checked_store()?;
        store.select_review_receiver_before_native_startup(rng)
    }

    pub(super) fn has_mounted_review_hold(&self) -> Result<bool, AppError> {
        self.current()?.checked_store()?.has_mounted_review_hold()
    }

    pub(super) fn recovered_cold_founder(
        &self,
    ) -> Result<Option<network::ColdFounderSelection>, AppError> {
        // A registry listing or another metadata reader can temporarily
        // hold the cell. Keep this nonblocking and distinguish contention
        // from poison or an incomplete transaction on disk.
        let cell = self.cell.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => AppError::ChatV2RegistryBusy,
            std::sync::TryLockError::Poisoned(_) => refused(),
        })?;
        cell.require_idle()?;
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        let store = self.current()?.checked_store()?;
        let inventory =
            UpgradeInventory::capture(store, self.handler.debit(), self.handler.pool())?;
        if inventory.seed().is_some() || inventory.index_stage().is_some() {
            return Err(refused());
        }
        let original = network::selected_cold_founder(store, &inventory, &self.cold_networks)?;
        if original.is_some() {
            store.require_empty_mounted_joined_pins()?;
        }
        inventory.require(store)?;
        Ok(original)
    }

    pub(super) fn capture_recovered_founder_source(
        &self,
        original: &network::ColdFounderSelection,
    ) -> Result<crate::store::RetainedBootstrapSourceData, AppError> {
        self.require_recovered_cold_founder(original)?;
        let store = self.current()?.checked_store()?;
        let source = store
            .read_saved_bootstrap_source_data(&original.binding.group, original.binding.signer)?;
        original.require_saved_source_data(&source)?;
        self.require_recovered_cold_founder(original)?;
        source.validate_original(store)?;
        Ok(source)
    }

    pub(super) fn require_recovered_cold_founder(
        &self,
        original: &network::ColdFounderSelection,
    ) -> Result<(), AppError> {
        let current = self.recovered_cold_founder()?.ok_or_else(refused)?;
        if current.status != original.status
            || current.invocation != original.invocation
            || current.request != original.request
            || current.binding != original.binding
            || !Arc::ptr_eq(&current.network, &original.network)
        {
            return Err(refused());
        }
        Ok(())
    }

    pub(super) fn require_empty_mounted_joined_pins(&self) -> Result<(), AppError> {
        self.require_mounted_joined_without_row(false)
    }

    pub(super) fn require_mounted_joined_without_row(
        &self,
        allow_native_pending: bool,
    ) -> Result<(), AppError> {
        let cell = self.cell.try_lock().map_err(|_| refused())?;
        cell.require_idle()?;
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        self.current()?
            .checked_store()?
            .require_mounted_joined_pin_without_row(allow_native_pending)
    }

    pub(super) fn require_registered_native_join(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
    ) -> Result<(), AppError> {
        let store = self.current()?.checked_store()?;
        if store
            .with_installed_rejoin(|original| original.row(store, data).map(drop))?
            .is_some()
        {
            return Ok(());
        }
        self.with_registered_native_join(data, |_, _| Ok(()))
    }

    pub(super) fn has_recovered_terminal_candidate(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
    ) -> Result<bool, AppError> {
        self.with_registered_native_join(data, |store, _| {
            store.has_recovered_terminal_candidate(data)
        })
    }

    pub(super) fn require_recovered_native_join_removal(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
        removal: catcoms_sync::SelectedRejoinRemoval,
    ) -> Result<(), AppError> {
        self.with_registered_native_join(data, |store, _| {
            store.require_recovered_joined_removal(data, removal)
        })
    }

    pub(super) fn prepare_retired_rejoin_catalog(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
        removal: catcoms_sync::SelectedRejoinRemoval,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<
        crate::store::persistence::governance_layout::joined::rejoin::RejoinCatalogPreparation,
        AppError,
    > {
        let cell = self.cell.try_lock().map_err(|_| refused())?;
        cell.require_idle()?;
        self.handler.require_idle_cached()?;
        if !self.handler.uses_joined_pin_layout() {
            return Err(refused());
        }
        let store = self.current()?.checked_store()?;
        // The handler funds the original classification. When its checked
        // fixed leaves survive the call, retain a separate scoped invoice.
        let pool = &store
            .governance_layout_policy
            .as_ref()
            .ok_or_else(refused)?
            .current
            .archive
            .pending
            .pool;
        let inventory =
            UpgradeInventory::capture(store, self.handler.debit(), self.handler.pool())?;
        super::super::super::super::lifecycle::preparation::require_registered_native_join(
            store, &inventory, data,
        )?;
        if store.recovered_join_registration_data()? != Some(data) {
            return Err(refused());
        }
        use crate::store::persistence::governance_layout::joined::rejoin::RetainedNativeRejoinInventory;
        let original = RetainedNativeRejoinInventory::retain(inventory, pool)?;
        store.prepare_retired_rejoin_catalog(data, removal, original, rng)
    }

    pub(super) fn require_fresh_rejoin_invite(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
        removal: catcoms_sync::SelectedRejoinRemoval,
        invite: &catcoms_replication::chat::join_bindings::SignedInviteBindingData,
        now_ms: u64,
    ) -> Result<(), AppError> {
        self.with_registered_native_join(data, |store, _| {
            store.require_fresh_rejoin_invite(data, removal, invite, now_ms)
        })
    }

    pub(super) fn require_original_rejoin_catalog(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
        removal: catcoms_sync::SelectedRejoinRemoval,
        prepared: &crate::store::persistence::governance_layout::joined::rejoin::RejoinCatalogPreparation,
    ) -> Result<(), AppError> {
        let cell = self.cell.try_lock().map_err(|_| refused())?;
        cell.require_idle()?;
        self.handler.require_idle_cached()?;
        let store = self.current()?.checked_store()?;
        store.require_recovered_joined_removal(data, removal)?;
        prepared.require_original(store)
    }

    /// Classification only. Subsequent A1 recovery still rechecks the
    /// exact selected physical files before exposing anything.
    pub(super) fn rejoin_catalog_selected(&self) -> Result<bool, AppError> {
        let store = self.current()?.checked_store()?;
        let catalog = store
            .pending_join_catalog
            .try_lock()
            .map_err(|_| refused())?;
        Ok(catalog
            .as_ref()
            .ok_or_else(refused)?
            .current
            .rejoin_payload_digest
            .is_some())
    }

    #[cfg(any(test, feature = "native-chat-test-hooks"))]
    pub(super) fn stage_retired_rejoin_catalog(
        &self,
        prepared: &mut crate::store::persistence::governance_layout::joined::rejoin::RejoinCatalogPreparation,
    ) -> Result<(), AppError> {
        let cell = self.cell.try_lock().map_err(|_| refused())?;
        cell.require_idle()?;
        self.handler.require_idle_cached()?;
        prepared.stage_catalog(self.current()?.checked_store()?, &|| Ok(()))
    }

    pub(super) fn select_retired_rejoin_catalog(
        &self,
        prepared: &mut crate::store::persistence::governance_layout::joined::rejoin::RejoinCatalogPreparation,
        admit: &impl Fn() -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let cell = self.cell.try_lock().map_err(|_| refused())?;
        cell.require_idle()?;
        self.handler.require_idle_cached()?;
        prepared.select_catalog(self.current()?.checked_store()?, admit)
    }

    pub(super) fn cover_selected_rejoin_catalog(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
        removal: catcoms_sync::SelectedRejoinRemoval,
    ) -> Result<[u8; 32], AppError> {
        if !self.rejoin_catalog_selected()? {
            return Err(refused());
        }
        self.with_registered_native_join(data, |store, inventory| {
            store.cover_selected_rejoin_catalog(data, removal, inventory)
        })
    }

    pub(super) fn prepare_rejoin_transport(
        &self,
        original: (
            crate::store::NativeTextJoinedRegistrationData,
            catcoms_sync::SelectedRejoinRemoval,
        ),
        warm: &mut Option<
            crate::store::persistence::governance_layout::joined::rejoin::RejoinPendingOwner,
        >,
        invitation: (&crate::store::NativeTextInvitePacketData<'_>, [u8; 32]),
    ) -> Result<
        crate::store::persistence::governance_layout::joined::rejoin::PreparedRejoinTransport,
        AppError,
    > {
        if !self.rejoin_catalog_selected()? {
            return Err(refused());
        }
        self.with_registered_native_join(original.0, |store, inventory| {
            store.prepare_rejoin_transport(inventory, original, warm, invitation)
        })
    }

    pub(super) fn cover_rejoin_pending(
        &self,
        original: (
            crate::store::NativeTextJoinedRegistrationData,
            catcoms_sync::SelectedRejoinRemoval,
        ),
        warm: &mut Option<
            crate::store::persistence::governance_layout::joined::rejoin::RejoinPendingOwner,
        >,
        request: (&crate::store::NativeTextInvitePacketData<'_>, [u8; 32], u64),
        native: crate::store::persistence::governance_layout::joined::rejoin::RejoinKeyEntry<'_>,
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<crate::store::NativeTextRejoinPendingReceipt, AppError> {
        if !self.rejoin_catalog_selected()? {
            return Err(refused());
        }
        self.with_registered_native_join(original.0, |store, inventory| {
            store.cover_rejoin_pending(
                inventory,
                original,
                warm,
                request,
                native,
                rng,
                require_current,
            )
        })
    }

    pub(super) fn cover_rejoin_received(
        &self,
        original: (
            crate::store::NativeTextJoinedRegistrationData,
            catcoms_sync::SelectedRejoinRemoval,
        ),
        funds: (&RejoinPendingOwner, &mut Option<RejoinReceivedOwner>),
        invite: (&crate::store::NativeTextInvitePacketData<'_>, [u8; 32]),
        input: RejoinReceivedInput<'_>,
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<Option<crate::store::NativeTextRejoinReceivedReceipt>, AppError> {
        if !self.rejoin_catalog_selected()? {
            return Err(refused());
        }
        // The Received writer performs its final inventory and exact file
        // proof here under the same cell. Do not do later physical checks
        // after it has established the original reply receipt.
        self.with_registered_native_join_inventory(original.0, |store, inventory| {
            store.cover_rejoin_received(
                inventory,
                original,
                funds,
                invite,
                input,
                rng,
                require_current,
            )
        })
    }

    pub(super) fn cover_rejoin_welcome(
        &self,
        original: (
            crate::store::NativeTextJoinedRegistrationData,
            catcoms_sync::SelectedRejoinRemoval,
        ),
        sources: RejoinWelcomeOwners<'_>,
        invite: (&crate::store::NativeTextInvitePacketData<'_>, [u8; 32]),
        authority: (
            Option<RejoinWelcomeKeyEntry<'_>>,
            &catcoms_rt::RequestCancellation,
        ),
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<crate::store::NativeTextRejoinWelcomeStatus, AppError> {
        if !self.rejoin_catalog_selected()? {
            return Err(refused());
        }
        self.with_registered_native_join_inventory(original.0, |store, inventory| {
            store.cover_rejoin_welcome(
                inventory,
                original,
                sources,
                invite,
                authority,
                rng,
                require_current,
            )
        })
    }

    pub(super) fn cover_rejoin_prefix(
        &self,
        original: (
            crate::store::NativeTextJoinedRegistrationData,
            catcoms_sync::SelectedRejoinRemoval,
        ),
        sources: RejoinPrefixInputs<'_>,
        invite: (&crate::store::NativeTextInvitePacketData<'_>, [u8; 32]),
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<crate::store::NativeTextRejoinPrefixReceipt, AppError> {
        if !self.rejoin_catalog_selected()? {
            return Err(refused());
        }
        self.with_registered_native_join_inventory(original.0, |store, inventory| {
            store.cover_rejoin_prefix(inventory, original, sources, invite, rng, require_current)
        })
    }

    pub(super) fn cover_rejoin_index(
        &self,
        original: (
            crate::store::NativeTextJoinedRegistrationData,
            catcoms_sync::SelectedRejoinRemoval,
        ),
        sources: RejoinIndexInputs<'_>,
        invite: (&crate::store::NativeTextInvitePacketData<'_>, [u8; 32]),
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<crate::store::NativeTextRejoinIndexReceipt, AppError> {
        if !self.rejoin_catalog_selected()? {
            return Err(refused());
        }
        self.with_registered_native_join_inventory(original.0, |store, inventory| {
            store.cover_rejoin_index(inventory, original, sources, invite, rng, require_current)
        })
    }

    pub(super) fn recover_rejoin_index(
        &self,
        original: (
            crate::store::NativeTextJoinedRegistrationData,
            catcoms_sync::SelectedRejoinRemoval,
        ),
        sources: RejoinIndexRecovery<'_>,
        caller: &catcoms_rt::RequestCancellation,
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<crate::store::NativeTextRejoinIndexReceipt, AppError> {
        if !self.rejoin_catalog_selected()? {
            return Err(refused());
        }
        self.with_registered_native_join_inventory(original.0, |store, inventory| {
            store.recover_rejoin_index(inventory, original, sources, caller, rng, require_current)
        })
    }

    fn with_registered_native_join<T>(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
        require: impl FnOnce(&ServerStore, &UpgradeInventory) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.with_registered_native_join_inventory(data, |store, inventory| {
            let output = require(store, inventory)?;
            inventory.require(store)?;
            Ok(output)
        })
    }

    fn with_registered_native_join_inventory<T>(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
        complete_under_cell: impl FnOnce(&ServerStore, &UpgradeInventory) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.with_owned_join_inventory(data, |store, inventory| {
            complete_under_cell(store, &inventory)
        })
    }

    fn with_owned_join_inventory<T>(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
        complete_under_cell: impl FnOnce(&ServerStore, UpgradeInventory) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let cell = self.cell.try_lock().map_err(|_| refused())?;
        cell.require_idle()?;
        self.handler.require_idle_cached()?;
        if !self.handler.uses_joined_pin_layout() {
            return Err(refused());
        }
        let store = self.current()?.checked_store()?;
        let inventory =
            UpgradeInventory::capture(store, self.handler.debit(), self.handler.pool())?;
        super::super::super::super::lifecycle::preparation::require_registered_native_join(
            store, &inventory, data,
        )?;
        if store.recovered_join_registration_data()? != Some(data) {
            return Err(refused());
        }
        complete_under_cell(store, inventory)
    }

    pub(super) fn has_rejoin_index(&self) -> Result<bool, AppError> {
        self.current()?.checked_store()?.has_rejoin_index()
    }

    pub(super) fn has_rejoin_evidence(&self) -> Result<bool, AppError> {
        self.current()?.checked_store()?.has_rejoin_evidence()
    }

    pub(super) fn recover_installed_rejoin(
        &self,
        registration: crate::store::NativeTextJoinedRegistrationData,
        removal: catcoms_sync::SelectedRejoinRemoval,
        negative: &catcoms_rt::RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<crate::store::NativeTextJoinedRegistrationData>, AppError> {
        if !self.rejoin_catalog_selected()? {
            return Err(refused());
        }
        self.with_owned_join_inventory(registration, |store, inventory| {
            if !store.has_rejoin_index()? {
                return Ok(None);
            }
            store
                .recover_installed_rejoin(inventory, (registration, removal), negative, rng)
                .map(Some)
        })
    }

    pub(super) fn attach_recovered_rejoin<R: CryptoRngCore>(
        &self,
        inputs: &mut Option<JoinedRuntimeInputs<catcoms_net::MeshService, R>>,
        output: &mut Option<catcoms_sync::ClosedV2JoinedNode<catcoms_net::MeshService, R>>,
    ) -> Result<(), AppError> {
        let store = self.current()?.checked_store()?;
        store
            .with_installed_rejoin_mut(|original| original.attach(store, inputs, output))?
            .ok_or_else(refused)
    }

    pub(super) fn prepare_recovered_native_join_tcp(
        &self,
        data: &crate::store::NativeTextJoinedRegistrationData,
    ) -> Result<(Zeroizing<[u8; 32]>, catcoms_net::NativeTcpPreparation), AppError> {
        use super::super::super::governed::JoinFramePayment;
        let store = self.current()?.checked_store()?;
        let (mount, fee) =
            self.joined_exchange_resources(catcoms_net::NATIVE_TCP_LIFETIME_BYTES)?;
        let seed = match store.with_installed_rejoin(|original| {
            original.row(store, *data)?;
            original.prepare_tcp(store)
        })? {
            Some(seed) => seed,
            None => store.recovered_join_transport_seed(data)?,
        };
        if mount.is_cancelled() {
            return Err(refused());
        }
        // The producer is still the unique pre-exposure physical owner. The
        // actor retains exclusion without cloning its mutable startup owner.
        let lease = store._session.retain_for_physical_work();
        Ok((
            seed,
            catcoms_net::NativeTcpPreparation::new(lease, Arc::new(JoinFramePayment { _fee: fee })),
        ))
    }

    pub(super) fn recovered_founder_reservation(
        &self,
        original: &network::ColdFounderSelection,
    ) -> Result<
        crate::store::persistence::native_registry::network::NativeFounderNetworkReservation,
        AppError,
    > {
        self.require_recovered_cold_founder(original)?;
        original
            .network
            .restore_reservation(self.current()?.checked_store()?)
    }

    pub(super) fn prepare_recovered_native_founder_tcp(
        &self,
        original: &network::ColdFounderSelection,
        source: &crate::store::RetainedBootstrapSourceData,
    ) -> Result<(Zeroizing<[u8; 32]>, catcoms_net::NativeTcpPreparation), AppError> {
        use super::super::super::governed::JoinFramePayment;
        let store = self.current()?.checked_store()?;
        let (mount, fee) =
            self.joined_exchange_resources(catcoms_net::NATIVE_TCP_LIFETIME_BYTES)?;
        let reservation = self.recovered_founder_reservation(original)?;
        source.validate_original(store)?;
        if mount.is_cancelled() {
            return Err(refused());
        }
        let lease = store._session.retain_for_physical_work();
        Ok((
            reservation.seed,
            catcoms_net::NativeTcpPreparation::new(
                (lease, original.network.clone()),
                Arc::new(JoinFramePayment { _fee: fee }),
            ),
        ))
    }

    pub(super) fn require_recovered_founder_source(
        &self,
        original: &network::ColdFounderSelection,
        source: &crate::store::RetainedBootstrapSourceData,
    ) -> Result<(), AppError> {
        self.require_recovered_cold_founder(original)?;
        source.validate_original(self.current()?.checked_store()?)
    }

    pub(super) fn restore_recovered_founder_add(
        &self,
        original: &network::ColdFounderSelection,
        source: &crate::store::RetainedBootstrapSourceData,
        captured: &mut Option<Box<crate::store::persistence::v2_text::HeldColdFounderAdd>>,
        sdk: &mut Option<catcoms_sync::owner_v2::PreparedV2FounderRecoveredAdd>,
    ) -> Result<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64), AppError> {
        use crate::store::persistence::v2_text::HeldColdFounderAdd;
        self.require_recovered_founder_source(original, source)?;
        let store = self.current()?.checked_store()?;
        if captured.is_none() {
            *captured = Some(Box::new(HeldColdFounderAdd::capture(
                store,
                source,
                original.binding.pin,
            )?));
        }
        let captured = captured.as_deref_mut().ok_or_else(refused)?;
        if sdk.is_none() {
            *sdk = Some(captured.restore_sdk(store, source)?);
        }
        let sdk = sdk.as_mut().ok_or_else(refused)?;
        let selected = captured.restore_core(store, source, sdk.fingerprint_data())?;
        sdk.bind_selected_member(captured.member_parts()?.0)?;
        self.require_recovered_founder_source(original, source)?;
        Ok(selected)
    }

    pub(super) fn with_recovered_founder_store<T>(
        &self,
        original: &network::ColdFounderSelection,
        source: &crate::store::RetainedBootstrapSourceData,
        read: impl FnOnce(&ServerStore) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.require_recovered_founder_source(original, source)?;
        let result = read(self.current()?.checked_store()?)?;
        self.require_recovered_founder_source(original, source)?;
        Ok(result)
    }

    pub(super) fn copy_selected_governing_proof(
        &self,
        pending: Option<&crate::store::persistence::governance_layout::joined::JoinedPendingIo>,
        headers: (
            catcoms_replication::chat::owner_header::OwnerHeaderId,
            catcoms_replication::chat::owner_header::OwnerHeaderId,
        ),
        output: &mut Vec<u8>,
    ) -> Result<(), AppError> {
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        let store = self.current()?.checked_store()?;
        match pending {
            Some(pending) => pending.copy_selected_governing_proof(store, headers, output),
            None => store.copy_recovered_governing_proof(headers, output),
        }
    }

    pub(super) fn copy_recovered_join_request(&self, output: &mut Vec<u8>) -> Result<(), AppError> {
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        self.current()?
            .checked_store()?
            .copy_recovered_join_request(output)
    }

    pub(super) fn validate_recovered_join_delivery(
        &self,
        request: &[u8],
        delivery: &[u8],
    ) -> Result<(), AppError> {
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        self.current()?
            .checked_store()?
            .validate_recovered_join_delivery(request, delivery)
    }

    pub(super) fn selected_pending_join_data(
        &self,
    ) -> Result<Option<([u8; 32], catcoms_sync::V2PendingJoinContextData)>, AppError> {
        self.current()?
            .checked_store()?
            .selected_pending_join_data()
    }

    pub(super) fn selected_joined_pin_context_data(
        &self,
        anchor: &catcoms_replication::chat::join_bindings::GenesisAnchorBindingData,
    ) -> Result<Option<catcoms_sync::V2PendingJoinContextData>, AppError> {
        self.current()?
            .checked_store()?
            .selected_joined_pin_context_data(anchor)
    }

    pub(super) fn stage_joined_pin_layout(
        &mut self,
        declaration: &mut Option<
            crate::store::persistence::governance_layout::joined::JoinedPinDeclaration,
        >,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.cell.get_mut().map_err(|_| refused())?.require_idle()?;
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        let store = self.current()?.checked_store()?;
        if declaration.is_none() {
            *declaration = Some(store.prepare_joined_pin_declaration(rng)?);
        }
        declaration.as_mut().ok_or_else(refused)?.stage(store)
    }

    pub(super) fn advance_joined_pin_layout(
        &mut self,
        declaration: &mut Option<
            crate::store::persistence::governance_layout::joined::JoinedPinDeclaration,
        >,
        governed: Option<
            &mut crate::store::critical_namespaces::governance_layout::native::Startup,
        >,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.cell.get_mut().map_err(|_| refused())?.require_idle()?;
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        let joined = self.handler.clone().with_joined_pin_layout();
        self.current
            .as_mut()
            .ok_or_else(refused)?
            .advance_native_joined_pin(
                declaration,
                NativeRegistryHandler::GovernedLifecycle(joined.clone()),
                governed,
                rng,
            )?;
        self.handler = joined;
        Ok(())
    }

    pub(super) fn require_joined_pin_layout(
        &self,
        declaration: &crate::store::persistence::governance_layout::joined::JoinedPinDeclaration,
    ) -> Result<(), AppError> {
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        let store = self.current()?.checked_store()?;
        if !self.handler.uses_joined_pin_layout() || !store.uses_joined_pin_layout() {
            return Err(refused());
        }
        declaration.require_promoted(store)
    }

    pub(super) fn advance_shared_history_layout(
        &mut self,
        previous: &mut Option<
            crate::store::persistence::governance_layout::joined::JoinedPinDeclaration,
        >,
        declaration: &mut Option<crate::store::persistence::governance_layout::joined::shared_history::SharedHistoryDeclaration>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.cell.get_mut().map_err(|_| refused())?.require_idle()?;
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        let handler = self.handler.clone().with_warm_review_layout();
        self.current
            .as_mut()
            .ok_or_else(refused)?
            .advance_native_shared_history(
                previous,
                declaration,
                NativeRegistryHandler::GovernedLifecycle(handler.clone()),
                rng,
            )?;
        self.handler = handler;
        Ok(())
    }

    pub(super) fn require_shared_history_layout(
        &self,
        declaration: &crate::store::persistence::governance_layout::joined::shared_history::SharedHistoryDeclaration,
    ) -> Result<(), AppError> {
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        let store = self.current()?.checked_store()?;
        store.require_mounted_shared_history()?;
        declaration.require_promoted(store)
    }

    pub(super) fn require_governance_completion(
        &self,
        receiver: &crate::store::critical_namespaces::governance_layout::native::Startup,
    ) -> Result<(), AppError> {
        receiver.require_complete(self.current()?.checked_store()?)
    }
    #[cfg(test)]
    pub(super) fn constructor_store_for_test(&self) -> Result<&ServerStore, AppError> {
        Ok(self.current()?.constructor_store_for_test())
    }
    pub(super) fn advance_governance(
        &mut self,
        receiver: &mut crate::store::critical_namespaces::governance_layout::native::Startup,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.cell.get_mut().map_err(|_| refused())?.require_idle()?;
        self.handler.require()?;
        self.handler.require_idle_cached()?;
        self.current
            .as_mut()
            .ok_or_else(refused)?
            .advance_native_governance(receiver, rng)
    }
    /// Allocate BEFORE original Current extraction, with the original upgrade
    /// still wholly armed. No physical allocation is exposed until install().
    pub(super) fn allocate(handler: lifecycle_admission::Handler) -> Arc<Self> {
        Arc::new(Self {
            cell: Mutex::default(),
            current: None,
            cold_networks: Vec::new(),
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
        self.handler
            .finish_cold(self.current()?.checked_store()?)
            .map(drop)
    }
    pub(super) fn finish_governed_cold(
        &mut self,
        tail: &crate::store::critical_namespaces::governance_layout::NativeColdTail,
        joined: JoinedPinOpening,
        keep_native_founder: bool,
    ) -> Result<(), AppError> {
        let networks = tail.finish(self.current()?.checked_store()?, joined)?;
        if keep_native_founder {
            self.cold_networks = networks;
        }
        Ok(())
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
    pub(in crate::store::critical_namespaces::native_registry) fn recovered_founder_resources(
        &self,
        bytes: usize,
    ) -> Result<(catcoms_rt::RequestCancellation, CatalogReadReservation), AppError> {
        self.physical.joined_exchange_resources(bytes)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn prepare_recovered_native_founder_tcp(
        &self,
        original: &ColdFounderSelection,
        source: &crate::store::RetainedBootstrapSourceData,
    ) -> Result<(Zeroizing<[u8; 32]>, catcoms_net::NativeTcpPreparation), AppError> {
        self.physical
            .prepare_recovered_native_founder_tcp(original, source)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn require_recovered_founder_source(
        &self,
        original: &ColdFounderSelection,
        source: &crate::store::RetainedBootstrapSourceData,
    ) -> Result<(), AppError> {
        self.physical
            .require_recovered_founder_source(original, source)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn restore_recovered_founder_add(
        &self,
        original: &ColdFounderSelection,
        source: &crate::store::RetainedBootstrapSourceData,
        captured: &mut Option<Box<crate::store::persistence::v2_text::HeldColdFounderAdd>>,
        sdk: &mut Option<catcoms_sync::owner_v2::PreparedV2FounderRecoveredAdd>,
    ) -> Result<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64), AppError> {
        self.physical
            .restore_recovered_founder_add(original, source, captured, sdk)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn with_recovered_founder_store<
        T,
    >(
        &self,
        original: &ColdFounderSelection,
        source: &crate::store::RetainedBootstrapSourceData,
        read: impl FnOnce(&ServerStore) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.physical
            .with_recovered_founder_store(original, source, read)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn with_recovered_founder_layout<
        T,
    >(
        &self,
        original: &ColdFounderSelection,
        source: &crate::store::RetainedBootstrapSourceData,
        work: impl FnOnce(&V2CurrentLayoutStore, &ServerStore) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.physical
            .require_recovered_founder_source(original, source)?;
        let layout = self.physical.current()?;
        let result = work(layout, layout.checked_store()?)?;
        self.physical
            .require_recovered_founder_source(original, source)?;
        Ok(result)
    }

    /// The final native commit edge is allocation- and IO-free. Its caller
    /// must have already proved the same physical founder selection.
    pub(in crate::store::critical_namespaces::native_registry) fn cached_founder_save_layout(
        &self,
    ) -> Result<&V2CurrentLayoutStore, AppError> {
        if !self.is_live_cached() {
            return Err(refused());
        }
        self.physical.current()
    }

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
        if self.physical.handler.uses_joined_pin_layout() {
            store.require_empty_mounted_joined_pins()?;
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
    ) -> Result<crate::store::NativeCreationStatusData, crate::store::NativeTextCreationRefusal>
    {
        self.with_creation_metadata(|original| {
            let (status, debt) = original.birth_founder_data(pending, rng)?;
            drop(debt); // Retire the claim after the cell/catalog guards leave.
            Ok(status)
        })
    }
    pub(in crate::store::critical_namespaces::native_registry) fn activate_native_founder(
        &self,
        request: &founder::Activate,
        network: &Arc<crate::store::persistence::native_registry::network::NetworkDependency>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<crate::store::NativeCreationStatusData, crate::store::NativeTextCreationRefusal>
    {
        self.with_creation_metadata(|original| {
            let (status, debt) = cell::activate_native_founder(
                &original.cell,
                original.current()?,
                &original.handler,
                request,
                network,
                rng,
            )?;
            drop(debt);
            Ok(status)
        })
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
    pub(in crate::store::critical_namespaces::native_registry) fn lookup_native_creation(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<
        Option<crate::store::NativeCreationStatusData>,
        crate::store::NativeTextCreationRefusal,
    > {
        self.with_creation_metadata(|original| original.lookup_creation_data(invocation, request))
    }
    /// Distinguish a live slot claimed by another row before any registry IO.
    /// Only that refusal permits the native caller to retry the same stage.
    fn with_creation_metadata<T>(
        &self,
        run: impl FnOnce(&Physical) -> Result<T, AppError>,
    ) -> Result<T, crate::store::NativeTextCreationRefusal> {
        use crate::store::NativeTextCreationRefusal as Error;
        let work = self.metadata().ok_or_else(|| {
            if self.is_live_cached() {
                Error::Busy
            } else {
                Error::Unavailable
            }
        })?;
        run(&work.physical).map_err(|_| Error::Storage)
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
    #[expect(
        clippy::result_large_err,
        reason = "Native refusal must return the complete original founder inputs without allocating another failure owner."
    )]
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

    /// Original retained member preparation only. The finite caller already
    /// moved the whole runtime outside writers and owns actual per-command
    /// native key cuts. There is no library-origin retry or raw Current getter.
    pub(in crate::store::critical_namespaces::native_registry) fn prepare_native_member<T, R>(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        request: crate::store::V2MemberSignRequest<'_>,
        negative: &catcoms_rt::RequestCancellation,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        keys: &mut crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
    ) -> Result<crate::store::V2MemberSaveOutcome, crate::store::V2MemberSaveRefusal>
    where
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    {
        let current = self
            .physical
            .current()
            .map_err(|_| crate::store::V2MemberSaveRefusal::Unavailable)?;
        driver.prepare_member_save_with_access(
            current,
            request,
            negative,
            rng,
            crate::store::critical_namespaces::archive_bootstrap::founder::FounderAccess::Native(
                keys,
            ),
        )
    }
    /// Actual physical Current stays private to the owning native source.
    pub(in crate::store::critical_namespaces::native_registry) fn prepare_native_invite_intent<
        T,
        R,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        request: catcoms_sync::owner_v2::V2FounderInviteRequestData,
        routing_plan: &[u8],
        keys: &mut crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
        rng: &mut impl CryptoRngCore,
    ) -> Result<([u8; 32], u64), AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    {
        driver.prepare_native_invite_intent_data(
            self.physical.current()?,
            request,
            routing_plan,
            keys,
            rng,
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn save_native_invite<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        request: catcoms_sync::owner_v2::V2FounderInviteRequestData,
        routing_plan: &[u8],
        keys: &mut crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
        rng: &mut impl CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<crate::store::NativeTextSavedInvite, AppError> {
        driver.save_native_invite_data(
            self.physical.current()?,
            request,
            routing_plan,
            keys,
            rng,
            negative,
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn inspect_native_invite<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        request: catcoms_sync::owner_v2::V2FounderInviteRequestData,
        routing_plan: &[u8],
        keys: &crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<crate::store::NativeTextInviteRecovery, AppError> {
        driver.inspect_native_invite_data(
            self.physical.current()?,
            request,
            routing_plan,
            keys,
            negative,
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn require_received_join_request<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        incoming: &catcoms_sync::V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<(), AppError> {
        driver.require_received_join_request(
            self.physical.current()?,
            incoming,
            routing_plan,
            keys,
            negative,
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn handle_received_join_request<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        incoming: &catcoms_sync::V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        signing: (
            &crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
            [catcoms_rt::NativeChatPreparedKeyCut; 3],
        ),
        rng: &mut impl CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<crate::store::NativeTextSavedJoinResponse, AppError> {
        driver.handle_received_join_request(
            self.physical.current()?,
            incoming,
            routing_plan,
            signing,
            rng,
            negative,
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn encode_selected_member_add<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        request: &catcoms_sync::V2MemberAddRequestData,
        keys: &crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
        caller: &catcoms_rt::RequestCancellation,
    ) -> Result<Vec<u8>, AppError> {
        driver.encode_selected_member_add(self.physical.current()?, request, keys, caller)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn remove_governed_member_native<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        target: catcoms_replication::chat::owner_header::OwnerBodyId,
        keys: &crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
        cuts: (
            &mut catcoms_rt::NativeChatPreparedKeyCut,
            &mut catcoms_rt::NativeChatPreparedKeyCut,
            &mut catcoms_rt::NativeChatKeyEnds,
        ),
        rng: &mut impl CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64), AppError> {
        driver.remove_governed_member_native(
            self.physical.current()?,
            target,
            keys,
            cuts,
            rng,
            negative,
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn governed_roster_native<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        keys: &crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<crate::store::NativeTextRoster, AppError> {
        driver.governed_roster_native(self.physical.current()?, keys, negative)
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store::critical_namespaces::native_registry) fn current_for_governed_add_test(
        &self,
    ) -> &V2CurrentLayoutStore {
        self.physical.current().expect("selected governed fixture")
    }
    /// Bounded history setup/recovery through the exact retained NativeRequired
    /// Host. No raw Current/Store getter or library-origin fallback escapes.
    pub(in crate::store::critical_namespaces::native_registry) fn prepare_native_member_history<
        T,
        R,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        read: crate::store::persistence::v2_member_outbox::MemberHistoryRead,
        negative: &catcoms_rt::RequestCancellation,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        keys: &mut crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
    ) -> Result<
        crate::store::persistence::v2_member_outbox::MemberHistoryPage,
        crate::store::V2MemberSaveRefusal,
    >
    where
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    {
        let current = self
            .physical
            .current()
            .map_err(|_| crate::store::V2MemberSaveRefusal::Unavailable)?;
        driver.prepare_member_history_with_access(
            current,
            read,
            negative,
            rng,
            crate::store::critical_namespaces::archive_bootstrap::founder::FounderAccess::Native(
                keys,
            ),
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn run_founder_member_receive<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        channel: u128,
        operation: crate::store::persistence::v2_member_transport::FounderReceiveOperation<'_>,
        caller: &catcoms_rt::RequestCancellation,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        keys: &mut crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
    ) -> Result<
        crate::store::persistence::v2_member_transport::FounderReceiveOutcome,
        crate::store::V2MemberSaveRefusal,
    > {
        let current = self
            .physical
            .current()
            .map_err(|_| crate::store::V2MemberSaveRefusal::Unavailable)?;
        driver.run_founder_member_receive(
            current,
            channel,
            operation,
            caller,
            rng,
            crate::store::critical_namespaces::archive_bootstrap::founder::FounderAccess::Native(
                keys,
            ),
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn run_founder_member_debt<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        operation: crate::store::persistence::v2_member_transport::FounderDebtOperation,
        caller: &catcoms_rt::RequestCancellation,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        keys: &mut crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
    ) -> Result<
        crate::store::persistence::v2_member_transport::FounderDebtOutcome,
        crate::store::V2MemberSaveRefusal,
    > {
        let current = self
            .physical
            .current()
            .map_err(|_| crate::store::V2MemberSaveRefusal::Unavailable)?;
        driver.run_founder_member_debt(
            current,
            operation,
            caller,
            rng,
            crate::store::critical_namespaces::archive_bootstrap::founder::FounderAccess::Native(
                keys,
            ),
        )
    }

    /// Finite native-aware final edge on this SAME original Current. Failure
    /// returns native custody whole; the outer runtime envelope restores only
    /// after that custody has actually released its native guard.
    pub(in crate::store::critical_namespaces::native_registry) fn admit_native_member<T, R>(
        &self,
        driver: &mut crate::store::PreparedFreshV2TextFounder<T, R>,
        token: [u8; 32],
        native: crate::store::MemberSaveNativeAdmission,
        keys: &mut crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys,
    ) -> Result<
        crate::store::MemberSaveJob,
        (
            crate::store::MemberSaveNativeAdmission,
            crate::store::V2MemberSaveRefusal,
        ),
    >
    where
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    {
        let current = match self.physical.current() {
            Ok(current) => current,
            Err(_) => return Err((native, crate::store::V2MemberSaveRefusal::Unavailable)),
        };
        driver.admit_member_save_with_access(
            current,
            token,
            native,
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
pub(in crate::store::critical_namespaces::native_registry) struct JoinedConnectionPreparation {
    pub(in crate::store::critical_namespaces::native_registry) request: Zeroizing<Vec<u8>>,
    pub(in crate::store::critical_namespaces::native_registry) mount:
        catcoms_rt::RequestCancellation,
    pub(in crate::store::critical_namespaces::native_registry) authentication:
        AuthenticationNegative,
    pub(in crate::store::critical_namespaces::native_registry) work: SourceWork,
    pub(in crate::store::critical_namespaces::native_registry) local_transport:
        (catcoms_rt::PeerId, u16),
    pub(in crate::store::critical_namespaces::native_registry) fee: CatalogReadReservation,
}

impl MetadataWork {
    pub(in crate::store::critical_namespaces::native_registry) fn with_registered_joined_member<
        T,
    >(
        &self,
        original: crate::store::NativeTextJoinedRegistrationData,
        warm: Option<
            &crate::store::persistence::governance_layout::joined::JoinedRegistrationDependency,
        >,
        row: crate::store::NativeTextJoinedRegistryRowData,
        run: impl FnOnce(
            &V2CurrentLayoutStore,
            &ServerStore,
        ) -> Result<T, crate::store::V2MemberSaveRefusal>,
    ) -> Result<T, crate::store::V2MemberSaveRefusal> {
        use crate::store::V2MemberSaveRefusal as Refusal;
        let guard = self.physical.cell.try_lock().map_err(|_| Refusal::Busy)?;
        guard.require_idle().map_err(|_| Refusal::Busy)?;
        self.require_joined_store()
            .map_err(|_| Refusal::Unavailable)?;
        let layout = self.physical.current().map_err(|_| Refusal::Unavailable)?;
        let store = layout.checked_store().map_err(|_| Refusal::Unavailable)?;
        let require_original = || {
            if self
                .registered_joined_row_in(store, original, warm)
                .map_err(|_| Refusal::Unavailable)?
                .0
                != row
            {
                return Err(Refusal::Stale);
            }
            Ok(())
        };
        require_original()?;
        let result = run(layout, store)?;
        require_original()?;
        Ok(result)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn with_joined_member_final(
        &self,
        native: crate::store::MemberSaveNativeAdmission,
        admit: impl FnOnce(
            &V2CurrentLayoutStore,
            crate::store::MemberSaveNativeAdmission,
        ) -> Result<
            crate::store::MemberSaveJob,
            (
                crate::store::MemberSaveNativeAdmission,
                crate::store::V2MemberSaveRefusal,
            ),
        >,
    ) -> Result<
        crate::store::MemberSaveJob,
        (
            crate::store::MemberSaveNativeAdmission,
            crate::store::V2MemberSaveRefusal,
        ),
    > {
        use crate::store::V2MemberSaveRefusal as Refusal;
        let Ok(guard) = self.physical.cell.try_lock() else {
            return Err((native, Refusal::Busy));
        };
        if guard.require_idle().is_err()
            || !self.physical.handler.uses_joined_pin_layout()
            || self.physical.handler.require_idle_cached().is_err()
            || !self.is_live_cached()
        {
            return Err((native, Refusal::Unavailable));
        }
        let Ok(layout) = self.physical.current() else {
            return Err((native, Refusal::Unavailable));
        };
        admit(layout, native)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn observe_registered_joined_row(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
        warm: Option<
            &crate::store::persistence::governance_layout::joined::JoinedRegistrationDependency,
        >,
    ) -> Result<
        (
            crate::store::NativeTextJoinedRegistryRowData,
            super::super::super::super::lifecycle::joined::LocalNames,
        ),
        AppError,
    > {
        self.with_joined_store(|store| self.registered_joined_row_in(store, data, warm))
    }

    fn registered_joined_row_in(
        &self,
        store: &ServerStore,
        data: crate::store::NativeTextJoinedRegistrationData,
        warm: Option<
            &crate::store::persistence::governance_layout::joined::JoinedRegistrationDependency,
        >,
    ) -> Result<
        (
            crate::store::NativeTextJoinedRegistryRowData,
            super::super::super::super::lifecycle::joined::LocalNames,
        ),
        AppError,
    > {
        let inventory = UpgradeInventory::capture(
            store,
            self.physical.handler.debit(),
            self.physical.handler.pool(),
        )?;
        if let Some((row, prior)) = store.with_installed_rejoin(|original| {
            if warm.is_some() {
                return Err(refused());
            }
            Ok((original.row(store, data)?, original.prior_registration()))
        })? {
            let (_, names) =
                super::super::super::super::lifecycle::preparation::observe_registered_native_join(
                    store, &inventory, prior,
                )?;
            inventory.require(store)?;
            return Ok((row, names));
        }
        // A genuine older A1 can still be restored from old files. Surviving
        // A2 evidence prevents publishing it through the normal row route.
        if store.has_rejoin_evidence()? {
            return Err(refused());
        }
        let result =
            super::super::super::super::lifecycle::preparation::observe_registered_native_join(
                store, &inventory, data,
            )?;
        if let Some(warm) = warm {
            if warm.data() != data {
                return Err(refused());
            }
            warm.require(store)?;
        } else if store.recovered_join_registration_data()? != Some(data) {
            return Err(refused());
        }
        inventory.require(store)?;
        Ok(result)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn lookup_saved_joined_member(
        &self,
        data: crate::store::NativeTextJoinedRegistrationData,
        warm: Option<
            &crate::store::persistence::governance_layout::joined::JoinedRegistrationDependency,
        >,
        selection: crate::store::NativeTextJoinedRegistryRowData,
        request: &crate::store::V2MemberSignRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<(catcoms_replication::chat::ActionId, u64)>, AppError> {
        use crate::store::persistence::v2_member_outbox::{
            member_outbox_key, MemberRequestData, MemberRequestToken,
        };
        self.with_joined_store(|store| {
            let require_original = || {
                if self.registered_joined_row_in(store, data, warm)?.0 != selection {
                    return Err(refused());
                }
                Ok(())
            };
            require_original()?;
            let mount = store.member_outbox_mount()?;
            let key = member_outbox_key(store)?;
            let receipt = mount
                .lookup_existing(
                    store,
                    data.member_outbox_context_data()?,
                    &key,
                    &MemberRequestData {
                        token: MemberRequestToken::from_bytes(request.token),
                        channel: request.channel,
                        parents: request.parents,
                        body: request.body,
                    },
                    rng,
                )
                .map_err(|_| refused())?;
            require_original()?;
            Ok(receipt.map(|receipt| (receipt.action, receipt.ordinal)))
        })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn require_recovered_joined(
        &self,
        receipt: crate::store::NativeTextJoinedRegistrationData,
    ) -> Result<(), AppError> {
        self.physical.require_registered_native_join(receipt)
    }

    fn with_joined_store<T>(
        &self,
        run: impl FnOnce(&ServerStore) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let cell = self.physical.cell.try_lock().map_err(|_| refused())?;
        cell.require_idle()?;
        self.require_joined_store()?;
        run(self.physical.current()?.checked_store()?)
    }

    fn require_joined_store(&self) -> Result<(), AppError> {
        self.physical.handler.require()?;
        self.physical.handler.require_idle_cached()?;
        if !self.physical.handler.uses_joined_pin_layout() {
            return Err(refused());
        }
        Ok(())
    }

    pub(in crate::store::critical_namespaces::native_registry) fn cover_accepted_first_join(
        &self,
        pending: &mut Option<JoinedPendingIo>,
        accepted: AcceptedFirstJoin<'_>,
        require_caller: impl Fn() -> Result<(), AppError>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<([u8; 32], catcoms_sync::V2PendingJoinContextData), AppError> {
        let require_current = || {
            self.require_joined_store()?;
            require_caller()
        };
        self.with_joined_store(|store| {
            if pending.is_none() {
                let handler = &self.physical.handler;
                let inventory = UpgradeInventory::capture(store, handler.debit(), handler.pool())?;
                super::super::super::super::lifecycle::preparation::require_no_selected_native_founder(
                    store, &inventory,
                )?;
            }
            store.cover_accepted_first_join(pending, accepted, require_current, rng)
        })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn prepare_join_connection(
        &self,
        pending: &JoinedPendingIo,
        invite: &crate::store::NativeTextInvitePacketData<'_>,
    ) -> Result<JoinedConnectionPreparation, AppError> {
        use super::super::super::governed::{connected_join_invoice, NativeTextJoinedStorage};
        self.with_joined_store(|store| {
            let request = pending.request_data(store)?;
            let (mount, fee) = self
                .physical
                .joined_exchange_resources(connected_join_invoice(request.len()))?;
            NativeTextJoinedStorage::require_invite_bytes(request, invite)?;
            let authentication = self.authentication_negative()?;
            let work = self.physical.capture_source().ok_or_else(refused)?;
            let local_transport = pending.native_transport_data()?;
            if pending.request_data(store)? != request {
                return Err(refused());
            }
            Ok(JoinedConnectionPreparation {
                request: Zeroizing::new(request.to_vec()),
                mount,
                authentication,
                work,
                local_transport,
                fee,
            })
        })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn prepare_join_tcp(
        &self,
        pending: &JoinedPendingIo,
    ) -> Result<(Zeroizing<[u8; 32]>, catcoms_net::NativeTcpPreparation), AppError> {
        use super::super::super::governed::JoinFramePayment;
        self.with_joined_store(|store| {
            pending.request_data(store)?;
            let seed = pending.native_transport_seed()?;
            let (_, fee) = self
                .physical
                .joined_exchange_resources(catcoms_net::NATIVE_TCP_LIFETIME_BYTES)?;
            let work = self.physical.capture_source().ok_or_else(refused)?;
            pending.request_data(store)?;
            Ok((
                seed,
                catcoms_net::NativeTcpPreparation::new(
                    work,
                    Arc::new(JoinFramePayment { _fee: fee }),
                ),
            ))
        })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn has_covered_join_request(
        &self,
    ) -> Result<bool, AppError> {
        self.with_joined_store(|store| Ok(store.selected_pending_join_data()?.is_some()))
    }

    pub(in crate::store::critical_namespaces::native_registry) fn require_pending_join_intent(
        &self,
        pending: Option<&JoinedPendingIo>,
        invite: &catcoms_replication::chat::join_bindings::SignedInviteBindingData,
        token: [u8; 32],
    ) -> Result<(), AppError> {
        self.with_joined_store(|store| match pending {
            Some(original) => {
                original.request_data(store)?;
                original.require_original_join_intent(invite, token)?;
                original.request_data(store).map(|_| ())
            }
            None => store.require_recovered_join_intent(invite, token),
        })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn cover_join_delivery(
        &self,
        pending: &mut JoinedPendingIo,
        request: &[u8],
        delivery: &[u8],
        negative: &catcoms_rt::RequestCancellation,
        rng: &mut impl CryptoRngCore,
        key: catcoms_sync::NativeV2JoinKeyEntry<'_>,
    ) -> Result<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64), AppError> {
        self.with_joined_store(|store| {
            if negative.is_cancelled() || pending.request_data(store)? != request {
                return Err(refused());
            }
            pending.finish_welcome(store, delivery, rng, Some(negative), Some(key))?;
            pending.warm_outcome_data(store)?.ok_or_else(refused)
        })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn warm_join_outcome(
        &self,
        pending: &JoinedPendingIo,
        request: &[u8],
    ) -> Result<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64), AppError> {
        self.with_joined_store(|store| {
            if pending.request_data(store)? != request {
                return Err(refused());
            }
            pending.warm_outcome_data(store)?.ok_or_else(refused)
        })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn observe_join_installation(
        &self,
        pending: &JoinedPendingIo,
        invite: &crate::store::NativeTextInvitePacketData<'_>,
        token: [u8; 32],
    ) -> Result<crate::store::NativeTextJoinedRegistrationData, AppError> {
        self.with_joined_store(|store| {
            pending.require_original_join_intent(invite.binding(), token)?;
            pending.warm_registration_data(store)
        })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn capture_join_registration(
        &self,
        pending: &JoinedPendingIo,
    ) -> Result<
        Arc<crate::store::persistence::governance_layout::joined::JoinedRegistrationDependency>,
        AppError,
    > {
        self.with_joined_store(|store| pending.capture_registration_dependency(store))
    }

    pub(in crate::store::critical_namespaces::native_registry) fn select_warm_review_receiver(
        &self,
        pending: &mut JoinedPendingIo,
        original: &mut Arc<
            crate::store::persistence::governance_layout::joined::JoinedRegistrationDependency,
        >,
        require_current: impl FnOnce() -> Result<(), AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<bool, AppError> {
        self.with_joined_store(|store| {
            pending.select_warm_review_receiver(
                store,
                original,
                || {
                    self.require_joined_store()?;
                    require_current()
                },
                rng,
            )
        })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn register_joined(
        &self,
        name: String,
        display: String,
        original: &Arc<
            crate::store::persistence::governance_layout::joined::JoinedRegistrationDependency,
        >,
        rng: &mut impl CryptoRngCore,
    ) -> Result<
        (
            crate::store::NativeTextJoinedRegistryRowData,
            Option<cell::CompletedDebt>,
        ),
        AppError,
    > {
        use crate::store::critical_namespaces::native_registry::lifecycle::joined;
        let request = joined::Register {
            name,
            display,
            original: original.data(),
        };
        cell::register_joined(
            &self.physical.cell,
            self.physical.current()?,
            &self.physical.handler,
            &request,
            original,
            rng,
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn attach_join_node<
        R: catcoms_rt::CryptoRngCore,
    >(
        &self,
        pending: &mut JoinedPendingIo,
        inputs: &mut Option<catcoms_sync::V2JoinedRuntimeInputs<catcoms_net::MeshService, R>>,
        node: &mut Option<catcoms_sync::ClosedV2JoinedNode<catcoms_net::MeshService, R>>,
    ) -> Result<(), AppError> {
        self.with_joined_store(|store| pending.attach_warm_node(store, inputs, node))
    }

    pub(in crate::store::critical_namespaces::native_registry) fn reserve_join_node(
        &self,
        bytes: usize,
    ) -> Result<CatalogReadReservation, AppError> {
        self.joined_transport_resources(bytes).map(|(_, fee)| fee)
    }

    pub(in crate::store::critical_namespaces::native_registry) fn joined_transport_resources(
        &self,
        bytes: usize,
    ) -> Result<(catcoms_rt::RequestCancellation, CatalogReadReservation), AppError> {
        self.physical.joined_exchange_resources(bytes)
    }

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
    pub(in crate::store::critical_namespaces::native_registry) fn capture_source_work(
        &self,
    ) -> Result<SourceWork, AppError> {
        self.physical.capture_source().ok_or_else(refused)
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
    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn row_count_data(
        &mut self,
    ) -> Result<usize, AppError> {
        self.physical.row_count_data()
    }
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Unwired storage-reference Rename or LegacyRefresh adapter; no native UI or invitation publication consumer exists."
        )
    )]
    pub(in crate::store::critical_namespaces::native_registry) fn refresh_legacy_invite_data(
        &mut self,
        request: &refresh::Request,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(rename::Metadata, Option<cell::CompletedDebt>), AppError> {
        self.physical.refresh_legacy_invite_data(request, rng)
    }
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Unwired storage-reference Rename or LegacyRefresh adapter; no native UI or invitation publication consumer exists."
        )
    )]
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
