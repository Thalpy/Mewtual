//! Prepare the one authenticated ID9 catalog successor on a retired mount.
use super::*;
use crate::store::critical_namespaces::rejoin_namespace;
use crate::store::persistence::native_registry::lifecycle::{
    upgrade::UpgradeInventory, EXPECTED_CURRENT_FEE, MAX_INDEX_FRAME,
};
use crate::store::persistence::v2_member_outbox::fs::{open_exact_write, WriteMode};
use crate::store::CriticalCatalogReadPool;
use crate::store::NativeTextJoinedRegistrationData;
use catcoms_sync::SelectedRejoinRemoval;

// The private Stage is inert. The shared JOIN Stage must remain absent, and
// no A2 output may be created until Current itself carries the ID9 fence.
const REJOIN_STAGE: &str = "chat-critical-namespaces.rejoin.stage";

mod pending;
pub use pending::NativeTextRejoinPendingReceipt;
pub(in crate::store) use pending::{RejoinKeyEntry, WarmRejoinPending};

/// The exact native inventory can outlive the handler scratch that checked
/// A1. Its buffers and files always retire before its dedicated invoice.
pub(in crate::store) struct RetainedNativeRejoinInventory {
    inventory: UpgradeInventory,
    _fee: CatalogReadReservation,
}

impl RetainedNativeRejoinInventory {
    pub(in crate::store) fn retain(
        inventory: UpgradeInventory,
        pool: &CriticalCatalogReadPool,
    ) -> Result<Self, AppError> {
        if inventory.seed().is_some()
            || inventory.catalog_stage().is_some()
            || inventory.index_stage().is_some()
            || inventory.has_upgrade_outputs()
        {
            return Err(denied());
        }
        // The handler paid to authenticate the whole native inventory.
        // Only Current, the native index, and their markers and directories
        // survive here. Migration files, WAL and row-network work are absent.
        const BYTES: usize = 2 * critical_namespaces::pending_join_codec::MAX_CATALOG_FRAMED
            + 2 * MAX_INDEX_FRAME
            + 4 * critical_namespaces::native_registry::preparation::MAX_MARKER_FRAME
            + EXPECTED_CURRENT_FEE
            + 128 * 1024;
        let fee = pool.try_reserve(BYTES)?;
        Ok(Self {
            inventory,
            _fee: fee,
        })
    }
}

/// Both encrypted configurations remain paid while the old catalog is selected.
/// Only the original, exclusive owner can advance this candidate to a writer.
pub(in crate::store) struct RejoinCatalogPreparation {
    original: OpenedArchiveObject,
    candidate: Zeroizing<Vec<u8>>,
    directory: GovernanceDirectoryCapture,
    mount: std::sync::Arc<()>,
    native: RetainedNativeRejoinInventory,
    registration: NativeTextJoinedRegistrationData,
    removal: SelectedRejoinRemoval,
    stage: Option<CreatedStage>,
    complete_stage: bool,
    selection_started: bool,
    // The actual vault session outlives the original File, both frames
    // and the directory handle, even if their retired owner closes.
    _lease: catcoms_storage::VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

/// Closed substitution of the catalog role for a terminal-only reproof.
/// The same complete Stage must now hold Current, and all other A1 files
/// still use their original observers.
pub(in crate::store) struct SelectedRejoinCatalog<'a> {
    preparation: &'a RejoinCatalogPreparation,
}

impl SelectedRejoinCatalog<'_> {
    pub(in crate::store) fn require_replaced_current(
        &self,
        store: &ServerStore,
        original_identity: SessionId,
        original_bytes: &[u8],
    ) -> Result<(), AppError> {
        let prepared = self.preparation;
        if session_id(&prepared.original.0).map_err(io)? != original_identity
            || prepared.original.1.as_slice() != original_bytes
        {
            return Err(denied());
        }
        prepared.require_selected_file(store)
    }
}

impl RejoinCatalogPreparation {
    fn require_catalog_base(&self, store: &ServerStore) -> Result<(), AppError> {
        if !std::sync::Arc::ptr_eq(&self.mount, &store.registry_mount()) {
            return Err(denied());
        }
        store.require_governance_directory_matches_root(&self.directory)?;
        absent(&fixed(store, false, governance::GOVERNANCE_SEED)?)?;
        absent(&fixed(
            store,
            false,
            critical_namespaces::Slot::Stage.name(),
        )?)?;
        Ok(())
    }

    fn require_catalog_files(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_catalog_base(store)?;
        let from = fixed(store, false, REJOIN_STAGE)?;
        let to = fixed(store, false, critical_namespaces::Slot::Current.name())?;
        if let Some(stage) = self.stage.as_ref() {
            let identity = session_id(&stage.file).map_err(io)?;
            if stage.identity.is_some_and(|original| original != identity) {
                return Err(denied());
            }
            if self.complete_stage {
                exact_held(&from, &stage.file, identity, &self.candidate)?;
            } else {
                // The writer checks the full already-written prefix before
                // it continues a short write on the same file.
                held(&from, &stage.file, identity)?;
            }
        } else {
            absent(&from)?;
        }
        let (original, bytes) = &self.original;
        exact_held(&to, original, session_id(original).map_err(io)?, bytes)
    }

    /// Sync the private, non-selecting rejoin Stage. Continued writes keep
    /// the original create_new handle. A cold retry has already checked a
    /// complete authenticated Stage before opening it for this role.
    pub(in crate::store) fn stage_catalog(&mut self, store: &ServerStore) -> Result<(), AppError> {
        self.require_original(store)?;
        let stage_path = fixed(store, false, REJOIN_STAGE)?;
        ensure_native_claim(&stage_path, &mut self.stage)?;
        sync_directory(&store.dir).map_err(io)?;
        ensure(
            &stage_path,
            &self.candidate,
            &mut self.stage,
            ArchiveOutputMode::Original,
            #[cfg(test)]
            0,
            #[cfg(test)]
            0,
        )?;
        sync_directory(&store.dir).map_err(io)?;
        self.require_original(store)?;
        exact_retained(
            &stage_path,
            self.stage.as_ref().ok_or_else(denied)?,
            &self.candidate,
        )?;
        self.complete_stage = true;
        Ok(())
    }

    fn require_selected_file(&self, store: &ServerStore) -> Result<(), AppError> {
        if !self.selection_started || !self.complete_stage {
            return Err(denied());
        }
        self.require_catalog_base(store)?;
        absent(&fixed(store, false, REJOIN_STAGE)?)?;
        exact_retained(
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            self.stage.as_ref().ok_or_else(denied)?,
            &self.candidate,
        )
    }

    fn require_selected(&self, store: &ServerStore) -> Result<(), AppError> {
        let selected = SelectedRejoinCatalog { preparation: self };
        self.native
            .inventory
            .require_after_rejoin(store, &selected)?;
        store.require_recovered_removal_after_rejoin(self.registration, self.removal, &selected)?;
        self.native.inventory.require_after_rejoin(store, &selected)
    }

    /// The only writer for the fully authenticated candidate. After a
    /// rename the retired A1 cache stays closed. A retry covers the same
    /// held file and original parent directory before returning success.
    pub(in crate::store) fn select_catalog(&mut self, store: &ServerStore) -> Result<(), AppError> {
        if !self.selection_started {
            self.stage_catalog(store)?;
        }
        match self.require_original(store) {
            Ok(()) => {
                let stage = fixed(store, false, REJOIN_STAGE)?;
                let current = fixed(store, false, critical_namespaces::Slot::Current.name())?;
                // Record the possible handoff before the syscall. A refused
                // or ambiguous return keeps both exact original file loans.
                self.selection_started = true;
                install_catalog_stage(&stage, &current).map_err(io)?;
            }
            Err(error) if !self.selection_started => return Err(error),
            Err(_) => {}
        }
        self.require_selected(store)?;
        sync_selected_catalog(&self.stage.as_ref().ok_or_else(denied)?.file).map_err(io)?;
        self.native.inventory.sync_rejoin_catalog_parent(store)?;
        self.require_selected(store)
    }

    pub(in crate::store) fn require_original(&self, store: &ServerStore) -> Result<(), AppError> {
        self.native.inventory.require(store)?;
        self.require_catalog_files(store)?;
        store.require_recovered_joined_removal(self.registration, self.removal)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        rejoin_namespace::verify_framed_declaration(
            &key,
            &self.original.1,
            &self.candidate,
            &store.chat_vault_domain()?,
        )?;
        self.native.inventory.require(store)?;
        self.require_catalog_files(store)
    }
}

impl ServerStore {
    /// Reprove and cover the catalog in a fresh retired mount. This fingerprint
    /// is historical data for the A2 journal, never admission to the key.
    pub(in crate::store) fn cover_selected_rejoin_catalog(
        &self,
        registration: NativeTextJoinedRegistrationData,
        removal: SelectedRejoinRemoval,
        inventory: &UpgradeInventory,
    ) -> Result<[u8; 32], AppError> {
        inventory.require(self)?;
        self.require_recovered_joined_removal(registration, removal)?;
        absent(&fixed(self, false, REJOIN_STAGE)?)?;
        let digest = {
            let slot = self.joined_pin_index.try_lock().map_err(|_| denied())?;
            let index = slot.as_ref().ok_or_else(denied)?;
            index.require(self)?;
            let catalog = index.cold_catalog.as_ref().ok_or_else(denied)?;
            let (file, recorded) = &catalog.current;
            let identity = session_id(file).map_err(io)?;
            if inventory.catalog_identity() != identity
                || inventory.catalog() != recorded.as_slice()
            {
                return Err(denied());
            }
            let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
            let _fee = policy.current.archive.pending.pool.try_reserve(
                3 * critical_namespaces::pending_join_codec::MAX_CATALOG_FRAMED + 16 * 1024,
            )?;
            let key = Zeroizing::new(self.keys.db_key()?);
            let digest = rejoin_namespace::selected_frame_digest(
                &key,
                recorded,
                &self.chat_vault_domain()?,
            )?;
            let selected = fixed(self, false, critical_namespaces::Slot::Current.name())?;
            let (writable, opened) = open_exact_write(&selected, WriteMode::Existing)?;
            if opened != identity {
                return Err(denied());
            }
            exact_held(&selected, &writable, identity, recorded)?;
            sync_selected_catalog(&writable).map_err(io)?;
            inventory.sync_rejoin_catalog_parent(self)?;
            exact_held(&selected, &writable, identity, recorded)?;
            index.require(self)?;
            digest
        };
        self.require_recovered_joined_removal(registration, removal)?;
        absent(&fixed(self, false, REJOIN_STAGE)?)?;
        inventory.require(self)?;
        Ok(digest)
    }

    pub(in crate::store) fn prepare_retired_rejoin_catalog(
        &self,
        registration: NativeTextJoinedRegistrationData,
        removal: SelectedRejoinRemoval,
        native: RetainedNativeRejoinInventory,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<RejoinCatalogPreparation, AppError> {
        self.require_recovered_joined_removal(registration, removal)?;
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        policy.require_persistent_capacity()?;
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(governance::codec::DECLARATION_FEE)?;
        let mut inventory = self.read_governance_inventory(&fee)?;
        if inventory.seed.is_some() || inventory.stage.is_some() || inventory.marker.is_none() {
            return Err(denied());
        }
        let original = inventory.current.take().ok_or_else(denied)?;
        // The marker was only an inventory precondition. Drop its File and
        // buffer before the invoice moves into the prepared value.
        drop(inventory.marker.take());
        let (candidate, stage) = {
            let key = Zeroizing::new(self.keys.db_key()?);
            let domain = self.chat_vault_domain()?;
            let path = fixed(self, false, REJOIN_STAGE)?;
            if let Some((captured, bytes)) = read(
                &path,
                critical_namespaces::pending_join_codec::MAX_CATALOG_FRAMED,
            )? {
                // A cold opener did not create the name. It can adopt only
                // a complete authenticated successor of its exact Current.
                rejoin_namespace::verify_framed_declaration(&key, &original.1, &bytes, &domain)?;
                let (file, id) = open_exact_write(&path, WriteMode::Existing)?;
                if session_id(&captured).map_err(io)? != id {
                    return Err(denied());
                }
                let stage = CreatedStage {
                    file,
                    identity: Some(id),
                };
                exact_retained(&path, &stage, &bytes)?;
                (bytes, Some(stage))
            } else {
                let candidate =
                    rejoin_namespace::prepare_framed_declaration(&key, &original.1, &domain, rng)?;
                (candidate, None)
            }
        };
        let result = RejoinCatalogPreparation {
            original,
            candidate,
            directory: inventory.directory,
            mount: self.registry_mount(),
            native,
            registration,
            removal,
            complete_stage: stage.is_some(),
            selection_started: false,
            stage,
            _lease: self._session.retain_for_physical_work(),
            _fee: fee,
        };
        self.require_recovered_joined_removal(registration, removal)?;
        result.require_original(self)?;
        Ok(result)
    }
}
