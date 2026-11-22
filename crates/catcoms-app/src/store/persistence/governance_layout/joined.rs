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

/// Retained across all existing startup gates before the selected catalog can
/// be exposed. A cold repair additionally retains the proven seed's leaves.
pub(in crate::store) struct ColdJoinedSelection {
    current: File,
    bytes: Zeroizing<Vec<u8>>,
    directory: GovernanceDirectoryCapture,
    original_leaves: Option<(OpenedArchiveObject, VerifiedGovernanceNative)>,
    _fee: CatalogReadReservation,
}

impl ColdJoinedSelection {
    fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        let seed_path = fixed(store, false, governance::GOVERNANCE_SEED)?;
        let stage_path = fixed(store, false, critical_namespaces::Slot::Stage.name())?;
        absent(&seed_path)?;
        absent(&stage_path)?;
        store.require_governance_directory_matches_root_if_mounted(&self.directory)?;
        if let Some(((marker, bytes), native)) = &self.original_leaves {
            native.require(store)?;
            exact_held(
                &fixed(store, true, governance::GOVERNANCE_MARKER)?,
                marker,
                session_id(marker).map_err(io)?,
                bytes,
            )?;
        }
        exact_held(
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            &self.current,
            session_id(&self.current).map_err(io)?,
            &self.bytes,
        )?;
        absent(&seed_path)?;
        absent(&stage_path)
    }
}

impl ServerStore {
    pub(in crate::store) fn uses_joined_pin_layout(&self) -> bool {
        self.current_layout_policy
            .as_ref()
            .and_then(|p| p.native_registry.as_ref())
            .is_some_and(|handler| handler.uses_joined_pin_layout())
    }

    /// Only the explicitly configured joined opener enters here. The entire
    /// compound record is classified before a file is synced or replaced.
    pub(in crate::store) fn select_joined_pin_catalog(
        &self,
    ) -> Result<ColdJoinedSelection, AppError> {
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        if !self.uses_joined_pin_layout() || policy.native_pool.is_none() {
            return Err(denied());
        }
        policy.require_persistent_capacity()?;
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(governance::codec::DECLARATION_FEE)?;
        let inventory = self.read_governance_inventory(&fee)?;
        let current_path = fixed(self, false, critical_namespaces::Slot::Current.name())?;
        let stage_path = fixed(self, false, critical_namespaces::Slot::Stage.name())?;
        let seed_path = fixed(self, false, governance::GOVERNANCE_SEED)?;
        let marker_path = fixed(self, true, governance::GOVERNANCE_MARKER)?;
        let (current, raw) = inventory.current.ok_or_else(denied)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let directory = inventory.directory;
        let (current, bytes, original_leaves) = if let Some((seed, raw_seed)) = inventory.seed {
            let seed_id = session_id(&seed).map_err(io)?;
            let seed_data = AuthenticatedSeed::open(&key, &raw_seed)?;
            let marker = inventory.marker.ok_or_else(denied)?;
            let marker_id = session_id(&marker.0).map_err(io)?;
            let native = NativeGovernanceInventory::capture(
                self,
                policy.native_pool.as_ref().ok_or_else(denied)?,
            )?;
            let layout = native.layout(self)?;
            let leaves = DeclarationLeaves::from_frames(
                &marker.1,
                native.member_marker(),
                native.marker(),
                native.index(),
            )?;
            let expected = seed_data.validate(&key, layout, leaves)?;
            let native = verify_joined_pin_parent(self, native, expected.original, &marker.1)?;
            let current_id = session_id(&current).map_err(io)?;
            let before_selection = || {
                self.require_governance_directory_matches_root_if_mounted(&directory)?;
                native.require(self)?;
                exact_held(&marker_path, &marker.0, marker_id, &marker.1)?;
                exact_held(&seed_path, &seed, seed_id, &raw_seed)?;
                exact_held(&current_path, &current, current_id, &raw)
            };
            let candidate_bytes = Zeroizing::new(expected.candidate.to_vec());
            let selected = if raw.as_slice() == expected.original {
                // The complete seed authorizes this single fixed stage, up to
                // its exact prefix. A stage seen at inventory keeps its handle.
                let mut stage = match inventory.stage {
                    Some((file, prefix)) if expected.candidate.starts_with(&prefix) => {
                        Some(CreatedStage {
                            identity: Some(session_id(&file).map_err(io)?),
                            file,
                        })
                    }
                    Some(_) => return Err(denied()),
                    None => None,
                };
                before_selection()?;
                if stage.is_none() {
                    ensure_legacy_stage_fence(&stage_path, &mut stage, expected.candidate)?;
                }
                ensure(
                    &stage_path,
                    expected.candidate,
                    &mut stage,
                    ArchiveOutputMode::SeedRepair,
                    #[cfg(test)]
                    0,
                    #[cfg(test)]
                    0,
                )?;
                sync_directory(&self.dir).map_err(io)?;
                #[cfg(test)]
                governance::before_stage_selection_for_test();
                before_selection()?;
                let stage = stage.ok_or_else(denied)?;
                exact_retained(&stage_path, &stage, expected.candidate)?;
                held(&current_path, &current, current_id)?;
                fs::rename(&stage_path, &current_path).map_err(io)?;
                stage.file
            } else if raw.as_slice() == expected.candidate && inventory.stage.is_none() {
                before_selection()?;
                absent(&stage_path)?;
                current
            } else {
                return Err(denied());
            };
            let durable =
                |error: std::io::Error| AppError::CommittedButNotDurable(error.to_string());
            selected.sync_all().map_err(durable)?;
            sync_directory(&self.dir).map_err(durable)?;
            absent(&stage_path)?;
            native.require(self)?;
            exact_held(&marker_path, &marker.0, marker_id, &marker.1)?;
            exact_held(
                &current_path,
                &selected,
                session_id(&selected).map_err(io)?,
                &candidate_bytes,
            )?;
            exact_held(&seed_path, &seed, seed_id, &raw_seed)?;
            fs::remove_file(&seed_path).map_err(durable)?;
            drop(seed);
            sync_directory(&self.dir).map_err(durable)?;
            (selected, candidate_bytes, Some((marker, native)))
        } else {
            if inventory.stage.is_some() {
                return Err(denied());
            }
            let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?)?);
            self.parse_joined_pin_catalog(&plain)?;
            (current, raw, None)
        };
        let value = ColdJoinedSelection {
            current,
            bytes,
            directory,
            original_leaves,
            _fee: fee,
        };
        value.require(self)?;
        Ok(value)
    }

    pub(in crate::store) fn retain_joined_pin_selection(
        &self,
        selected: ColdJoinedSelection,
    ) -> Result<(), AppError> {
        selected.require(self)?;
        let mut slot = self.joined_pin_selection.lock().map_err(|_| denied())?;
        if slot.is_some() {
            return Err(denied());
        }
        *slot = Some(selected);
        Ok(())
    }

    pub(in crate::store) fn finish_joined_pin_selection(&self) -> Result<(), AppError> {
        if !self.uses_joined_pin_layout() {
            return Ok(());
        }
        let selected = self
            .joined_pin_selection
            .lock()
            .map_err(|_| denied())?
            .take()
            .ok_or_else(denied)?;
        selected.require(self)
    }

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
