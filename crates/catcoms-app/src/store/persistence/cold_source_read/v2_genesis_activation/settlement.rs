//! Original activation.seed cleanup only, after a covering Current2 scope barrier. The
//! actual owning library callers, typed guards and seed-ABSENT reader are jointly paired.
//! This does not activate ordinary messages or expose a native cleanup command.
//! source.seed, source controls, group pin and local head are NEVER cleanup destinations.
use super::genesis_inventory::{
    require_fixed_genesis_inventory, GenesisSeedInventoryRole, GENESIS_FIXED_INVENTORY_BYTES,
};
use super::*;
use crate::store::chat_admission::v2_current::{
    DurableGenesisCurrent2SettlementAdmission, GenesisCurrent2SettlementReadAdmission,
};
use crate::store::critical_namespaces::current_layout::GenesisSeedSettlementGroupGuard;
use crate::store::persistence::GenesisCurrentOwnerRef;
use catcoms_sync::owner_v2::OriginalV2GenesisSeedObservation;

/// NEW independent logical metadata class, not credit against the still-live warm512KiB /
/// cold1MiB original proof, Source/provider/SDK/node fees or temporary scope serialization.
pub(in crate::store) const GENESIS_SEED_SETTLEMENT_METADATA_FEE: usize = 1024 * 1024;
// Current's physical opener retains its existing4096-byte base cap. This independent path
// ceiling includes its closed Root/group/target components without inheriting the historical
// DATA reader's looser32KiB scratch ceiling or widening the physical constructor.
const PATH_LIMIT: usize =
    crate::store::critical_namespaces::current_layout::MAX_OPEN_BASE_PATH + 1024;

/// Borrowed original held objects/frames, NEVER a raw caller-selected path or parsed proof.
/// Only the actual paid warm/cold receipt's private methods construct these views. Their
/// original Arc stays retained in SettlementBody throughout every effect/result/refusal.
pub(super) struct SeedPresentFiles<'original> {
    pub(super) directory: (
        &'original Path,
        &'original File,
        catcoms_storage::kept::SessionId,
    ),
    pub(super) seed: SelectedControl<'original>,
    pub(super) pin: SelectedControl<'original>,
    pub(super) head: SelectedControl<'original>,
    pub(super) expected: &'original SavedSourceExpectationData,
}
pub(super) struct SelectedControl<'original> {
    pub(super) file: &'original File,
    pub(super) id: catcoms_storage::kept::SessionId,
    pub(super) frame: &'original [u8],
}

enum OriginalSeedPresent {
    Warm(Arc<SavedV2GenesisHead>),
    Cold(Arc<ColdV2GenesisCurrentHead>),
}
impl OriginalSeedPresent {
    fn files(&self) -> Result<SeedPresentFiles<'_>, AppError> {
        match self {
            Self::Warm(head) => head.settlement_files(),
            Self::Cold(head) => head.settlement_files(),
        }
    }
    fn require_same_owner(&self, owner: &GenesisCurrentOwnerRef<'_>) -> Result<(), AppError> {
        let same = match self {
            Self::Warm(original) => owner
                .warm_head()
                .is_some_and(|head| Arc::ptr_eq(original, head)),
            Self::Cold(original) => owner
                .cold_head()
                .is_some_and(|head| Arc::ptr_eq(original, head)),
        };
        if same {
            Ok(())
        } else {
            Err(critical_namespaces::refused())
        }
    }
}

struct SettlementPaths {
    seed: PathBuf,
    pin: PathBuf,
    head: PathBuf,
    stage: PathBuf,
}

/// No sender, owning graph, transport, key, Store or physical session. Original paid
/// metadata/files stay strong; read/SDK/task/writer observers only close this SAME owner.
/// Its independent fee is LAST, including all tiny outliving weak-control allocations.
struct SettlementBody {
    original: OriginalSeedPresent,
    original_read: OriginalBootstrapReadObservation,
    original_sdk: OriginalV2GenesisSeedObservation,
    task: OriginalV2GenesisGroupObservation,
    writer: V2WriterObservation,
    paths: SettlementPaths,
    fee: critical_namespaces::read_pool::CatalogReadReservation,
}

/// SAME immutable actual payment/custody loan. Clone outside admission so the concrete
/// namespace guard can borrow this while the original worker is separately mutable.
/// No cleanup state, positive permission, sender, source/SDK graph or Store is retained.
#[derive(Clone)]
pub(in crate::store) struct GenesisSeedSettlementResources {
    body: Arc<SettlementBody>,
}
impl GenesisSeedSettlementResources {
    pub(in crate::store) fn require_original_negative(
        &self,
        store: &ServerStore,
        owner: &GenesisCurrentOwnerRef<'_>,
    ) -> Result<(), AppError> {
        self.body.require_same_owner(store, owner)
    }
    /// May wait on the actual pool counter. Before selectors/full proofs, never afterward.
    pub(in crate::store) fn validate_original(
        &self,
        store: &ServerStore,
        owner: &GenesisCurrentOwnerRef<'_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(store, owner)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .pending
            .pool;
        self.body
            .fee
            .validate(pool, GENESIS_SEED_SETTLEMENT_METADATA_FEE)?;
        owner.validate_resources(store, &self.body.writer)?;
        self.require_original_negative(store, owner)
    }
    pub(in crate::store) fn writer_observation(&self) -> &V2WriterObservation {
        &self.body.writer
    }
}

/// Private in-process effect history, never decoded from a flag or minted by NotFound.
/// UnlinkAdmitted is installed BEFORE the syscall; an ambiguous error can retry original
/// presence or genuine absence using the independent closure, never old named-seed proof.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CleanupPhase {
    OriginalPresent,
    UnlinkAdmitted,
    AbsentParentPending,
    ParentSynced,
}

/// Test-only DATA observation, never a phase setter or admission constructor.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum GenesisSeedSettlementProgressData {
    OriginalPresent,
    UnlinkAdmitted,
    AbsentParentPending,
    ParentSynced,
}

pub(in crate::store) struct PreparedGenesisSeedSettlement {
    phase: CleanupPhase,
    saved: Option<Arc<SavedGenesisSeedSettlement>>,
    // LAST. Every error returns to the SAME outside owner with exact cleanup state/custody.
    body: Arc<SettlementBody>,
}

/// Actual unlink + held group-parent barrier + complete final independent physical proof.
/// Only the real worker below can return this opaque receipt. It is not a scope/Current
/// issuer and cannot replace any node/source/SDK/task/writer resource owner.
pub(in crate::store) struct SavedGenesisSeedSettlement {
    body: Arc<SettlementBody>,
}
sanitized_debug!(
    PreparedGenesisSeedSettlement,
    SavedGenesisSeedSettlement,
    GenesisSeedSettlementResources
);

impl SavedV2GenesisHead {
    fn settlement_files(&self) -> Result<SeedPresentFiles<'_>, AppError> {
        Ok(SeedPresentFiles {
            directory: self.seed.directory.directory_data(),
            seed: SelectedControl {
                file: &self.seed.file.file,
                id: self
                    .seed
                    .file
                    .identity
                    .ok_or_else(critical_namespaces::refused)?,
                frame: &self.candidates.seed,
            },
            pin: SelectedControl {
                file: &self.pin.file,
                id: self.pin.identity.ok_or_else(critical_namespaces::refused)?,
                frame: &self.candidates.pin,
            },
            head: SelectedControl {
                file: &self.head.file,
                id: self
                    .head
                    .identity
                    .ok_or_else(critical_namespaces::refused)?,
                frame: &self.candidates.head,
            },
            expected: &self.candidates.expected,
        })
    }
}

fn settlement_child(base: &Path, leaf: &str) -> Result<PathBuf, AppError> {
    let length = base
        .as_os_str()
        .len()
        .checked_add(1)
        .and_then(|n| n.checked_add(leaf.len()))
        .filter(|n| *n <= PATH_LIMIT)
        .ok_or_else(critical_namespaces::refused)?;
    let mut path = PathBuf::with_capacity(length);
    path.push(base);
    path.push(leaf);
    if path.capacity() > PATH_LIMIT {
        return Err(critical_namespaces::refused());
    }
    Ok(path)
}

impl PreparedGenesisSeedSettlement {
    pub(in crate::store) fn resources(&self) -> GenesisSeedSettlementResources {
        GenesisSeedSettlementResources {
            body: self.body.clone(),
        }
    }
    // Only the actual owning Current implementation sees this cached result. It must
    // revalidate the real saved closure through Scope/Factory before any positive return.
    pub(in crate::store::persistence::cold_source_read) fn saved_outcome(
        &self,
    ) -> Option<&Arc<SavedGenesisSeedSettlement>> {
        self.saved.as_ref()
    }

    pub(in crate::store::persistence::cold_source_read) fn requires_independent_validation(
        &self,
    ) -> bool {
        self.phase != CleanupPhase::OriginalPresent
    }
    /// Outside every writer: reserve BEFORE path/Arc/observation allocation. Refusal leaves
    /// the actual owning Current/node/T/R unchanged. Cache this returned worker before I/O.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        owner: &GenesisCurrentOwnerRef<'_>,
    ) -> Result<Self, AppError> {
        // This first native inventory profile is audited only on Windows. Refuse BEFORE
        // fee/path/Arc capture, not after returning a prepared object on another platform.
        require_supported_settlement_profile()?;
        validate_settlement_invoice()?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(GENESIS_SEED_SETTLEMENT_METADATA_FEE)?;
        let writer = owner.observe_writer();
        owner.validate_resources(store, &writer)?;
        let original = if let Some(head) = owner.warm_head() {
            OriginalSeedPresent::Warm(head.clone())
        } else {
            OriginalSeedPresent::Cold(
                owner
                    .cold_head()
                    .ok_or_else(critical_namespaces::refused)?
                    .clone(),
            )
        };
        let files = original.files()?;
        let source = owner.source().source();
        let paths = SettlementPaths {
            seed: settlement_child(files.directory.0, SEED)?,
            pin: settlement_child(files.directory.0, PIN)?,
            head: settlement_child(&source.capture.target, HEAD)?,
            stage: settlement_child(&source.capture.target, STAGE)?,
        };
        if !files.directory.0.is_absolute()
            || !source.capture.target.is_absolute()
            || files.directory.0.as_os_str().len() > PATH_LIMIT
            || source.capture.target.as_os_str().len() > PATH_LIMIT
        {
            return Err(critical_namespaces::refused());
        }
        // On an error, declaration order drains paths/observations before the local fee.
        let body = Arc::new(SettlementBody {
            original,
            original_read: source.observe_original(),
            original_sdk: owner.source().seed().observe_original_seed_data(),
            task: owner.task().observe_original(),
            writer,
            paths,
            fee,
        });
        let prepared = Self {
            phase: CleanupPhase::OriginalPresent,
            saved: None,
            body,
        };
        prepared.require_same_owner(store, owner)?;
        Ok(prepared)
    }

    /// Cached only. This NEVER calls an old mandatory-seed full validator, so an admitted
    /// unlink/parent-sync refusal does not prevent the same actual owner reaching repair.
    pub(in crate::store) fn require_same_owner(
        &self,
        store: &ServerStore,
        owner: &GenesisCurrentOwnerRef<'_>,
    ) -> Result<(), AppError> {
        self.body.require_same_owner(store, owner)
    }

    #[cfg(test)]
    pub(in crate::store) fn progress_data_for_test(&self) -> GenesisSeedSettlementProgressData {
        match self.phase {
            CleanupPhase::OriginalPresent => GenesisSeedSettlementProgressData::OriginalPresent,
            CleanupPhase::UnlinkAdmitted => GenesisSeedSettlementProgressData::UnlinkAdmitted,
            CleanupPhase::AbsentParentPending => {
                GenesisSeedSettlementProgressData::AbsentParentPending
            }
            CleanupPhase::ParentSynced => GenesisSeedSettlementProgressData::ParentSynced,
        }
    }

    /// Scope calls this under its SAME actual owner2/Runtime before and AFTER the covering
    /// write. Brief Root is released on return, before any global scope serialization.
    /// Phase dispatch precedes every old seed-present validator: none is called here.
    pub(in crate::store) fn validate_under_scope_admission(
        &self,
        admission: &GenesisCurrent2SettlementReadAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        self.require_same_owner(admission.store(), admission.owner_ref())?;
        let root = admission
            .store()
            .archive_bootstrap_root
            .lock()
            .map_err(|_| critical_namespaces::refused())?;
        self.body.validate_under_archive_root(
            admission.store(),
            admission.owner_ref(),
            root.as_ref().ok_or_else(critical_namespaces::refused)?,
            self.phase,
        )?;
        admission.validate_source_only()
    }

    /// The only deletion edge. This concrete guard borrows an actual covering scope2
    /// admission continuously through AD -> pins -> canonical -> Root. No read/proof guard
    /// or absent-seed capture can be substituted. Every error retains this original state.
    pub(in crate::store) fn settle(
        &mut self,
        guard: &GenesisSeedSettlementGroupGuard<'_, '_>,
    ) -> Result<Arc<SavedGenesisSeedSettlement>, AppError> {
        let admission: &DurableGenesisCurrent2SettlementAdmission<'_> = guard.scope_admission();
        if !Arc::ptr_eq(&self.body, &guard.resources().body) {
            return Err(critical_namespaces::refused());
        }
        admission.validate_source_only()?;
        self.require_same_owner(admission.store(), admission.owner_ref())?;
        guard.require_installed_writer_observation(&self.body.writer)?;
        let proven_presence = self.body.validate_under_archive_root(
            admission.store(),
            admission.owner_ref(),
            guard.archive_root(),
            self.phase,
        )?;
        guard.validate_source_only()?;
        if self.phase == CleanupPhase::OriginalPresent {
            #[cfg(test)]
            admission.store().archive_declaration_fault(210)?;
            // The complete original seed comparison precedes the full source/ID3/names
            // proof which returned proven_presence. Only cached negatives/private phase
            // moves intervene before the effect; do not append a new comparer here.
            self.phase = CleanupPhase::UnlinkAdmitted;
            #[cfg(test)]
            admission.store().archive_declaration_fault(211)?;
        }
        if self.phase == CleanupPhase::UnlinkAdmitted {
            match proven_presence {
                SeedPresence::Present => {
                    guard.validate_source_only()?;
                    fs::remove_file(&self.body.paths.seed).map_err(io)?;
                    // Successful syscall is cached BEFORE any fallible parent barrier.
                    self.phase = CleanupPhase::AbsentParentPending;
                    #[cfg(test)]
                    admission.store().archive_declaration_fault(212)?;
                }
                SeedPresence::Absent => self.phase = CleanupPhase::AbsentParentPending,
            }
        }
        if self.phase == CleanupPhase::AbsentParentPending {
            let files = self.body.original.files()?;
            super::super::require_absent(&self.body.paths.seed)?;
            require_directory_name(files.directory.0, files.directory.1, files.directory.2)?;
            files.directory.1.sync_all().map_err(io)?;
            self.phase = CleanupPhase::ParentSynced;
            #[cfg(test)]
            admission.store().archive_declaration_fault(213)?;
        }
        self.body.validate_under_archive_root(
            admission.store(),
            admission.owner_ref(),
            guard.archive_root(),
            self.phase,
        )?;
        guard.require_installed_writer_observation(&self.body.writer)?;
        guard.validate_source_only()?;
        if self.saved.is_none() {
            #[cfg(test)]
            admission.store().archive_declaration_fault(214)?;
            // The unlink, held-parent barrier and complete independent proof are actual.
            // Cache before the LAST full proof/return so a lost result cannot reallocate or
            // reread/adopt another closure. This private cache is never exposed unchecked.
            self.saved = Some(Arc::new(SavedGenesisSeedSettlement {
                body: self.body.clone(),
            }));
        }
        #[cfg(test)]
        admission.store().archive_declaration_fault(215)?;
        self.body.validate_under_archive_root(
            admission.store(),
            admission.owner_ref(),
            guard.archive_root(),
            CleanupPhase::ParentSynced,
        )?;
        guard.require_installed_writer_observation(&self.body.writer)?;
        guard.validate_source_only()?;
        // Only a paid Arc clone follows the last proof. No original payload/account drains.
        Ok(self
            .saved
            .as_ref()
            .expect("cached after actual settlement")
            .clone())
    }
}

impl SettlementBody {
    fn require_same_owner(
        &self,
        store: &ServerStore,
        owner: &GenesisCurrentOwnerRef<'_>,
    ) -> Result<(), AppError> {
        self.original.require_same_owner(owner)?;
        self.original_read
            .require_same_original(owner.source().source())?;
        self.original_sdk
            .require_same_seed_data(owner.source().seed())?;
        self.task.require_same_original(owner.task())?;
        owner.require_original_negative(store, &self.writer)
    }

    fn validate_under_archive_root(
        &self,
        store: &ServerStore,
        owner: &GenesisCurrentOwnerRef<'_>,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
        phase: CleanupPhase,
    ) -> Result<SeedPresence, AppError> {
        self.require_same_owner(store, owner)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .pending
            .pool;
        self.fee
            .validate(pool, GENESIS_SEED_SETTLEMENT_METADATA_FEE)?;
        owner.validate_resources(store, &self.writer)?;
        let source = owner.source();
        source.source().validate_under_archive_root(store, root)?;
        let files = self.original.files()?;
        exact_selected(&self.paths.pin, &files.pin)?;
        exact_selected(&self.paths.head, &files.head)?;
        super::super::require_absent(&self.paths.stage)?;

        // Authenticate pin/head independently of activation.seed. Their SAME ciphertexts
        // must bind the actual independent SourceSaved controls/public representatives.
        let key = Zeroizing::new(store.keys.db_key()?);
        let pin = codec::open_pin_data(
            &key,
            files.expected.layout_data(),
            files.expected.context.spec_data().group,
            files.pin.frame,
        )?;
        pin.require_source_data(files.expected)?;
        let head = codec::open_head_data(
            &key,
            files.expected,
            codec::V2GroupPinId::of(files.pin.frame),
            &pin,
            files.head.frame,
        )?;
        let public = source.data();
        head.require_original_public_data(
            &pin,
            &codec::GenesisPublicData {
                profile: public.profile,
                admission: public.admission,
                header: public.header,
                image: public.state,
            },
        )?;
        // Release all copied public/secret decode buffers before the last source/ID3/name
        // proofs; only original paid ciphertexts/files/identities remain retained.
        drop(head);
        drop(pin);
        drop(key);

        let presence = seed_presence(&self.paths.seed)?;
        let role = match (phase, presence) {
            (CleanupPhase::OriginalPresent, SeedPresence::Present)
            | (CleanupPhase::UnlinkAdmitted, SeedPresence::Present) => {
                exact_selected(&self.paths.seed, &files.seed)?;
                GenesisSeedInventoryRole::Present
            }
            (CleanupPhase::UnlinkAdmitted, SeedPresence::Absent)
            | (CleanupPhase::AbsentParentPending, SeedPresence::Absent)
            | (CleanupPhase::ParentSynced, SeedPresence::Absent) => {
                GenesisSeedInventoryRole::Absent
            }
            _ => return Err(critical_namespaces::refused()),
        };
        #[cfg(test)]
        run_after_last_seed_comparison_for_test();
        let capture = &source.source().capture;
        require_fixed_genesis_inventory(
            files.directory,
            (
                &capture.target,
                &capture.target_directory,
                capture.target_id,
            ),
            role,
        )?;
        // All account/large full comparers precede the final source -> ID3 -> original names
        // edge. These are separate proofs, not a claim of hostile-filesystem atomicity.
        source.source().validate_under_archive_root(store, root)?;
        store.validate_mounted_current_layout_under_archive_root(root)?;
        require_directory_name(&capture.root, &capture.root_directory, capture.root_id)?;
        require_directory_name(
            &capture.target,
            &capture.target_directory,
            capture.target_id,
        )?;
        require_directory_name(files.directory.0, files.directory.1, files.directory.2)?;
        require_selected_name(&self.paths.pin, &files.pin)?;
        require_selected_name(&self.paths.head, &files.head)?;
        super::super::require_absent(&self.paths.stage)?;
        match role {
            GenesisSeedInventoryRole::Present => {
                require_selected_name(&self.paths.seed, &files.seed)?
            }
            GenesisSeedInventoryRole::Absent => super::super::require_absent(&self.paths.seed)?,
        }
        self.require_same_owner(store, owner)?;
        Ok(presence)
    }
}

impl SavedGenesisSeedSettlement {
    pub(in crate::store) fn resources(&self) -> GenesisSeedSettlementResources {
        GenesisSeedSettlementResources {
            body: self.body.clone(),
        }
    }
    pub(in crate::store) fn require_same_owner(
        &self,
        store: &ServerStore,
        owner: &GenesisCurrentOwnerRef<'_>,
    ) -> Result<(), AppError> {
        self.body.require_same_owner(store, owner)
    }

    /// Distinct settled Scope role. No optional-seed switch can weaken the old read lane.
    pub(in crate::store) fn validate_under_scope_admission(
        &self,
        admission: &crate::store::chat_admission::v2_current::SettledGenesisCurrent2ReadAdmission<
            '_,
        >,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        self.require_same_owner(admission.store(), admission.owner_ref())?;
        let root = admission
            .store()
            .archive_bootstrap_root
            .lock()
            .map_err(|_| critical_namespaces::refused())?;
        self.body.validate_under_archive_root(
            admission.store(),
            admission.owner_ref(),
            root.as_ref().ok_or_else(critical_namespaces::refused)?,
            CleanupPhase::ParentSynced,
        )?;
        admission.validate_source_only()
    }

    pub(in crate::store) fn validate_under_group_guard(
        &self,
        guard: &crate::store::critical_namespaces::current_layout::SettledGenesisCurrentGroupGuard<
            '_,
            '_,
        >,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.body, &guard.resources().body) {
            return Err(critical_namespaces::refused());
        }
        let admission = guard.scope_admission();
        admission.validate_source_only()?;
        self.body.validate_under_archive_root(
            admission.store(),
            admission.owner_ref(),
            guard.archive_root(),
            CleanupPhase::ParentSynced,
        )?;
        guard.require_installed_writer_observation(&self.body.writer)?;
        guard.validate_source_only()
    }
}

#[derive(Clone, Copy)]
enum SeedPresence {
    Present,
    Absent,
}
fn require_supported_settlement_profile() -> Result<(), AppError> {
    #[cfg(windows)]
    {
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err(critical_namespaces::refused())
    }
}

#[cfg(test)]
std::thread_local! {
    static AFTER_LAST_SEED_COMPARISON: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(in crate::store) fn after_last_seed_comparison_for_test(hook: Box<dyn FnOnce()>) {
    AFTER_LAST_SEED_COMPARISON.with(|slot| {
        assert!(slot.borrow_mut().replace(hook).is_none());
    });
}
#[cfg(test)]
fn run_after_last_seed_comparison_for_test() {
    let hook = AFTER_LAST_SEED_COMPARISON.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
fn seed_presence(path: &Path) -> Result<SeedPresence, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(SeedPresence::Present),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SeedPresence::Absent),
        _ => Err(critical_namespaces::refused()),
    }
}
fn require_selected_name(path: &Path, original: &SelectedControl<'_>) -> Result<(), AppError> {
    if catcoms_storage::kept::session_id(original.file).map_err(io)? != original.id
        || !matches!(
            catcoms_storage::kept::session_at(path, original.id),
            catcoms_storage::kept::SessionAt::Ours
        )
        || original.file.metadata().map_err(io)?.len() != original.frame.len() as u64
    {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}
fn exact_selected(path: &Path, original: &SelectedControl<'_>) -> Result<(), AppError> {
    require_selected_name(path, original)?;
    let (file, actual) =
        crate::store::persistence::read_archive_object(path, original.frame.len())?
            .ok_or_else(critical_namespaces::refused)?;
    if catcoms_storage::kept::session_id(&file).map_err(io)? != original.id
        || actual.as_slice() != original.frame
    {
        return Err(critical_namespaces::refused());
    }
    require_selected_name(path, original)
}

/// Explicit simultaneous new allocation inventory. Existing original warm/cold/source/SDK
/// bodies keep their INDEPENDENT actual payments while this additional class is live.
fn validate_settlement_invoice() -> Result<(), AppError> {
    let bytes = 4usize
        .checked_mul(PATH_LIMIT)
        .and_then(|n| n.checked_add(24 * PATH_LIMIT)) // Windows root/wildcard/native UTF16 overlap
        .and_then(|n| {
            n.checked_add(
                12 * (codec::MAX_ACTIVATION_FRAME + codec::MAX_HEAD_FRAME + codec::MAX_PIN_FRAME),
            )
        })
        .and_then(|n| n.checked_add(GENESIS_FIXED_INVENTORY_BYTES))
        .and_then(|n| n.checked_add(crate::store::critical_namespaces::current_layout::GENESIS_SEED_SETTLEMENT_GUARD_BYTES))
        .and_then(|n| n.checked_add(crate::store::chat_admission::v2_current::GENESIS_SEED_SETTLEMENT_SCOPE_METADATA_BYTES))
        .and_then(|n| n.checked_add(2 * std::mem::size_of::<SettlementBody>()))
        .and_then(|n| n.checked_add(std::mem::size_of::<PreparedGenesisSeedSettlement>()))
        .and_then(|n| n.checked_add(2 * std::mem::size_of::<SavedGenesisSeedSettlement>()))
        .and_then(|n| n.checked_add(2 * std::mem::size_of::<GenesisSeedSettlementResources>()))
        .and_then(|n| n.checked_add(ORIGINAL_BOOTSTRAP_READ_OBSERVATION_BYTES))
        .and_then(|n| n.checked_add(std::mem::size_of::<OriginalV2GenesisSeedObservation>() + 64))
        .and_then(|n| n.checked_add(std::mem::size_of::<OriginalV2GenesisGroupObservation>()))
        .and_then(|n| n.checked_add(64 * 1024)) // native entry/OS name/error + fixed decoded metadata
        .ok_or_else(critical_namespaces::refused)?;
    if bytes > GENESIS_SEED_SETTLEMENT_METADATA_FEE {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}
