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
    // The actual vault session outlives the original File, both frames
    // and the directory handle, even if their retired owner closes.
    _lease: catcoms_storage::VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

impl RejoinCatalogPreparation {
    fn require_catalog_files(&self, store: &ServerStore) -> Result<(), AppError> {
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
            stage,
            _lease: self._session.retain_for_physical_work(),
            _fee: fee,
        };
        self.require_recovered_joined_removal(registration, removal)?;
        result.require_original(self)?;
        Ok(result)
    }
}
