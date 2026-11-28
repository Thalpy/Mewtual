//! Declare the recipient journal before its first mount. The shared
//! governance claim fences older readers until Current carries ID12.
use super::{initial::InitialManifest, manifest::ManifestKey, write::cover_file, *};
use crate::store::{
    critical_namespaces::{
        native_registry::VerifiedRecipientReviewParent,
        review_receive_namespace::bootstrap::{
            PreparedReceiverDeclaration, ReceiverDeclarationData, ReceiverDeclarationSeed,
            RecipientParent, MAX_SEED_BYTES,
        },
    },
    persistence::v2_member_outbox::fs::{
        create_namespace, observe, observe_directory, remove_listed, Presence, WriteMode,
    },
};
use catcoms_rt::CryptoRngCore;

const SCRATCH_NAME: &str = "chat-critical-namespaces.review-receiver.claim";
const _: () = assert!(MAX_SEED_BYTES <= governance::codec::MAX_SEED_FRAME);

type CoveredReceiver = (OpenedArchiveObject, Option<SelectedReviewReceiver>);

struct Parent {
    current: OpenedArchiveObject,
    pins: OpenedArchiveObject,
    governance: OpenedArchiveObject,
    history: shared_history::SelectedSharedHistory,
    native: VerifiedRecipientReviewParent,
    directory: GovernanceDirectoryCapture,
    root: HeldDirectory,
    _fee: CatalogReadReservation,
}

impl Parent {
    fn capture(
        store: &ServerStore,
        inventory: GovernanceInventory,
        key: &[u8; 32],
        fee: CatalogReadReservation,
    ) -> Result<Self, AppError> {
        review_hold::require_no_scratch(store)?;
        review_hold::require_absent(store)?;
        review_decision::require_absent(store)?;
        let current = inventory.current.ok_or_else(denied)?;
        let governance = inventory.marker.ok_or_else(denied)?;
        let native = VerifiedRecipientReviewParent::capture(store)?;
        if native.current() != (current.1.as_slice(), session_id(&current.0).map_err(io)?) {
            return Err(denied());
        }
        let plain = Zeroizing::new(unseal(key, &unframe(&current.1)?)?);
        let parsed = store.parse_joined_catalog_descriptors(&plain)?;
        if parsed.layout != native.layout || parsed.rejoin_declared {
            return Err(denied());
        }
        governance::codec::require_marker(key, parsed.governance, &governance.1)?;
        let history = shared_history::SelectedSharedHistory::capture(
            store,
            parsed.shared_history.ok_or_else(denied)?,
            native.layout,
            key,
        )?;
        let pins = read(&fixed(store, false, INDEX_NAME)?, MAX_INDEX_FRAME)?.ok_or_else(denied)?;
        JoinedPinIndex::open(key, native.layout, &pins.1)?
            .require_original_context(&native.group, native.context)?;
        let root = HeldDirectory::open(store.dir.clone())?;
        let parent = Self {
            current,
            pins,
            governance,
            history,
            native,
            directory: inventory.directory,
            root,
            _fee: fee,
        };
        parent.require_current(store)?;
        Ok(parent)
    }

    fn data(&self) -> RecipientParent {
        RecipientParent {
            layout: self.native.layout,
            joined: *blake3::hash(&self.pins.1).as_bytes(),
            native: self.native.digest(),
            history: self.history.marker_digest(),
        }
    }

    fn require_leaves(&self, store: &ServerStore) -> Result<(), AppError> {
        self.root.require()?;
        store.require_governance_directory_matches_root_if_mounted(&self.directory)?;
        self.native.require_preserved(store)?;
        self.history.require(store)?;
        require_object(&fixed(store, false, INDEX_NAME)?, &self.pins)?;
        require_object(
            &fixed(store, true, governance::GOVERNANCE_MARKER)?,
            &self.governance,
        )?;
        self.root.require()
    }

    fn require_current(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_leaves(store)?;
        require_object(
            &self.root.child(critical_namespaces::Slot::Current.name()),
            &self.current,
        )
    }
}

fn require_object(path: &Path, (file, bytes): &OpenedArchiveObject) -> Result<(), AppError> {
    exact_held(path, file, session_id(file).map_err(io)?, bytes)
}

pub(in crate::store::persistence::governance_layout::joined) fn require_no_claim_stage(
    store: &ServerStore,
) -> Result<(), AppError> {
    absent(&fixed(store, false, SCRATCH_NAME)?)
}

fn prepare_inventory(
    store: &ServerStore,
) -> Result<(CatalogReadReservation, GovernanceInventory), AppError> {
    let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
    policy.require_persistent_capacity()?;
    let fee = policy
        .current
        .archive
        .pending
        .pool
        .try_reserve(governance::codec::DECLARATION_FEE + 3 * MAX_INDEX_FRAME)?;
    let inventory = store.read_governance_inventory(&fee)?;
    Ok((fee, inventory))
}

impl ServerStore {
    /// Called only by the unpublished recovered joined native owner. An
    /// existing ID9/A2 catalog is immutable under this initial declaration.
    pub(in crate::store) fn select_review_receiver_before_native_startup(
        &self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<bool, AppError> {
        Ok(self
            .cover_initial_review_receiver(None, |_| Ok(()), rng)?
            .is_some())
    }

    pub(in crate::store::persistence::governance_layout::joined) fn cover_warm_review_receiver(
        &self,
        original: &OpenedArchiveObject,
        before_write: impl FnOnce(usize) -> Result<(), AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Option<(OpenedArchiveObject, SelectedReviewReceiver)>, AppError> {
        self.cover_initial_review_receiver(Some(original), before_write, rng)?
            .map(|(current, mount)| Ok((current, mount.ok_or_else(denied)?)))
            .transpose()
    }

    fn cover_initial_review_receiver(
        &self,
        warm: Option<&OpenedArchiveObject>,
        before_write: impl FnOnce(usize) -> Result<(), AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Option<CoveredReceiver>, AppError> {
        let (fee, inventory) = prepare_inventory(self)?;
        if inventory.seed.is_some() || inventory.stage.is_some() {
            return Err(denied());
        }
        let (file, current) = inventory.current.as_ref().ok_or_else(denied)?;
        if let Some((original_file, original)) = warm {
            if original != current
                || session_id(original_file).map_err(io)? != session_id(file).map_err(io)?
            {
                return Err(denied());
            }
        }
        let key = Zeroizing::new(self.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(current)?)?);
        let parsed = self.parse_joined_catalog_descriptors(&plain)?;
        if parsed.review_receive.is_some() {
            // A damaged private journal must never make ordinary chat
            // unavailable. The review operation will retry the same owner
            // and return the original storage refusal.
            if warm.is_none() {
                if let Ok(mount) = self.review_receive_mount.try_lock() {
                    if let Some(selected) = mount.as_ref() {
                        let _ = selected.recover_admitted(self);
                    }
                }
            }
            return Ok(None);
        }
        // An unpublished ID9 Stage still binds this exact Current. Leave
        // the optional receiver absent so native startup can return the
        // retired owner that can resume the original rejoin.
        if parsed.rejoin_declared
            || parsed.shared_history.is_none()
            || self.has_rejoin_evidence()?
        {
            return Ok(None);
        }
        require_absent(self)?;
        let parent = Parent::capture(self, inventory, &key, fee)?;
        let prepared = PreparedReceiverDeclaration::prepare(
            &key,
            &parent.data(),
            &parent.current.1,
            |marker, rng| ManifestKey::new(&key, marker).seal_initial(rng),
            rng,
        )?;
        let seed = ReceiverDeclarationSeed::open(&key, &prepared.seed)?;
        let data = seed.validate(&key, &parent.data())?;
        #[cfg(test)]
        if warm.is_some() {
            super::fault::cut(ReceiverFault::CatalogPreflight)?;
        }
        let mount_fee = warm
            .map(|_| {
                SelectedReviewReceiver::reserve_mount(self).map_err(|_| AppError::ChatV2ArchiveBusy)
            })
            .transpose()?;
        parent.require_current(self)?;
        before_write(data.candidate.len())?;
        // The following local transaction owns its exact original Current
        // and every distinct output. Once admitted, finish those bytes even
        // if the caller closes. Its native publication still checks the UI.
        let mut scratch = cover_file(&parent.root, SCRATCH_NAME, &prepared.seed, true)?;
        parent.root.sync()?;
        scratch.require(&parent.root, SCRATCH_NAME, &prepared.seed)?;
        let original =
            read(&parent.root.child(SCRATCH_NAME), MAX_SEED_BYTES)?.ok_or_else(denied)?;
        if original.1 != prepared.seed || session_id(&original.0).map_err(io)? != scratch.identity()
        {
            return Err(denied());
        }
        drop(scratch);
        finish(self, &parent, (&key, data), original, true, mount_fee).map(Some)
    }
}

pub(in crate::store::persistence::governance_layout::joined) fn recover(
    store: &ServerStore,
) -> Result<Option<OpenedArchiveObject>, AppError> {
    let scratch_path = fixed(store, false, SCRATCH_NAME)?;
    let claim_path = fixed(store, false, governance::GOVERNANCE_SEED)?;
    let scratch_exists = observe(&scratch_path)? == Presence::Present;
    if !scratch_exists && observe(&claim_path)? == Presence::Absent {
        return Ok(None);
    }
    let (fee, mut inventory) = prepare_inventory(store)?;
    let (original, scratch) = match (inventory.seed.take(), scratch_exists) {
        (Some(claim), false) if ReceiverDeclarationSeed::recognizes(&claim.1) => (claim, false),
        (None, true) => (
            read(&scratch_path, MAX_SEED_BYTES)?.ok_or_else(denied)?,
            true,
        ),
        (_, true) => return Err(denied()),
        (_, false) => return Ok(None),
    };
    let key = Zeroizing::new(store.keys.db_key()?);
    if scratch && ReceiverDeclarationSeed::incomplete_scratch(&original.1) {
        // A known torn first scratch could not have admitted any effect.
        // Confirm the untouched joined baseline and all negative names
        // before retiring precisely this unselected file.
        if inventory.stage.is_some() {
            return Err(denied());
        }
        let parent = Parent::capture(store, inventory, &key, fee)?;
        let plain = Zeroizing::new(unseal(&key, &unframe(&parent.current.1)?)?);
        if store
            .parse_joined_catalog_descriptors(&plain)?
            .review_receive
            .is_some()
        {
            return Err(denied());
        }
        require_unstarted(&parent.root)?;
        parent.require_current(store)?;
        require_object(&scratch_path, &original)?;
        remove_listed(&scratch_path)?;
        drop(original);
        parent.root.sync()?;
        absent(&scratch_path)?;
        require_unstarted(&parent.root)?;
        return Ok(None);
    }
    let seed = ReceiverDeclarationSeed::open(&key, &original.1)?;
    let parent = Parent::capture(store, inventory, &key, fee)?;
    let data = seed.validate(&key, &parent.data())?;
    finish(store, &parent, (&key, data), original, scratch, None).map(|(current, _)| Some(current))
}

fn require_unstarted(root: &HeldDirectory) -> Result<(), AppError> {
    for name in [
        governance::GOVERNANCE_SEED,
        critical_namespaces::Slot::Stage.name(),
        MARKER_NAME,
        DIRECTORY_NAME,
    ] {
        absent(&root.child(name))?;
    }
    root.require()
}

fn finish(
    store: &ServerStore,
    parent: &Parent,
    (key, data): (&[u8; 32], ReceiverDeclarationData<'_>),
    claim: OpenedArchiveObject,
    scratch: bool,
    mount_fee: Option<CatalogReadReservation>,
) -> Result<CoveredReceiver, AppError> {
    let root = &parent.root;
    let claim_path = root.child(governance::GOVERNANCE_SEED);
    let scratch_path = root.child(SCRATCH_NAME);
    let current_name = critical_namespaces::Slot::Current.name();
    let stage_name = critical_namespaces::Slot::Stage.name();
    let stage_path = root.child(stage_name);
    let selected = parent.current.1.as_slice() == data.candidate;
    if !selected && parent.current.1.as_slice() != data.original {
        return Err(denied());
    }
    let has_stage = observe(&stage_path)? == Presence::Present;
    if selected && (has_stage || scratch) {
        return Err(denied());
    }
    if scratch {
        require_unstarted(root)?;
        parent.require_current(store)?;
        require_object(&scratch_path, &claim)?;
        install_catalog_stage(&scratch_path, &claim_path).map_err(io)?;
        root.sync()?;
        #[cfg(test)]
        super::fault::cut(ReceiverFault::CatalogClaim)?;
    }
    absent(&scratch_path)?;
    require_object(&claim_path, &claim)?;

    let can_create_outputs = !selected && !has_stage;
    let mut marker = cover_file(root, MARKER_NAME, &data.marker, can_create_outputs)?;
    let path = root.child(DIRECTORY_NAME);
    if observe_directory(&path)? == Presence::Absent {
        if !can_create_outputs {
            return Err(denied());
        }
        parent.require_current(store)?;
        require_object(&claim_path, &claim)?;
        create_namespace(&path)?;
        root.sync()?;
    }
    let directory = HeldDirectory::open(path)?;
    let mode =
        if can_create_outputs && observe(&directory.child(MANIFEST_NAME))? == Presence::Absent {
            WriteMode::Create
        } else {
            WriteMode::Existing
        };
    let mut initial = InitialManifest::open(
        &directory,
        &ManifestKey::new(key, &data.marker),
        data.initial,
        mode,
    )?;
    if can_create_outputs {
        initial.cover(&directory, data.initial)?;
    } else {
        initial.require(&directory, data.initial)?;
    }
    root.sync()?;

    let mut installed = if selected {
        cover_file(root, current_name, data.candidate, false)?
    } else {
        parent.require_current(store)?;
        require_object(&claim_path, &claim)?;
        let mut stage = cover_file(root, stage_name, data.candidate, true)?;
        root.sync()?;
        #[cfg(test)]
        super::fault::cut(ReceiverFault::CatalogStage)?;
        marker.require(root, MARKER_NAME, &data.marker)?;
        initial.require(&directory, data.initial)?;
        parent.require_current(store)?;
        require_object(&claim_path, &claim)?;
        stage.require(root, stage_name, data.candidate)?;
        install_catalog_stage(&stage_path, &root.child(current_name)).map_err(io)?;
        stage
    };
    root.sync()?;
    #[cfg(test)]
    super::fault::cut(ReceiverFault::CatalogSelected)?;
    parent.require_leaves(store)?;
    marker.require(root, MARKER_NAME, &data.marker)?;
    initial.require(&directory, data.initial)?;
    installed.require(root, current_name, data.candidate)?;
    absent(&stage_path)?;
    require_object(&claim_path, &claim)?;
    remove_listed(&claim_path)?;
    drop(claim);
    root.sync()?;
    absent(&claim_path)?;
    let current = read(&root.child(current_name), data.candidate.len())?.ok_or_else(denied)?;
    if current.1.as_slice() != data.candidate
        || session_id(&current.0).map_err(io)? != installed.identity()
    {
        return Err(denied());
    }
    let mount = if let Some(fee) = mount_fee {
        let plaintext = Zeroizing::new(unseal(key, &unframe(&current.1)?)?);
        let parsed = store.parse_joined_catalog_descriptors(&plaintext)?;
        let descriptor = parsed.review_receive.ok_or_else(denied)?;
        let mount = SelectedReviewReceiver::capture_prepaid(store, descriptor, key, fee)?;
        if session_id(&mount.marker.0).map_err(io)? != marker.identity()
            || !mount.directory.same_original(&directory)
        {
            return Err(denied());
        }
        marker.require(root, MARKER_NAME, &data.marker)?;
        initial.require(&directory, data.initial)?;
        parent.require_leaves(store)?;
        Some(mount)
    } else {
        None
    };
    installed.require(root, current_name, data.candidate)?;
    Ok((current, mount))
}
