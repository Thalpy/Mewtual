//! Private physical staging of the history layout before a native row exists.
//! A staged declaration stays under the predecessor-visible governance claim.
use super::*;
use crate::store::critical_namespaces::{
    native_registry::verify_shared_history_parent, shared_history_namespace::declaration as history,
};

pub(in crate::store) const MARKER_NAME: &str = "chat-critical-namespaces.shared-history.marker";
const _: () = assert!(history::MAX_SEED_FRAME <= governance::codec::MAX_SEED_FRAME);

pub(in crate::store) struct SharedHistoryDeclaration {
    current: OpenedArchiveObject,
    governance: OpenedArchiveObject,
    pins: OpenedArchiveObject,
    directory: GovernanceDirectoryCapture,
    prepared: history::PreparedDeclaration,
    claim: Option<CreatedStage>,
    marker: Option<CreatedStage>,
    stage: Option<CreatedStage>,
    native: VerifiedGovernanceNative,
    _fee: CatalogReadReservation,
}

impl ServerStore {
    /// Keep all the original immutable dependencies before any seed or marker
    /// can be written. The native owner calls this before exposing row work.
    pub(in crate::store) fn prepare_empty_shared_history_declaration(
        &self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<SharedHistoryDeclaration, AppError> {
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        policy.require_persistent_capacity()?;
        if !self.uses_joined_pin_layout() {
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
        absent(&fixed(self, false, MARKER_NAME)?)?;
        let current = inventory.current.ok_or_else(denied)?;
        let governance = inventory.marker.ok_or_else(denied)?;
        let native = NativeGovernanceInventory::capture(
            self,
            policy.native_pool.as_ref().ok_or_else(denied)?,
        )?;
        let layout = native.layout(self)?;
        let pins = {
            let slot = self.joined_pin_index.lock().map_err(|_| denied())?;
            let index = slot.as_ref().ok_or_else(denied)?;
            index.require(self)?;
            if !index.index.is_empty() || index.pending.is_some() {
                return Err(denied());
            }
            (
                index.selected.0.try_clone().map_err(io)?,
                Zeroizing::new(index.selected.1.to_vec()),
            )
        };
        let leaves = DeclarationLeaves::from_frames(
            &governance.1,
            native.member_marker(),
            native.marker(),
            native.index(),
        )?;
        let leaves = history::Leaves::from_frames(leaves, &pins.1)?;
        let native = verify_shared_history_parent(self, native, &current.1, &governance.1)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let prepared =
            history::PreparedDeclaration::prepare(&key, &current.1, layout, leaves, rng)?;
        let seed = history::AuthenticatedSeed::open(&key, &prepared.seed)?;
        let checked = seed.validate(&key, layout, leaves)?;
        if checked.original != current.1.as_slice()
            || checked.candidate != prepared.candidate.as_slice()
            || checked.marker != prepared.marker.as_slice()
        {
            return Err(denied());
        }
        let value = SharedHistoryDeclaration {
            current,
            governance,
            pins,
            directory: inventory.directory,
            prepared,
            claim: None,
            marker: None,
            stage: None,
            native,
            _fee: fee,
        };
        value.require_original(self)?;
        Ok(value)
    }
}

impl SharedHistoryDeclaration {
    fn require_original(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_governance_directory_matches_root(&self.directory)?;
        self.native.require_initial_catalog(store)?;
        pending::require_pending_names(store, None)?;
        for (path, (file, expected)) in [
            (
                fixed(store, false, critical_namespaces::Slot::Current.name())?,
                &self.current,
            ),
            (
                fixed(store, true, governance::GOVERNANCE_MARKER)?,
                &self.governance,
            ),
            (fixed(store, false, INDEX_NAME)?, &self.pins),
        ] {
            exact_held(&path, file, session_id(file).map_err(io)?, expected)?;
        }
        Ok(())
    }

    /// Retain the original claim, marker and stage across every short write.
    /// Promotion is a distinct continuation and must authenticate this tuple.
    pub(in crate::store) fn stage(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let claim = fixed(store, false, governance::GOVERNANCE_SEED)?;
        let marker = fixed(store, false, MARKER_NAME)?;
        let stage = fixed(store, false, critical_namespaces::Slot::Stage.name())?;
        self.require_original(store)?;
        if self.stage.is_none() {
            absent(&stage)?;
        }
        stage_output(store, &claim, &mut self.claim, &self.prepared.seed)?;
        self.require_original(store)?;
        exact_retained(
            &claim,
            self.claim.as_ref().ok_or_else(denied)?,
            &self.prepared.seed,
        )?;
        stage_output(store, &marker, &mut self.marker, &self.prepared.marker)?;
        self.require_original(store)?;
        exact_retained(
            &claim,
            self.claim.as_ref().ok_or_else(denied)?,
            &self.prepared.seed,
        )?;
        exact_retained(
            &marker,
            self.marker.as_ref().ok_or_else(denied)?,
            &self.prepared.marker,
        )?;
        stage_output(store, &stage, &mut self.stage, &self.prepared.candidate)?;
        self.require_original(store)?;
        for (path, held, bytes) in [
            (claim, &self.claim, self.prepared.seed.as_slice()),
            (marker, &self.marker, self.prepared.marker.as_slice()),
            (stage, &self.stage, self.prepared.candidate.as_slice()),
        ] {
            exact_retained(&path, held.as_ref().ok_or_else(denied)?, bytes)?;
        }
        Ok(())
    }
}

fn stage_output(
    store: &ServerStore,
    path: &Path,
    held: &mut Option<CreatedStage>,
    bytes: &[u8],
) -> Result<(), AppError> {
    ensure_native_claim(path, held)?;
    sync_directory(&store.dir).map_err(io)?;
    ensure(
        path,
        bytes,
        held,
        ArchiveOutputMode::Original,
        #[cfg(test)]
        0,
        #[cfg(test)]
        0,
    )?;
    sync_directory(&store.dir).map_err(io)
}
