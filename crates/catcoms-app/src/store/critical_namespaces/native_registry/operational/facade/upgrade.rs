//! Owning native factory continuation. The genuine index2 owner moves through
//! its existing paid upgrade and becomes selected3 before any Source or SDK is
//! created. Errors retain this SAME original; no alternate opener is attempted.
use super::super::upgrade_source::{self, owning};
use super::*;
use crate::store::critical_namespaces::native_registry::lifecycle::preparation;
use catcoms_rt::CryptoRngCore;

enum Phase {
    Index2(mount::NativeIndexStore),
    Source(upgrade_source::UpgradeSource),
    Prepared(Box<upgrade_source::PreparedUpgrade>),
    Writing(Box<owning::Upgrade>),
    Selected3(owning::Store),
}

/// Retain outside native/Root guards. This performs local storage migration
/// only; supplied host atomics establish no Tauri caller or UI authorization.
/// Closing/dropping this whole continuation retires its original physical
/// owner. An uncertain attempt must retry this value, never another opener.
pub struct NativeTextVaultUpgrade {
    // UI admission closes before actual index file/policy owners are destroyed.
    ui: Option<UiAdmissionOwner>,
    phase: Option<Phase>,
    policy: preparation::Policy,
    // LAST: pays this controller/phase boxes independently until they retire.
    _metadata: CatalogReadReservation,
}

/// Pre-effect refusal keeps the caller's genuine opaque index2 owner. A caller
/// can retry the same startup plan without reopening files or replacing custody.
pub struct NativeTextVaultUpgradeRefusal {
    original: mount::NativeIndexStore,
    error: AppError,
}
impl NativeTextVaultUpgradeRefusal {
    pub fn into_parts(self) -> (mount::NativeIndexStore, AppError) {
        (self.original, self.error)
    }
}
impl std::fmt::Debug for NativeTextVaultUpgradeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextVaultUpgradeRefusal")
            .finish_non_exhaustive()
    }
}

impl NativeTextVaultUpgrade {
    /// No writes, SDK or device generation. The index is an actual consumed
    /// opaque owner; serialized status and an arbitrary directory cannot stand
    /// in for it. The distinct3 account is created before upgrade preparation.
    pub fn prepare(
        index: mount::NativeIndexStore,
        generation: Arc<AtomicU64>,
        authorized: Arc<AtomicU64>,
        requested: Arc<AtomicBool>,
        budget: NativeUiInvocationBudget,
    ) -> Result<Self, NativeTextVaultUpgradeRefusal> {
        let prepare = (|| -> Result<_, AppError> {
            // Conservative fixed control overlap, separate from the existing
            // upgrade's paid decoded/frame/IO and physical-wrapper terms.
            const METADATA: usize = 4096
                + 16 * (std::mem::size_of::<NativeTextVaultUpgrade>()
                    + std::mem::size_of::<Phase>());
            let metadata = CriticalCatalogReadPool::new(METADATA)?.try_reserve(METADATA)?;
            let ui = UiAdmissionOwner::new(generation, authorized, requested, budget)?;
            let policy = preparation::Policy::new(
                CriticalCatalogReadPool::new(preparation::PREPARATION_FEE)?,
                preparation::Policy::required_persistent_capacity(),
            )?;
            Ok((metadata, ui, policy))
        })();
        match prepare {
            Ok((metadata, ui, policy)) => Ok(Self {
                ui: Some(ui),
                phase: Some(Phase::Index2(index)),
                policy,
                _metadata: metadata,
            }),
            Err(error) => Err(NativeTextVaultUpgradeRefusal {
                original: index,
                error,
            }),
        }
    }

    /// All reads, sealing and writes happen outside native/operation writers.
    /// Every returned refusal caches the original owner BEFORE reporting it.
    /// After preparation, retries never generate a replacement upgrade frame.
    pub fn retry(&mut self, rng: &mut impl CryptoRngCore) -> Result<(), AppError> {
        loop {
            match self.phase.take().ok_or_else(refused)? {
                Phase::Index2(index) => match index.into_upgrade_source() {
                    Ok(source) => self.phase = Some(Phase::Source(source)),
                    Err(index) => {
                        self.phase = Some(Phase::Index2(index));
                        return Err(refused());
                    }
                },
                Phase::Source(source) => match source.prepare(self.policy.clone(), rng) {
                    Ok(prepared) => self.phase = Some(Phase::Prepared(Box::new(prepared))),
                    Err(source) => {
                        self.phase = Some(Phase::Source(source));
                        return Err(refused());
                    }
                },
                Phase::Prepared(prepared) => match (*prepared).admit() {
                    Ok(writing) => self.phase = Some(Phase::Writing(Box::new(writing))),
                    Err(prepared) => {
                        self.phase = Some(Phase::Prepared(prepared));
                        return Err(refused());
                    }
                },
                Phase::Writing(mut writing) => {
                    if let Err(error) = writing.retry() {
                        self.phase = Some(Phase::Writing(writing));
                        return Err(error);
                    }
                    match (*writing).into_store() {
                        Ok(index) => self.phase = Some(Phase::Selected3(index)),
                        Err(writing) => {
                            self.phase = Some(Phase::Writing(writing));
                            return Err(refused());
                        }
                    }
                }
                Phase::Selected3(index) => {
                    self.phase = Some(Phase::Selected3(index));
                    return Ok(());
                }
            }
        }
    }

    /// This moves the whole mounted owner. No borrowed Store/Current, raw
    /// index, Source, signing key or preconstructed backend can escape.
    pub fn into_vault(mut self) -> Result<NativeTextVault, Self> {
        let ready = matches!(self.phase.as_ref(), Some(Phase::Selected3(index)) if index.native_row_count_data().is_ok());
        if !ready {
            return Err(self);
        }
        let Some(Phase::Selected3(index)) = self.phase.take() else {
            unreachable!()
        };
        Ok(NativeTextVault {
            ui: self.ui.take().expect("whole native UI owner"),
            index: IndexOwner::Version3(index),
        })
    }
}
impl std::fmt::Debug for NativeTextVaultUpgrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextVaultUpgrade")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::critical_namespaces::native_registry::preparation::tests::{
        current_policy, initialize, records, SECRET,
    };
    use crate::store::persistence::native_registry::operational as cuts;
    use crate::store::{
        NativeIndexPolicy, NativeIndexUpgrade, NativeRegistryInstallation, NativeRegistryPolicy,
    };
    use rand_chacha::ChaCha20Rng;
    use rand_core::{CryptoRng, RngCore, SeedableRng};
    use std::{path::Path, sync::atomic::Ordering};

    struct NeverRng;
    impl CryptoRng for NeverRng {}
    impl RngCore for NeverRng {
        fn next_u32(&mut self) -> u32 {
            panic!("retained factory drew RNG")
        }
        fn next_u64(&mut self) -> u64 {
            panic!("retained factory drew RNG")
        }
        fn fill_bytes(&mut self, _: &mut [u8]) {
            panic!("retained factory drew RNG")
        }
        fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
            panic!("retained factory drew RNG")
        }
    }
    fn index2(path: &Path, rng: &mut ChaCha20Rng) -> mount::NativeIndexStore {
        initialize(path, Some(&records()), rng);
        let policy = NativeRegistryPolicy::new(
            CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
            NativeRegistryPolicy::required_persistent_capacity(),
        )
        .unwrap();
        let mut first =
            NativeRegistryInstallation::prepare(path, SECRET, rng, current_policy(), policy)
                .unwrap();
        first.retry().unwrap();
        let policy = NativeIndexPolicy::new(
            CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity()).unwrap(),
            NativeIndexPolicy::required_persistent_capacity(),
        )
        .unwrap();
        let mut second =
            NativeIndexUpgrade::prepare(first.into_native_registry().unwrap(), policy, rng)
                .unwrap();
        second.retry().unwrap();
        second.into_index_store().unwrap()
    }
    fn ui() -> NativeUiInvocationBudget {
        NativeUiInvocationBudget::new(
            CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity()).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn native_factory_upgrade_retains_same_index_and_ui_across_uncertain_writes() {
        // These real files and host atoms are Rust storage/lifecycle evidence;
        // no fixture claims to authenticate a Tauri caller or issue an SDK.
        for cut in [
            cuts::Fault::UpgradeIndexRename,
            cuts::Fault::UpgradeCatalogRename,
            // Normal3 retires its seed here; fence-unlink cuts belong
            // exclusively to cold first-torn cleanup, not this writer.
            cuts::Fault::UpgradeSeedUnlink,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut rng = ChaCha20Rng::seed_from_u64(0xcc01);
            let index = index2(dir.path(), &mut rng);
            let generation = Arc::new(AtomicU64::new(7));
            let authorized = Arc::new(AtomicU64::new(7));
            let requested = Arc::new(AtomicBool::new(false));
            let budget = ui();
            let mut plan = NativeTextVaultUpgrade::prepare(
                index,
                generation.clone(),
                authorized.clone(),
                requested,
                budget.clone(),
            )
            .unwrap();
            cuts::arm_fault(cut);
            assert!(plan.retry(&mut rng).is_err(), "actual normal3 cut {cut:?}");
            assert!(cuts::take_fault_seen());
            plan.retry(&mut NeverRng).unwrap();
            let vault = plan.into_vault().unwrap();
            assert_eq!(vault.row_count_data().unwrap(), records().len());
            assert!(vault
                .lookup_creation_data([1; 32], [2; 32])
                .unwrap()
                .is_none());
            let mut candidate = vault
                .capture_ui(7, budget.prepare().unwrap())
                .finish()
                .unwrap();
            let capture = vault.admit_ui(&mut candidate).unwrap();
            vault.require_ui(&capture).unwrap();
            authorized.store(0, Ordering::SeqCst);
            generation.store(8, Ordering::SeqCst);
            assert!(vault.require_ui(&capture).is_err());
            drop(vault);
            let mut cold = mount::NativeIndexStore::open_lifecycle3_for_test(
                dir.path(),
                SECRET,
                &mut NeverRng,
                current_policy(),
            )
            .unwrap();
            assert_eq!(cold.row_count_data().unwrap(), records().len());
        }
    }

    #[test]
    fn native_factory_upgrade_unready_transfer_retains_owner_and_close_fences_selected3() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xcc11);
        let index = index2(dir.path(), &mut rng);
        let generation = Arc::new(AtomicU64::new(7));
        let mut plan = NativeTextVaultUpgrade::prepare(
            index,
            generation,
            Arc::new(AtomicU64::new(7)),
            Arc::new(AtomicBool::new(false)),
            ui(),
        )
        .unwrap()
        .into_vault()
        .unwrap_err();
        plan.retry(&mut rng).unwrap();
        let vault = plan.into_vault().unwrap();
        assert_eq!(vault.row_count_data().unwrap(), records().len());
        vault.close_admission();
        assert!(vault.row_count_data().is_err());
        assert!(vault.lookup_creation_data([1; 32], [2; 32]).is_err());
        drop(vault);
        let mut cold = mount::NativeIndexStore::open_lifecycle3_for_test(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
        )
        .unwrap();
        assert_eq!(cold.row_count_data().unwrap(), records().len());
    }
}
