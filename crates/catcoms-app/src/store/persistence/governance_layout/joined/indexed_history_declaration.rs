//! Native pre-row issuance of the indexed companion. The previous archive
//! must be empty and the declaration keeps the original ID8 files in place.
use super::*;
use crate::store::{
    critical_namespaces::{
        indexed_shared_history_namespace::bootstrap::{
            AuthenticatedArchiveSeed, IndexedArchiveDeclaration, IndexedArchiveParent,
            PreparedArchiveDeclaration,
        },
        native_registry::verify_indexed_history_parent,
    },
    persistence::{
        v2_member_outbox::{
            fs::{bounded_entries, finish_exact_write_on},
            member_outbox_key,
        },
        v2_shared_history::{
            prepare_empty_indexed_history_head, verify_empty_indexed_history_head,
            INDEXED_HEAD_CAP, INDEXED_HEAD_NAME,
        },
    },
};
use shared_history::{SelectedSharedHistory, INDEXED_MARKER_NAME};

pub(in crate::store) const CLAIM_STAGE_NAME: &str =
    "chat-critical-namespaces.indexed-shared-history.claim";

const _: () = assert!(
    critical_namespaces::indexed_shared_history_namespace::bootstrap::MAX_SEED
        <= governance::codec::MAX_SEED_FRAME
);

struct Original {
    current: OpenedArchiveObject,
    governance: OpenedArchiveObject,
    pins: OpenedArchiveObject,
    directory: GovernanceDirectoryCapture,
    archive: SelectedSharedHistory,
    archive_directory: crate::store::persistence::v2_member_outbox::fs::HeldDirectory,
    native: VerifiedGovernanceNative,
    layout: CurrentLayoutData,
    cold: bool,
    _fee: CatalogReadReservation,
}

impl Original {
    fn capture(store: &ServerStore, db_key: &[u8; 32]) -> Result<Self, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        policy.require_persistent_capacity()?;
        store.validate_mounted_current_layout()?;
        store.validate_mounted_member_outbox_layout()?;
        store.validate_mounted_governance_layout()?;
        require_no_successor_evidence(store)?;
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(governance::codec::DECLARATION_FEE)?;
        let inventory = store.read_governance_inventory(&fee)?;
        if inventory.seed.is_some() || inventory.stage.is_some() {
            return Err(denied());
        }
        let current = inventory.current.ok_or_else(denied)?;
        let governance = inventory.marker.ok_or_else(denied)?;
        let plain = Zeroizing::new(unseal(db_key, &unframe(&current.1)?)?);
        let parsed = store.parse_joined_catalog_descriptors(&plain)?;
        let native = NativeGovernanceInventory::capture(
            store,
            policy.native_pool.as_ref().ok_or_else(denied)?,
        )?;
        let native = verify_indexed_history_parent(store, native, &current.1, &governance.1)?;
        let pins = {
            let slot = store.joined_pin_index.lock().map_err(|_| denied())?;
            let index = slot.as_ref().ok_or_else(denied)?;
            index.require(store)?;
            if !index.index.is_empty() || index.pending.is_some() {
                return Err(denied());
            }
            (
                index.selected.0.try_clone().map_err(io)?,
                Zeroizing::new(index.selected.1.to_vec()),
            )
        };
        let archive = SelectedSharedHistory::capture(
            store,
            parsed.shared_history.ok_or_else(denied)?,
            parsed.layout,
            db_key,
        )?;
        let archive_directory = archive.clone_original_directory()?;
        let original = Self {
            current,
            governance,
            pins,
            directory: inventory.directory,
            archive,
            archive_directory,
            native,
            layout: parsed.layout,
            cold: false,
            _fee: fee,
        };
        original.require(store, false)?;
        original.require_warm_base(store)?;
        original.require_archive_names(false)?;
        Ok(original)
    }

    fn parent(&self) -> IndexedArchiveParent {
        let tuple = self.native.data();
        let mut native = blake3::Hasher::new_derive_key(
            "catcoms/private-chat-v2-indexed-archive-native-parent/v1",
        );
        native.update(&tuple.index_len.to_le_bytes());
        native.update(&tuple.index);
        native.update(&tuple.marker_len.to_le_bytes());
        native.update(&tuple.marker);
        native.update(&tuple.descriptor);
        IndexedArchiveParent {
            layout: self.layout,
            native_registry: *native.finalize().as_bytes(),
            joined_pins: *blake3::hash(&self.pins.1).as_bytes(),
            original_archive: self.archive.marker_digest(),
        }
    }

    fn require(&self, store: &ServerStore, promoted: bool) -> Result<(), AppError> {
        if self.cold {
            store.require_governance_directory_matches_root_if_mounted(&self.directory)?;
        } else {
            store.require_governance_directory_matches_root(&self.directory)?;
        }
        self.native.require(store)?;
        pending::require_pending_names(store, None)?;
        for (path, (file, bytes)) in [
            (
                fixed(store, true, governance::GOVERNANCE_MARKER)?,
                &self.governance,
            ),
            (fixed(store, false, INDEX_NAME)?, &self.pins),
        ] {
            exact_held(&path, file, session_id(file).map_err(io)?, bytes)?;
        }
        if !promoted {
            self.native.require_initial_catalog(store)?;
            let (file, bytes) = &self.current;
            let path = fixed(store, false, critical_namespaces::Slot::Current.name())?;
            exact_held(&path, file, session_id(file).map_err(io)?, bytes)?;
        }
        self.archive.require(store)?;
        self.archive_directory.require()
    }

    fn require_archive_names(&self, has_head: bool) -> Result<(), AppError> {
        let names = bounded_entries(&self.archive_directory, usize::from(has_head))?;
        match names.as_slice() {
            [] if !has_head => Ok(()),
            [name] if has_head && name == INDEXED_HEAD_NAME => Ok(()),
            _ => Err(denied()),
        }
    }

    /// Warm startup already holds the real ID8 mount. An authentic copy
    /// substituted before this declaration cannot issue the first claim.
    fn require_warm_base(&self, store: &ServerStore) -> Result<(), AppError> {
        if self.cold {
            return Ok(());
        }
        let mounted = store.shared_history_mount.lock().map_err(|_| denied())?;
        let mounted = mounted.as_ref().ok_or_else(denied)?;
        mounted.require(store)?;
        if !mounted.matches_original(&self.archive)? {
            return Err(denied());
        }
        Ok(())
    }
}

/// Held by the unique original native startup continuation before publishing
/// any row. A legacy reader sees the existing governance claim before it
/// can interpret any of the new files as an ordinary ID8 vault.
pub(in crate::store) struct IndexedHistoryDeclaration {
    original: Original,
    parent: IndexedArchiveParent,
    prepared: PreparedArchiveDeclaration,
    claim_stage: Option<CreatedStage>,
    claim: Option<CreatedStage>,
    marker: Option<CreatedStage>,
    head: Option<CreatedStage>,
    stage: Option<CreatedStage>,
    claim_retired: bool,
}

impl ServerStore {
    pub(in crate::store) fn prepare_empty_indexed_history_declaration(
        &self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<IndexedHistoryDeclaration, AppError> {
        let _physical = self._session.retain_for_physical_work();
        absent(&fixed(self, false, CLAIM_STAGE_NAME)?)?;
        absent(&fixed(self, false, INDEXED_MARKER_NAME)?)?;
        let db_key = Zeroizing::new(self.keys.db_key()?);
        let original = Original::capture(self, &db_key)?;
        let parent = original.parent();
        let key = member_outbox_key(self)?;
        let prepared = PreparedArchiveDeclaration::prepare(
            &db_key,
            parent,
            &original.current.1,
            |layout, rng| prepare_empty_indexed_history_head(&key, *layout, rng),
            rng,
        )?;
        original.require(self, false)?;
        Ok(IndexedHistoryDeclaration {
            original,
            parent,
            prepared,
            claim_stage: None,
            claim: None,
            marker: None,
            head: None,
            stage: None,
            claim_retired: false,
        })
    }
}

impl IndexedHistoryDeclaration {
    #[cfg(test)]
    pub(in crate::store) fn prepared_seed_for_test(&self) -> &[u8] {
        &self.prepared.seed
    }

    /// Order the real writes from the predecessor-visible claim through the
    /// exact marker and empty head to the selected catalog. Reusing the
    /// same continuation after a barrier error never resamples bytes.
    pub(in crate::store) fn promote(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let _physical = store._session.retain_for_physical_work();
        let db_key = Zeroizing::new(store.keys.db_key()?);
        let key = member_outbox_key(store)?;
        let seed = AuthenticatedArchiveSeed::open(&db_key, &self.prepared.seed)?;
        let selected = seed.validate(&db_key, self.parent, |layout, frame| {
            verify_empty_indexed_history_head(&key, *layout, frame)
        })?;
        let current = fixed(store, false, critical_namespaces::Slot::Current.name())?;
        let stage = fixed(store, false, critical_namespaces::Slot::Stage.name())?;
        let claim = fixed(store, false, governance::GOVERNANCE_SEED)?;
        let moved = self
            .stage
            .as_ref()
            .and_then(|file| file.identity)
            .is_some_and(|id| matches!(session_at(&current, id), SessionAt::Ours));
        if !moved {
            if selected.original != self.original.current.1.as_slice() {
                return Err(denied());
            }
            self.original.require(store, false)?;
            self.original.require_warm_base(store)?;
            self.original.require_archive_names(self.head.is_some())?;
            require_no_successor_evidence(store)?;
            if self.stage.is_none() {
                absent(&stage)?;
            }
            self.cover_claim(store, &claim)?;
            self.require_preselection(store, &claim)?;
            let marker = fixed(store, false, INDEXED_MARKER_NAME)?;
            self.cover_marker(store, &marker, &selected.marker)?;
            self.require_preselection(store, &claim)?;
            let head = self.original.archive_directory.child(INDEXED_HEAD_NAME);
            shared_history::stage_output(store, &head, &mut self.head, selected.initial_head)?;
            self.original.archive_directory.sync()?;
            self.require_preselection(store, &claim)?;
            shared_history::stage_output(store, &stage, &mut self.stage, selected.candidate)?;
            #[cfg(test)]
            governance::before_stage_selection_for_test();
            self.require_preselection(store, &claim)?;
            self.require_outputs(store, &stage, &selected)?;
            self.require_preselection(store, &claim)?;
            let stage_file = self.stage.as_ref().ok_or_else(denied)?;
            held(
                &stage,
                &stage_file.file,
                stage_file.identity.ok_or_else(denied)?,
            )?;
            install_catalog_stage(&stage, &current).map_err(io)?;
        }
        let durable = |error: std::io::Error| AppError::CommittedButNotDurable(error.to_string());
        self.original.require(store, true)?;
        require_no_successor_evidence(store)?;
        self.require_outputs(store, &current, &selected)?;
        let output = self.stage.as_ref().ok_or_else(denied)?;
        sync_selected_catalog(&output.file).map_err(durable)?;
        sync_directory(&store.dir).map_err(durable)?;
        absent(&stage)?;
        if !self.claim_retired {
            self.original.require(store, true)?;
            self.require_outputs(store, &current, &selected)?;
            require_no_successor_evidence(store)?;
            exact_retained(
                &claim,
                self.claim.as_ref().ok_or_else(denied)?,
                &self.prepared.seed,
            )?;
            retire_declaration_claim(&claim).map_err(durable)?;
            // Windows can keep the name delete-pending until the last handle
            // closes. Keep the distinct promoted state across a sync error.
            drop(self.claim.take());
            self.claim_retired = true;
        }
        sync_directory(&store.dir).map_err(durable)?;
        absent(&claim)?;
        absent(&fixed(store, false, CLAIM_STAGE_NAME)?)?;
        absent(&stage)?;
        self.original.require(store, true)?;
        require_no_successor_evidence(store)?;
        self.require_outputs(store, &current, &selected)
    }

    fn require_preselection(&self, store: &ServerStore, claim: &Path) -> Result<(), AppError> {
        self.original.require(store, false)?;
        self.original.require_warm_base(store)?;
        self.original.require_archive_names(self.head.is_some())?;
        require_no_successor_evidence(store)?;
        exact_retained(
            claim,
            self.claim.as_ref().ok_or_else(denied)?,
            &self.prepared.seed,
        )
    }

    fn cover_marker(
        &mut self,
        store: &ServerStore,
        path: &Path,
        expected: &[u8; 32],
    ) -> Result<(), AppError> {
        // This marker is a fixed keyed commitment. The governance
        // frame writer requires a full AEAD envelope and cannot cover it.
        ensure_native_claim(path, &mut self.marker)?;
        sync_directory(&store.dir).map_err(io)?;
        let marker = self.marker.as_mut().ok_or_else(denied)?;
        let identity = marker.identity.ok_or_else(denied)?;
        finish_exact_write_on(path, &mut marker.file, identity, expected)?;
        sync_directory(&store.dir).map_err(io)
    }

    fn cover_claim(&mut self, store: &ServerStore, claim: &Path) -> Result<(), AppError> {
        let scratch = fixed(store, false, CLAIM_STAGE_NAME)?;
        if self.claim.is_none() {
            let moved = self
                .claim_stage
                .as_ref()
                .and_then(|file| file.identity)
                .is_some_and(|id| matches!(session_at(claim, id), SessionAt::Ours));
            if !moved {
                absent(claim)?;
                shared_history::stage_output(
                    store,
                    &scratch,
                    &mut self.claim_stage,
                    &self.prepared.seed,
                )?;
                self.original.require(store, false)?;
                self.original.require_warm_base(store)?;
                self.original.require_archive_names(false)?;
                require_no_successor_evidence(store)?;
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
        shared_history::stage_output(store, claim, &mut self.claim, &self.prepared.seed)
    }

    fn require_outputs(
        &self,
        store: &ServerStore,
        catalog: &Path,
        selected: &IndexedArchiveDeclaration<'_>,
    ) -> Result<(), AppError> {
        let marker = fixed(store, false, INDEXED_MARKER_NAME)?;
        let head = self.original.archive_directory.child(INDEXED_HEAD_NAME);
        for (path, holder, exact) in [
            (marker.as_path(), &self.marker, selected.marker.as_slice()),
            (head.as_path(), &self.head, selected.initial_head),
            (catalog, &self.stage, selected.candidate),
        ] {
            exact_retained(path, holder.as_ref().ok_or_else(denied)?, exact)?;
        }
        self.original.require_archive_names(true)
    }

    pub(in crate::store) fn require_promoted(&self, store: &ServerStore) -> Result<(), AppError> {
        if !self.claim_retired {
            return Err(denied());
        }
        absent(&fixed(store, false, CLAIM_STAGE_NAME)?)?;
        absent(&fixed(store, false, governance::GOVERNANCE_SEED)?)?;
        absent(&fixed(
            store,
            false,
            critical_namespaces::Slot::Stage.name(),
        )?)?;
        let db_key = Zeroizing::new(store.keys.db_key()?);
        let key = member_outbox_key(store)?;
        let seed = AuthenticatedArchiveSeed::open(&db_key, &self.prepared.seed)?;
        let selected = seed.validate(&db_key, self.parent, |layout, frame| {
            verify_empty_indexed_history_head(&key, *layout, frame)
        })?;
        self.original.require(store, true)?;
        require_no_successor_evidence(store)?;
        self.require_outputs(
            store,
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            &selected,
        )
    }
}

impl ServerStore {
    /// Bind the actual selected output to the same previously unexposed
    /// physical owner. Its original ID8 directory is never replaced.
    pub(in crate::store) fn install_warm_indexed_history(
        &self,
        declaration: &IndexedHistoryDeclaration,
    ) -> Result<(), AppError> {
        use critical_namespaces::pending_join::Settle;
        declaration.require_promoted(self)?;
        let opened = self.inspect_pending_join_catalog()?;
        let current = opened.current.as_ref().ok_or_else(denied)?;
        let stage_id = declaration.stage.as_ref().and_then(|file| file.identity);
        if !matches!(opened.settle, Settle::None)
            || stage_id != Some(session_id(current).map_err(io)?)
        {
            return Err(denied());
        }
        opened.validate_original(self)?;
        let db_key = Zeroizing::new(self.keys.db_key()?);
        let key = member_outbox_key(self)?;
        let seed = AuthenticatedArchiveSeed::open(&db_key, &declaration.prepared.seed)?;
        let selected = seed.validate(&db_key, declaration.parent, |layout, frame| {
            verify_empty_indexed_history_head(&key, *layout, frame)
        })?;
        if *blake3::hash(selected.candidate).as_bytes() != opened.selected.framed {
            return Err(denied());
        }
        let plain = Zeroizing::new(unseal(&db_key, &unframe(selected.candidate)?)?);
        let descriptors = self.parse_joined_catalog_descriptors(&plain)?;
        let mounted = SelectedSharedHistory::capture_indexed(
            self,
            descriptors.shared_history.ok_or_else(denied)?,
            descriptors.indexed_history.ok_or_else(denied)?,
            descriptors.layout,
            &db_key,
        )?;
        let marker = declaration.marker.as_ref().ok_or_else(denied)?;
        let head = declaration.head.as_ref().ok_or_else(denied)?;
        mounted.require_original_indexed_outputs(
            &marker.file,
            &head.file,
            selected.initial_head,
        )?;
        let mut catalog = self.pending_join_catalog.lock().map_err(|_| denied())?;
        let catalog = catalog.as_mut().ok_or_else(denied)?;
        let mut archive = self.shared_history_mount.lock().map_err(|_| denied())?;
        let original = archive.as_ref().ok_or_else(denied)?;
        if catalog.pending.is_some() || catalog.networks.iter().any(Option::is_some) {
            return Err(denied());
        }
        if catalog.current.framed == opened.selected.framed {
            if !original.matches_original(&mounted)? {
                return Err(denied());
            }
            declaration.require_promoted(self)?;
            return original.require(self);
        }
        if catalog.current.framed != *blake3::hash(selected.original).as_bytes()
            || !original.matches_original(&declaration.original.archive)?
        {
            return Err(denied());
        }
        mounted.require(self)?;
        declaration.require_promoted(self)?;
        catalog.current = opened.selected;
        *archive = Some(mounted);
        Ok(())
    }
}

/// Keep the original outputs across the common cold opener. A recovered
/// directory with byte-identical replacement of Current or Head must not
/// be mistaken for the transaction this owner just selected.
pub(super) struct RecoveredIndexedDeclaration {
    catalog: CreatedStage,
    catalog_digest: [u8; 32],
    marker: CreatedStage,
    head: CreatedStage,
    frame: Zeroizing<Vec<u8>>,
    archive_directory: crate::store::persistence::v2_member_outbox::fs::HeldDirectory,
    _fee: CatalogReadReservation,
}

impl RecoveredIndexedDeclaration {
    pub(super) fn require(
        &self,
        store: &ServerStore,
        selected: &ColdJoinedSelection,
    ) -> Result<(), AppError> {
        if *blake3::hash(&selected.bytes).as_bytes() != self.catalog_digest
            || self.catalog.identity != Some(session_id(&selected.current).map_err(io)?)
        {
            return Err(denied());
        }
        let archive = selected.shared_history.as_ref().ok_or_else(denied)?;
        archive.require_original_indexed_outputs(
            &self.marker.file,
            &self.head.file,
            &self.frame,
        )?;
        exact_retained(
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            &self.catalog,
            &selected.bytes,
        )?;
        selected.require(store)?;
        self.archive_directory.require()
    }
}

/// Called only by an explicit indexed-layout opener. Unrelated governance
/// seed domains pass through to their original owners. No output can be
/// reopened writable before the entire authenticated tuple is classified.
pub(super) fn recover(
    store: &ServerStore,
) -> Result<Option<RecoveredIndexedDeclaration>, AppError> {
    let claim_path = fixed(store, false, governance::GOVERNANCE_SEED)?;
    let scratch = fixed(store, false, CLAIM_STAGE_NAME)?;
    if absent(&claim_path).is_ok() && absent(&scratch).is_ok() {
        return Ok(None);
    }
    let _physical = store._session.retain_for_physical_work();
    let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
    let fee = policy
        .current
        .archive
        .pending
        .pool
        .try_reserve(governance::codec::DECLARATION_FEE)?;
    let inventory = store.read_governance_inventory(&fee)?;
    let scratch_file = read(
        &scratch,
        critical_namespaces::indexed_shared_history_namespace::bootstrap::MAX_SEED,
    )?;
    let ((claim_file, claim_bytes), staged) = match (inventory.seed, scratch_file) {
        (Some(canonical), None) => (canonical, false),
        (None, Some(staged)) => (staged, true),
        _ => return Err(denied()),
    };
    let db_key = Zeroizing::new(store.keys.db_key()?);
    let seed = match AuthenticatedArchiveSeed::open(&db_key, &claim_bytes) {
        Ok(seed) => Some(seed),
        Err(_)
            if staged
                && inventory.stage.is_none()
                && AuthenticatedArchiveSeed::is_incomplete_scratch(&db_key, &claim_bytes) =>
        {
            None
        }
        Err(error) if staged => return Err(error),
        Err(_) => return Ok(None),
    };
    require_no_successor_evidence(store)?;
    absent(&fixed(store, false, shared_history::CLAIM_STAGE_NAME)?)?;
    let current = inventory.current.ok_or_else(denied)?;
    let governance = inventory.marker.ok_or_else(denied)?;
    let original_catalog = match &seed {
        Some(seed) => seed.original_catalog()?,
        None => current.1.as_slice(),
    };
    let plain = Zeroizing::new(unseal(&db_key, &unframe(original_catalog)?)?);
    let descriptors = store.parse_joined_catalog_descriptors(&plain)?;
    let layout = descriptors.layout;
    let native =
        NativeGovernanceInventory::capture(store, policy.native_pool.as_ref().ok_or_else(denied)?)?;
    let native = verify_indexed_history_parent(store, native, original_catalog, &governance.1)?;
    let pins = read(&fixed(store, false, INDEX_NAME)?, MAX_INDEX_FRAME)?.ok_or_else(denied)?;
    if !JoinedPinIndex::open(&db_key, layout, &pins.1)?.is_empty() {
        return Err(denied());
    }
    let archive = SelectedSharedHistory::capture(
        store,
        descriptors.shared_history.ok_or_else(denied)?,
        layout,
        &db_key,
    )?;
    let archive_directory = archive.clone_original_directory()?;
    let original = Original {
        current,
        governance,
        pins,
        directory: inventory.directory,
        archive,
        archive_directory,
        native,
        layout,
        cold: true,
        _fee: fee,
    };
    let Some(seed) = seed else {
        // Only the private scratch failed before the real claim. A live
        // exact empty ID8 parent and absence of all competing effects
        // prove that retiring the torn scratch cannot discard a selector.
        let marker = fixed(store, false, INDEXED_MARKER_NAME)?;
        let stage = fixed(store, false, critical_namespaces::Slot::Stage.name())?;
        let id = session_id(&claim_file).map_err(io)?;
        let require_uncovered = || {
            original.require(store, false)?;
            original.require_archive_names(false)?;
            require_no_successor_evidence(store)?;
            absent(&claim_path)?;
            absent(&stage)?;
            absent(&marker)
        };
        require_uncovered()?;
        exact_held(&scratch, &claim_file, id, &claim_bytes)?;
        retire_declaration_claim(&scratch).map_err(io)?;
        drop(claim_file);
        sync_directory(&store.dir).map_err(io)?;
        absent(&scratch)?;
        require_uncovered()?;
        return Ok(None);
    };
    let parent = original.parent();
    let key = member_outbox_key(store)?;
    let checked = seed.validate(&db_key, parent, |layout, frame| {
        verify_empty_indexed_history_head(&key, *layout, frame)
    })?;
    let moved = original.current.1.as_slice() == checked.candidate;
    if (moved && (inventory.stage.is_some() || staged))
        || (!moved && original.current.1.as_slice() != checked.original)
    {
        return Err(denied());
    }
    let marker_path = fixed(store, false, INDEXED_MARKER_NAME)?;
    let marker = read(&marker_path, checked.marker.len())?;
    let head_path = original.archive_directory.child(INDEXED_HEAD_NAME);
    let head = read(&head_path, INDEXED_HEAD_CAP)?;
    if staged && (marker.is_some() || head.is_some() || inventory.stage.is_some()) {
        return Err(denied());
    }
    original.require(store, moved)?;
    original.require_archive_names(head.is_some())?;
    let stage_path = if moved {
        fixed(store, false, critical_namespaces::Slot::Current.name())?
    } else {
        fixed(store, false, critical_namespaces::Slot::Stage.name())?
    };
    let stage = if moved {
        Some(shared_history::reopen_held_output(
            &stage_path,
            &original.current.0,
        )?)
    } else {
        shared_history::reopen_output(&stage_path, inventory.stage, checked.candidate, false)?
    };
    let marker = shared_history::reopen_output(&marker_path, marker, &checked.marker, moved)?;
    let head = shared_history::reopen_output(&head_path, head, checked.initial_head, moved)?;
    let (claim, claim_stage) = if staged {
        (
            None,
            Some(shared_history::reopen_held_output(&scratch, &claim_file)?),
        )
    } else {
        (
            Some(shared_history::reopen_held_output(
                &claim_path,
                &claim_file,
            )?),
            None,
        )
    };
    drop(claim_file);
    let mut declaration = IndexedHistoryDeclaration {
        original,
        parent,
        prepared: PreparedArchiveDeclaration { seed: claim_bytes },
        claim_stage,
        claim,
        marker,
        head,
        stage,
        claim_retired: false,
    };
    let retained = policy.current.archive.pending.pool.try_reserve(16 * 1024)?;
    let catalog_digest = *blake3::hash(checked.candidate).as_bytes();
    let frame = Zeroizing::new(checked.initial_head.to_vec());
    let archive_directory = declaration.original.archive_directory.try_clone()?;
    declaration.promote(store)?;
    Ok(Some(RecoveredIndexedDeclaration {
        catalog: declaration.stage.take().ok_or_else(denied)?,
        catalog_digest,
        marker: declaration.marker.take().ok_or_else(denied)?,
        head: declaration.head.take().ok_or_else(denied)?,
        frame,
        archive_directory,
        _fee: retained,
    }))
}

fn require_no_successor_evidence(store: &ServerStore) -> Result<(), AppError> {
    review_receive::require_no_claim_stage(store)?;
    review_hold::require_no_scratch(store)?;
    review_hold::require_absent(store)?;
    review_decision::require_absent(store)?;
    review_receive::require_absent(store)?;
    if store.has_rejoin_evidence()? {
        return Err(denied());
    }
    Ok(())
}
