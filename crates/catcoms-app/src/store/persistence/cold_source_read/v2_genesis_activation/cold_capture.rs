//! Role-specific cold genesis closure, paired with the actual Factory guards, scope
//! read roles and Current2 owning caller. Initializing1 remains test-only.
//!
//! These are NEW cold receipts, never reconstructed warm SavedSeed/SavedHead capabilities.
//! Present seeds must authenticate the complete pin/genesis-head closure. On the audited
//! Windows path only, an unowned Current2 may independently prove a complete genesis closure
//! after genuine seed absence. Partial/unknown/successor state never falls back to genesis.
//! Seed unlink/settlement and later-head selection are distinct owning operations.
use super::*;
mod settlement;
use crate::store::chat_admission::v2_current::ColdV2GenesisScopeBasis;
use crate::store::critical_namespaces::current_layout::ColdV2CurrentGroupReadGuard;
#[cfg(test)]
use crate::store::critical_namespaces::current_layout::ColdV2InitializingGroupReadGuard;
use catcoms_sync::owner_v2::OriginalV2GenesisSeedObservation;
use codec::V2GroupPinId;
#[cfg(test)]
use codec::{V2ActivationSeedId, V2GenesisHeadId};
mod settled;

// Cold selection and Factory's path/directory guards coexist. Their named invoice exceeds
// the warm 512KiB class. This separate debit changes no wire/content limit, SDK or source
// payment, persistent allowance or execution grant.
pub(in crate::store) const COLD_CAPTURE_METADATA_FEE: usize = 1024 * 1024;
#[cfg(test)]
thread_local! {
    // Last source-control I/O has finished; the final ID3 proof must still follow this.
    static AFTER_FINAL_SOURCE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// One real independent metadata debit. No graph, source plaintext, key, Store, session,
/// task life or sender is retained; weak identities cannot keep their originals alive.
struct ResourceBody {
    original_read: OriginalBootstrapReadObservation,
    original_seed: OriginalV2GenesisSeedObservation,
    mount: Arc<()>,
    negative: catcoms_rt::RequestCancellation,
    expected: SavedSourceExpectationData,
    // LAST: contexts, weak control blocks and mount observation drain before the refund.
    fee: critical_namespaces::read_pool::CatalogReadReservation,
}

/// Actual shared allocation layout, not an invoice for a capture or writer. Every concrete
/// caller must additionally account for its OWN receipts, paths, guards and scratch buffers.
pub(super) const COLD_READ_RESOURCE_BODY_BYTES: usize = std::mem::size_of::<ResourceBody>();

/// Clone this SAME concrete loan before entering the scope callback. Factory guards borrow
/// it through all held File/path cleanup; the mutable capture remains independently usable.
/// Arc cloning allocates no new body and grants no scope, file, writer or Current authority.
#[derive(Clone)]
pub(in crate::store) struct ColdV2GenesisReadResources {
    body: Arc<ResourceBody>,
}
impl ColdV2GenesisReadResources {
    /// Observe THIS concrete debit's final release. Install before creating guard/body
    /// clones; this test-only edge never copies a fee or supplies source/read authority.
    #[cfg(all(test, windows))]
    pub(super) fn observe_fee_drop_for_test(
        &mut self,
        notice: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(), AppError> {
        Arc::get_mut(&mut self.body)
            .ok_or_else(critical_namespaces::refused)?
            .fee
            .observe_drop_for_test(notice);
        Ok(())
    }

    /// Immutable original expectation DATA only. Neither these labels nor their equality
    /// supplies a read admission; concrete guards still require this SAME live paid loan.
    pub(super) fn expectation_data(&self) -> &SavedSourceExpectationData {
        &self.body.expected
    }

    /// Root-family resource construction only. Callers first validate their own complete
    /// simultaneous metadata invoice against the fixed cold class. This creates no capture,
    /// scope basis, task, owner, writer or file permission; those concrete roles stay separate.
    pub(super) fn prepare_paid(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<Self, AppError> {
        source.validate_original(store)?;
        store.validate_mounted_current_layout()?;
        let policy = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        let fee = policy.pending.pool.try_reserve(COLD_CAPTURE_METADATA_FEE)?;
        source.validate_original(store)?;
        store.validate_mounted_current_layout()?;
        let capture = &source.source().capture;
        let key = Zeroizing::new(store.keys.db_key()?);
        let debit = founder::open_debit_data(&key, capture.invocation, &capture.debit.frame)?;
        let resources = Self {
            body: Arc::new(ResourceBody {
                original_read: source.source().observe_original(),
                original_seed: source.seed().observe_original_seed_data(),
                mount: store.registry_mount(),
                negative: store.chat_mount.observer(),
                expected: SavedSourceExpectationData {
                    invocation: capture.invocation,
                    debit_id: founder::DebitId::of(&capture.debit.frame),
                    terminal_id: founder::IntentId::of(&capture.terminal.frame),
                    source_id: capture.source_id,
                    marker_id: founder::TargetMarkerId::of(&capture.origin.frame),
                    context: capture.context.clone(),
                    debit_display_name: debit.name,
                },
                fee,
            }),
        };
        resources.validate_original(store, source)?;
        source.validate_original(store)?;
        store.validate_mounted_current_layout()?;
        Ok(resources)
    }

    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        if Arc::ptr_eq(&self.body, &other.body) {
            Ok(())
        } else {
            Err(critical_namespaces::refused())
        }
    }

    /// Cached negative/identity proof only: no pool, filesystem, host mutex, SDK or encoding.
    /// Use this AFTER full comparer/fee waits and final original named-object checks.
    pub(in crate::store) fn require_original_negative(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        source.require_original_negative(store)?;
        self.body
            .original_read
            .require_same_original(source.source())?;
        self.body
            .original_seed
            .require_same_seed_data(source.seed())?;
        if self.body.negative.is_cancelled()
            || !Arc::ptr_eq(&self.body.mount, &store.registry_mount())
            || self.body.expected.context != *source.data().context
            || self.body.expected.source_id != source.data().source
        {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }

    /// Bounded validation, NOT a nonblocking predicate: checking the original paid account
    /// can wait on its counter mutex. It acquires no new credit and performs no I/O/SDK work.
    /// Actual source/namespace/file proofs must follow this wait, never precede it alone.
    pub(in crate::store) fn validate_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(store, source)?;
        let policy = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        self.body
            .fee
            .validate(&policy.pending.pool, COLD_CAPTURE_METADATA_FEE)?;
        self.require_original_negative(store, source)
    }
}

/// Captured SAME physical objects plus bounded exact ciphertext. Basis is historical DATA;
/// its equality never authenticates a file or substitutes for a scope owner's live guard.
/// A capture selects exactly one physical role. An absent seed is selected only by the
/// actual unowned Current2 reader after genuine NotFound under the original directories.
/// Once selected, neither a later seed appearance nor a failed present-seed read changes it.
enum ColdSeedSelection {
    Present(ReadControl),
    Absent,
}
impl ColdSeedSelection {
    /// The cleanup borrower may use only a real retained original seed, never a synthetic
    /// frame/id after settlement. Descendant settlement code cannot turn absence into bytes.
    fn present(&self) -> Result<&ReadControl, AppError> {
        match self {
            Self::Present(seed) => Ok(seed),
            Self::Absent => Err(critical_namespaces::refused()),
        }
    }
}

struct CompleteColdClosure {
    basis: ColdV2GenesisScopeBasis,
    group_path: PathBuf,
    seed_path: PathBuf,
    pin_path: PathBuf,
    head_path: PathBuf,
    stage_path: PathBuf,
    group_directory: File,
    group_id: catcoms_storage::kept::SessionId,
    target_directory: File,
    target_id: catcoms_storage::kept::SessionId,
    seed: ColdSeedSelection,
    pin: ReadControl,
    head: ReadControl,
    // LAST: the SAME prepaid loan covers every original file, frame, path and basis copy.
    resources: ColdV2GenesisReadResources,
}

/// Complete seed-present cold Initializing1 proof. It neither repairs partial objects nor
/// installs an owner; Scope needs its distinct cold preparation before an exact continuation.
#[cfg(test)]
pub(in crate::store) struct ColdV2GenesisActivationSeed {
    closure: Arc<CompleteColdClosure>,
}

/// Complete cold genesis Current2 proof. The private selection retains either its actual
/// seed or genuine absence plus the complete strict inventory. Missing pin/head, a malformed
/// present seed or ANY head.stage refuses; no failed read reconstructs a warm SavedHead.
pub(in crate::store) struct ColdV2GenesisCurrentHead {
    closure: Arc<CompleteColdClosure>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    #[cfg(test)]
    Initializing,
    Current,
}

/// Mutable capture is owned outside ALL writers. Results are cached before returning, so a
/// lost reply can only return the SAME selected objects/fee, never recapture equal replacements.
/// The private role latch prevents reusing one attempt across two distinct admission roles.
pub(in crate::store) struct PreparedColdV2GenesisCapture {
    role: Option<Role>,
    // Cache the complete SAME held selection BEFORE authentication/comparison/fsync can
    // fail. In particular an uncertain barrier must not let retry adopt equal replacements.
    captured: Option<Arc<CompleteColdClosure>>,
    // Partial read failures occur before a complete original selection exists. This attempt
    // stays closed; a separate paid cold read may start again, never this old object's retry.
    partial_read_failed: bool,
    #[cfg(test)]
    initializing: Option<Arc<ColdV2GenesisActivationSeed>>,
    current: Option<Arc<ColdV2GenesisCurrentHead>>,
    #[cfg(test)]
    after_receipt_allocation: Option<Box<dyn FnOnce() + Send>>,
    // LAST; each guard borrows a separately owned clone, made before it enters Runtime.
    resources: ColdV2GenesisReadResources,
}

macro_rules! sanitized_cold_debug {
    ($($ty:ident),* $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )*};
}
sanitized_cold_debug!(
    ColdV2GenesisReadResources,
    ColdV2GenesisCurrentHead,
    PreparedColdV2GenesisCapture
);
#[cfg(test)]
sanitized_cold_debug!(ColdV2GenesisActivationSeed);

/// Internal sum of the TWO real borrowed guard types, not a caller-selected role/boolean.
/// Keeping the concrete type here prevents a trait adopter or arbitrary callback from
/// replacing the original scope/namespace/Root validation at the final admission edge.
enum ReadGuard<'guard, 'reader, 'writer> {
    #[cfg(test)]
    Initializing(&'guard ColdV2InitializingGroupReadGuard<'reader, 'writer>),
    Current(&'guard ColdV2CurrentGroupReadGuard<'reader, 'writer>),
}
impl ReadGuard<'_, '_, '_> {
    fn permits_settled_seed_selection(&self) -> bool {
        // This is a closed concrete role match, not a caller-provided fallback option.
        match self {
            #[cfg(test)]
            Self::Initializing(_) => false,
            Self::Current(_) => true,
        }
    }
    fn store(&self) -> &ServerStore {
        match self {
            #[cfg(test)]
            Self::Initializing(g) => g.store(),
            Self::Current(g) => g.store(),
        }
    }
    fn source(&self) -> &GenesisSourceDataRef<'_> {
        match self {
            #[cfg(test)]
            Self::Initializing(g) => g.source(),
            Self::Current(g) => g.source(),
        }
    }
    fn root(&self) -> &critical_namespaces::archive_bootstrap::MountedRoot {
        match self {
            #[cfg(test)]
            Self::Initializing(g) => g.archive_root(),
            Self::Current(g) => g.archive_root(),
        }
    }
    fn directory_data(&self) -> (&Path, &File, catcoms_storage::kept::SessionId) {
        match self {
            #[cfg(test)]
            Self::Initializing(g) => g.directory_data(),
            Self::Current(g) => g.directory_data(),
        }
    }
    fn resources(&self) -> &ColdV2GenesisReadResources {
        match self {
            #[cfg(test)]
            Self::Initializing(g) => g.resources(),
            Self::Current(g) => g.resources(),
        }
    }
    fn basis(&self) -> &ColdV2GenesisScopeBasis {
        match self {
            #[cfg(test)]
            Self::Initializing(g) => g.scope_basis_data(),
            Self::Current(g) => g.scope_basis_data(),
        }
    }
    fn validate_source_only(&self) -> Result<(), AppError> {
        match self {
            #[cfg(test)]
            Self::Initializing(g) => g.validate_source_only(),
            Self::Current(g) => g.validate_source_only(),
        }
    }
}

impl PreparedColdV2GenesisCapture {
    /// Actual SourceSaved/read/SDK plus independent metadata payment, outside host writers.
    /// No nonce, source-write unit, graph creation, scope owner or filesystem mutation occurs.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<Self, AppError> {
        validate_cold_invoice()?;
        let resources = ColdV2GenesisReadResources::prepare_paid(store, source)?;
        Ok(Self {
            role: None,
            captured: None,
            partial_read_failed: false,
            #[cfg(test)]
            initializing: None,
            current: None,
            #[cfg(test)]
            after_receipt_allocation: None,
            resources,
        })
    }

    /// Clone BEFORE Runtime/host writers. Factory borrows this owned alias, not `&mut self`.
    pub(in crate::store) fn read_resources(&self) -> ColdV2GenesisReadResources {
        self.resources.clone()
    }

    fn bind_role(&mut self, role: Role) -> Result<(), AppError> {
        match self.role {
            None => {
                self.role = Some(role);
                Ok(())
            }
            Some(original) if original == role => Ok(()),
            Some(_) => Err(critical_namespaces::refused()),
        }
    }

    fn complete_for_guard(
        &mut self,
        guard: &ReadGuard<'_, '_, '_>,
    ) -> Result<Arc<CompleteColdClosure>, AppError> {
        if self.partial_read_failed {
            return Err(critical_namespaces::refused());
        }
        if self.captured.is_none() {
            match capture_complete(&self.resources, guard) {
                Ok(closure) => self.captured = Some(Arc::new(closure)),
                Err(error) => {
                    self.partial_read_failed = true;
                    return Err(error);
                }
            }
        }
        let closure = self
            .captured
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        closure.require_original_content(guard.store(), guard.source())?;
        closure.validate_for_guard(guard)?;
        closure.sync_existing(guard.source(), guard.root())?;
        closure.validate_for_guard(guard)?;
        Ok(closure.clone())
    }

    #[cfg(test)]
    pub(in crate::store) fn capture_initializing(
        &mut self,
        guard: &ColdV2InitializingGroupReadGuard<'_, '_>,
    ) -> Result<Arc<ColdV2GenesisActivationSeed>, AppError> {
        self.resources.require_same_original(guard.resources())?;
        self.bind_role(Role::Initializing)?;
        let guard = ReadGuard::Initializing(guard);
        if let Some(saved) = &self.initializing {
            saved.closure.validate_for_guard(&guard)?;
            return Ok(saved.clone());
        }
        let closure = self.complete_for_guard(&guard)?;
        let saved = Arc::new(ColdV2GenesisActivationSeed { closure });
        self.initializing = Some(saved.clone());
        #[cfg(test)]
        if let Some(hook) = self.after_receipt_allocation.take() {
            hook();
        }
        // Cache BEFORE the final post-allocation proof. Allocation can wait while the
        // original mount/source is revoked; that refusal must retain this SAME selection.
        saved.closure.validate_for_guard(&guard)?;
        Ok(saved)
    }

    pub(in crate::store) fn capture_current(
        &mut self,
        guard: &ColdV2CurrentGroupReadGuard<'_, '_>,
    ) -> Result<Arc<ColdV2GenesisCurrentHead>, AppError> {
        self.resources.require_same_original(guard.resources())?;
        self.bind_role(Role::Current)?;
        let guard = ReadGuard::Current(guard);
        if let Some(saved) = &self.current {
            saved.closure.validate_for_guard(&guard)?;
            return Ok(saved.clone());
        }
        let closure = self.complete_for_guard(&guard)?;
        let saved = Arc::new(ColdV2GenesisCurrentHead { closure });
        self.current = Some(saved.clone());
        #[cfg(test)]
        if let Some(hook) = self.after_receipt_allocation.take() {
            hook();
        }
        saved.closure.validate_for_guard(&guard)?;
        Ok(saved)
    }
}

// Root-family read helper only. It creates no capture/role and is also used by the distinct
// authenticated Initializing1 repair capture; its fixed maximum remains caller-checked.
pub(super) fn read_cold_control(path: &Path, maximum: usize) -> Result<ReadControl, AppError> {
    let (file, frame) = crate::store::persistence::read_archive_object(path, maximum)?
        .ok_or_else(critical_namespaces::refused)?;
    let id = catcoms_storage::kept::session_id(&file).map_err(io)?;
    let control = ReadControl {
        object: ReadFile {
            file,
            id,
            len: frame.len(),
        },
        frame,
    };
    verify_read_name(path, &control.object)?;
    Ok(control)
}

fn capture_complete(
    resources: &ColdV2GenesisReadResources,
    guard: &ReadGuard<'_, '_, '_>,
) -> Result<CompleteColdClosure, AppError> {
    resources.require_same_original(guard.resources())?;
    resources.validate_original(guard.store(), guard.source())?;
    guard.validate_source_only()?;
    guard
        .store()
        .validate_mounted_current_layout_under_archive_root(guard.root())?;
    let (base, directory, group_id) = guard.directory_data();
    let capture = &guard.source().source().capture;
    let target = &capture.target;
    let group_path = owned_path(base)?;
    let seed_path = child(base, SEED)?;
    let pin_path = child(base, PIN)?;
    let head_path = child(target, HEAD)?;
    let stage_path = child(target, STAGE)?;
    super::super::require_absent(&stage_path)?;
    let seed = match fs::symlink_metadata(&seed_path) {
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && guard.permits_settled_seed_selection() =>
        {
            // NotFound alone could mean a missing ancestor. Compare both original held
            // directory objects BEFORE selecting absence; the complete closure repeats them.
            require_directory_name(base, directory, group_id)?;
            require_directory_name(target, &capture.target_directory, capture.target_id)?;
            ColdSeedSelection::Absent
        }
        Err(_) => return Err(critical_namespaces::refused()),
        Ok(_) => {
            // Every present object takes the original strict reader. Partial/authentication/
            // barrier failures are never caught and reinterpreted as a settled seed.
            ColdSeedSelection::Present(read_cold_control(&seed_path, codec::MAX_ACTIVATION_FRAME)?)
        }
    };
    let pin = read_cold_control(&pin_path, codec::MAX_PIN_FRAME)?;
    let head = read_cold_control(&head_path, codec::MAX_HEAD_FRAME)?;
    let group_directory = directory.try_clone().map_err(io)?;
    let target_directory = capture.target_directory.try_clone().map_err(io)?;
    let closure = CompleteColdClosure {
        basis: guard.basis().clone(),
        group_path,
        seed_path,
        pin_path,
        head_path,
        stage_path,
        group_directory,
        group_id,
        target_directory,
        target_id: capture.target_id,
        seed,
        pin,
        head,
        resources: resources.clone(),
    };
    // Caller caches this complete held selection BEFORE any authentication/comparison or
    // durability effect. Nothing here claims it is valid, Saved or current merely by reading.
    Ok(closure)
}

pub(super) fn owned_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().len() > MAX_PATH + 256 {
        return Err(critical_namespaces::refused());
    }
    let mut owned = PathBuf::with_capacity(path.as_os_str().len());
    owned.push(path);
    if owned.capacity() > MAX_PATH + 256 {
        return Err(critical_namespaces::refused());
    }
    Ok(owned)
}

impl CompleteColdClosure {
    fn require_original_content(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        if matches!(self.seed, ColdSeedSelection::Absent) {
            // The settled branch independently authenticates the selected pin/head against
            // the actual source; it never reconstructs a warm activation receipt or S0 head.
            return settled::require_original_genesis_content(self, store, source);
        }
        let seed = self.seed.present()?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = codec::open_activation_seed_data(&key, &seed.frame)?;
        let decoded =
            codec::parse_activation_seed_data(&key, self.resources.expectation_data(), &plain)?;
        let public = source.data();
        decoded.head.require_original_public_data(
            &decoded.pin,
            &codec::GenesisPublicData {
                profile: public.profile,
                admission: public.admission,
                header: public.header,
                image: public.state,
            },
        )?;
        if decoded.pin_frame != self.pin.frame.as_slice()
            || decoded.head_frame != self.head.frame.as_slice()
        {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }

    fn validate_for_guard(&self, guard: &ReadGuard<'_, '_, '_>) -> Result<(), AppError> {
        self.resources.require_same_original(guard.resources())?;
        if &self.basis != guard.basis() {
            return Err(critical_namespaces::refused());
        }
        guard.validate_source_only()?;
        let (path, file, id) = guard.directory_data();
        if path != self.group_path
            || id != self.group_id
            || catcoms_storage::kept::session_id(file).map_err(io)? != self.group_id
        {
            return Err(critical_namespaces::refused());
        }
        self.validate_under_archive_root(guard.store(), guard.source(), guard.root())?;
        guard.validate_source_only()
    }

    /// Full PHYSICAL/source proof deliberately independent of owner absence or historical
    /// row1/row2. Scope separately checks the original basis before NEW owner insertion and
    /// the SAME owned preparation after a legitimate row transition. This receipt cannot
    /// make an owner appear, keep it alive or authorize a different/current row.
    fn validate_under_archive_root(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        self.resources.validate_original(store, source)?;
        store.validate_mounted_current_layout_under_archive_root(root)?;
        source.source().validate_under_archive_root(store, root)?;
        self.require_original_content(store, source)?;
        super::super::require_absent(&self.stage_path)?;
        match &self.seed {
            ColdSeedSelection::Present(seed) => verify_control(&self.seed_path, seed)?,
            ColdSeedSelection::Absent => settled::require_absent_inventory(self)?,
        }
        verify_control(&self.pin_path, &self.pin)?;
        verify_control(&self.head_path, &self.head)?;
        // All bounded content comparers/account waits precede these final original proofs.
        source.source().validate_under_archive_root(store, root)?;
        #[cfg(test)]
        if let Some(hook) = AFTER_FINAL_SOURCE.with(|slot| slot.borrow_mut().take()) {
            hook();
        }
        if matches!(self.seed, ColdSeedSelection::Absent) {
            // Source reads may wait while an incomplete newer operation creates a private
            // object. Repeat the full closed inventory after that wait, then repeat ID3.
            settled::require_absent_inventory(self)?;
        }
        // Source-control reads can wait. The last ID3 proof must follow them, so a layout
        // marker replaced during that work cannot reuse a positive proof from beforehand.
        store.validate_mounted_current_layout_under_archive_root(root)?;
        let capture = &source.source().capture;
        require_directory_name(&capture.root, &capture.root_directory, capture.root_id)?;
        require_directory_name(&capture.target, &self.target_directory, self.target_id)?;
        require_directory_name(&self.group_path, &self.group_directory, self.group_id)?;
        match &self.seed {
            ColdSeedSelection::Present(seed) => verify_read_name(&self.seed_path, &seed.object)?,
            ColdSeedSelection::Absent => super::super::require_absent(&self.seed_path)?,
        }
        verify_read_name(&self.pin_path, &self.pin.object)?;
        verify_read_name(&self.head_path, &self.head.object)?;
        super::super::require_absent(&self.stage_path)?;
        self.resources.require_original_negative(store, source)
    }

    fn sync_existing(
        &self,
        source: &GenesisSourceDataRef<'_>,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        // Files were opened R/W without create/truncate so Windows can actually flush them.
        // These fixed metadata destinations are outside all epoch inventory families.
        if let ColdSeedSelection::Present(seed) = &self.seed {
            seed.object.file.sync_all().map_err(io)?;
        }
        self.pin.object.file.sync_all().map_err(io)?;
        self.head.object.file.sync_all().map_err(io)?;
        self.group_directory.sync_all().map_err(io)?;
        self.target_directory.sync_all().map_err(io)?;
        root.directory.sync_all().map_err(io)?;
        source.source().capture.require_open()
    }
}

macro_rules! cold_receipt_methods {
    ($ty:ident) => {
        impl $ty {
            pub(in crate::store) fn scope_basis_data(&self) -> &ColdV2GenesisScopeBasis {
                &self.closure.basis
            }
            pub(in crate::store) fn expectation_data(&self) -> &SavedSourceExpectationData {
                self.closure.resources.expectation_data()
            }
            pub(in crate::store) fn directory_data(
                &self,
            ) -> (&Path, &File, catcoms_storage::kept::SessionId) {
                (
                    &self.closure.group_path,
                    &self.closure.group_directory,
                    self.closure.group_id,
                )
            }
            #[cfg(test)]
            pub(in crate::store) fn activation_id_data(&self) -> Option<V2ActivationSeedId> {
                match &self.closure.seed {
                    ColdSeedSelection::Present(seed) => Some(V2ActivationSeedId::of(&seed.frame)),
                    ColdSeedSelection::Absent => None,
                }
            }
            #[cfg(test)]
            pub(in crate::store) fn head_id_data(&self) -> V2GenesisHeadId {
                V2GenesisHeadId::of(&self.closure.head.frame)
            }
            #[cfg(test)]
            pub(in crate::store) fn pin_id_data(&self) -> V2GroupPinId {
                V2GroupPinId::of(&self.closure.pin.frame)
            }
            pub(in crate::store) fn retains_original(
                &self,
                store: &ServerStore,
                source: &GenesisSourceDataRef<'_>,
            ) -> Result<(), AppError> {
                self.closure
                    .resources
                    .require_original_negative(store, source)
            }
            pub(in crate::store) fn validate_under_archive_root(
                &self,
                store: &ServerStore,
                source: &GenesisSourceDataRef<'_>,
                root: &critical_namespaces::archive_bootstrap::MountedRoot,
            ) -> Result<(), AppError> {
                self.closure
                    .validate_under_archive_root(store, source, root)
            }
        }
    };
}
#[cfg(test)]
cold_receipt_methods!(ColdV2GenesisActivationSeed);
cold_receipt_methods!(ColdV2GenesisCurrentHead);

impl ColdV2GenesisCurrentHead {
    /// DATA classification of the actual cached selection, not a durability receipt. Scope
    /// must pay and complete a new covering write before final output for the absent role.
    pub(in crate::store) fn requires_scope_covering(&self) -> bool {
        matches!(self.closure.seed, ColdSeedSelection::Absent)
    }
    /// Clone this SAME paid loan outside admission before borrowing the actual cold guard.
    /// No new body, source capture or authority is created by the Arc clone.
    pub(in crate::store) fn read_resources(&self) -> ColdV2GenesisReadResources {
        self.closure.resources.clone()
    }
}

fn validate_cold_invoice() -> Result<(), AppError> {
    // Include the simultaneously held Factory guards. These paths are additional to the
    // closure's own paths; there is no overlap credit in this bounded concrete invoice.
    let bytes = 8 * (MAX_PATH + 256)
        + 12 * (codec::MAX_ACTIVATION_FRAME + codec::MAX_PIN_FRAME + codec::MAX_HEAD_FRAME)
        + COLD_READ_RESOURCE_BODY_BYTES
        + 2 * std::mem::size_of::<CompleteColdClosure>()
        + std::mem::size_of::<PreparedColdV2GenesisCapture>()
        + 2 * std::mem::size_of::<ColdV2GenesisCurrentHead>()
        + std::mem::size_of::<ColdV2GenesisScopeBasis>()
        + 2 * crate::store::persistence::ORIGINAL_BOOTSTRAP_READ_OBSERVATION_BYTES
        + 2 * (std::mem::size_of::<OriginalV2GenesisSeedObservation>() + 64)
        + crate::store::critical_namespaces::current_layout::COLD_V2_GROUP_READ_GUARD_BYTES
        + crate::store::chat_admission::v2_current::COLD_V2_CURRENT_SCOPE_METADATA_BYTES
        + super::cold_writer::COLD_WRITER_RECEIPT_METADATA_BYTES
        + super::genesis_inventory::GENESIS_FIXED_INVENTORY_BYTES
        + 32 * 1024;
    if bytes > COLD_CAPTURE_METADATA_FEE {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}

// Both receipts own exactly one closure Arc. Keep the same conservative two-receipt
// allowance after removing the unused Initializing1 production entry point.
#[cfg(test)]
const _: () = {
    assert!(
        std::mem::size_of::<ColdV2GenesisActivationSeed>()
            == std::mem::size_of::<ColdV2GenesisCurrentHead>()
    );
};

#[cfg(test)]
mod tests;
