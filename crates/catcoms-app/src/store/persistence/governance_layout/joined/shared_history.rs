//! Private physical staging of the history layout before a native row exists.
//! A staged declaration stays under the predecessor-visible governance claim.
use super::*;
use crate::store::critical_namespaces::{
    native_registry::verify_shared_history_parent, read_pool::CriticalCatalogReadPool,
    shared_history_namespace::declaration as history,
};
use crate::store::persistence::v2_member_outbox::{
    fs::{bounded_entries, create_namespace, open_exact_write, HeldDirectory, WriteMode},
    member_outbox_key,
};
use crate::store::persistence::v2_shared_history::{
    disk::{
        PreparedSharedHistorySave, SavedSharedOriginal, SelectedHistoryRecord,
        SelectedSharedHistoryAuditPage, SharedHistoryDisk, SharedHistoryDiskQuota,
        SharedHistoryRelayIndex, DISK_FEE, RELAY_INDEX_FEE,
    },
    ArchiveRelayEntry, SharedHistoryActionPage, SharedHistoryActionQuery, SharedHistoryScope,
};
use catcoms_replication::chat::{ActionId, SignedAction};
use catcoms_rt::RequestCancellation;
use catcoms_storage::private_chat_page::PrivateChatPageId;

pub(in crate::store) const MARKER_NAME: &str = "chat-critical-namespaces.shared-history.marker";
pub(in crate::store) const DIRECTORY_NAME: &str = "chat-v2-received-history";
pub(in crate::store) const CLAIM_STAGE_NAME: &str = "chat-critical-namespaces.shared-history.claim";
#[cfg(test)]
pub(in crate::store) use crate::store::persistence::v2_shared_history::disk::fail_next_history_wal_write_for_test;
const _: () = assert!(history::MAX_SEED_FRAME <= governance::codec::MAX_SEED_FRAME);

/// Selected catalog marker custody retained through the common opener.
pub(in crate::store) struct SelectedSharedHistory {
    marker: OpenedArchiveObject,
    directory: HeldDirectory,
    pending: Option<PreparedSharedHistorySave>,
    disk: Option<SharedHistoryDisk>,
    relay_index: Option<SharedHistoryRelayIndex>,
    mount: crate::store::chat_admission::PhysicalMountNegative,
    _fee: CatalogReadReservation,
}

impl SelectedSharedHistory {
    pub(super) fn capture(
        store: &ServerStore,
        descriptor: &[u8],
        layout: CurrentLayoutData,
        key: &[u8; 32],
    ) -> Result<Self, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let fee = policy.current.archive.pending.pool.try_reserve(16 * 1024)?;
        let path = fixed(store, false, MARKER_NAME)?;
        let directory = HeldDirectory::open(fixed(store, false, DIRECTORY_NAME)?)?;
        let marker = read(&path, history::MARKER_BYTES)?.ok_or_else(denied)?;
        history::validate_marker(key, layout, descriptor, &marker.1)?;
        let value = Self {
            marker,
            directory,
            pending: None,
            disk: None,
            relay_index: None,
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
    claim_stage: Option<CreatedStage>,
    claim: Option<CreatedStage>,
    marker: Option<CreatedStage>,
    stage: Option<CreatedStage>,
    native: VerifiedGovernanceNative,
    cold: bool,
    _fee: CatalogReadReservation,
}

impl ServerStore {
    /// Install the already promoted archive into the original pre-exposure
    /// native owner. The joined pins and the original vault session stay put.
    pub(in crate::store) fn install_warm_shared_history(
        &self,
        declaration: &SharedHistoryDeclaration,
    ) -> Result<(), AppError> {
        use critical_namespaces::pending_join::Settle;
        declaration.require_promoted(self)?;
        let opened = self.inspect_pending_join_catalog()?;
        let file = opened.current.as_ref().ok_or_else(denied)?;
        let stage = declaration.stage.as_ref().ok_or_else(denied)?;
        if !matches!(opened.settle, Settle::None)
            || opened.selected.framed != *blake3::hash(&declaration.prepared.candidate).as_bytes()
            || stage.identity != Some(session_id(file).map_err(io)?)
        {
            return Err(denied());
        }
        opened.validate_original(self)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(&declaration.prepared.candidate)?)?);
        let (layout, descriptor) = self.parse_joined_catalog_descriptors(&plain)?;
        let descriptor = descriptor.ok_or_else(denied)?;
        let mounted = SelectedSharedHistory::capture(self, descriptor, layout, &key)?;
        let expected_marker = declaration.marker.as_ref().ok_or_else(denied)?;
        if expected_marker.identity != Some(session_id(&mounted.marker.0).map_err(io)?) {
            return Err(denied());
        }
        let mut catalog = self.pending_join_catalog.lock().map_err(|_| denied())?;
        let catalog = catalog.as_mut().ok_or_else(denied)?;
        let mut history = self.shared_history_mount.lock().map_err(|_| denied())?;
        if catalog.pending.is_some() || catalog.networks.iter().any(Option::is_some) {
            return Err(denied());
        }
        declaration.require_promoted(self)?;
        if let Some(existing) = history.as_ref() {
            if catalog.current.framed != opened.selected.framed {
                return Err(denied());
            }
            return existing.require(self);
        }
        if catalog.current.framed != *blake3::hash(&declaration.current.1).as_bytes() {
            return Err(denied());
        }
        mounted.require(self)?;
        catalog.current = opened.selected;
        *history = Some(mounted);
        Ok(())
    }

    /// Access the one archive scope on the mounted joined layout. The caller
    /// supplies scope DATA derived from its original registered joined row.
    fn with_shared_history<T>(
        &self,
        scope: SharedHistoryScope,
        perform: impl FnOnce(
            &SharedHistoryDisk,
            &mut Option<PreparedSharedHistorySave>,
            &mut Option<SharedHistoryRelayIndex>,
            &[u8; 32],
            CatalogReadReservation,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        if scope.vault != self.chat_vault_domain()? {
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
        if let Some(previous) = mounted.pending.as_mut() {
            previous.commit(disk, &key)?;
            note_relay_selection(&mut mounted.relay_index, disk, previous.relay_entry());
            mounted.pending = None;
        }
        let result = perform(
            disk,
            &mut mounted.pending,
            &mut mounted.relay_index,
            &key,
            pool.try_reserve(DISK_FEE)?,
        )?;
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
        self.with_shared_history(scope, |disk, pending, relay, key, payment| {
            let prepared = disk.prepare_save(
                key,
                SelectedHistoryRecord { action, manifest },
                bundle,
                rng,
                &self._session,
                payment,
            )?;
            let original = pending.insert(prepared);
            let result = original.commit(disk, key);
            if result.is_ok() {
                note_relay_selection(relay, disk, original.relay_entry());
            }
            if result.is_ok() || !original.has_wal() {
                *pending = None;
            }
            Ok(result?.manifest)
        })
    }

    /// Reconcile one attempted write in the selected local archive.
    /// An unavailable namespace or unresolved WAL is distinct from absence.
    pub(in crate::store) fn selected_shared_history_target(
        &self,
        scope: SharedHistoryScope,
        action: ActionId,
    ) -> Result<Option<ActionId>, AppError> {
        if !self.has_mounted_shared_history()? {
            return Err(denied());
        }
        self.read_shared_history_record(scope, action)
            .map(|record| {
                record.map(|record| {
                    record
                        .original
                        .action()
                        .spec()
                        .body
                        .create()
                        .unwrap_or(action)
                })
            })
    }

    pub(in crate::store) fn read_shared_history_record(
        &self,
        scope: SharedHistoryScope,
        action: ActionId,
    ) -> Result<Option<SavedSharedOriginal>, AppError> {
        self.with_shared_history(scope, |disk, _, _, key, payment| {
            disk.read(key, action, &self._session, payment)
        })
    }

    /// Serve only a selected record. Remote requests must never finish a
    /// local in-flight save or recover a cold WAL to make an answer visible.
    pub(in crate::store) fn read_selected_shared_history_record(
        &self,
        scope: SharedHistoryScope,
        action: ActionId,
    ) -> Result<Option<SavedSharedOriginal>, AppError> {
        self.with_selected_shared_history(scope, |disk, _, key, pool| {
            disk.read(key, action, &self._session, pool.try_reserve(DISK_FEE)?)
        })
    }

    /// Inventory work belongs to the local owner. At most one scan plus
    /// eight exact bundle reads occur on the initial turn. Later turns
    /// resume that scan while concurrent selected writes extend the index.
    pub(in crate::store) fn advance_shared_history_relay_index(
        &self,
        scope: SharedHistoryScope,
    ) -> Result<bool, AppError> {
        self.with_shared_history(scope, |disk, _, relay, key, scratch| {
            if relay.is_none() {
                let payment = scratch.reserve_more(RELAY_INDEX_FEE)?;
                *relay = Some(SharedHistoryRelayIndex::start(
                    disk,
                    &self._session,
                    payment,
                    &scratch,
                )?);
            }
            relay
                .as_mut()
                .ok_or_else(denied)?
                .advance(disk, key, &self._session, &scratch)
        })
    }

    /// Visit the built index under its original selected storage. The
    /// caller owns payment for any filtered results kept after this borrow.
    pub(in crate::store) fn inspect_shared_history_relay_index(
        &self,
        scope: SharedHistoryScope,
        mut inspect: impl FnMut(ArchiveRelayEntry),
    ) -> Result<bool, AppError> {
        self.with_selected_shared_history(scope, |disk, relay, _, _| {
            disk.require_clean_relay()?;
            let Some(entries) = relay.and_then(SharedHistoryRelayIndex::entries) else {
                return Ok(false);
            };
            for entry in entries {
                inspect(entry);
            }
            Ok(true)
        })
    }

    pub(in crate::store) fn visit_selected_shared_history_relay_page(
        &self,
        scope: SharedHistoryScope,
        entries: &[ArchiveRelayEntry],
        require_current: impl Fn() -> Result<(), AppError>,
        visit: impl FnMut(ArchiveRelayEntry, &SignedAction) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        self.with_selected_shared_history(scope, |disk, _, key, pool| {
            disk.visit_relay_page(
                key,
                &self._session,
                &pool.try_reserve(DISK_FEE)?,
                entries,
                require_current,
                visit,
            )
        })
    }

    fn with_selected_shared_history<T>(
        &self,
        scope: SharedHistoryScope,
        perform: impl FnOnce(
            &SharedHistoryDisk,
            Option<&SharedHistoryRelayIndex>,
            &[u8; 32],
            &CriticalCatalogReadPool,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        if scope.vault != self.chat_vault_domain()? {
            return Err(denied());
        }
        self.validate_mounted_governance_layout()?;
        let slot = self.shared_history_mount.try_lock().map_err(|_| denied())?;
        let mounted = slot.as_ref().ok_or_else(denied)?;
        mounted.require(self)?;
        if mounted.pending.is_some() {
            return Err(denied());
        }
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let pool = &policy.current.archive.pending.pool;
        let key = member_outbox_key(self)?;
        let unmounted;
        let disk = if let Some(disk) = &mounted.disk {
            disk
        } else {
            unmounted = SharedHistoryDisk::from_held(
                mounted.directory.try_clone()?,
                scope,
                SharedHistoryDiskQuota::native(),
                pool.clone(),
                &key,
            )?;
            &unmounted
        };
        disk.require_scope(scope)?;
        let result = perform(disk, mounted.relay_index.as_ref(), &key, pool)?;
        mounted.require(self)?;
        Ok(result)
    }

    pub(in crate::store) fn shared_history_action_page(
        &self,
        scope: SharedHistoryScope,
        query: SharedHistoryActionQuery,
    ) -> Result<SharedHistoryActionPage, AppError> {
        self.with_shared_history(scope, |disk, _, _, key, payment| {
            disk.action_page(key, query, &self._session, payment)
        })
    }

    /// Read-only access for the original founder's audit. A pending save
    /// remains the responsibility of the local writer or recovery owner.
    pub(in crate::store) fn selected_shared_history_audit_page(
        &self,
        scope: SharedHistoryScope,
        query: SharedHistoryActionQuery,
        caller: &RequestCancellation,
    ) -> Result<Option<SelectedSharedHistoryAuditPage>, AppError> {
        if !self.has_mounted_shared_history()? {
            return Ok(None);
        }
        self.with_selected_shared_history(scope, |disk, _, key, _| {
            disk.selected_audit_page(key, query, &self._session, caller)
                .map(Some)
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
        absent(&fixed(self, false, CLAIM_STAGE_NAME)?)?;
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
            claim_stage: None,
            claim: None,
            marker: None,
            stage: None,
            native,
            cold: false,
            _fee: fee,
        };
        value.require_original(self)?;
        Ok(value)
    }
}

fn note_relay_selection(
    index: &mut Option<SharedHistoryRelayIndex>,
    disk: &SharedHistoryDisk,
    entry: ArchiveRelayEntry,
) {
    if index
        .as_mut()
        .is_some_and(|index| index.note_selected(entry, disk.relay_quota()).is_err())
    {
        // This cache cannot prevent or turn an already selected write into a
        // failure. Drop inconsistent metadata and let maintenance rebuild it.
        *index = None;
    }
}

impl SharedHistoryDeclaration {
    pub(in crate::store) fn require_promoted(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_leaves(store)?;
        absent(&fixed(store, false, governance::GOVERNANCE_SEED)?)?;
        absent(&fixed(store, false, CLAIM_STAGE_NAME)?)?;
        absent(&fixed(
            store,
            false,
            critical_namespaces::Slot::Stage.name(),
        )?)?;
        self.require_outputs(
            store,
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
        )
    }

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
        if self.cold {
            store.require_governance_directory_matches_root_if_mounted(&self.directory)?;
        } else {
            store.require_governance_directory_matches_root(&self.directory)?;
        }
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
        absent(&fixed(store, false, CLAIM_STAGE_NAME)?)?;
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
        self.cover_claim(store, &claim)?;
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

    fn cover_claim(&mut self, store: &ServerStore, claim: &Path) -> Result<(), AppError> {
        let scratch = fixed(store, false, CLAIM_STAGE_NAME)?;
        if self.claim.is_none() {
            let moved = self
                .claim_stage
                .as_ref()
                .and_then(|stage| stage.identity)
                .is_some_and(|id| matches!(session_at(claim, id), SessionAt::Ours));
            if !moved {
                absent(claim)?;
                stage_output(store, &scratch, &mut self.claim_stage, &self.prepared.seed)?;
                self.require_original(store)?;
                exact_retained(
                    &scratch,
                    self.claim_stage.as_ref().ok_or_else(denied)?,
                    &self.prepared.seed,
                )?;
                absent(claim)?;
                install_catalog_stage(&scratch, claim).map_err(io)?;
            }
            self.claim = self.claim_stage.take();
        }
        absent(&scratch)?;
        stage_output(store, claim, &mut self.claim, &self.prepared.seed)
    }
}

/// Compact original promoted files held across the common cold opener. The
/// complete restart seed and work invoice retire before that opener runs.
pub(super) struct RecoveredSelection {
    catalog: CreatedStage,
    catalog_digest: [u8; 32],
    marker: CreatedStage,
    archive: HeldDirectory,
    _fee: CatalogReadReservation,
}

impl RecoveredSelection {
    pub(super) fn require(
        &self,
        store: &ServerStore,
        selected: &ColdJoinedSelection,
    ) -> Result<(), AppError> {
        let marker = selected.shared_history.as_ref().ok_or_else(denied)?;
        if blake3::hash(&selected.bytes).as_bytes() != &self.catalog_digest
            || self.catalog.identity != Some(session_id(&selected.current).map_err(io)?)
            || self.marker.identity != Some(session_id(&marker.marker.0).map_err(io)?)
        {
            return Err(denied());
        }
        exact_retained(
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            &self.catalog,
            &selected.bytes,
        )?;
        exact_retained(
            &fixed(store, false, MARKER_NAME)?,
            &self.marker,
            &marker.marker.1,
        )?;
        self.archive.require()
    }
}

pub(super) fn recover_declaration(
    store: &ServerStore,
) -> Result<Option<RecoveredSelection>, AppError> {
    let claim_path = fixed(store, false, governance::GOVERNANCE_SEED)?;
    let scratch = fixed(store, false, CLAIM_STAGE_NAME)?;
    if absent(&claim_path).is_ok() && absent(&scratch).is_ok() {
        return Ok(None);
    }
    let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
    let pool = &policy.current.archive.pending.pool;
    let fee = pool.try_reserve(governance::codec::DECLARATION_FEE)?;
    let inventory = store.read_governance_inventory(&fee)?;
    let scratch_file = read(&scratch, history::MAX_SEED_FRAME)?;
    let (claim_file, raw_claim, staged) = match (inventory.seed, scratch_file) {
        (Some((file, raw)), None) => (file, raw, false),
        (None, Some((file, raw))) => (file, raw, true),
        _ => return Err(denied()),
    };
    let key = Zeroizing::new(store.keys.db_key()?);
    let Ok(seed) = history::AuthenticatedSeed::open(&key, &raw_claim) else {
        // The joined-pin inspector separately authenticates its own claim.
        return Ok(None);
    };
    let current = inventory.current.ok_or_else(denied)?;
    let governance = inventory.marker.ok_or_else(denied)?;
    let native =
        NativeGovernanceInventory::capture(store, policy.native_pool.as_ref().ok_or_else(denied)?)?;
    let layout = native.layout(store)?;
    let pins = read(&fixed(store, false, INDEX_NAME)?, MAX_INDEX_FRAME)?.ok_or_else(denied)?;
    if !JoinedPinIndex::open(&key, layout, &pins.1)?.is_empty() {
        return Err(denied());
    }
    let leaves = DeclarationLeaves::from_frames(
        &governance.1,
        native.member_marker(),
        native.marker(),
        native.index(),
    )?;
    let opened = seed.validate(&key, layout, history::Leaves::from_frames(leaves, &pins.1)?)?;
    let native = verify_shared_history_parent(store, native, opened.original, &governance.1)?;
    let moved = current.1.as_slice() == opened.candidate;
    if (moved && (inventory.stage.is_some() || staged))
        || (!moved && current.1.as_slice() != opened.original)
    {
        return Err(denied());
    }
    pending::require_pending_names(store, None)?;
    let marker_path = fixed(store, false, MARKER_NAME)?;
    let archived = read(&marker_path, history::MARKER_BYTES)?;
    let archive_path = fixed(store, false, DIRECTORY_NAME)?;
    if staged && (inventory.stage.is_some() || archived.is_some() || absent(&archive_path).is_err())
    {
        return Err(denied());
    }
    let archive_directory = match fs::symlink_metadata(&archive_path) {
        Ok(_) => {
            let directory = HeldDirectory::open(archive_path)?;
            if !bounded_entries(&directory, 0)?.is_empty() {
                return Err(denied());
            }
            Some(directory)
        }
        Err(error) if !moved && error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(denied()),
    };
    let marker = reopen_output(&marker_path, archived, opened.marker, moved)?;
    let stage = if moved {
        Some(reopen_held_output(
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            &current.0,
        )?)
    } else {
        reopen_output(
            &fixed(store, false, critical_namespaces::Slot::Stage.name())?,
            inventory.stage,
            opened.candidate,
            false,
        )?
    };
    let (claim, claim_stage) = if staged {
        (None, Some(reopen_held_output(&scratch, &claim_file)?))
    } else {
        (Some(reopen_held_output(&claim_path, &claim_file)?), None)
    };
    let mut declaration = SharedHistoryDeclaration {
        current,
        governance,
        pins,
        directory: inventory.directory,
        prepared: history::PreparedDeclaration {
            candidate: Zeroizing::new(opened.candidate.to_vec()),
            marker: Zeroizing::new(opened.marker.to_vec()),
            seed: raw_claim,
        },
        archive_directory,
        claim_stage,
        claim,
        marker,
        stage,
        native,
        cold: true,
        _fee: fee,
    };
    // Prepay the retained original names before the selector crosses its
    // first effect. On failure the authenticated claim remains recoverable.
    let retained = pool.try_reserve(16 * 1024)?;
    declaration.promote(store)?;
    Ok(Some(RecoveredSelection {
        catalog: declaration.stage.take().ok_or_else(denied)?,
        catalog_digest: *blake3::hash(&declaration.prepared.candidate).as_bytes(),
        marker: declaration.marker.take().ok_or_else(denied)?,
        archive: declaration.archive_directory.take().ok_or_else(denied)?,
        _fee: retained,
    }))
}

fn reopen_output(
    path: &Path,
    captured: Option<OpenedArchiveObject>,
    expected: &[u8],
    complete: bool,
) -> Result<Option<CreatedStage>, AppError> {
    match captured {
        Some((file, raw))
            if raw.as_slice() == expected || (!complete && expected.starts_with(&raw)) =>
        {
            reopen_held_output(path, &file).map(Some)
        }
        None if !complete => Ok(None),
        _ => Err(denied()),
    }
}

fn reopen_held_output(path: &Path, captured: &File) -> Result<CreatedStage, AppError> {
    let id = session_id(captured).map_err(io)?;
    held(path, captured, id)?;
    let (file, original) = open_exact_write(path, WriteMode::Existing)?;
    if original != id {
        return Err(denied());
    }
    Ok(CreatedStage {
        file,
        identity: Some(id),
    })
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
