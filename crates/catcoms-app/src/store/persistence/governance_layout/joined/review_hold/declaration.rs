//! Hold the original authorized founder and the unchanged leaves through
//! every claim/Stage/Current cut of the ID10 declaration.
use super::*;
use crate::store::critical_namespaces::{
    member_outbox_layout::{MEMBER_OUTBOX_MARKER, MEMBER_OUTBOX_PARENT},
    native_registry::VerifiedFounderReviewParent,
    review_hold_namespace::declaration::FounderLeaves,
};

struct DeclarationParent {
    founder: VerifiedFounderReviewParent,
    pins: OpenedArchiveObject,
    history: shared_history::SelectedSharedHistory,
    governance: OpenedArchiveObject,
    outbox: OpenedArchiveObject,
    outbox_parent: HeldDirectory,
}

impl DeclarationParent {
    fn capture(
        store: &ServerStore,
        current: &OpenedArchiveObject,
        governance: OpenedArchiveObject,
        key: &[u8; 32],
    ) -> Result<Self, AppError> {
        let founder = VerifiedFounderReviewParent::capture(store)?;
        if founder.original_catalog() != current.1.as_slice()
            || founder.catalog_identity() != session_id(&current.0).map_err(io)?
        {
            return Err(denied());
        }
        let layout = founder.layout();
        let pins_path = fixed(store, false, INDEX_NAME)?;
        let pins = read(&pins_path, MAX_INDEX_FRAME)?.ok_or_else(denied)?;
        if !JoinedPinIndex::open(key, layout, &pins.1)?.is_empty() {
            return Err(denied());
        }
        pending::require_pending_names(store, None)?;
        let plain = Zeroizing::new(unseal(key, &unframe(&current.1)?)?);
        let (checked, history, _) = store.parse_joined_catalog_descriptors(&plain)?;
        if checked != layout {
            return Err(denied());
        }
        let history = shared_history::SelectedSharedHistory::capture(
            store,
            history.ok_or_else(denied)?,
            layout,
            key,
        )?;
        let outbox = read(
            &fixed(store, true, MEMBER_OUTBOX_MARKER)?,
            critical_namespaces::native_registry::preparation::MAX_MARKER_FRAME,
        )?
        .ok_or_else(denied)?;
        let outbox_parent = HeldDirectory::open(fixed(store, true, MEMBER_OUTBOX_PARENT)?)?;
        founder.require_declared_storage_markers(key, (&governance.1, &outbox.1))?;
        let value = Self {
            founder,
            pins,
            history,
            governance,
            outbox,
            outbox_parent,
        };
        value.require_original(store)?;
        Ok(value)
    }

    fn data(&self) -> FounderLeaves {
        let mut row =
            blake3::Hasher::new_derive_key("catcoms/private-chat-v2-review-declaration-leaves/v1");
        row.update(&self.founder.founder_digest());
        row.update(blake3::hash(&self.governance.1).as_bytes());
        row.update(blake3::hash(&self.outbox.1).as_bytes());
        FounderLeaves {
            row: *row.finalize().as_bytes(),
            index: self.founder.index_digest(),
            pins: *blake3::hash(&self.pins.1).as_bytes(),
            history: self.history.marker_digest(),
        }
    }

    fn require_original(&self, store: &ServerStore) -> Result<(), AppError> {
        self.founder.require_original(store)?;
        self.require_preserved(store)?;
        self.founder.require_original(store)
    }

    fn require_preserved(&self, store: &ServerStore) -> Result<(), AppError> {
        self.founder.require_preserved_leaves(store)?;
        self.history.require(store)?;
        self.outbox_parent.require()?;
        for (root, name, (file, bytes)) in [
            (false, INDEX_NAME, &self.pins),
            (true, governance::GOVERNANCE_MARKER, &self.governance),
            (true, MEMBER_OUTBOX_MARKER, &self.outbox),
        ] {
            exact_held(
                &fixed(store, root, name)?,
                file,
                session_id(file).map_err(io)?,
                bytes,
            )?;
        }
        pending::require_pending_names(store, None)?;
        self.outbox_parent.require()
    }
}

use crate::store::critical_namespaces::review_hold_namespace::declaration::{
    AuthenticatedSeed, PreparedDeclaration, MAX_SEED_FRAME,
};
use shared_history::{reopen_held_output, reopen_output, stage_output};

const CLAIM_STAGE: &str = "chat-critical-namespaces.review-hold.claim";
const _: () = assert!(MAX_SEED_FRAME <= governance::codec::MAX_SEED_FRAME);

pub(in crate::store) struct ReviewHoldDeclaration {
    original: OpenedArchiveObject,
    parent: DeclarationParent,
    vault_directory: GovernanceDirectoryCapture,
    prepared: PreparedDeclaration,
    scratch: Option<CreatedStage>,
    claim: Option<CreatedStage>,
    claim_retired: bool,
    stage: Option<CreatedStage>,
    marker: Option<CreatedStage>,
    directory: Option<HeldDirectory>,
    _fee: CatalogReadReservation,
}

impl ServerStore {
    /// The native caller retains the unpublished founder vault for the
    /// whole declaration. Prepare pays and rechecks all source files
    /// before making a candidate or creating the predecessor-visible claim.
    pub(in crate::store) fn prepare_founder_review_declaration(
        &self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<ReviewHoldDeclaration, AppError> {
        require_absent(self)?;
        absent(&fixed(self, false, CLAIM_STAGE)?)?;
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
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
        let key = Zeroizing::new(self.keys.db_key()?);
        let parent = DeclarationParent::capture(
            self,
            &original,
            inventory.marker.ok_or_else(denied)?,
            &key,
        )?;
        let prepared = PreparedDeclaration::prepare(
            &key,
            &original.1,
            parent.founder.layout(),
            parent.data(),
            rng,
        )?;
        parent.require_original(self)?;
        Ok(ReviewHoldDeclaration {
            original,
            parent,
            vault_directory: inventory.directory,
            prepared,
            scratch: None,
            claim: None,
            claim_retired: false,
            stage: None,
            marker: None,
            directory: None,
            _fee: fee,
        })
    }
}

impl ReviewHoldDeclaration {
    #[cfg(test)]
    pub(in crate::store) fn seed_for_test(&self) -> &[u8] {
        &self.prepared.seed
    }

    #[cfg(test)]
    pub(in crate::store) fn stage_for_test(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let claim = fixed(store, false, governance::GOVERNANCE_SEED)?;
        let stage = fixed(store, false, critical_namespaces::Slot::Stage.name())?;
        self.prepare_outputs(store, &claim, &stage)
    }

    fn require_parent(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_governance_directory_matches_root_if_mounted(&self.vault_directory)?;
        self.parent.require_original(store)?;
        let (file, bytes) = &self.original;
        exact_held(
            &fixed(store, false, critical_namespaces::Slot::Current.name())?,
            file,
            session_id(file).map_err(io)?,
            bytes,
        )
    }

    fn require_claim(&self, path: &Path) -> Result<(), AppError> {
        exact_retained(
            path,
            self.claim.as_ref().ok_or_else(denied)?,
            &self.prepared.seed,
        )
    }

    fn cover_claim(&mut self, store: &ServerStore, claim: &Path) -> Result<(), AppError> {
        let scratch = fixed(store, false, CLAIM_STAGE)?;
        if self.claim.is_none() {
            let moved = self
                .scratch
                .as_ref()
                .and_then(|stage| stage.identity)
                .is_some_and(|id| matches!(session_at(claim, id), SessionAt::Ours));
            if !moved {
                absent(claim)?;
                stage_output(store, &scratch, &mut self.scratch, &self.prepared.seed)?;
                self.require_parent(store)?;
                exact_retained(
                    &scratch,
                    self.scratch.as_ref().ok_or_else(denied)?,
                    &self.prepared.seed,
                )?;
                absent(claim)?;
                install_catalog_stage(&scratch, claim).map_err(io)?;
            }
            self.claim = self.scratch.take();
        }
        absent(&scratch)?;
        stage_output(store, claim, &mut self.claim, &self.prepared.seed)?;
        self.require_claim(claim)
    }

    fn require_outputs(&self, store: &ServerStore, selected: &Path) -> Result<(), AppError> {
        let directory = self.directory.as_ref().ok_or_else(denied)?;
        directory.require()?;
        // No holder may write into the namespace before the claim retires.
        if !bounded_entries(directory, 0)?.is_empty() {
            return Err(denied());
        }
        exact_retained(
            &fixed(store, false, MARKER_NAME)?,
            self.marker.as_ref().ok_or_else(denied)?,
            &self.prepared.marker,
        )?;
        exact_retained(
            selected,
            self.stage.as_ref().ok_or_else(denied)?,
            &self.prepared.candidate,
        )?;
        directory.require()
    }

    fn prepare_outputs(
        &mut self,
        store: &ServerStore,
        claim: &Path,
        stage: &Path,
    ) -> Result<(), AppError> {
        self.require_parent(store)?;
        self.cover_claim(store, claim)?;
        self.require_parent(store)?;
        if self.directory.is_none() {
            let directory = fixed(store, false, DIRECTORY_NAME)?;
            create_namespace(&directory)?;
            self.directory = Some(HeldDirectory::open(directory)?);
        }
        self.directory.as_ref().ok_or_else(denied)?.sync()?;
        sync_directory(&store.dir).map_err(io)?;
        self.require_parent(store)?;
        self.require_claim(claim)?;
        let marker = fixed(store, false, MARKER_NAME)?;
        // The keyed marker is a 32-byte tag. The catalog frame writer
        // requires an encrypted frame's larger minimum length.
        ensure_native_claim(&marker, &mut self.marker)?;
        sync_directory(&store.dir).map_err(io)?;
        let writer = self.marker.as_mut().ok_or_else(denied)?;
        crate::store::persistence::v2_member_outbox::fs::finish_exact_write_on(
            &marker,
            &mut writer.file,
            writer.identity.ok_or_else(denied)?,
            &self.prepared.marker,
        )?;
        sync_directory(&store.dir).map_err(io)?;
        self.require_parent(store)?;
        self.require_claim(claim)?;
        stage_output(store, stage, &mut self.stage, &self.prepared.candidate)?;
        self.require_outputs(store, stage)
    }

    /// Select only the exact recorded Stage. An authenticated cold Stage is
    /// covered again before rename. A retry after rename proves the same
    /// inode as Current, then finishes only the covering and claim retirement.
    pub(in crate::store) fn select(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let claim = fixed(store, false, governance::GOVERNANCE_SEED)?;
        let stage = fixed(store, false, critical_namespaces::Slot::Stage.name())?;
        let current = fixed(store, false, critical_namespaces::Slot::Current.name())?;
        let selected = self
            .stage
            .as_ref()
            .and_then(|stage| stage.identity)
            .is_some_and(|id| matches!(session_at(&current, id), SessionAt::Ours));
        if !selected {
            self.prepare_outputs(store, &claim, &stage)?;
            self.require_parent(store)?;
            self.require_claim(&claim)?;
            self.require_outputs(store, &stage)?;
            install_catalog_stage(&stage, &current).map_err(io)?;
        }
        absent(&stage)?;
        absent(&fixed(store, false, CLAIM_STAGE)?)?;
        if self.claim_retired {
            absent(&claim)?;
        } else {
            self.require_claim(&claim)?;
        }
        self.require_outputs(store, &current)?;
        self.parent.require_preserved(store)?;
        sync_selected_catalog(&self.stage.as_ref().ok_or_else(denied)?.file).map_err(io)?;
        sync_directory(&store.dir).map_err(io)?;
        self.require_outputs(store, &current)?;
        self.parent.require_preserved(store)?;
        if !self.claim_retired {
            self.require_claim(&claim)?;
            retire_declaration_claim(&claim).map_err(io)?;
            self.claim_retired = true;
            drop(self.claim.take());
        }
        absent(&claim)?;
        sync_directory(&store.dir).map_err(io)?;
        absent(&claim)?;
        self.require_outputs(store, &current)?;
        self.parent.require_preserved(store)
    }
}

pub(in crate::store::persistence::governance_layout::joined) fn require_no_scratch(
    store: &ServerStore,
) -> Result<(), AppError> {
    absent(&fixed(store, false, CLAIM_STAGE)?)
}

/// Compact named owners survive through the independent cold catalog mount.
/// The large claim and native-leaf buffers drain before its next reservation.
pub(in crate::store::persistence::governance_layout::joined) struct RecoveredReviewSelection {
    catalog: CreatedStage,
    marker: CreatedStage,
    directory: HeldDirectory,
    pins: File,
    pins_digest: [u8; 32],
    history: shared_history::SelectedSharedHistory,
    digest: [u8; 32],
    _fee: CatalogReadReservation,
}

impl RecoveredReviewSelection {
    pub(in crate::store::persistence::governance_layout::joined) fn require(
        &self,
        store: &ServerStore,
        selected: &ColdJoinedSelection,
    ) -> Result<(), AppError> {
        let review = selected.review_hold.as_ref().ok_or_else(denied)?;
        let history = selected.shared_history.as_ref().ok_or_else(denied)?;
        let (pins, pin_bytes) = &selected.index.selected;
        if !self.history.matches_original(history)?
            || session_id(&self.pins).map_err(io)? != session_id(pins).map_err(io)?
            || self.pins_digest != *blake3::hash(pin_bytes).as_bytes()
            || self.digest != *blake3::hash(&selected.bytes).as_bytes()
            || self.catalog.identity != Some(session_id(&selected.current).map_err(io)?)
            || self.marker.identity != Some(session_id(&review.marker.0).map_err(io)?)
            || !self.directory.same_original(&review.directory)
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
            &review.marker.1,
        )?;
        self.directory.require()?;
        exact_held(
            &fixed(store, false, INDEX_NAME)?,
            &self.pins,
            session_id(&self.pins).map_err(io)?,
            pin_bytes,
        )?;
        self.history.require(store)?;
        review.require(store)
    }
}

/// Pay and authenticate an ID10 claim before the older governance/Stage
/// handlers can classify it. An authentic but inconsistent ID10 is held.
pub(in crate::store::persistence::governance_layout::joined) fn recover(
    store: &ServerStore,
) -> Result<Option<RecoveredReviewSelection>, AppError> {
    let scratch_path = fixed(store, false, CLAIM_STAGE)?;
    let claim_path = fixed(store, false, governance::GOVERNANCE_SEED)?;
    if absent(&scratch_path).is_ok() && absent(&claim_path).is_ok() {
        return Ok(None);
    }
    let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
    let fee = policy
        .current
        .archive
        .pending
        .pool
        .try_reserve(governance::codec::DECLARATION_FEE)?;
    let inventory = store.read_governance_inventory(&fee)?;
    let scratch = read(&scratch_path, MAX_SEED_FRAME)?;
    let (claim_file, claim_raw, staged) = match (inventory.seed, scratch) {
        (Some((file, bytes)), None) => (file, bytes, false),
        (None, Some((file, bytes))) => (file, bytes, true),
        _ => return Err(denied()),
    };
    let key = Zeroizing::new(store.keys.db_key()?);
    let seed = match AuthenticatedSeed::open(&key, &claim_raw) {
        Ok(seed) => seed,
        Err(_)
            if !staged
                && !AuthenticatedSeed::recognizes_frame(&claim_raw)
                && require_absent(store).is_ok() =>
        {
            return Ok(None);
        }
        // Only a truncated private scratch can be retired before any
        // declaration effects. Complete corrupt claims remain held.
        Err(_)
            if staged
                && inventory.stage.is_none()
                && AuthenticatedSeed::is_incomplete_scratch(&claim_raw) =>
        {
            let current = inventory.current.ok_or_else(denied)?;
            let plain = Zeroizing::new(unseal(&key, &unframe(&current.1)?)?);
            if store.parse_joined_catalog_descriptors(&plain)?.2.is_some() {
                return Err(denied());
            }
            let parent = DeclarationParent::capture(
                store,
                &current,
                inventory.marker.ok_or_else(denied)?,
                &key,
            )?;
            let require_unstarted = || -> Result<(), AppError> {
                store.require_governance_directory_matches_root_if_mounted(&inventory.directory)?;
                parent.require_original(store)?;
                require_absent(store)?;
                absent(&claim_path)?;
                absent(&fixed(
                    store,
                    false,
                    critical_namespaces::Slot::Stage.name(),
                )?)
            };
            require_unstarted()?;
            exact_held(
                &scratch_path,
                &claim_file,
                session_id(&claim_file).map_err(io)?,
                &claim_raw,
            )?;
            retire_declaration_claim(&scratch_path).map_err(io)?;
            drop(claim_file);
            absent(&scratch_path)?;
            sync_directory(&store.dir).map_err(io)?;
            absent(&scratch_path)?;
            require_unstarted()?;
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let original = inventory.current.ok_or_else(denied)?;
    let parent =
        DeclarationParent::capture(store, &original, inventory.marker.ok_or_else(denied)?, &key)?;
    let opened = seed.validate(&key, parent.founder.layout(), parent.data())?;
    let selected = original.1.as_slice() == opened.candidate;
    if (selected && (staged || inventory.stage.is_some()))
        || (!selected && original.1.as_slice() != opened.original)
    {
        return Err(denied());
    }
    let marker_path = fixed(store, false, MARKER_NAME)?;
    let directory_path = fixed(store, false, DIRECTORY_NAME)?;
    let marker = read(&marker_path, opened.marker.len())?;
    let directory = match observe_directory(&directory_path)? {
        Presence::Present => Some(HeldDirectory::open(directory_path)?),
        Presence::Absent if !selected => None,
        Presence::Absent => return Err(denied()),
    };
    if staged && (marker.is_some() || directory.is_some() || inventory.stage.is_some()) {
        return Err(denied());
    }
    if let Some(directory) = &directory {
        if !bounded_entries(directory, 0)?.is_empty() {
            return Err(denied());
        }
    }
    let marker = reopen_output(&marker_path, marker, &opened.marker, selected)?;
    let stage = if selected {
        let current = fixed(store, false, critical_namespaces::Slot::Current.name())?;
        Some(reopen_held_output(&current, &original.0)?)
    } else {
        let path = fixed(store, false, critical_namespaces::Slot::Stage.name())?;
        reopen_output(&path, inventory.stage, opened.candidate, false)?
    };
    let (scratch, claim) = if staged {
        (Some(reopen_held_output(&scratch_path, &claim_file)?), None)
    } else {
        (None, Some(reopen_held_output(&claim_path, &claim_file)?))
    };
    // The reopened writer already holds the exact original inode. Drop
    // the classification handle before unlink so Windows can retire the name.
    drop(claim_file);
    let mut declaration = ReviewHoldDeclaration {
        original,
        parent,
        vault_directory: inventory.directory,
        prepared: PreparedDeclaration {
            seed: claim_raw,
            candidate: Zeroizing::new(opened.candidate.to_vec()),
            marker: opened.marker,
        },
        scratch,
        claim,
        claim_retired: false,
        stage,
        marker,
        directory,
        _fee: fee,
    };
    let retained = policy.current.archive.pending.pool.try_reserve(16 * 1024)?;
    declaration.select(store)?;
    Ok(Some(RecoveredReviewSelection {
        catalog: declaration.stage.take().ok_or_else(denied)?,
        marker: declaration.marker.take().ok_or_else(denied)?,
        directory: declaration.directory.take().ok_or_else(denied)?,
        pins: declaration.parent.pins.0,
        pins_digest: *blake3::hash(&declaration.parent.pins.1).as_bytes(),
        history: declaration.parent.history,
        digest: *blake3::hash(&declaration.prepared.candidate).as_bytes(),
        _fee: retained,
    }))
}
