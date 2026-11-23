//! A read-only cold founder role for the original selected first Add.
use super::*;
use crate::store::critical_namespaces::{
    archive_current_codec as genesis, read_pool::CatalogReadReservation,
};
use crate::store::persistence::v2_genesis_activation::require_directory_name;
use crate::store::persistence::v2_group_directory::original_group_path;
use crate::store::persistence::v2_text::{
    capture::original_context_from_genesis,
    control::governed_add::held::require_exact_held,
    head::TextRootData,
    io::{
        archive_child, fixed_child, read_control, read_control_into, OpenedTextControl,
        MAX_TEXT_PATH,
    },
    promotion::seed::parse_data,
    promotion::{page_child, validate_initial_outputs},
};
use crate::store::persistence::{open_archive_directory, OriginalBootstrapReadObservation};
use crate::store::{RetainedBootstrapSourceData, ServerStore};
use catcoms_replication::chat::{
    owner_header::OwnerHeaderId, owner_state::transition::REPLAY_MATH_PRIVATE_INVOICE,
    FounderGenesisAnchorData, ProjectionGroup, ProjectionGroupView, V2GeneralExpectation,
    V2GeneralPrefixVerifier, V2GeneralRetiredData, V2HostSelectionExpectation, V2ProjectionOrigin,
};
use catcoms_storage::kept::{session_id, SessionId};
use catcoms_storage::VaultPhysicalWorkLease;
use catcoms_sync::owner_v2::{PreparedV2FounderRecoveredAdd, V2InstalledFounderAddData};
use catcoms_sync::{V2AdmissionStepData, V2FirstAddProofData, V2FirstJoinDeliveryData};
use std::sync::Arc;
use std::{
    fs,
    fs::File,
    path::{Path, PathBuf},
};

const ABSENT: [&str; 5] = [
    "head.stage",
    "text.seed.stage.v1",
    "text.wal",
    "text.wal.stage.v1",
    selected_head::NAME,
];
const SEED_FRAME_LIMIT: usize = 32768 + 40;

pub(in crate::store) struct HeldColdFounderAdd {
    core: Option<ColdFounderCore>,
    files: ColdFounderFiles,
    recovered: RecoveredGovernedAdd,
    _lease: VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

struct ColdFounderFiles {
    origin: ColdFounderOrigin,
    head: OpenedTextControl,
    archive_path: PathBuf,
    archive: OpenedTextControl,
    selected: selected_head::GovernedHeadData,
}

struct ColdFounderOrigin {
    group: PathBuf,
    directory: File,
    directory_id: SessionId,
    pin: OpenedTextControl,
    seed: OpenedTextControl,
    original: OriginalBootstrapReadObservation,
}

struct OpenedColdFounderOrigin {
    retained: ColdFounderOrigin,
    key: Zeroizing<[u8; 32]>,
    plain: Zeroizing<Vec<u8>>,
}

/// The original selected promoted genesis before any governing Add admission.
/// This proof cannot restore a successor or authorize a new SDK operation.
pub(in crate::store) struct HeldColdFounderGenesis {
    origin: ColdFounderOrigin,
    head: OpenedTextControl,
    pages_path: PathBuf,
    pages: File,
    pages_id: SessionId,
    archive: (PathBuf, OpenedTextControl),
    directory_page: (PathBuf, OpenedTextControl),
    header: OwnerHeaderId,
    _lease: VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

impl HeldColdFounderGenesis {
    pub(in crate::store) fn capture(
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        pin_id: [u8; 32],
    ) -> Result<Self, AppError> {
        const FEE: usize = 8 * SEED_FRAME_LIMIT
            + 12 * MAX_TEXT_PATH
            + 32 * 1024
            + 4 * std::mem::size_of::<HeldColdFounderGenesis>();
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(FEE)?;
        let lease = store._session.retain_for_physical_work();
        let public = genesis_public(source);
        let opened = OpenedColdFounderOrigin::capture(store, source, pin_id, &public)?;
        let (target, _, _) = source.original_target_data();
        require_add_not_entered(target)?;
        let seed = parse_data(&opened.plain)?;
        let head = read_control(&fixed_child(target, "head.bin")?, seed.head_frame.len())?;
        if head.frame.as_slice() != seed.head_frame {
            return Err(refused());
        }
        let manifest = source.data().view_data().context.spec_data().manifest;
        let initial = validate_initial_outputs(&opened.key, &seed, &public, manifest)?;
        let TextRootData::Present(directory_ref) = initial.directory else {
            return Err(refused());
        };
        let pages_path = fixed_child(target, "text.pages")?;
        let pages = open_archive_directory(&pages_path)?;
        let pages_id = session_id(&pages).map_err(|error| AppError::Io(error.to_string()))?;
        require_directory_name(&pages_path, &pages, pages_id)?;
        let archive_path = archive_child(&pages_path, initial.archive.as_bytes())?;
        let archive = read_control(&archive_path, seed.archive_frame.len())?;
        let directory_path = page_child(&pages_path, directory_ref.page.as_bytes())?;
        let directory_page = read_control(&directory_path, seed.directory_frame.len())?;
        if archive.frame.as_slice() != seed.archive_frame
            || directory_page.frame.as_slice() != seed.directory_frame
        {
            return Err(refused());
        }
        let mut result = Self {
            origin: opened.retained,
            head,
            pages_path,
            pages,
            pages_id,
            archive: (archive_path, archive),
            directory_page: (directory_path, directory_page),
            header: public.header.id(),
            _lease: lease,
            _fee: fee,
        };
        result.require_current(store, source)?;
        Ok(result)
    }

    pub(in crate::store) fn require_current(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
    ) -> Result<OwnerHeaderId, AppError> {
        self.origin.require_current(store, source)?;
        let (target, target_file, target_id) = source.original_target_data();
        require_add_not_entered(target)?;
        require_directory_name(&self.pages_path, &self.pages, self.pages_id)?;
        require_exact_held(&fixed_child(target, "head.bin")?, &mut self.head)?;
        require_exact_held(&self.archive.0, &mut self.archive.1)?;
        require_exact_held(&self.directory_page.0, &mut self.directory_page.1)?;
        require_directory_name(&self.pages_path, &self.pages, self.pages_id)?;
        require_directory_name(target, target_file, target_id)?;
        require_directory_name(
            &self.origin.group,
            &self.origin.directory,
            self.origin.directory_id,
        )?;
        require_stages_absent(target)?;
        require_add_not_entered(target)?;
        source.require_original_negative(store)?;
        Ok(self.header)
    }
}

fn require_add_not_entered(target: &Path) -> Result<(), AppError> {
    for name in [
        "text.add.request.v2",
        "text.add.sdk-entry.v2",
        "text.add.outcome.v2",
        "text.add.header-attempt.v2",
        "text.add.raw-header.v2",
        "text.add.governing.v2",
        "text.add.welcome-intent.v2",
        "text.add.welcome-attempt.v2",
        "text.add.raw-welcome.v2",
        "text.add.response.v2",
    ] {
        if !matches!(fs::symlink_metadata(fixed_child(target, name)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound)
        {
            return Err(refused());
        }
    }
    Ok(())
}

impl OpenedColdFounderOrigin {
    fn capture(
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        pin_id: [u8; 32],
        public: &GenesisPublicData<'_>,
    ) -> Result<Self, AppError> {
        store.validate_mounted_current_layout()?;
        source.validate_original(store)?;
        let expected = source.saved_expectation_data(store)?;
        let (target, _, _) = source.original_target_data();
        require_stages_absent(target)?;
        let group = original_group_path(
            store,
            expected.layout_data(),
            expected.context.spec_data().group,
        )?;
        let directory = open_archive_directory(&group)?;
        let directory_id =
            session_id(&directory).map_err(|error| AppError::Io(error.to_string()))?;
        require_directory_name(&group, &directory, directory_id)?;
        let pin = read_control(&group.join("pin.bin"), genesis::MAX_PIN_FRAME)?;
        let actual_pin = genesis::V2GroupPinId::of(&pin.frame);
        if actual_pin.as_bytes() != &pin_id {
            return Err(refused());
        }

        let key = Zeroizing::new(store.keys.db_key()?);
        let seed = read_control(&fixed_child(target, "text.seed")?, SEED_FRAME_LIMIT)?;
        let plain = open_control_data(
            &key,
            TextControlKindData::Seed.id_data(&seed.frame),
            &seed.frame,
        )?;
        let promotion = parse_data(&plain)?;
        let context = original_context_from_genesis(
            &expected,
            promotion.original_head,
            &pin.frame,
            public,
            &key,
        )?;
        let pin_data = genesis::open_pin_data(
            &key,
            expected.layout_data(),
            expected.context.spec_data().group,
            &pin.frame,
        )?;
        genesis::open_head_data(
            &key,
            &expected,
            actual_pin,
            &pin_data,
            promotion.original_head,
        )?
        .require_original_public_data(&pin_data, public)?;
        if promotion.context != context {
            return Err(refused());
        }
        Ok(Self {
            retained: ColdFounderOrigin {
                group,
                directory,
                directory_id,
                pin,
                seed,
                original: source.observe_original(),
            },
            key,
            plain,
        })
    }
}

fn genesis_public(source: &RetainedBootstrapSourceData) -> GenesisPublicData<'_> {
    let data = source.data().genesis_public_data();
    GenesisPublicData {
        profile: data.profile_data(),
        admission: data.admission_data(),
        header: data.header_data(),
        image: data.image_data(),
    }
}

struct ColdFounderCore {
    verifier: V2GeneralPrefixVerifier,
    _retired: V2GeneralRetiredData,
    _view: ProjectionGroupView,
    _group: ProjectionGroup,
    _fee: CatalogReadReservation,
}

impl HeldColdFounderAdd {
    pub(in crate::store) fn covers_request_data(&self, request: &[u8]) -> bool {
        self.recovered.request.as_slice() == request
    }

    pub(in crate::store) fn member_parts(
        &self,
    ) -> Result<(&V2GeneralPrefixVerifier, &ProjectionGroupView), AppError> {
        let core = self.core.as_ref().ok_or_else(refused)?;
        Ok((&core.verifier, &core._view))
    }

    pub(in crate::store) fn with_checked_member_parts<T>(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        perform: impl FnOnce(
            &V2GeneralPrefixVerifier,
            &ProjectionGroupView,
            &mut dyn FnMut() -> Result<(), AppError>,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let core = self.core.as_ref().ok_or_else(refused)?;
        let mut require = || self.files.require_current(store, source).map(drop);
        require()?;
        perform(&core.verifier, &core._view, &mut require)
    }

    pub(in crate::store) fn encode_join_delivery(
        &self,
        source: &RetainedBootstrapSourceData,
    ) -> Result<Vec<u8>, AppError> {
        self.files.origin.original.require_same_original(source)?;
        if self.core.is_none() {
            return Err(refused());
        }
        let genesis = source.data().genesis_public_data();
        let proof = V2FirstAddProofData::encode(
            genesis.profile_data(),
            V2AdmissionStepData {
                admission: genesis.admission_data(),
                header: genesis.header_data(),
                image: genesis.image_data(),
            },
            V2AdmissionStepData {
                admission: &self.recovered.admission,
                header: &self.recovered.header,
                image: self.recovered.image_data()?,
            },
        )?;
        Ok(V2FirstJoinDeliveryData::encode(
            &self.recovered.response,
            &proof,
        )?)
    }

    pub(in crate::store) fn capture(
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        pin_id: [u8; 32],
    ) -> Result<Self, AppError> {
        const BYTES: usize = 6 * (MAX_BYTES + 1)
            + 12 * MAX_TEXT_PATH
            + 4 * SEED_FRAME_LIMIT
            + 256 * 1024
            + REPLAY_MATH_PRIVATE_INVOICE
            + 4 * std::mem::size_of::<HeldColdFounderAdd>();
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(BYTES)?;
        let lease = store._session.retain_for_physical_work();
        let public = genesis_public(source);
        let opened = OpenedColdFounderOrigin::capture(store, source, pin_id, &public)?;
        let (retained, key, plain) = (opened.retained, opened.key, opened.plain);
        let promotion = parse_data(&plain)?;
        let context = &promotion.context;
        let (target, _, _) = source.original_target_data();

        let head = read_control(&fixed_child(target, "head.bin")?, selected_head::FRAME_LEN)?;
        let selected = selected_head::GovernedHeadData::open_data(&key, &head.frame)?;
        if selected.transition.context != context.id_data()?
            || selected.transition.before_head != TextHeadIdData::of_data(promotion.head_frame)
            || selected.transition.before_archive.as_bytes()
                != TextControlKindData::Archive
                    .id_data(promotion.archive_frame)
                    .as_bytes()
        {
            return Err(refused());
        }
        let archive_path = archive_child(target, selected.archive.as_bytes())?;
        let archive = read_control_into(
            &archive_path,
            MAX_BYTES,
            &mut Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1))),
        )?;
        let recovered =
            authenticate_selected(&key, &selected, &archive.frame, context.id_data()?, &public)?;
        let mut result = Self {
            core: None,
            files: ColdFounderFiles {
                origin: retained,
                head,
                archive_path,
                archive,
                selected,
            },
            recovered,
            _lease: lease,
            _fee: fee,
        };
        result.require_current(store, source)?;
        Ok(result)
    }

    pub(in crate::store) fn require_current(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
    ) -> Result<(OwnerHeaderId, u64), AppError> {
        self.files.require_current(store, source)
    }

    pub(in crate::store) fn restore_sdk(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
    ) -> Result<PreparedV2FounderRecoveredAdd, AppError> {
        self.require_current(store, source)?;
        let head = self.files.selected.transition;
        let restored = PreparedV2FounderRecoveredAdd::restore_from_bootstrap(
            source.data(),
            V2InstalledFounderAddData {
                epoch: head.epoch,
                authenticator: head.authenticator,
                header: head.header,
                state: head.state,
            },
            &self.recovered.header,
            self.recovered.image_data()?,
            self.recovered.snapshot_data()?,
            super::held::restore_admission(store)?,
        )?;
        self.require_current(store, source)?;
        Ok(restored)
    }

    pub(in crate::store) fn restore_core(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        sdk: V2InstalledFounderAddData,
    ) -> Result<(OwnerHeaderId, u64), AppError> {
        let selected = self.require_current(store, source)?;
        let head = self.files.selected.transition;
        if sdk.header != head.header
            || sdk.state != head.state
            || sdk.epoch != head.epoch
            || sdk.authenticator != head.authenticator
        {
            return Err(refused());
        }
        let original = source.data().genesis_public_data();
        let founder = original.admission_data().admission_id();
        let joined = self.recovered.admission.admission_id();
        if self.core.is_none() {
            const BYTES: usize = 2 * REPLAY_MATH_PRIVATE_INVOICE
                + V2GeneralPrefixVerifier::metadata_layout_bytes()
                + ProjectionGroup::initial_metadata_layout_bytes()
                + 4 * std::mem::size_of::<ColdFounderCore>();
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            let fee = pool.try_reserve(BYTES)?;
            let profile = original.profile_data();
            let genesis = original.header_data();
            let context = original.context_data().spec_data();
            let mut group = ProjectionGroup::new(context.group).map_err(|_| refused())?;
            let mut view = group.begin_view();
            if !view.try_ready() {
                return Err(refused());
            }
            let origin = V2ProjectionOrigin::new(Arc::new(
                store.chat_mount.capture_cached_physical_negative(),
            ));
            let anchor = FounderGenesisAnchorData {
                group: context.group.to_vec(),
                founder: context.signer,
                profile: profile.id(),
                admission: founder,
                tenure: profile.tenure(),
                header: genesis.id(),
                state: genesis.spec().state,
            };
            let expectation =
                V2GeneralExpectation::from_trusted_host(anchor).map_err(|_| refused())?;
            let mut verifier = V2GeneralPrefixVerifier::from_genesis(
                expectation,
                profile,
                original.admission_data(),
                genesis,
                original.image_data(),
                &view,
                origin,
            )
            .map_err(|_| refused())?;
            let mut candidate = Some(
                verifier
                    .derive_successor(
                        &self.recovered.header,
                        ReplayBody::Admission(&self.recovered.admission),
                        self.recovered.image_data()?,
                    )
                    .map_err(|_| refused())?,
            );
            self.require_current(store, source)?;
            let expectation =
                V2HostSelectionExpectation::from_trusted_host(head.header, head.state);
            let retired = verifier
                .advance_host_selected(&expectation, &mut candidate)
                .map_err(|_| refused())?;
            if retired.parent_data().header().id() != genesis.id() {
                return Err(refused());
            }
            self.core = Some(ColdFounderCore {
                verifier,
                _retired: retired,
                _view: view,
                _group: group,
                _fee: fee,
            });
        }
        let core = self.core.as_ref().ok_or_else(refused)?;
        let replay = core.verifier.replay_data();
        if replay.header().id() != selected.0
            || replay.mls_epoch_data() != selected.1
            || replay.header().spec().state != head.state
        {
            return Err(refused());
        }
        for admission in [founder, joined] {
            core.verifier
                .observe_live_member(admission)
                .map_err(|_| refused())?;
        }
        self.require_current(store, source)
    }
}

impl ColdFounderFiles {
    fn require_current(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
    ) -> Result<(OwnerHeaderId, u64), AppError> {
        self.origin.require_current(store, source)?;
        let (target, directory, id) = source.original_target_data();
        require_directory_name(target, directory, id)?;
        require_exact_held(&self.archive_path, &mut self.archive)?;
        require_exact_held(&fixed_child(target, "head.bin")?, &mut self.head)?;
        require_stages_absent(target)?;
        require_directory_name(target, directory, id)?;
        require_directory_name(
            &self.origin.group,
            &self.origin.directory,
            self.origin.directory_id,
        )?;
        source.require_original_negative(store)?;
        Ok((
            self.selected.transition.header,
            self.selected.transition.epoch,
        ))
    }
}

impl ColdFounderOrigin {
    fn require_current(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        source.validate_original(store)?;
        store.validate_mounted_current_layout()?;
        let (target, directory, id) = source.original_target_data();
        require_directory_name(target, directory, id)?;
        require_directory_name(&self.group, &self.directory, self.directory_id)?;
        require_exact_held(&self.group.join("pin.bin"), &mut self.pin)?;
        require_exact_held(&fixed_child(target, "text.seed")?, &mut self.seed)?;
        require_stages_absent(target)?;
        require_directory_name(target, directory, id)?;
        require_directory_name(&self.group, &self.directory, self.directory_id)?;
        source.require_original_negative(store)
    }
}

fn require_stages_absent(target: &Path) -> Result<(), AppError> {
    for name in ABSENT {
        if !matches!(fs::symlink_metadata(fixed_child(target, name)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound)
        {
            return Err(refused());
        }
    }
    Ok(())
}
