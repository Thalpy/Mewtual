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
    owner_facts::OwnerFactId,
    owner_header::OwnerHeaderId,
    owner_state::{transition::REPLAY_MATH_PRIVATE_INVOICE, StateFrame},
    FounderGenesisAnchorData, ProjectionGroup, ProjectionGroupView, V2FrozenAdmission,
    V2GeneralExpectation, V2GeneralPrefixVerifier, V2HostSelectionExpectation, V2ProjectionOrigin,
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

mod repair;

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
    /// Require the exact nonzero epoch and Header on this fully replayed
    /// membership. Another signed candidate at the same epoch cannot
    /// become a historical review anchor.
    fn require_review_membership_anchor(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        selected: (OwnerHeaderId, u64),
        anchor: (OwnerHeaderId, u64),
    ) -> Result<(), AppError> {
        let replay = verifier.replay_data();
        if (replay.header().id(), replay.mls_epoch_data()) != selected {
            return Err(refused());
        }
        self.require_review_anchor_on_selected(selected, anchor)
    }

    fn require_review_anchor_on_selected(
        &self,
        selected: (OwnerHeaderId, u64),
        anchor: (OwnerHeaderId, u64),
    ) -> Result<(), AppError> {
        if anchor.1 == 0 || anchor.1 > selected.1 {
            return Err(refused());
        }
        if anchor == selected {
            return Ok(());
        }
        let Self::Later(chain) = self else {
            return Err(refused());
        };
        let step = chain
            .public_steps()
            .find(|step| step.header.id() == anchor.0)
            .ok_or_else(refused)?;
        let image = StateFrame::decode(&step.image).map_err(|_| refused())?;
        image
            .verify_header_binding(&step.header)
            .map_err(|_| refused())?;
        if image.meta().mls_epoch != anchor.1 {
            return Err(refused());
        }
        Ok(())
    }

    fn selected_admission(&self) -> Option<OwnerFactId> {
        match self {
            Self::First(first) => Some(first.admission.admission_id()),
            Self::Later(chain) => chain.journal.selected_admission(),
        }
    }

    fn require_predecessor_contents(
        &self,
        source: &RetainedBootstrapSourceData,
    ) -> Result<(), AppError> {
        if let Self::Later(chain) = self {
            chain.require_predecessor_contents(source.original_target_data().0)?;
        }
        Ok(())
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

/// Physical writer scope for the exact recovered parent. It carries the
/// already-held archive Root, never a warm Current owner or a file path
/// supplied by the request. The owning Add separately checks its records.
pub(in crate::store::persistence::v2_text) struct ColdGovernedWriteScope<'a> {
    store: &'a ServerStore,
    source: &'a RetainedBootstrapSourceData,
    root: &'a crate::store::critical_namespaces::archive_bootstrap::MountedRoot,
    native: &'a catcoms_rt::RequestCancellation,
    parent: (TextContextIdData, TextHeadIdData),
}

impl ColdGovernedWriteScope<'_> {
    pub(in crate::store::persistence::v2_text) fn directory_data(
        &self,
    ) -> (&Path, &File, SessionId) {
        self.source.original_target_data()
    }

    pub(in crate::store::persistence::v2_text) fn require_current(
        &self,
        parent: (TextContextIdData, TextHeadIdData),
    ) -> Result<(), AppError> {
        if self.native.is_cancelled()
            || parent != self.parent
            || !self
                .store
                .current_layout_policy
                .as_ref()
                .and_then(|policy| policy.native_registry.as_ref())
                .is_some_and(|handler| handler.is_governed())
        {
            return Err(refused());
        }
        self.source
            .validate_under_archive_root(self.store, self.root)?;
        self.store
            .validate_mounted_current_layout_under_archive_root(self.root)?;
        self.store
            .validate_mounted_governance_layout_under_archive_root(self.root)?;
        let (target, directory, id) = self.directory_data();
        require_directory_name(target, directory, id)?;
        self.require_live_cached(parent)
    }

    pub(in crate::store::persistence::v2_text) fn require_live_cached(
        &self,
        parent: (TextContextIdData, TextHeadIdData),
    ) -> Result<(), AppError> {
        if self.native.is_cancelled() || parent != self.parent {
            return Err(refused());
        }
        self.source.require_original_negative(self.store)
    }
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
    retired_history: Vec<V2FrozenAdmission>,
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
    /// Borrow the same physical parent for an owned successor's file work.
    /// The ordinary reader still requires absent successor stages. Here the
    /// actual writer must check every existing stage and downstream absence.
    pub(in crate::store::persistence::v2_text) fn with_cold_governed_writer(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        caller: &catcoms_rt::RequestCancellation,
        write: impl FnOnce(
            &ColdGovernedWriteScope<'_>,
            &mut dyn FnMut() -> Result<(), AppError>,
        ) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        if self.core.is_none() || caller.is_cancelled() {
            return Err(refused());
        }
        self.files.origin.original.require_same_original(source)?;
        let parent = (
            self.files.selected.transition.context,
            TextHeadIdData::of_data(&self.files.head.frame),
        );
        let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let scope = ColdGovernedWriteScope {
            store,
            source,
            root: root.as_ref().ok_or_else(refused)?,
            native: caller,
            parent,
        };
        // Finish real Root/source/layout waits before the final held names
        // and contents. A mutation while a layout waits cannot reach a key.
        let mut require_parent = || {
            scope.require_current(parent)?;
            self.recovered.require_predecessor_contents(source)?;
            self.files.origin.require_named_files(source)?;
            self.files.require_named_parent(source)?;
            scope.require_live_cached(parent)
        };
        require_parent()?;
        write(&scope, &mut require_parent)
    }

    /// Read or advance the one owned selector under the original Root. The
    /// owner must prove the exact held parent or its own already-entered child.
    /// Unlike a normal read, this tolerates an owned late rename result.
    pub(in crate::store::persistence::v2_text) fn with_cold_governed_selection<T>(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        caller: &catcoms_rt::RequestCancellation,
        select: impl FnOnce(
            &ColdGovernedWriteScope<'_>,
            &mut OpenedTextControl,
            &mut V2GeneralPrefixVerifier,
            &mut dyn FnMut() -> Result<(), AppError>,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.files.origin.original.require_same_original(source)?;
        self.recovered.require_predecessor_contents(source)?;
        let parent = (
            self.files.selected.transition.context,
            TextHeadIdData::of_data(&self.files.head.frame),
        );
        let core = self.core.as_mut().ok_or_else(refused)?;
        let result;
        {
            let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let scope = ColdGovernedWriteScope {
                store,
                source,
                root: root.as_ref().ok_or_else(refused)?,
                native: caller,
                parent,
            };
            scope.require_current(parent)?;
            let mut require_parent_archive = || {
                scope.require_current(parent)?;
                self.recovered.require_predecessor_contents(source)?;
                self.files.origin.require_named_files(source)?;
                require_exact_held(&self.files.archive_path, &mut self.files.archive)?;
                scope.require_live_cached(parent)
            };
            require_parent_archive()?;
            result = select(
                &scope,
                &mut self.files.head,
                &mut core.verifier,
                &mut require_parent_archive,
            );
        }
        result
    }

    /// Refresh only the physical archive authority after the original
    /// verifier and SDK installed the same durable successor. Moving the
    /// exact live Core keeps the founder's bound Admission identity.
    pub(in crate::store) fn refresh_selected_add(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        pin: [u8; 32],
        head: TextHeadIdData,
        installed: V2InstalledFounderAddData,
    ) -> Result<Self, AppError> {
        let core = self.core.as_ref().ok_or_else(refused)?;
        let replay = core.verifier.replay_data();
        if replay.header().id() != installed.header
            || replay.header().spec().state != installed.state
            || replay.mls_epoch_data() != installed.epoch
        {
            return Err(refused());
        }
        let mut next = Self::capture(store, source, pin)?;
        next.require_installed_parent(source, head, installed)?;
        next.require_current(store, source)?;
        next.core = self.core.take();
        Ok(next)
    }

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
        let binding = self.invite_parent_coordinates(store, source)?;
        self.with_checked_member_parts(store, source, |verifier, _, _, require| {
            let result = perform(&binding, verifier, require)?;
            require()?;
            Ok(result)
        })
    }

    /// Labels for checking a read-only invitation against this exact parent.
    /// Data only. The caller still needs the scope's original physical
    /// writer, or a checked current parent, before using it for key entry.
    pub(in crate::store) fn invite_parent_coordinates<'a>(
        &self,
        store: &'a ServerStore,
        source: &'a RetainedBootstrapSourceData,
    ) -> Result<ColdInviteParent<'a>, AppError> {
        self.files.origin.original.require_same_original(source)?;
        source.require_original_negative(store)?;
        Ok(ColdInviteParent {
            store,
            target: source.original_target_data().0,
            parent: TextHeadIdData::of_data(&self.files.head.frame),
            context: self.files.origin.invite_context,
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

    /// The warm writer has already proved this actual parent is referenced by
    /// its selected child. Reverify the exact retained archive for an older
    /// review anchor; never rebuild or replace the warm membership issuer.
    pub(in crate::store::persistence::v2_text) fn require_warm_review_parent_anchor(
        &mut self,
        guard: &crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard<'_, '_>,
        anchor: (OwnerHeaderId, u64),
    ) -> Result<(), AppError> {
        self.require_writer_parent_archive(guard)?;
        let parent = self.files.selected.transition;
        self.recovered
            .require_review_anchor_on_selected((parent.header, parent.epoch), anchor)?;
        if anchor != (parent.header, parent.epoch) {
            self.recovered
                .require_predecessor_contents(guard.source().source())?;
        }
        self.require_writer_parent_archive(guard)?;
        guard.validate_source_only()
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

    /// Derivation changes only the original verifier's paid candidate slot.
    /// The selector independently proves files and native authority.
    pub(in crate::store) fn derive_add_selection(
        &mut self,
        sdk: &PreparedV2FounderRecoveredAdd,
        source: &RetainedBootstrapSourceData,
    ) -> Result<catcoms_replication::chat::V2GeneralCandidate, AppError> {
        self.files.origin.original.require_same_original(source)?;
        let core = &mut self.core.as_mut().ok_or_else(refused)?.verifier;
        Ok(sdk.derive_recovered_add_selection_data(core, source.data())?)
    }

    pub(in crate::store) fn member_parts(
        &self,
    ) -> Result<(&V2GeneralPrefixVerifier, &ProjectionGroupView), AppError> {
        let core = self.core.as_ref().ok_or_else(refused)?;
        Ok((&core.verifier, &core._view))
    }

    pub(in crate::store) fn retired_member_history(
        &self,
    ) -> Result<&[V2FrozenAdmission], AppError> {
        Ok(&self.core.as_ref().ok_or_else(refused)?.retired_history)
    }

    pub(in crate::store) fn with_checked_member_parts<T>(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        perform: impl FnOnce(
            &V2GeneralPrefixVerifier,
            &ProjectionGroupView,
            &[V2FrozenAdmission],
            &mut dyn FnMut() -> Result<(), AppError>,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.with_checked_review_parts(store, source, |verifier, view, retired, require| {
            perform(verifier, view, retired, &mut || require(None))
        })
    }

    pub(in crate::store) fn with_checked_review_parts<T>(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        perform: impl FnOnce(
            &V2GeneralPrefixVerifier,
            &ProjectionGroupView,
            &[V2FrozenAdmission],
            &mut dyn FnMut(Option<(OwnerHeaderId, u64)>) -> Result<(), AppError>,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let core = self.core.as_ref().ok_or_else(refused)?;
        let mut require = |anchor| {
            self.recovered.require_predecessors(source)?;
            let selected = self.files.require_current(store, source)?;
            if let Some(anchor) = anchor {
                self.recovered.require_review_membership_anchor(
                    &core.verifier,
                    selected,
                    anchor,
                )?;
                if anchor != selected {
                    self.recovered.require_predecessor_contents(source)?;
                    if self.files.require_current(store, source)? != selected {
                        return Err(refused());
                    }
                }
            }
            Ok(())
        };
        require(None)?;
        perform(
            &core.verifier,
            &core._view,
            &core.retired_history,
            &mut require,
        )
    }

    /// Return the exact original delivery, or checked absence for an unused
    /// nonce. Corrupt or ambiguous history is an error. Checked absence
    /// carries no permission to admit the new request.
    pub(in crate::store) fn classify_join_delivery(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        packet: &V2JoinRequestPacketData<'_>,
    ) -> Result<Option<Vec<u8>>, AppError> {
        self.require_current(store, source)?;
        if self.core.is_none() {
            return Err(refused());
        }
        let result = self.classify_join_delivery_checked(store, source, packet)?;
        self.require_current(store, source)?;
        Ok(result)
    }

    fn classify_join_delivery_checked(
        &self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        packet: &V2JoinRequestPacketData<'_>,
    ) -> Result<Option<Vec<u8>>, AppError> {
        let request = packet.packet_bytes_data();
        let genesis = source.data().genesis_public_data();
        let invitation = packet.founder_add_data().invite.spec_data();
        let anchor = &invitation.anchor;
        let context = genesis.context_data().spec_data();
        if invitation.inviter != context.signer
            || anchor.founder != context.signer
            || anchor.group != context.group
            || anchor.profile != *genesis.profile_data().id().as_bytes()
            || anchor.admission != *genesis.admission_data().admission_id().as_bytes()
            || anchor.tenure != *genesis.profile_data().tenure().as_bytes()
            || anchor.header != *genesis.header_data().id().as_bytes()
            || anchor.state != *genesis.header_data().spec().state.as_bytes()
        {
            return Err(refused());
        }
        let nonce = invitation.nonce;
        let first = match &self.recovered {
            ColdRecoveredArchive::First(first) if first.request.as_slice() == request => first,
            ColdRecoveredArchive::First(first) if first.consumed_nonce == nonce => {
                return Err(refused());
            }
            ColdRecoveredArchive::First(_) => return Ok(None),
            ColdRecoveredArchive::Later(chain) => {
                let key = Zeroizing::new(store.keys.db_key()?);
                return chain.classify_join_delivery(
                    &key,
                    source.original_target_data().0,
                    &GenesisPublicData {
                        profile: genesis.profile_data(),
                        admission: genesis.admission_data(),
                        header: genesis.header_data(),
                        image: genesis.image_data(),
                    },
                    request,
                    nonce,
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
        Ok(Some(V2FirstJoinDeliveryData::encode(
            &first.response,
            &proof,
        )?))
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
                + 4 * std::mem::size_of::<ColdFounderCore>()
                + catcoms_sync::MAX_V2_GOVERNING_JOIN_STEPS
                    * V2FrozenAdmission::retained_layout_bytes();
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
            let mut retired_history = Vec::with_capacity(catcoms_sync::MAX_V2_GOVERNING_JOIN_STEPS);
            match &self.recovered {
                ColdRecoveredArchive::First(first) => select_archived_step(
                    &mut verifier,
                    &mut retired_history,
                    &first.header,
                    ReplayBody::Admission(&first.admission),
                    first.image_data()?,
                )?,
                ColdRecoveredArchive::Later(chain) => {
                    for step in chain.public_steps() {
                        select_archived_step(
                            &mut verifier,
                            &mut retired_history,
                            &step.header,
                            step.body_data(),
                            &step.image,
                        )?;
                    }
                }
            }
            self.require_current(store, source)?;
            self.core = Some(ColdFounderCore {
                retired_history,
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
    retired_history: &mut Vec<V2FrozenAdmission>,
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
    if let Some(frozen) = retired.take_frozen_data() {
        if retired_history.len() >= catcoms_sync::MAX_V2_GOVERNING_JOIN_STEPS {
            return Err(refused());
        }
        retired_history.push(frozen);
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
        self.require_named_parent(source)?;
        let (target, directory, id) = source.original_target_data();
        require_successor_stages_absent(target, TextHeadIdData::of_data(&self.head.frame))?;
        require_directory_name(target, directory, id)?;
        source.require_original_negative(store)?;
        Ok((
            self.selected.transition.header,
            self.selected.transition.epoch,
        ))
    }

    fn require_named_parent(
        &mut self,
        source: &RetainedBootstrapSourceData,
    ) -> Result<(), AppError> {
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
        Ok(())
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
        self.require_named_files(source)?;
        source.require_original_negative(store)
    }

    fn require_named_files(
        &mut self,
        source: &RetainedBootstrapSourceData,
    ) -> Result<(), AppError> {
        let (target, directory, id) = source.original_target_data();
        require_directory_name(target, directory, id)?;
        require_directory_name(&self.group, &self.directory, self.directory_id)?;
        require_exact_held(&self.group.join("pin.bin"), &mut self.pin)?;
        require_exact_held(&fixed_child(target, "text.seed")?, &mut self.seed)?;
        require_stages_absent(target)?;
        require_directory_name(target, directory, id)?;
        require_directory_name(&self.group, &self.directory, self.directory_id)?;
        Ok(())
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
