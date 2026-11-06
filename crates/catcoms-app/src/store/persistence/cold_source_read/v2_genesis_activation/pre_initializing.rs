//! Complete original activation journal BEFORE its first mounted scope row. This physical
//! reader never treats a missing/torn journal as empty, invents a row, or adopts a warm
//! Saved receipt. Scope separately owns the actual absence/row1 admission and its sender.
//!
//! The born successor deliberately copies the same selected objects into an independently
//! paid normal repair body. Only its temporary receipt links the two physical generations;
//! dropping that linkage outside writers can release the old metadata class before Current.
use super::*;
use crate::store::chat_admission::v2_current::{
    ColdV2GenesisScopeBasis, ColdV2GenesisVacancyBasis,
};
use crate::store::critical_namespaces::current_layout::{
    ColdV2GenesisVacantGroupReadGuard, DurableBornV2GenesisRow1GroupGuard,
};
use catcoms_rt::RequestCancellation;
use std::sync::OnceLock;

mod inventory;

struct PreCandidates {
    pin: Zeroizing<Vec<u8>>,
    head: Zeroizing<Vec<u8>>,
}

struct PreFiles {
    vacancy: ColdV2GenesisVacancyBasis,
    group_path: PathBuf,
    seed_path: PathBuf,
    pin_path: PathBuf,
    head_path: PathBuf,
    stage_path: PathBuf,
    group_directory: File,
    group_id: catcoms_storage::kept::SessionId,
    target_directory: File,
    target_id: catcoms_storage::kept::SessionId,
    seed: ReadControl,
    candidates: OnceLock<PreCandidates>,
    // LAST: all paths, original Files, bounded frames and weak identities drain first.
    resources: ColdV2GenesisReadResources,
}

/// New physical journal receipt. Its vacancy basis records an ACTUAL historical capture,
/// not a planned Record or permanent assertion that Runtime must remain ownerless.
pub(in crate::store) struct ColdV2PreInitializingSeed {
    files: Arc<PreFiles>,
}

/// Retain this outside all writers. A failed incomplete selection closes only this attempt;
/// once selected, authentication/barrier/refusal retries use exactly those same held objects.
pub(in crate::store) struct PreparedColdV2PreInitializingCapture {
    selected: Option<Arc<PreFiles>>,
    saved: Option<Arc<ColdV2PreInitializingSeed>>,
    selection_failed: bool,
    // LAST; Factory borrows separately cloned loans made BEFORE entering admission.
    resources: ColdV2GenesisReadResources,
}

struct BornOrigin {
    task: OriginalV2GenesisGroupObservation,
    identity: Arc<()>,
    basis: ColdV2GenesisScopeBasis,
    // This already-funded negative observer keeps neither a sender nor source graph alive.
    negative: RequestCancellation,
}

/// Owns a SECOND actual resource body acquired before Runtime. The original pre-seed
/// payment is not relabelled, augmented by a sidecar, or moved into the normal repair seed.
pub(in crate::store) struct PreparedBornV2GenesisRepairCapture {
    origin: Option<BornOrigin>,
    repair: Option<Arc<initializing_repair::ColdV2GenesisRepairSeed>>,
    saved: Option<Arc<BornV2GenesisRepairSeed>>,
    pre_initializing: Arc<ColdV2PreInitializingSeed>,
    resources: ColdV2GenesisReadResources,
}

/// Temporary checked bridge only. The normal RepairSeed has NO backlink to this receipt
/// or the original pre-seed. Scope/task conversion retains the same owner and drains these
/// old origin/capture links outside all writers before entering normal repair/Current.
pub(in crate::store) struct BornV2GenesisRepairSeed {
    pre_initializing: Arc<ColdV2PreInitializingSeed>,
    repair: Arc<initializing_repair::ColdV2GenesisRepairSeed>,
    origin: BornOrigin,
}

sanitized_debug!(
    ColdV2PreInitializingSeed,
    PreparedColdV2PreInitializingCapture,
    PreparedBornV2GenesisRepairCapture,
    BornV2GenesisRepairSeed
);

/// Only the private physical family can obtain these loans, from an actual authenticated
/// pre-seed receipt. No path, File, candidate bytes or planned scope basis enters via a
/// caller constructor. The bridge clones each SAME File and its already-captured identity.
pub(super) struct PreInitializingFileLoan<'original> {
    pub(super) group_path: &'original Path,
    pub(super) seed_path: &'original Path,
    pub(super) pin_path: &'original Path,
    pub(super) head_path: &'original Path,
    pub(super) stage_path: &'original Path,
    pub(super) group_directory: &'original File,
    pub(super) group_id: catcoms_storage::kept::SessionId,
    pub(super) target_directory: &'original File,
    pub(super) target_id: catcoms_storage::kept::SessionId,
    pub(super) seed: &'original ReadControl,
    pub(super) pin_frame: &'original [u8],
    pub(super) head_frame: &'original [u8],
}

impl PreparedColdV2PreInitializingCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<Self, AppError> {
        initializing_repair::require_supported_profile()?;
        validate_pre_invoice()?;
        let resources = ColdV2GenesisReadResources::prepare_paid(store, source)?;
        Ok(Self {
            selected: None,
            saved: None,
            selection_failed: false,
            resources,
        })
    }

    pub(in crate::store) fn read_resources(&self) -> ColdV2GenesisReadResources {
        self.resources.clone()
    }

    #[cfg(all(test, windows))]
    pub(in crate::store) fn observe_metadata_drop_for_test(
        &mut self,
        notice: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(), AppError> {
        self.resources.observe_fee_drop_for_test(notice)
    }

    pub(in crate::store) fn capture_vacant(
        &mut self,
        guard: &ColdV2GenesisVacantGroupReadGuard<'_, '_>,
    ) -> Result<Arc<ColdV2PreInitializingSeed>, AppError> {
        self.resources.require_same_original(guard.resources())?;
        guard.validate_source_only()?;
        if self.selection_failed {
            return Err(critical_namespaces::refused());
        }
        if self.selected.is_none() {
            match select_files(&self.resources, guard) {
                Ok(files) => self.selected = Some(Arc::new(files)),
                Err(error) => {
                    self.selection_failed = true;
                    return Err(error);
                }
            }
        }
        let files = self
            .selected
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        if &files.vacancy != guard.vacancy_basis_data() {
            return Err(critical_namespaces::refused());
        }
        files.validate_under_archive_root(guard.store(), guard.source(), guard.archive_root())?;
        // A cold loaded file is not a new durability proof. These ORIGINAL object barriers
        // precede the receipt; the separate actual row1 covering write still follows it.
        files.seed.object.file.sync_all().map_err(io)?;
        files.group_directory.sync_all().map_err(io)?;
        files.target_directory.sync_all().map_err(io)?;
        guard.archive_root().directory.sync_all().map_err(io)?;
        files.validate_under_archive_root(guard.store(), guard.source(), guard.archive_root())?;
        if self.saved.is_none() {
            self.saved = Some(Arc::new(ColdV2PreInitializingSeed {
                files: files.clone(),
            }));
        }
        let saved = self
            .saved
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        run_hook(TestEdge::Receipt);
        saved.validate_under_archive_root(guard.store(), guard.source(), guard.archive_root())?;
        guard.validate_source_only()?;
        Ok(saved.clone())
    }
}

fn select_files(
    resources: &ColdV2GenesisReadResources,
    guard: &ColdV2GenesisVacantGroupReadGuard<'_, '_>,
) -> Result<PreFiles, AppError> {
    resources.validate_original(guard.store(), guard.source())?;
    guard.validate_source_only()?;
    let (base, directory, group_id) = guard.directory_data();
    let capture = &guard.source().source().capture;
    Ok(PreFiles {
        vacancy: guard.vacancy_basis_data().clone(),
        group_path: cold_capture::owned_path(base)?,
        seed_path: child(base, SEED)?,
        pin_path: child(base, PIN)?,
        head_path: child(&capture.target, HEAD)?,
        stage_path: child(&capture.target, STAGE)?,
        group_directory: directory.try_clone().map_err(io)?,
        group_id,
        target_directory: capture.target_directory.try_clone().map_err(io)?,
        target_id: capture.target_id,
        seed: cold_capture::read_cold_control(&child(base, SEED)?, codec::MAX_ACTIVATION_FRAME)?,
        candidates: OnceLock::new(),
        resources: resources.clone(),
    })
}

impl PreFiles {
    fn authenticate_candidates(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.resources.validate_original(store, source)?;
        verify_control(&self.seed_path, &self.seed)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = codec::open_activation_seed_data(&key, &self.seed.frame)?;
        let decoded =
            codec::parse_activation_seed_data(&key, self.resources.expectation_data(), &plain)?;
        let data = source.data();
        decoded.head.require_original_public_data(
            &decoded.pin,
            &codec::GenesisPublicData {
                profile: data.profile,
                admission: data.admission,
                header: data.header,
                image: data.state,
            },
        )?;
        if let Some(original) = self.candidates.get() {
            if original.pin.as_slice() != decoded.pin_frame
                || original.head.as_slice() != decoded.head_frame
            {
                return Err(critical_namespaces::refused());
            }
        } else {
            self.candidates
                .set(PreCandidates {
                    pin: Zeroizing::new(decoded.pin_frame.to_vec()),
                    head: Zeroizing::new(decoded.head_frame.to_vec()),
                })
                .map_err(|_| critical_namespaces::refused())?;
        }
        self.resources.require_original_negative(store, source)
    }

    fn require_targets_absent(&self) -> Result<(), AppError> {
        for path in [&self.pin_path, &self.head_path, &self.stage_path] {
            super::super::require_absent(path)?;
        }
        Ok(())
    }

    fn require_inventory(&self, source: &GenesisSourceDataRef<'_>) -> Result<(), AppError> {
        inventory::require_pre_initializing_inventory(
            (&self.group_path, &self.group_directory, self.group_id),
            (
                &source.source().capture.target,
                &self.target_directory,
                self.target_id,
            ),
        )
    }

    fn validate_under_archive_root(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        self.authenticate_candidates(store, source)?;
        self.require_targets_absent()?;
        self.require_inventory(source)?;
        run_hook(TestEdge::Comparers);
        source.source().validate_under_archive_root(store, root)?;
        self.require_inventory(source)?;
        source.source().validate_under_archive_root(store, root)?;
        run_hook(TestEdge::Source);
        // No resource/codec/comparer can follow this last namespace/held-name edge.
        store.validate_mounted_current_layout_under_archive_root(root)?;
        let capture = &source.source().capture;
        require_directory_name(&capture.root, &capture.root_directory, capture.root_id)?;
        require_directory_name(&capture.target, &self.target_directory, self.target_id)?;
        require_directory_name(&self.group_path, &self.group_directory, self.group_id)?;
        verify_read_name(&self.seed_path, &self.seed.object)?;
        self.require_targets_absent()?;
        self.resources.require_original_negative(store, source)
    }
}

impl ColdV2PreInitializingSeed {
    pub(in crate::store) fn read_resources(&self) -> ColdV2GenesisReadResources {
        self.files.resources.clone()
    }
    pub(in crate::store) fn vacancy_basis_data(&self) -> &ColdV2GenesisVacancyBasis {
        &self.files.vacancy
    }
    pub(in crate::store) fn expectation_data(&self) -> &SavedSourceExpectationData {
        self.files.resources.expectation_data()
    }
    pub(in crate::store) fn retains_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.files
            .resources
            .require_original_negative(store, source)
    }
    pub(in crate::store) fn validate_under_archive_root(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        // Intentionally physical only: the exact scope owner may now exist. Scope checks
        // its own phase/lineage instead of turning historical absence into a lasting permit.
        self.files.validate_under_archive_root(store, source, root)
    }
    pub(super) fn original_files(&self) -> Result<PreInitializingFileLoan<'_>, AppError> {
        let files = &self.files;
        let candidates = files
            .candidates
            .get()
            .ok_or_else(critical_namespaces::refused)?;
        Ok(PreInitializingFileLoan {
            group_path: &files.group_path,
            seed_path: &files.seed_path,
            pin_path: &files.pin_path,
            head_path: &files.head_path,
            stage_path: &files.stage_path,
            group_directory: &files.group_directory,
            group_id: files.group_id,
            target_directory: &files.target_directory,
            target_id: files.target_id,
            seed: &files.seed,
            pin_frame: &candidates.pin,
            head_frame: &candidates.head,
        })
    }
}

impl PreparedBornV2GenesisRepairCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        pre_initializing: Arc<ColdV2PreInitializingSeed>,
    ) -> Result<Self, AppError> {
        initializing_repair::require_supported_profile()?;
        validate_born_invoice()?;
        pre_initializing.retains_original(store, source)?;
        let resources = ColdV2GenesisReadResources::prepare_paid(store, source)?;
        pre_initializing.retains_original(store, source)?;
        Ok(Self {
            origin: None,
            repair: None,
            saved: None,
            pre_initializing,
            resources,
        })
    }
    pub(in crate::store) fn read_resources(&self) -> ColdV2GenesisReadResources {
        self.resources.clone()
    }

    pub(in crate::store) fn capture_born(
        &mut self,
        guard: &DurableBornV2GenesisRow1GroupGuard<'_, '_>,
    ) -> Result<Arc<BornV2GenesisRepairSeed>, AppError> {
        self.resources.require_same_original(guard.resources())?;
        guard.validate_source_only()?;
        if !Arc::ptr_eq(&self.pre_initializing, guard.pre_initializing_seed()) {
            return Err(critical_namespaces::refused());
        }
        if self.origin.is_none() {
            self.origin = Some(BornOrigin {
                task: guard.task().observe_original(),
                identity: guard.owner_identity().clone(),
                basis: guard.scope_basis_data().clone(),
                negative: guard.owner_observer().clone(),
            });
        }
        let origin = self
            .origin
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        origin.require_guard(guard)?;
        if self.repair.is_none() {
            // This private family constructor borrows ONLY this concrete post-barrier role.
            // It copies actual ColdBasis, never a planned Record or historical absence.
            self.repair =
                Some(initializing_repair::ColdV2GenesisRepairSeed::from_durable_born(guard)?);
        }
        let repair = self
            .repair
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        if self.saved.is_none() {
            self.saved = Some(Arc::new(BornV2GenesisRepairSeed {
                pre_initializing: self.pre_initializing.clone(),
                repair: repair.clone(),
                origin: BornOrigin {
                    task: origin.task.clone(),
                    identity: origin.identity.clone(),
                    basis: origin.basis.clone(),
                    negative: origin.negative.clone(),
                },
            }));
        }
        let saved = self
            .saved
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        saved.validate_under_archive_root(guard.store(), guard.source(), guard.archive_root())?;
        origin.require_guard(guard)?;
        guard.validate_source_only()?;
        Ok(saved.clone())
    }
}

impl BornOrigin {
    fn require_guard(
        &self,
        guard: &DurableBornV2GenesisRow1GroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.task.require_same_original(guard.task())?;
        if !Arc::ptr_eq(&self.identity, guard.owner_identity())
            || &self.basis != guard.scope_basis_data()
            || self.negative.is_cancelled()
            || guard.owner_observer().is_cancelled()
        {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }
}

impl BornV2GenesisRepairSeed {
    pub(in crate::store) fn repair_seed(
        &self,
    ) -> &Arc<initializing_repair::ColdV2GenesisRepairSeed> {
        &self.repair
    }
    pub(in crate::store) fn pre_initializing_seed(&self) -> &Arc<ColdV2PreInitializingSeed> {
        &self.pre_initializing
    }
    pub(in crate::store) fn scope_basis_data(&self) -> &ColdV2GenesisScopeBasis {
        self.repair.scope_basis_data()
    }
    pub(in crate::store) fn retains_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        identity: &Arc<()>,
    ) -> Result<(), AppError> {
        self.pre_initializing.retains_original(store, source)?;
        self.repair.retains_original(store, source)?;
        self.origin.task.require_same_original(task)?;
        if !Arc::ptr_eq(identity, &self.origin.identity)
            || self.origin.negative.is_cancelled()
            || self.repair.scope_basis_data() != &self.origin.basis
        {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }
    pub(in crate::store) fn validate_under_archive_root(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        self.pre_initializing
            .validate_under_archive_root(store, source, root)?;
        self.repair
            .validate_under_archive_root(store, source, root)?;
        // The normal repair proof permits authorized prefixes. Birth itself permits NONE:
        // repeat the strict pre-seed absence/source proof AFTER that comparator can wait.
        self.pre_initializing
            .validate_under_archive_root(store, source, root)?;
        if self.origin.negative.is_cancelled() {
            return Err(critical_namespaces::refused());
        }
        self.repair.retains_original(store, source)
    }
}

fn validate_pre_invoice() -> Result<(), AppError> {
    let bytes = 8 * (MAX_PATH + 256)
        + 12 * (codec::MAX_ACTIVATION_FRAME + codec::MAX_PIN_FRAME + codec::MAX_HEAD_FRAME)
        + cold_capture::COLD_READ_RESOURCE_BODY_BYTES
        + 2 * std::mem::size_of::<PreFiles>()
        + std::mem::size_of::<PreparedColdV2PreInitializingCapture>()
        + 2 * std::mem::size_of::<ColdV2PreInitializingSeed>()
        + std::mem::size_of::<ColdV2GenesisVacancyBasis>()
        + crate::store::critical_namespaces::current_layout::COLD_V2_GROUP_READ_GUARD_BYTES
        + 2 * std::mem::size_of::<ColdV2GenesisVacantGroupReadGuard<'static, 'static>>()
        + crate::store::critical_namespaces::current_layout::COLD_PREINITIALIZING_GROUP_GUARD_BYTES
        + crate::store::chat_admission::v2_current::COLD_PREINITIALIZING_SCOPE_METADATA_BYTES
        + super::genesis_inventory::GENESIS_FIXED_INVENTORY_BYTES
        + 8 * 64
        + 16 * 1024;
    if bytes > cold_capture::COLD_CAPTURE_METADATA_FEE {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}
fn validate_born_invoice() -> Result<(), AppError> {
    // This path does not pass through PreparedColdV2GenesisRepairCapture::prepare. Check
    // the existing normal repair/Current invoice too, BEFORE the new fee is acquired.
    // The owning caller drains temporary Born metadata before invoking normal repair;
    // both concrete phase inventories must independently fit this same NEW1MiB class.
    initializing_repair::ColdV2GenesisRepairSeed::require_born_successor_invoice()?;
    // The old pre-seed/capture/guard are separately still paid by THEIR original loan.
    // This SECOND class pays all new RepairFiles/copies, born controls and retained tails.
    let bytes = initializing_repair::ColdV2GenesisRepairSeed::born_metadata_bytes_data()
        + cold_capture::COLD_READ_RESOURCE_BODY_BYTES
        + std::mem::size_of::<PreparedBornV2GenesisRepairCapture>()
        + 2 * std::mem::size_of::<BornOrigin>()
        + 2 * std::mem::size_of::<BornV2GenesisRepairSeed>()
        + 2 * std::mem::size_of::<DurableBornV2GenesisRow1GroupGuard<'static, 'static>>()
        + crate::store::critical_namespaces::current_layout::COLD_PREINITIALIZING_GROUP_GUARD_BYTES
        + crate::store::chat_admission::v2_current::COLD_PREINITIALIZING_SCOPE_METADATA_BYTES
        + 8 * (MAX_PATH + 256)
        + 12 * (codec::MAX_ACTIVATION_FRAME + codec::MAX_PIN_FRAME + codec::MAX_HEAD_FRAME)
        + super::genesis_inventory::GENESIS_FIXED_INVENTORY_BYTES
        + 32 * 1024;
    if bytes > cold_capture::COLD_CAPTURE_METADATA_FEE {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}

// Test callbacks run only AFTER actual comparers/cache seams. They never create a receipt
// or replace an admission, and are removed from RefCell before adversarial code is invoked.
#[derive(Clone, Copy)]
enum TestEdge {
    Comparers,
    Source,
    Receipt,
}
#[cfg(test)]
type TestEdgeHook = Option<(TestEdge, Box<dyn FnOnce()>)>;
#[cfg(test)]
thread_local! {
    static EDGE_HOOK: std::cell::RefCell<TestEdgeHook> = const { std::cell::RefCell::new(None) };
}
fn run_hook(edge: TestEdge) {
    #[cfg(not(test))]
    let _ = edge;
    #[cfg(test)]
    {
        let hook = EDGE_HOOK.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.as_ref().is_some_and(|(expected, _)| {
                std::mem::discriminant(expected) == std::mem::discriminant(&edge)
            }) {
                slot.take().map(|(_, callback)| callback)
            } else {
                None
            }
        });
        if let Some(hook) = hook {
            hook();
        }
    }
}

#[cfg(test)]
mod tests;
