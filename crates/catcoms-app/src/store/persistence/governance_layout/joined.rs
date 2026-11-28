//! Same-file first declaration claim and stage for the joined-pin catalog.
use super::*;

use crate::store::persistence::native_registry::lifecycle::governed::{
    NativeGovernanceInventory, INVENTORY_FEE,
};
pub(in crate::store) use catcoms_sync::V2JoinedRuntimeInputs as JoinedRuntimeInputs;
use critical_namespaces::{
    archive_current_namespace_codec::CurrentLayoutData,
    joined_pin::declaration::{AuthenticatedSeed, DeclarationLeaves, PreparedDeclaration},
    joined_pin::index::{JoinedPinIndex, MAX_FRAME as MAX_INDEX_FRAME},
    native_registry::{verify_joined_pin_parent, VerifiedGovernanceNative},
    pending_join::CreatedStage,
    shared_history_namespace::Opening,
};

pub(in crate::store) const INDEX_NAME: &str = "chat-critical-namespaces.joined-pin.index";
const INDEX_FEE: usize = 12 * MAX_INDEX_FRAME + 512 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) enum JoinedPinOpening {
    Any,
    Idle,
    NativeWithoutRow,
}
mod pending;
pub(in crate::store) mod rejoin;
pub(in crate::store) mod review_decision;
pub(in crate::store) mod review_hold;
pub(in crate::store) mod review_receive;
pub(in crate::store) mod shared_history;
pub(in crate::store) use pending::{
    AcceptedFirstJoin, AcceptedJoinTransport, JoinedPendingIo, JoinedRegistrationDependency,
};
#[cfg(test)]
pub(in crate::store) use pending::{AFTER_PIN_INDEX_SELECT, BEFORE_WELCOME_SDK};

fn install_catalog_stage(stage: &Path, current: &Path) -> std::io::Result<()> {
    fs::rename(stage, current)
}

fn sync_selected_catalog(selected: &File) -> std::io::Result<()> {
    selected.sync_all()
}

fn retire_declaration_claim(claim: &Path) -> std::io::Result<()> {
    fs::remove_file(claim)
}

/// Kept by the unique native startup continuation before any Source is exposed.
/// The existing governance seed name is the predecessor-visible first claim.
pub(in crate::store) struct JoinedPinDeclaration {
    original: OpenedArchiveObject,
    marker: OpenedArchiveObject,
    directory: GovernanceDirectoryCapture,
    native: VerifiedGovernanceNative,
    layout: CurrentLayoutData,
    leaves: DeclarationLeaves,
    prepared: PreparedDeclaration,
    seed: Option<CreatedStage>,
    index: Option<CreatedStage>,
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
    index: SelectedPinIndex,
    shared_history: Option<shared_history::SelectedSharedHistory>,
    review_hold: Option<review_hold::SelectedReviewHold>,
    review_decision: Option<review_decision::SelectedReviewDecision>,
    review_decision_declared: bool,
    review_receive: Option<review_receive::SelectedReviewReceiver>,
    review_receive_declared: bool,
    _fee: CatalogReadReservation,
}

pub(in crate::store) struct SelectedPinIndex {
    selected: OpenedArchiveObject,
    index: JoinedPinIndex,
    pending: Option<pending::SelectedPending>,
    cold_catalog: Option<ColdPendingCatalog>,
    _fee: CatalogReadReservation,
}

/// The physical Current file selected while recovering the private Pending.
/// Keep it beside that Pending after the transient constructor invoice ends.
struct ColdPendingCatalog {
    current: OpenedArchiveObject,
    directory: GovernanceDirectoryCapture,
    _fee: CatalogReadReservation,
}

impl ColdPendingCatalog {
    fn require_with_rejoin_catalog(
        &self,
        store: &ServerStore,
        rejoin_catalog: Option<&rejoin::SelectedRejoinCatalog<'_>>,
    ) -> Result<(), AppError> {
        validate_directory(store, &self.directory)?;
        absent(&fixed(store, false, governance::GOVERNANCE_SEED)?)?;
        absent(&fixed(
            store,
            false,
            critical_namespaces::Slot::Stage.name(),
        )?)?;
        let (file, raw) = &self.current;
        if let Some(rejoin) = rejoin_catalog {
            return rejoin.require_replaced_current(store, session_id(file).map_err(io)?, raw);
        }
        exact_held(
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            file,
            session_id(file).map_err(io)?,
            raw,
        )
    }
}

impl SelectedPinIndex {
    fn open(
        store: &ServerStore,
        key: &[u8; 32],
        layout: CurrentLayoutData,
        expected: Option<&CreatedStage>,
        fee: CatalogReadReservation,
    ) -> Result<Self, AppError> {
        let path = fixed(store, false, INDEX_NAME)?;
        let selected = read(&path, MAX_INDEX_FRAME)?.ok_or_else(denied)?;
        if let Some(original) = expected {
            if original.identity != Some(session_id(&selected.0).map_err(io)?) {
                return Err(denied());
            }
        }
        let index = JoinedPinIndex::open(key, layout, &selected.1)?;
        let value = Self {
            selected,
            index,
            pending: None,
            cold_catalog: None,
            _fee: fee,
        };
        value.require_file(store)?;
        Ok(value)
    }

    fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_with_rejoin_catalog(store, None)
    }

    fn require_with_rejoin_catalog(
        &self,
        store: &ServerStore,
        rejoin_catalog: Option<&rejoin::SelectedRejoinCatalog<'_>>,
    ) -> Result<(), AppError> {
        self.require_file(store)?;
        pending::require_pending_names(store, self.pending.as_ref())?;
        if let Some(catalog) = &self.cold_catalog {
            catalog.require_with_rejoin_catalog(store, rejoin_catalog)?;
        } else if rejoin_catalog.is_some() {
            return Err(denied());
        }
        Ok(())
    }

    fn require_file(&self, store: &ServerStore) -> Result<(), AppError> {
        exact_held(
            &fixed(store, false, INDEX_NAME)?,
            &self.selected.0,
            session_id(&self.selected.0).map_err(io)?,
            &self.selected.1,
        )
    }
}

impl ColdJoinedSelection {
    fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        let seed_path = fixed(store, false, governance::GOVERNANCE_SEED)?;
        let stage_path = fixed(store, false, critical_namespaces::Slot::Stage.name())?;
        absent(&seed_path)?;
        absent(&stage_path)?;
        store.require_governance_directory_matches_root_if_mounted(&self.directory)?;
        self.index.require(store)?;
        if let Some(shared) = &self.shared_history {
            shared.require(store)?;
        }
        if let Some(review) = &self.review_hold {
            review.require(store)?;
        }
        // Review repair debt is independent of ordinary joined history.
        // When an authenticated descriptor is selected its missing files
        // cannot create a fresh review journal during ordinary startup.
        if !self.review_decision_declared {
            review_decision::require_absent(store)?;
        }
        if !self.review_receive_declared {
            review_receive::require_absent(store)?;
        }
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

    fn shared_history_opening(&self) -> Opening {
        self.current_layout_policy
            .as_ref()
            .and_then(|p| p.native_registry.as_ref())
            .map_or(Opening::Disabled, |handler| {
                handler.shared_history_opening()
            })
    }

    /// Only the explicitly configured joined opener enters here. The entire
    /// compound record is classified before a file is synced or replaced.
    pub(in crate::store) fn select_joined_pin_catalog(
        &self,
    ) -> Result<ColdJoinedSelection, AppError> {
        let supports_review = self
            .current_layout_policy
            .as_ref()
            .and_then(|policy| policy.native_registry.as_ref())
            .is_some_and(|handler| handler.accepts_review_recovery());
        let receiver = if supports_review {
            review_receive::recover(self)?
        } else {
            review_receive::require_no_claim_stage(self)?;
            None
        };
        let review = if receiver.is_some() {
            None
        } else if supports_review {
            review_hold::recover(self)?
        } else {
            review_hold::require_no_scratch(self)?;
            None
        };
        let recovered = if receiver.is_none()
            && review.is_none()
            && self.shared_history_opening() != Opening::Disabled
        {
            shared_history::recover_declaration(self)?
        } else {
            None
        };
        let selected = self.select_joined_pin_catalog_with(None)?;
        if let Some(recovered) = recovered {
            recovered.require(self, &selected)?;
        }
        if let Some(review) = review {
            review.require(self, &selected)?;
        }
        if let Some((file, bytes)) = receiver {
            if session_id(&file).map_err(io)? != session_id(&selected.current).map_err(io)?
                || bytes != selected.bytes
                || selected.review_receive.is_none()
            {
                return Err(denied());
            }
            selected.require(self)?;
        }
        Ok(selected)
    }

    pub(in crate::store) fn select_warm_joined_pin_catalog(
        &self,
        declaration: &JoinedPinDeclaration,
    ) -> Result<ColdJoinedSelection, AppError> {
        self.select_joined_pin_catalog_with(Some(declaration))
    }

    fn select_joined_pin_catalog_with(
        &self,
        warm: Option<&JoinedPinDeclaration>,
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
        let index_fee = policy.current.archive.pending.pool.try_reserve(INDEX_FEE)?;
        let inventory = self.read_governance_inventory(&fee)?;
        if let Some(warm) = warm {
            warm.require_inventory(self, &inventory)?;
        }
        let current_path = fixed(self, false, critical_namespaces::Slot::Current.name())?;
        let stage_path = fixed(self, false, critical_namespaces::Slot::Stage.name())?;
        let seed_path = fixed(self, false, governance::GOVERNANCE_SEED)?;
        let marker_path = fixed(self, true, governance::GOVERNANCE_MARKER)?;
        let index_path = fixed(self, false, INDEX_NAME)?;
        let (current, raw) = inventory.current.ok_or_else(denied)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let directory = inventory.directory;
        let opening = self.shared_history_opening();
        let (shared_history, review_hold, review_decision, review_receive, review_receive_declared) =
            if opening != Opening::Disabled && inventory.seed.is_none() && inventory.stage.is_none()
            {
                let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?)?);
                let parsed = self.parse_joined_catalog_descriptors(&plain)?;
                let history = parsed
                    .shared_history
                    .map(|descriptor| {
                        shared_history::SelectedSharedHistory::capture(
                            self,
                            descriptor,
                            parsed.layout,
                            &key,
                        )
                    })
                    .transpose()?;
                let review = parsed
                    .review_hold
                    .map(|descriptor| {
                        review_hold::SelectedReviewHold::capture(self, &key, descriptor)
                    })
                    .transpose()?;
                let journal = parsed.review_decision.map(|descriptor| {
                    review_decision::SelectedReviewDecision::capture(self, &key, descriptor)
                });
                let receiver = parsed.review_receive.and_then(|descriptor| {
                    review_receive::SelectedReviewReceiver::capture(self, descriptor, &key).ok()
                });
                (
                    history,
                    review,
                    journal,
                    receiver,
                    parsed.review_receive.is_some(),
                )
            } else {
                (None, None, None, None, false)
            };
        if review_hold.is_none() {
            review_hold::require_absent(self)?;
        }
        if review_decision.is_none() {
            review_decision::require_absent(self)?;
        }
        if !review_receive_declared {
            review_receive::require_absent(self)?;
        }
        if shared_history.is_none() {
            if opening == Opening::Required {
                return Err(denied());
            }
            absent(&fixed(self, false, shared_history::MARKER_NAME)?)?;
            absent(&fixed(self, false, shared_history::DIRECTORY_NAME)?)?;
        }
        let (current, bytes, original_leaves, index) = if let Some((seed, raw_seed)) =
            inventory.seed
        {
            let seed_id = session_id(&seed).map_err(io)?;
            let seed_data = AuthenticatedSeed::open(&key, &raw_seed)?;
            let marker = inventory.marker.ok_or_else(denied)?;
            let marker_id = session_id(&marker.0).map_err(io)?;
            // Warm selection already holds the authenticated leaves. The prior
            // governed declaration can still own the startup inventory slot.
            let (layout, leaves, cold_inventory) = if let Some(warm) = warm {
                (warm.layout, warm.leaves, None)
            } else {
                let inventory = NativeGovernanceInventory::capture(
                    self,
                    policy.native_pool.as_ref().ok_or_else(denied)?,
                )?;
                let layout = inventory.layout(self)?;
                let leaves = DeclarationLeaves::from_frames(
                    &marker.1,
                    inventory.member_marker(),
                    inventory.marker(),
                    inventory.index(),
                )?;
                (layout, leaves, Some(inventory))
            };
            let expected = seed_data.validate(&key, layout, leaves)?;
            let cold_native = cold_inventory
                .map(|inventory| {
                    verify_joined_pin_parent(self, inventory, expected.original, &marker.1)
                })
                .transpose()?;
            let native = warm
                .map(|warm| &warm.native)
                .or(cold_native.as_ref())
                .ok_or_else(denied)?;
            let current_id = session_id(&current).map_err(io)?;
            let before_selection = || {
                self.require_governance_directory_matches_root_if_mounted(&directory)?;
                native.require(self)?;
                exact_held(&marker_path, &marker.0, marker_id, &marker.1)?;
                exact_held(&seed_path, &seed, seed_id, &raw_seed)?;
                exact_held(&current_path, &current, current_id, &raw)
            };
            let candidate_bytes = Zeroizing::new(expected.candidate.to_vec());
            let promote = raw.as_slice() == expected.original;
            if !promote && (raw.as_slice() != expected.candidate || inventory.stage.is_some()) {
                return Err(denied());
            }
            // The index preceded any promotion. Only an authenticated seed on
            // the old catalog can create or complete its initial empty frame.
            let mut index_file = match read(&index_path, expected.initial_index.len())? {
                Some((file, prefix)) if expected.initial_index.starts_with(&prefix) => {
                    Some(CreatedStage {
                        identity: Some(session_id(&file).map_err(io)?),
                        file,
                    })
                }
                Some(_) => return Err(denied()),
                None => None,
            };
            if let Some(warm) = warm {
                if index_file.as_ref().and_then(|file| file.identity)
                    != warm.index.as_ref().and_then(|file| file.identity)
                {
                    return Err(denied());
                }
            }
            let stage = inventory.stage;
            if stage
                .as_ref()
                .is_some_and(|(_, raw)| !expected.candidate.starts_with(raw))
            {
                return Err(denied());
            }
            before_selection()?;
            if promote && index_file.is_none() {
                ensure_native_claim(&index_path, &mut index_file)?;
            }
            if index_file.is_none() {
                return Err(denied());
            }
            ensure(
                &index_path,
                expected.initial_index,
                &mut index_file,
                if promote {
                    ArchiveOutputMode::SeedRepair
                } else {
                    ArchiveOutputMode::SavedExact
                },
                #[cfg(test)]
                0,
                #[cfg(test)]
                0,
            )?;
            sync_directory(&self.dir).map_err(io)?;
            pending::require_pending_names(self, None)?;
            let index = SelectedPinIndex::open(
                self,
                &key,
                layout,
                Some(index_file.as_ref().ok_or_else(denied)?),
                index_fee,
            )?;
            if index.selected.1.as_slice() != expected.initial_index {
                return Err(denied());
            }
            let selected = if promote {
                // The complete seed authorizes this single fixed stage, up to
                // its exact prefix. A stage seen at inventory keeps its handle.
                let mut stage = match stage {
                    Some((file, _)) => Some(CreatedStage {
                        identity: Some(session_id(&file).map_err(io)?),
                        file,
                    }),
                    None => None,
                };
                before_selection()?;
                index.require(self)?;
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
                install_catalog_stage(&stage_path, &current_path).map_err(io)?;
                stage.file
            } else {
                before_selection()?;
                absent(&stage_path)?;
                current
            };
            let durable =
                |error: std::io::Error| AppError::CommittedButNotDurable(error.to_string());
            sync_selected_catalog(&selected).map_err(durable)?;
            sync_directory(&self.dir).map_err(durable)?;
            absent(&stage_path)?;
            index.require(self)?;
            native.require(self)?;
            exact_held(&marker_path, &marker.0, marker_id, &marker.1)?;
            exact_held(
                &current_path,
                &selected,
                session_id(&selected).map_err(io)?,
                &candidate_bytes,
            )?;
            exact_held(&seed_path, &seed, seed_id, &raw_seed)?;
            retire_declaration_claim(&seed_path).map_err(durable)?;
            drop(seed);
            sync_directory(&self.dir).map_err(durable)?;
            let original_leaves = cold_native.map(|native| (marker, native));
            (selected, candidate_bytes, original_leaves, index)
        } else {
            if inventory.stage.is_some() {
                return Err(denied());
            }
            let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?)?);
            let layout = self.parse_joined_pin_catalog(&plain)?;
            let index = if let Some(warm) = warm {
                pending::require_pending_names(self, None)?;
                let expected = warm.index.as_ref().ok_or_else(denied)?;
                let index = SelectedPinIndex::open(self, &key, layout, Some(expected), index_fee)?;
                if index.selected.1 != warm.prepared.initial_index {
                    return Err(denied());
                }
                index
            } else {
                pending::select(
                    self,
                    &key,
                    layout,
                    &directory,
                    (&current, &raw),
                    index_fee,
                    None,
                )?
            };
            (current, raw, None, index)
        };
        if review_hold.is_some() && (!index.index.is_empty() || index.pending.is_some()) {
            return Err(denied());
        }
        let value = ColdJoinedSelection {
            current,
            bytes,
            directory,
            original_leaves,
            index,
            shared_history,
            review_hold,
            review_decision_declared: review_decision.is_some(),
            review_decision: review_decision.and_then(Result::ok),
            review_receive,
            review_receive_declared,
            _fee: fee,
        };
        value.require(self)?;
        if let Some(warm) = warm {
            let durable =
                |error: std::io::Error| AppError::CommittedButNotDurable(error.to_string());
            value.current.sync_all().map_err(durable)?;
            sync_directory(&self.dir).map_err(durable)?;
            warm.require_promoted(self)?;
        }
        Ok(value)
    }

    pub(in crate::store) fn install_warm_joined_pin_selection(
        &self,
        declaration: &JoinedPinDeclaration,
        selected: ColdJoinedSelection,
    ) -> Result<(), AppError> {
        use critical_namespaces::pending_join::Settle;
        let opened = self.inspect_pending_join_catalog()?;
        let file = opened.current.as_ref().ok_or_else(denied)?;
        if !matches!(opened.settle, Settle::None)
            || opened.selected.framed != *blake3::hash(&selected.bytes).as_bytes()
            || session_id(file).map_err(io)? != session_id(&selected.current).map_err(io)?
            || selected.index.pending.is_some()
        {
            return Err(denied());
        }
        opened.validate_original(self)?;
        let mut catalog = self.pending_join_catalog.lock().map_err(|_| denied())?;
        let catalog = catalog.as_mut().ok_or_else(denied)?;
        let mut index = self.joined_pin_index.lock().map_err(|_| denied())?;
        if catalog.pending.is_some()
            || catalog.networks.iter().any(Option::is_some)
            || catalog.current.framed != *blake3::hash(&declaration.original.1).as_bytes()
            || index.is_some()
        {
            return Err(denied());
        }
        // This constructor still owns the whole original physical Store.
        // Both metadata guards precede the last real files and mount checks.
        selected.require(self)?;
        declaration.require_promoted(self)?;
        catalog.current = opened.selected;
        *index = Some(selected.index);
        Ok(())
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

    pub(in crate::store) fn finish_joined_pin_selection(
        &self,
        requirement: JoinedPinOpening,
    ) -> Result<(), AppError> {
        if !self.uses_joined_pin_layout() {
            return if requirement == JoinedPinOpening::Any {
                Ok(())
            } else {
                Err(denied())
            };
        }
        let mut selected = self
            .joined_pin_selection
            .lock()
            .map_err(|_| denied())?
            .take()
            .ok_or_else(denied)?;
        selected.require(self)?;
        match requirement {
            JoinedPinOpening::Any => (),
            JoinedPinOpening::Idle => {
                self.require_joined_pin_without_row(&selected.index, false)?
            }
            JoinedPinOpening::NativeWithoutRow => {
                self.require_joined_pin_without_row(&selected.index, true)?
            }
        }
        if let Some(pending) = selected.index.pending.as_mut() {
            pending.finish_core()?;
        }
        selected.require(self)?;
        if selected.index.pending.is_some() {
            let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
            let bytes = selected
                .bytes
                .capacity()
                .checked_add(16384 + std::mem::size_of::<ColdPendingCatalog>())
                .ok_or_else(denied)?;
            let fee = policy.current.archive.pending.pool.try_reserve(bytes)?;
            selected.index.cold_catalog = Some(ColdPendingCatalog {
                current: (selected.current, selected.bytes),
                directory: selected.directory,
                _fee: fee,
            });
        }
        selected.index.require(self)?;
        let mut index = self.joined_pin_index.lock().map_err(|_| denied())?;
        let mut shared = self.shared_history_mount.lock().map_err(|_| denied())?;
        let mut review = self.review_hold_mount.lock().map_err(|_| denied())?;
        let mut decisions = self.review_decision_mount.lock().map_err(|_| denied())?;
        let mut received = self.review_receive_mount.lock().map_err(|_| denied())?;
        if index.is_some()
            || shared.is_some()
            || review.is_some()
            || decisions.is_some()
            || received.is_some()
        {
            return Err(denied());
        }
        *received = selected.review_receive;
        *decisions = selected.review_decision;
        *review = selected.review_hold;
        *shared = selected.shared_history;
        *index = Some(selected.index);
        Ok(())
    }

    pub(in crate::store) fn has_mounted_review_hold(&self) -> Result<bool, AppError> {
        let review = self.review_hold_mount.try_lock().map_err(|_| denied())?;
        if let Some(owner) = review.as_ref() {
            owner.require(self)?;
        }
        Ok(review.is_some())
    }

    #[cfg(test)]
    pub(in crate::store) fn has_mounted_review_decision(&self) -> Result<bool, AppError> {
        let decisions = self
            .review_decision_mount
            .try_lock()
            .map_err(|_| denied())?;
        if let Some(owner) = decisions.as_ref() {
            owner.require(self)?;
        }
        Ok(decisions.is_some())
    }

    pub(in crate::store) fn require_mounted_shared_history(&self) -> Result<(), AppError> {
        self.validate_mounted_governance_layout()?;
        self.shared_history_mount
            .lock()
            .map_err(|_| denied())?
            .as_ref()
            .ok_or_else(denied)?
            .require(self)
    }

    pub(in crate::store) fn has_mounted_shared_history(&self) -> Result<bool, AppError> {
        Ok(self
            .shared_history_mount
            .try_lock()
            .map_err(|_| denied())?
            .is_some())
    }

    pub(in crate::store) fn selected_joined_pin_context_data(
        &self,
        anchor: &catcoms_replication::chat::join_bindings::GenesisAnchorBindingData,
    ) -> Result<Option<catcoms_sync::V2PendingJoinContextData>, AppError> {
        self.validate_mounted_governance_layout()?;
        let slot = self.joined_pin_index.lock().map_err(|_| denied())?;
        let index = slot.as_ref().ok_or_else(denied)?;
        let result = index.index.pending_context_data(anchor)?;
        index.require(self)?;
        Ok(result)
    }

    pub(in crate::store) fn require_empty_mounted_joined_pins(&self) -> Result<(), AppError> {
        self.require_mounted_joined_pin_without_row(false)
    }

    pub(in crate::store) fn require_mounted_joined_pin_without_row(
        &self,
        allow_native_pending: bool,
    ) -> Result<(), AppError> {
        self.validate_mounted_governance_layout()?;
        let slot = self.joined_pin_index.lock().map_err(|_| denied())?;
        let index = slot.as_ref().ok_or_else(denied)?;
        index.require(self)?;
        self.require_joined_pin_without_row(index, allow_native_pending)
    }

    fn require_joined_pin_without_row(
        &self,
        index: &SelectedPinIndex,
        allow_native_pending: bool,
    ) -> Result<(), AppError> {
        if index.index.is_empty() && index.pending.is_none() {
            Ok(())
        } else if allow_native_pending {
            self.require_native_pre_welcome(index)
        } else {
            Err(denied())
        }
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
        pending::require_pending_names(self, None)?;
        absent(&fixed(self, false, INDEX_NAME)?)?;
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
            || checked.initial_index != prepared.initial_index.as_slice()
        {
            return Err(denied());
        }
        let value = JoinedPinDeclaration {
            original,
            marker,
            directory: inventory.directory,
            native,
            layout,
            leaves,
            prepared,
            seed: None,
            index: None,
            stage: None,
            _fee: fee,
        };
        value.require_original(self)?;
        Ok(value)
    }
}

impl JoinedPinDeclaration {
    fn require_inventory(
        &self,
        store: &ServerStore,
        inventory: &GovernanceInventory,
    ) -> Result<(), AppError> {
        store.require_governance_directory_matches_root(&self.directory)?;
        self.native.require(store)?;
        let current = inventory.current.as_ref().ok_or_else(denied)?;
        let stage = self.stage.as_ref().ok_or_else(denied)?;
        let index_path = fixed(store, false, INDEX_NAME)?;
        exact_retained(
            &index_path,
            self.index.as_ref().ok_or_else(denied)?,
            &self.prepared.initial_index,
        )?;
        let marker = inventory.marker.as_ref().ok_or_else(denied)?;
        if marker.1 != self.marker.1
            || session_id(&marker.0).map_err(io)? != session_id(&self.marker.0).map_err(io)?
        {
            return Err(denied());
        }
        if let Some(seed) = &inventory.seed {
            if seed.1 != self.prepared.seed
                || Some(session_id(&seed.0).map_err(io)?)
                    != self.seed.as_ref().and_then(|file| file.identity)
            {
                return Err(denied());
            }
        }
        match &inventory.stage {
            Some(captured) if inventory.seed.is_some() => {
                if current.1 != self.original.1
                    || session_id(&current.0).map_err(io)?
                        != session_id(&self.original.0).map_err(io)?
                    || captured.1 != self.prepared.candidate
                    || Some(session_id(&captured.0).map_err(io)?) != stage.identity
                {
                    return Err(denied());
                }
            }
            None if current.1 == self.prepared.candidate
                && Some(session_id(&current.0).map_err(io)?) == stage.identity => {}
            _ => return Err(denied()),
        }
        Ok(())
    }

    pub(in crate::store) fn require_promoted(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_governance_directory_matches_root(&self.directory)?;
        self.native.require(store)?;
        absent(&fixed(store, false, governance::GOVERNANCE_SEED)?)?;
        absent(&fixed(
            store,
            false,
            critical_namespaces::Slot::Stage.name(),
        )?)?;
        pending::require_pending_names(store, None)?;
        exact_held(
            &fixed(store, true, governance::GOVERNANCE_MARKER)?,
            &self.marker.0,
            session_id(&self.marker.0).map_err(io)?,
            &self.marker.1,
        )?;
        exact_retained(
            &fixed(store, false, INDEX_NAME)?,
            self.index.as_ref().ok_or_else(denied)?,
            &self.prepared.initial_index,
        )?;
        exact_retained(
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            self.stage.as_ref().ok_or_else(denied)?,
            &self.prepared.candidate,
        )
    }

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
        let index_path = fixed(store, false, INDEX_NAME)?;
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
        ensure_native_claim(&index_path, &mut self.index)?;
        ensure(
            &index_path,
            &self.prepared.initial_index,
            &mut self.index,
            ArchiveOutputMode::Original,
            #[cfg(test)]
            0,
            #[cfg(test)]
            0,
        )?;
        sync_directory(&store.dir).map_err(io)?;
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
            &index_path,
            self.index.as_ref().ok_or_else(denied)?,
            &self.prepared.initial_index,
        )?;
        exact_retained(
            &stage_path,
            self.stage.as_ref().ok_or_else(denied)?,
            &self.prepared.candidate,
        )
    }
}
