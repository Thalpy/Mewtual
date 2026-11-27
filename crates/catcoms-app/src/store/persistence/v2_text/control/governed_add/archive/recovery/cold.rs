//! A read-only cold founder role for the original selected governing archive.
use super::{chain::RecoveredArchiveChain, *};
use crate::store::critical_namespaces::{
    archive_current_codec as genesis, read_pool::CatalogReadReservation,
};
use crate::store::persistence::v2_genesis_activation::require_directory_name;
use crate::store::persistence::v2_group_directory::original_group_path;
use crate::store::persistence::v2_text::{
    capture::original_context_from_genesis,
    control::governed_add::held::{
        require_exact_held, require_successor_stages_absent, STAGE as GOVERNED_ADD_FENCE,
    },
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
    owner_facts::OwnerFactId, owner_header::OwnerHeaderId,
    owner_state::transition::REPLAY_MATH_PRIVATE_INVOICE, FounderGenesisAnchorData,
    ProjectionGroup, ProjectionGroupView, V2GeneralExpectation, V2GeneralPrefixVerifier,
    V2HostSelectionExpectation, V2ProjectionOrigin,
};
use catcoms_storage::kept::{session_id, SessionId};
use catcoms_storage::VaultPhysicalWorkLease;
use catcoms_sync::owner_v2::{PreparedV2FounderRecoveredAdd, V2InstalledFounderAddData};
use catcoms_sync::{
    V2AdmissionStepData, V2FirstAddProofData, V2FirstJoinDeliveryData, V2GoverningJoinPrefixData,
    V2GoverningJoinStepData,
};
use std::sync::Arc;
use std::{
    fs,
    fs::File,
    path::{Path, PathBuf},
};

const ABSENT: [&str; 4] = [
    "head.stage",
    "text.seed.stage.v1",
    "text.wal",
    selected_head::NAME,
];
const SEED_FRAME_LIMIT: usize = 32768 + 40;

pub(in crate::store) struct HeldColdFounderAdd {
    core: Option<ColdFounderCore>,
    files: ColdFounderFiles,
    recovered: ColdRecoveredArchive,
    _lease: VaultPhysicalWorkLease,
    _chain_fee: Option<CatalogReadReservation>,
    _fee: CatalogReadReservation,
}

#[expect(
    clippy::large_enum_variant,
    reason = "The founder archive read prepays one whole recovered selection inline."
)]
enum ColdRecoveredArchive {
    First(RecoveredGovernedAdd),
    Later(RecoveredArchiveChain),
}

impl ColdRecoveredArchive {
    fn selected_admission(&self) -> Option<OwnerFactId> {
        match self {
            Self::First(first) => Some(first.admission.admission_id()),
            Self::Later(chain) => chain.journal.selected_admission(),
        }
    }

    fn require_predecessors(&self, source: &RetainedBootstrapSourceData) -> Result<(), AppError> {
        if let Self::Later(chain) = self {
            let (directory, _, _) = source.original_target_data();
            chain.require_original_predecessors(directory)?;
        }
        Ok(())
    }
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
    invite_context: [u8; 32],
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
        let data = source.data().genesis_public_data();
        let public = GenesisPublicData {
            profile: data.profile_data(),
            admission: data.admission_data(),
            header: data.header_data(),
            image: data.image_data(),
        };
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
        require_successor_stages_absent(target, TextHeadIdData::of_data(&self.head.frame))?;
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
    // A completed Add retains its fence as evidence. Its presence rules out
    // epoch-zero recovery even when the Add has not written another stage.
    for name in [
        GOVERNED_ADD_FENCE,
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
        selected_head::NAME,
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
        let invite_context = crate::store::persistence::v2_text::invite_base_context(
            &expected.context,
            promotion.original_head,
            expected.context.spec_data().scope_nonce,
            &pin.frame,
        )?;
        Ok(Self {
            retained: ColdFounderOrigin {
                group,
                directory,
                directory_id,
                pin,
                seed,
                original: source.observe_original(),
                invite_context,
            },
            key,
            plain,
        })
    }
}

struct ColdFounderCore {
    verifier: V2GeneralPrefixVerifier,
    _view: ProjectionGroupView,
    _group: ProjectionGroup,
    _fee: CatalogReadReservation,
}

/// Scoped journal labels from the selected cold files. The constructor is
/// private to the verifier that revalidates the original before and after use.
pub(in crate::store) struct ColdInviteParent<'a> {
    store: &'a ServerStore,
    target: &'a Path,
    parent: TextHeadIdData,
    context: [u8; 32],
}

impl<'a> ColdInviteParent<'a> {
    pub(in crate::store) fn store(&self) -> &'a ServerStore {
        self.store
    }

    pub(in crate::store) fn target(&self) -> &'a Path {
        self.target
    }

    pub(in crate::store) fn context_data(&self) -> (TextHeadIdData, [u8; 32]) {
        (self.parent, self.context)
    }
}

impl HeldColdFounderAdd {
    /// Use the existing cold-selected proof without borrowing a warm Current
    /// writer. The native owning loan stays outside these physical checks.
    pub(in crate::store) fn with_checked_invite_parent<T>(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        perform: impl FnOnce(
            &ColdInviteParent<'_>,
            &V2GeneralPrefixVerifier,
            &mut dyn FnMut() -> Result<(), AppError>,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let binding = ColdInviteParent {
            store,
            target: source.original_target_data().0,
            parent: TextHeadIdData::of_data(&self.files.head.frame),
            context: self.files.origin.invite_context,
        };
        self.with_checked_member_parts(store, source, |verifier, _, require| {
            let result = perform(&binding, verifier, require)?;
            require()?;
            Ok(result)
        })
    }

    /// Check the authenticated archive against the earlier warm writer's
    /// exact selected receipt before it can become a successor's parent.
    pub(in crate::store::persistence::v2_text) fn require_installed_parent(
        &self,
        source: &RetainedBootstrapSourceData,
        head: TextHeadIdData,
        installed: V2InstalledFounderAddData,
    ) -> Result<(), AppError> {
        self.files.origin.original.require_same_original(source)?;
        let selected = self.files.selected.transition;
        if TextHeadIdData::of_data(&self.files.head.frame) != head
            || selected.header != installed.header
            || selected.state != installed.state
            || selected.epoch != installed.epoch
            || selected.authenticator != installed.authenticator
        {
            return Err(refused());
        }
        Ok(())
    }

    pub(in crate::store::persistence::v2_text) fn writer_parent_data(
        &self,
    ) -> (
        TextContextIdData,
        TextHeadIdData,
        ArchiveHeadId,
        OwnerHeaderId,
        &[u8],
    ) {
        let selected = self.files.selected;
        (
            selected.transition.context,
            TextHeadIdData::of_data(&self.files.head.frame),
            selected.archive,
            selected.transition.header,
            &self.files.head.frame,
        )
    }

    pub(in crate::store::persistence::v2_text) fn require_writer_parent_current(
        &mut self,
        guard: &crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        guard.require_governed_native_invite_parent()?;
        guard.require_text_control_write_idle_for_query()?;
        let source = guard.source().source();
        self.files.origin.original.require_same_original(source)?;
        let (target, directory, id) = guard.target_directory_data();
        self.require_writer_parent_archive(guard)?;
        require_exact_held(&fixed_child(target, "head.bin")?, &mut self.files.head)?;
        require_directory_name(target, directory, id)?;
        guard.validate_source_only()
    }

    pub(in crate::store::persistence::v2_text) fn require_writer_parent_archive(
        &mut self,
        guard: &crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        let source = guard.source().source();
        self.files.origin.original.require_same_original(source)?;
        self.recovered.require_predecessors(source)?;
        require_exact_held(&self.files.archive_path, &mut self.files.archive)
    }

    pub(in crate::store::persistence::v2_text) fn encode_successor_proof(
        &self,
        genesis: &GenesisPublicData<'_>,
        next: V2GoverningJoinStepData<'_>,
    ) -> Result<Vec<u8>, AppError> {
        match &self.recovered {
            ColdRecoveredArchive::First(first) => {
                let proof = V2FirstAddProofData::encode(
                    genesis.profile,
                    V2AdmissionStepData {
                        admission: genesis.admission,
                        header: genesis.header,
                        image: genesis.image,
                    },
                    V2AdmissionStepData {
                        admission: &first.admission,
                        header: &first.header,
                        image: first.image_data()?,
                    },
                )?;
                Ok(V2GoverningJoinPrefixData::encode(&proof, &[next])?)
            }
            ColdRecoveredArchive::Later(chain) => chain.encode_successor_proof(genesis, next),
        }
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
        let mut require = || {
            self.recovered.require_predecessors(source)?;
            self.files.require_current(store, source).map(drop)
        };
        require()?;
        perform(&core.verifier, &core._view, &mut require)
    }

    pub(in crate::store) fn encode_join_delivery(
        &self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        request: &[u8],
    ) -> Result<Vec<u8>, AppError> {
        self.files.origin.original.require_same_original(source)?;
        if self.core.is_none() {
            return Err(refused());
        }
        let genesis = source.data().genesis_public_data();
        let first = match &self.recovered {
            ColdRecoveredArchive::First(first) if first.request.as_slice() == request => first,
            ColdRecoveredArchive::First(_) => return Err(refused()),
            ColdRecoveredArchive::Later(chain) => {
                let key = Zeroizing::new(store.keys.db_key()?);
                return chain.encode_join_delivery(
                    &key,
                    source.original_target_data().0,
                    &GenesisPublicData {
                        profile: genesis.profile_data(),
                        admission: genesis.admission_data(),
                        header: genesis.header_data(),
                        image: genesis.image_data(),
                    },
                    request,
                );
            }
        };
        let proof = V2FirstAddProofData::encode(
            genesis.profile_data(),
            V2AdmissionStepData {
                admission: genesis.admission_data(),
                header: genesis.header_data(),
                image: genesis.image_data(),
            },
            V2AdmissionStepData {
                admission: &first.admission,
                header: &first.header,
                image: first.image_data()?,
            },
        )?;
        Ok(V2FirstJoinDeliveryData::encode(&first.response, &proof)?)
    }

    pub(in crate::store) fn encode_member_add(
        &self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        request: &V2MemberAddRequestData,
    ) -> Result<Vec<u8>, AppError> {
        if self.core.is_none() {
            return Err(refused());
        }
        self.encode_retained_member_add(store, source, request)
    }

    /// Use a retained predecessor under a newer writer's exact selected
    /// archive checks. The current native guard is checked by that writer.
    pub(in crate::store::persistence::v2_text) fn encode_retained_member_add(
        &self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        request: &V2MemberAddRequestData,
    ) -> Result<Vec<u8>, AppError> {
        self.files.origin.original.require_same_original(source)?;
        let ColdRecoveredArchive::Later(chain) = &self.recovered else {
            return Err(refused());
        };
        let genesis = source.data().genesis_public_data();
        let key = Zeroizing::new(store.keys.db_key()?);
        chain.encode_member_add(
            &key,
            source.original_target_data().0,
            &GenesisPublicData {
                profile: genesis.profile_data(),
                admission: genesis.admission_data(),
                header: genesis.header_data(),
                image: genesis.image_data(),
            },
            request,
        )
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
        let data = source.data().genesis_public_data();
        let public = GenesisPublicData {
            profile: data.profile_data(),
            admission: data.admission_data(),
            header: data.header_data(),
            image: data.image_data(),
        };
        let opened = OpenedColdFounderOrigin::capture(store, source, pin_id, &public)?;
        let (mut retained, key, plain) = (opened.retained, opened.key, opened.plain);
        let promotion = parse_data(&plain)?;
        let context = &promotion.context;
        let (target, _, _) = source.original_target_data();

        let head = read_control(&fixed_child(target, "head.bin")?, selected_head::FRAME_LEN)?;
        let selected = selected_head::GovernedHeadData::open_governed_data(&key, &head.frame)?;
        require_successor_stages_absent(target, TextHeadIdData::of_data(&head.frame))?;
        let context = context.id_data()?;
        let original_head = TextHeadIdData::of_data(promotion.head_frame);
        let original_archive = ArchiveHeadId::from_bytes(
            *TextControlKindData::Archive
                .id_data(promotion.archive_frame)
                .as_bytes(),
        );
        if selected.transition.context != context
            || (selected.transition.epoch == 1
                && (selected.transition.before_head != original_head
                    || selected.transition.before_archive != original_archive))
        {
            return Err(refused());
        }
        let archive_path = archive_child(target, selected.archive.as_bytes())?;
        let archive = read_control_into(
            &archive_path,
            MAX_BYTES,
            &mut Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1))),
        )?;
        let (recovered, chain_fee) = if selected.transition.epoch == 1 {
            let first = authenticate_selected(&key, &head.frame, &archive.frame, context, &public)?;
            (ColdRecoveredArchive::First(first), None)
        } else {
            let fee = pool.try_reserve(RecoveredArchiveChain::invoice_bytes())?;
            let chain = RecoveredArchiveChain::capture(
                &key,
                target,
                &head.frame,
                &archive.frame,
                (context, original_head, original_archive),
                &public,
                || retained.require_current(store, source),
            )?;
            (ColdRecoveredArchive::Later(chain), Some(fee))
        };
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
            _chain_fee: chain_fee,
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
        self.recovered.require_predecessors(source)?;
        self.files.require_current(store, source)
    }

    pub(in crate::store) fn restore_sdk(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
    ) -> Result<PreparedV2FounderRecoveredAdd, AppError> {
        self.require_current(store, source)?;
        let head = self.files.selected.transition;
        let expected = V2InstalledFounderAddData {
            epoch: head.epoch,
            authenticator: head.authenticator,
            header: head.header,
            state: head.state,
        };
        let admission = restore_admission(store)?;
        let restored = match &self.recovered {
            ColdRecoveredArchive::First(first) => {
                PreparedV2FounderRecoveredAdd::restore_from_bootstrap(
                    source.data(),
                    expected,
                    &first.header,
                    first.image_data()?,
                    first.snapshot_data()?,
                    admission,
                )?
            }
            ColdRecoveredArchive::Later(chain) => {
                let proof = V2GoverningJoinPrefixData::decode(&chain.prefix)?;
                let snapshot = chain.journal.snapshot_data()?;
                match chain.journal.selected_admission() {
                    Some(joined) => PreparedV2FounderRecoveredAdd::restore_from_governing_prefix(
                        source.data(),
                        expected,
                        joined,
                        &proof,
                        snapshot,
                        admission,
                    )?,
                    None => PreparedV2FounderRecoveredAdd::restore_selected_removal(
                        source.data(),
                        expected,
                        &proof,
                        snapshot,
                        admission,
                    )?,
                }
            }
        };
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
        let joined = self.recovered.selected_admission();
        if self.core.is_none() {
            const BYTES: usize = 3 * REPLAY_MATH_PRIVATE_INVOICE
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
            self.require_current(store, source)?;
            match &self.recovered {
                ColdRecoveredArchive::First(first) => select_archived_step(
                    &mut verifier,
                    &first.header,
                    ReplayBody::Admission(&first.admission),
                    first.image_data()?,
                )?,
                ColdRecoveredArchive::Later(chain) => {
                    for step in chain.public_steps() {
                        select_archived_step(
                            &mut verifier,
                            &step.header,
                            step.body_data(),
                            &step.image,
                        )?;
                    }
                }
            }
            self.require_current(store, source)?;
            self.core = Some(ColdFounderCore {
                verifier,
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
        for admission in std::iter::once(founder).chain(joined) {
            core.verifier
                .observe_live_member(admission)
                .map_err(|_| refused())?;
        }
        self.require_current(store, source)
    }
}

fn select_archived_step(
    verifier: &mut V2GeneralPrefixVerifier,
    header: &SignedOwnerHeader,
    body: ReplayBody<'_>,
    image: &[u8],
) -> Result<(), AppError> {
    let predecessor = verifier.replay_data().header().id();
    let mut candidate = Some(
        verifier
            .derive_successor(header, body, image)
            .map_err(|_| refused())?,
    );
    let expectation =
        V2HostSelectionExpectation::from_trusted_host(header.id(), header.spec().state);
    let mut retired = verifier
        .advance_host_selected(&expectation, &mut candidate)
        .map_err(|_| refused())?;
    if retired.parent_data().header().id() != predecessor {
        return Err(refused());
    }
    if retired
        .take_close_completion()
        .is_some_and(|mut close| !close.try_complete())
    {
        return Err(refused());
    }
    // The old model holds the verifier's only candidate slot. The next
    // archived step can be replayed only after this owner has drained.
    drop(retired);
    Ok(())
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
        require_successor_stages_absent(target, TextHeadIdData::of_data(&self.head.frame))?;
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
