//! Private physical staging of the history layout before a native row exists.
//! A staged declaration stays under the predecessor-visible governance claim.
use super::*;
use crate::store::critical_namespaces::{
    native_registry::verify_shared_history_parent, shared_history_namespace::declaration as history,
};
use crate::store::persistence::v2_member_outbox::{
    fs::{create_namespace, HeldDirectory},
    member_outbox_key,
};
use crate::store::persistence::v2_shared_history::{
    disk::{
        SavedSharedOriginal, SelectedHistoryRecord, SharedHistoryDisk, SharedHistoryDiskQuota,
        DISK_FEE,
    },
    SharedHistoryScope,
};
use catcoms_replication::chat::ActionId;
use catcoms_storage::private_chat_page::PrivateChatPageId;

pub(in crate::store) const MARKER_NAME: &str = "chat-critical-namespaces.shared-history.marker";
pub(in crate::store) const DIRECTORY_NAME: &str = "chat-v2-received-history";
const _: () = assert!(history::MAX_SEED_FRAME <= governance::codec::MAX_SEED_FRAME);

/// Selected catalog marker custody retained through the common opener.
pub(in crate::store) struct SelectedSharedHistory {
    marker: OpenedArchiveObject,
    directory: HeldDirectory,
    disk: Option<SharedHistoryDisk>,
    mount: crate::store::chat_admission::PhysicalMountNegative,
    _fee: CatalogReadReservation,
}

impl SelectedSharedHistory {
    pub(super) fn capture(
        store: &ServerStore,
        plain: &[u8],
        layout: CurrentLayoutData,
        key: &[u8; 32],
    ) -> Result<Self, AppError> {
        let descriptor = history::selected_descriptor(plain, layout, true)?;
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let fee = policy.current.archive.pending.pool.try_reserve(16 * 1024)?;
        let path = fixed(store, false, MARKER_NAME)?;
        let directory = HeldDirectory::open(fixed(store, false, DIRECTORY_NAME)?)?;
        let marker = read(&path, history::MARKER_BYTES)?.ok_or_else(denied)?;
        history::validate_marker(key, layout, descriptor, &marker.1)?;
        let value = Self {
            marker,
            directory,
            disk: None,
            mount: store.capture_physical_mount_negative(),
            _fee: fee,
        };
        value.require(store)?;
        Ok(value)
    }

    pub(super) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_original_physical_mount(&self.mount)?;
        self.directory.require()?;
        exact_held(
            &fixed(store, false, MARKER_NAME)?,
            &self.marker.0,
            session_id(&self.marker.0).map_err(io)?,
            &self.marker.1,
        )?;
        self.directory.require()
    }
}

pub(in crate::store) struct SharedHistoryDeclaration {
    current: OpenedArchiveObject,
    governance: OpenedArchiveObject,
    pins: OpenedArchiveObject,
    directory: GovernanceDirectoryCapture,
    prepared: history::PreparedDeclaration,
    archive_directory: Option<HeldDirectory>,
    claim: Option<CreatedStage>,
    marker: Option<CreatedStage>,
    stage: Option<CreatedStage>,
    native: VerifiedGovernanceNative,
    _fee: CatalogReadReservation,
}

impl ServerStore {
    /// Access the one archive scope on the mounted joined layout. The caller
    /// supplies scope DATA derived from its original registered joined row.
    fn with_shared_history<T>(
        &self,
        scope: SharedHistoryScope,
        perform: impl FnOnce(
            &SharedHistoryDisk,
            &[u8; 32],
            CatalogReadReservation,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        if !self.uses_shared_history_layout() || scope.vault != self.chat_vault_domain()? {
            return Err(denied());
        }
        self.validate_mounted_governance_layout()?;
        let mut slot = self.shared_history_mount.try_lock().map_err(|_| denied())?;
        let mounted = slot.as_mut().ok_or_else(denied)?;
        mounted.require(self)?;
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let pool = &policy.current.archive.pending.pool;
        let key = member_outbox_key(self)?;
        if mounted.disk.is_none() {
            let disk = SharedHistoryDisk::from_held(
                mounted.directory.try_clone()?,
                scope,
                SharedHistoryDiskQuota::native(),
                pool.clone(),
                &key,
            )?;
            disk.recover(&key, &self._session, pool.try_reserve(DISK_FEE)?)?;
            mounted.require(self)?;
            mounted.disk = Some(disk);
        }
        let disk = mounted.disk.as_ref().ok_or_else(denied)?;
        disk.require_scope(scope)?;
        let result = perform(disk, &key, pool.try_reserve(DISK_FEE)?)?;
        mounted.require(self)?;
        Ok(result)
    }

    pub(in crate::store) fn save_shared_history_record(
        &self,
        scope: SharedHistoryScope,
        action: ActionId,
        manifest: PrivateChatPageId,
        bundle: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<PrivateChatPageId, AppError> {
        self.with_shared_history(scope, |disk, key, payment| {
            let mut prepared = disk.prepare_save(
                key,
                SelectedHistoryRecord { action, manifest },
                bundle,
                rng,
                &self._session,
                payment,
            )?;
            Ok(prepared.commit(key)?.manifest)
        })
    }

    pub(in crate::store) fn read_shared_history_record(
        &self,
        scope: SharedHistoryScope,
        action: ActionId,
    ) -> Result<Option<SavedSharedOriginal>, AppError> {
        self.with_shared_history(scope, |disk, key, payment| {
            disk.read(key, action, &self._session, payment)
        })
    }

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
        absent(&fixed(self, false, DIRECTORY_NAME)?)?;
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
            archive_directory: None,
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
        self.require_leaves(store)?;
        self.native.require_initial_catalog(store)?;
        let (file, expected) = &self.current;
        exact_held(
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            file,
            session_id(file).map_err(io)?,
            expected,
        )
    }

    fn require_leaves(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_governance_directory_matches_root(&self.directory)?;
        self.native.require(store)?;
        pending::require_pending_names(store, None)?;
        for (path, (file, expected)) in [
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

    /// Select the exact held candidate. A late barrier failure keeps the
    /// same candidate and claim available to a retry on this continuation.
    pub(in crate::store) fn promote(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let current_path = fixed(store, false, critical_namespaces::Slot::Current.name())?;
        let stage_path = fixed(store, false, critical_namespaces::Slot::Stage.name())?;
        let claim_path = fixed(store, false, governance::GOVERNANCE_SEED)?;
        let moved = self
            .stage
            .as_ref()
            .and_then(|stage| stage.identity)
            .is_some_and(|id| matches!(session_at(&current_path, id), SessionAt::Ours));
        if !moved {
            self.stage(store)?;
            #[cfg(test)]
            governance::before_stage_selection_for_test();
            self.require_original(store)?;
            self.require_outputs(store, &stage_path)?;
            exact_retained(
                &claim_path,
                self.claim.as_ref().ok_or_else(denied)?,
                &self.prepared.seed,
            )?;
            self.require_original(store)?;
            let stage = self.stage.as_ref().ok_or_else(denied)?;
            held(&stage_path, &stage.file, stage.identity.ok_or_else(denied)?)?;
            install_catalog_stage(&stage_path, &current_path).map_err(io)?;
        }
        let durable = |error: std::io::Error| AppError::CommittedButNotDurable(error.to_string());
        self.require_leaves(store)?;
        self.require_outputs(store, &current_path)?;
        let stage = self.stage.as_ref().ok_or_else(denied)?;
        sync_selected_catalog(&stage.file).map_err(durable)?;
        sync_directory(&store.dir).map_err(durable)?;
        absent(&stage_path)?;
        self.require_leaves(store)?;
        self.require_outputs(store, &current_path)?;
        let claim = self.claim.as_ref().ok_or_else(denied)?;
        match session_at(&claim_path, claim.identity.ok_or_else(denied)?) {
            SessionAt::Ours => {
                exact_retained(&claim_path, claim, &self.prepared.seed)?;
                retire_declaration_claim(&claim_path).map_err(durable)?;
            }
            SessionAt::Absent => {}
            SessionAt::Other | SessionAt::Unknown => return Err(denied()),
        }
        sync_directory(&store.dir).map_err(durable)?;
        absent(&claim_path)?;
        absent(&stage_path)?;
        self.require_outputs(store, &current_path)
    }

    fn require_outputs(&self, store: &ServerStore, candidate: &Path) -> Result<(), AppError> {
        let directory = self.archive_directory.as_ref().ok_or_else(denied)?;
        directory.require()?;
        exact_retained(
            &fixed(store, false, MARKER_NAME)?,
            self.marker.as_ref().ok_or_else(denied)?,
            &self.prepared.marker,
        )?;
        exact_retained(
            candidate,
            self.stage.as_ref().ok_or_else(denied)?,
            &self.prepared.candidate,
        )?;
        directory.require()
    }

    /// Retain the original claim, marker and stage across every short write.
    /// Promotion is a distinct continuation and must authenticate this tuple.
    pub(in crate::store) fn stage(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let claim = fixed(store, false, governance::GOVERNANCE_SEED)?;
        let marker = fixed(store, false, MARKER_NAME)?;
        let archive = fixed(store, false, DIRECTORY_NAME)?;
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
        if self.archive_directory.is_none() {
            create_namespace(&archive)?;
            self.archive_directory = Some(HeldDirectory::open(archive)?);
        }
        let directory = self.archive_directory.as_ref().ok_or_else(denied)?;
        directory.sync()?;
        sync_directory(&store.dir).map_err(io)?;
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
        directory.require()?;
        self.require_original(store)?;
        self.require_outputs(store, &stage)?;
        exact_retained(
            &claim,
            self.claim.as_ref().ok_or_else(denied)?,
            &self.prepared.seed,
        )
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
