//! Same-file first declaration claim and stage for the joined-pin catalog.
use super::*;
use crate::store::persistence::native_registry::lifecycle::governed::{
    NativeGovernanceInventory, INVENTORY_FEE,
};
use critical_namespaces::{
    joined_pin::declaration::{AuthenticatedSeed, DeclarationLeaves, PreparedDeclaration},
    native_registry::{verify_joined_pin_parent, VerifiedGovernanceNative},
    pending_join::CreatedStage,
};

/// Kept by the unique native startup continuation before any Source is exposed.
/// The existing governance seed name is the predecessor-visible first claim.
pub(in crate::store) struct JoinedPinDeclaration {
    original: OpenedArchiveObject,
    marker: OpenedArchiveObject,
    directory: GovernanceDirectoryCapture,
    native: VerifiedGovernanceNative,
    prepared: PreparedDeclaration,
    seed: Option<CreatedStage>,
    stage: Option<CreatedStage>,
    _fee: CatalogReadReservation,
}

impl ServerStore {
    pub(in crate::store) fn prepare_joined_pin_declaration(
        &self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<JoinedPinDeclaration, AppError> {
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        policy.require_persistent_capacity()?;
        if !self
            .current_layout_policy
            .as_ref()
            .and_then(|p| p.native_registry.as_ref())
            .is_some_and(|handler| handler.is_governed())
        {
            return Err(denied());
        }
        self.validate_mounted_current_layout()?;
        self.validate_mounted_member_outbox_layout()?;
        self.validate_mounted_governance_layout()?;
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(governance::codec::DECLARATION_FEE)?;
        let inventory = self.read_governance_inventory(&fee)?;
        if inventory.seed.is_some() || inventory.stage.is_some() {
            return Err(denied());
        }
        let original = inventory.current.ok_or_else(denied)?;
        let marker = inventory.marker.ok_or_else(denied)?;
        let pool = critical_namespaces::read_pool::CriticalCatalogReadPool::new(INVENTORY_FEE)?;
        let native = NativeGovernanceInventory::capture(self, &pool)?;
        let layout = native.layout(self)?;
        let leaves = DeclarationLeaves::from_frames(
            &marker.1,
            native.member_marker(),
            native.marker(),
            native.index(),
        )?;
        let native = verify_joined_pin_parent(self, native, &original.1, &marker.1)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let prepared = PreparedDeclaration::prepare(&key, &original.1, layout, leaves, rng)?;
        let seed = AuthenticatedSeed::open(&key, &prepared.seed)?;
        let checked = seed.validate(&key, layout, leaves)?;
        if checked.original != original.1.as_slice()
            || checked.candidate != prepared.candidate.as_slice()
        {
            return Err(denied());
        }
        let value = JoinedPinDeclaration {
            original,
            marker,
            directory: inventory.directory,
            native,
            prepared,
            seed: None,
            stage: None,
            _fee: fee,
        };
        value.require_original(self)?;
        Ok(value)
    }
}

impl JoinedPinDeclaration {
    fn require_original(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_governance_directory_matches_root(&self.directory)?;
        self.native.require_initial_catalog(store)?;
        for (path, (file, bytes)) in [
            (
                fixed(store, false, critical_namespaces::Slot::Current.name())?,
                &self.original,
            ),
            (
                fixed(store, true, governance::GOVERNANCE_MARKER)?,
                &self.marker,
            ),
        ] {
            exact_held(&path, file, session_id(file).map_err(io)?, bytes)?;
        }
        Ok(())
    }

    /// Retain the claim handle before writing any bytes. A retry completes only
    /// these original files and never accepts an equal-byte replacement.
    pub(in crate::store) fn stage(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let claim_path = fixed(store, false, governance::GOVERNANCE_SEED)?;
        let stage_path = fixed(store, false, critical_namespaces::Slot::Stage.name())?;
        self.require_original(store)?;
        if self.stage.is_none() {
            absent(&stage_path)?;
        }
        ensure_native_claim(&claim_path, &mut self.seed)?;
        sync_directory(&store.dir).map_err(io)?;
        self.require_original(store)?;
        ensure(
            &claim_path,
            &self.prepared.seed,
            &mut self.seed,
            ArchiveOutputMode::Original,
            #[cfg(test)]
            0,
            #[cfg(test)]
            0,
        )?;
        let claim = self.seed.as_ref().ok_or_else(denied)?;
        exact_retained(&claim_path, claim, &self.prepared.seed)?;
        self.require_original(store)?;
        ensure_legacy_stage_fence(&stage_path, &mut self.stage, &self.prepared.candidate)?;
        sync_directory(&store.dir).map_err(io)?;
        ensure(
            &stage_path,
            &self.prepared.candidate,
            &mut self.stage,
            ArchiveOutputMode::Original,
            #[cfg(test)]
            0,
            #[cfg(test)]
            0,
        )?;
        sync_directory(&store.dir).map_err(io)?;
        self.require_original(store)?;
        exact_retained(&claim_path, claim, &self.prepared.seed)?;
        exact_retained(
            &stage_path,
            self.stage.as_ref().ok_or_else(denied)?,
            &self.prepared.candidate,
        )
    }
}
