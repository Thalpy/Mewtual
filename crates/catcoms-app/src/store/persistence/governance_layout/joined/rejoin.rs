//! Prepare the one authenticated ID9 catalog successor on a retired mount.
use super::*;
use crate::store::critical_namespaces::rejoin_namespace;
use crate::store::NativeTextJoinedRegistrationData;
use catcoms_sync::SelectedRejoinRemoval;

/// Both encrypted configurations remain paid while the old catalog is selected.
/// Only the original, exclusive owner can advance this candidate to a writer.
pub(in crate::store) struct RejoinCatalogPreparation {
    original: OpenedArchiveObject,
    candidate: Zeroizing<Vec<u8>>,
    directory: GovernanceDirectoryCapture,
    mount: std::sync::Arc<()>,
    // The actual vault session outlives the original File, both frames
    // and the directory handle, even if their retired owner closes.
    _lease: catcoms_storage::VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

impl RejoinCatalogPreparation {
    pub(in crate::store) fn require_original(&self, store: &ServerStore) -> Result<(), AppError> {
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
        let (file, bytes) = &self.original;
        exact_held(
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            file,
            session_id(file).map_err(io)?,
            bytes,
        )?;
        let key = Zeroizing::new(store.keys.db_key()?);
        rejoin_namespace::verify_framed_declaration(
            &key,
            bytes,
            &self.candidate,
            &store.chat_vault_domain()?,
        )
    }
}

impl ServerStore {
    pub(in crate::store) fn prepare_retired_rejoin_catalog(
        &self,
        registration: NativeTextJoinedRegistrationData,
        removal: SelectedRejoinRemoval,
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
        let candidate = {
            let key = Zeroizing::new(self.keys.db_key()?);
            rejoin_namespace::prepare_framed_declaration(
                &key,
                &original.1,
                &self.chat_vault_domain()?,
                rng,
            )?
        };
        let result = RejoinCatalogPreparation {
            original,
            candidate,
            directory: inventory.directory,
            mount: self.registry_mount(),
            _lease: self._session.retain_for_physical_work(),
            _fee: fee,
        };
        self.require_recovered_joined_removal(registration, removal)?;
        result.require_original(self)?;
        Ok(result)
    }
}
