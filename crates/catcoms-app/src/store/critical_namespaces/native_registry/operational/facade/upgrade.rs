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

    struct AuthFixture {
        vault: NativeTextVault,
        budget: NativeUiInvocationBudget,
        pool: CriticalCatalogReadPool,
        generation: Arc<AtomicU64>,
        authorized: Arc<AtomicU64>,
        requested: Arc<AtomicBool>,
    }
    fn auth_fixture(path: &Path, seed: u64) -> AuthFixture {
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let pool =
            CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity()).unwrap();
        let budget = NativeUiInvocationBudget::new(pool.clone()).unwrap();
        let generation = Arc::new(AtomicU64::new(7));
        let authorized = Arc::new(AtomicU64::new(7));
        let requested = Arc::new(AtomicBool::new(false));
        let mut plan = NativeTextVaultUpgrade::prepare(
            index2(path, &mut rng),
            generation.clone(),
            authorized.clone(),
            requested.clone(),
            budget.clone(),
        )
        .unwrap();
        plan.retry(&mut rng).unwrap();
        AuthFixture {
            vault: plan.into_vault().unwrap(),
            budget,
            pool,
            generation,
            authorized,
            requested,
        }
    }
    fn original_index(
        vault: &NativeTextVault,
    ) -> &super::super::super::upgrade_source::owning::Store {
        match &vault.index {
            IndexOwner::Version3(index) => index,
            _ => panic!("real selected3 control"),
        }
    }
    fn wrapper_calls() -> usize {
        super::super::AUTH_VERIFY_CALLS.with(|count| count.replace(0))
    }

    #[test]
    fn native_auth_check_releases_original_slot_and_session_without_retaining_physical_store() {
        let dir = tempfile::tempdir().unwrap();
        let f = auth_fixture(dir.path(), 0xcd01);
        let baseline = f.pool.charged_for_test();
        let prepared = f
            .vault
            .prepare_native_authentication(f.budget.prepare().unwrap());
        assert!(original_index(&f.vault).capture_metadata_work().is_none());
        assert_eq!(
            f.pool.charged_for_test(),
            baseline + super::super::super::native_ui::INVOCATION_BYTES
        );
        wrapper_calls();
        let check = prepared.verify(SECRET).unwrap();
        assert_eq!(wrapper_calls(), 1);
        assert!(check.is_live_cached());
        let mut next = original_index(&f.vault)
            .capture_metadata_work()
            .expect("original physical slot released before finalization");
        assert_eq!(next.row_count_data().unwrap(), records().len());
        drop(next);
        // UI lock is deliberately narrower than whole mount authentication.
        f.generation.store(8, Ordering::SeqCst);
        f.authorized.store(0, Ordering::SeqCst);
        f.requested.store(true, Ordering::SeqCst);
        assert!(check.is_live_cached());
        drop(f.vault);
        assert!(!check.is_live_cached());
        // This is the actual strict3 storage opener and real vault session.
        // A retained Handler.cold/Physical/Current would return VaultBusy here.
        let mut reopened = mount::NativeIndexStore::open_lifecycle3_for_test(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
        )
        .unwrap();
        assert_eq!(reopened.row_count_data().unwrap(), records().len());
        assert!(
            !check.is_live_cached(),
            "replacement must not retarget the original negative"
        );
        drop(reopened);
        drop(check);
        assert_eq!(f.pool.charged_for_test(), 0);
    }

    #[test]
    fn native_auth_check_validates_original_credit_before_wrapper_and_keeps_refusal_payment() {
        let dir = tempfile::tempdir().unwrap();
        let f = auth_fixture(dir.path(), 0xcd02);
        let foreign_pool =
            CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity()).unwrap();
        let foreign = NativeUiInvocationBudget::new(foreign_pool.clone()).unwrap();
        wrapper_calls();
        let prepared = f
            .vault
            .prepare_native_authentication(foreign.prepare().unwrap());
        assert!(
            prepared.work.is_some(),
            "the original prepaid capture remains separate from later fee validation"
        );
        assert!(prepared.verify(SECRET).is_err());
        assert_eq!(
            wrapper_calls(),
            0,
            "foreign credit reached actual wrapper verification"
        );
        assert_eq!(foreign_pool.charged_for_test(), 0);
        let busy = original_index(&f.vault).capture_metadata_work().unwrap();
        let baseline = f.pool.charged_for_test();
        let prepared = f
            .vault
            .prepare_native_authentication(f.budget.prepare().unwrap());
        assert_eq!(
            f.pool.charged_for_test(),
            baseline + super::super::super::native_ui::INVOCATION_BYTES,
            "Busy capture refunded the caller's owned fee inside its guard"
        );
        assert!(prepared.verify(SECRET).is_err());
        assert_eq!(wrapper_calls(), 0);
        assert_eq!(f.pool.charged_for_test(), baseline);
        drop(busy);
        assert!(f
            .vault
            .prepare_native_authentication(f.budget.prepare().unwrap())
            .verify(SECRET)
            .is_ok());
    }

    #[test]
    fn native_auth_check_independently_tracks_original_host_gate_and_mount() {
        for n in 0..3 {
            let dir = tempfile::tempdir().unwrap();
            let f = auth_fixture(dir.path(), 0xcd10 + n);
            let check = f
                .vault
                .prepare_native_authentication(f.budget.prepare().unwrap())
                .verify(SECRET)
                .unwrap();
            assert!(check.is_live_cached());
            match n {
                0 => {
                    f.vault.ui.close_first();
                    assert!(original_index(&f.vault).handler_for_test().is_open_cached());
                    assert!(original_index(&f.vault)
                        .actual_store_for_test()
                        .unwrap()
                        .chat_mount
                        .is_live_cached());
                }
                1 => {
                    original_index(&f.vault).close_admission();
                    assert!(check.host.is_open());
                    assert!(original_index(&f.vault)
                        .actual_store_for_test()
                        .unwrap()
                        .chat_mount
                        .is_live_cached());
                }
                _ => {
                    original_index(&f.vault)
                        .actual_store_for_test()
                        .unwrap()
                        .revoke_chat_mount();
                    assert!(check.host.is_open());
                    assert!(original_index(&f.vault).handler_for_test().is_open_cached());
                }
            }
            assert!(
                !check.is_live_cached(),
                "each independent original negative must fence finalization"
            );
        }
    }

    #[test]
    fn native_auth_check_version2_refuses_outside_capture_without_refunding_owned_credit() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xcd20);
        let pool =
            CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity()).unwrap();
        let budget = NativeUiInvocationBudget::new(pool.clone()).unwrap();
        let vault = NativeTextVault::from_trusted_host(
            index2(dir.path(), &mut rng),
            Arc::new(AtomicU64::new(7)),
            Arc::new(AtomicU64::new(7)),
            Arc::new(AtomicBool::new(false)),
            budget.clone(),
        )
        .unwrap();
        let baseline = pool.charged_for_test();
        let prepared = vault.prepare_native_authentication(budget.prepare().unwrap());
        assert_eq!(
            pool.charged_for_test(),
            baseline + super::super::super::native_ui::INVOCATION_BYTES
        );
        wrapper_calls();
        assert!(prepared.verify(SECRET).is_err());
        assert_eq!(wrapper_calls(), 0);
        assert_eq!(pool.charged_for_test(), baseline);
    }

    fn ui_io_budget() -> (crate::store::NativeUiStateBudget, CriticalCatalogReadPool) {
        let pool =
            CriticalCatalogReadPool::new(crate::store::NativeUiStateBudget::required_capacity())
                .unwrap();
        (
            crate::store::NativeUiStateBudget::new(pool.clone()).unwrap(),
            pool,
        )
    }
    #[test]
    fn native_ui_state_io_uses_an_independent_slot_and_keeps_missing_distinct() {
        let dir = tempfile::tempdir().unwrap();
        let f = auth_fixture(dir.path(), 0xce01);
        let (budget, pool) = ui_io_budget();
        let index = original_index(&f.vault);
        let request =
            crate::store::critical_namespaces::native_registry::lifecycle::cell::tests::request(
                index,
            );
        let mut index_work = index.capture_metadata_work().unwrap();
        // Leave an ACTUAL uncertain Rename WAL in the index cell, not merely
        // an unused metadata loan. UI sidecar progress must not need Idle.
        use crate::store::persistence::native_registry::operational as cuts;
        cuts::arm_fault(cuts::Fault::IndexStage);
        assert!(index_work
            .rename_metadata_data(&request, &mut ChaCha20Rng::seed_from_u64(0xce04))
            .is_err());
        assert!(cuts::take_fault_seen());
        assert!(f.vault.row_count_data().is_err());
        let missing = f
            .vault
            .prepare_ui_state(budget.prepare().unwrap())
            .read()
            .unwrap();
        assert!(missing.bytes().is_none());
        assert!(
            f.vault
                .prepare_ui_state(budget.prepare().unwrap())
                .read()
                .is_err(),
            "one UI slot remains held by its result"
        );
        drop(missing);
        let json = br#"{"version":1,"drafts":{"room":"private"},"readMarks":{}}"#;
        let saved = f
            .vault
            .prepare_ui_state(budget.prepare().unwrap())
            .save(json, &mut ChaCha20Rng::seed_from_u64(0xce02))
            .unwrap();
        assert!(saved.is_live_cached());
        drop(saved);
        let read = f
            .vault
            .prepare_ui_state(budget.prepare().unwrap())
            .read()
            .unwrap();
        assert_eq!(read.bytes(), Some(json.as_slice()));
        drop(read);
        let saved = f
            .vault
            .prepare_ui_state(budget.prepare().unwrap())
            .save(b"", &mut ChaCha20Rng::seed_from_u64(0xce03))
            .unwrap();
        drop(saved);
        let empty = f
            .vault
            .prepare_ui_state(budget.prepare().unwrap())
            .read()
            .unwrap();
        assert_eq!(
            empty.bytes(),
            Some(b"".as_slice()),
            "only Missing may become first-run defaults"
        );
        drop(empty);
        assert!(
            f.vault.row_count_data().is_err(),
            "UI IO must not settle the index WAL"
        );
        let (covered, debt) = index_work
            .rename_metadata_data(&request, &mut NeverRng)
            .unwrap();
        assert_eq!(covered.revision, request.expected_revision + 1);
        drop(debt);
        drop(index_work);
        assert_eq!(f.vault.row_count_data().unwrap(), records().len());
        assert_eq!(pool.charged_for_test(), 0);
    }
    #[test]
    fn native_ui_state_io_keeps_actual_session_and_all_buffer_credits_until_collection() {
        let dir = tempfile::tempdir().unwrap();
        let f = auth_fixture(dir.path(), 0xce10);
        let (budget, pool) = ui_io_budget();
        let read = f
            .vault
            .prepare_ui_state(budget.prepare().unwrap())
            .read()
            .unwrap();
        drop(f.vault);
        assert!(!read.is_live_cached());
        assert!(matches!(
            mount::NativeIndexStore::open_lifecycle3_for_test(
                dir.path(),
                SECRET,
                &mut NeverRng,
                current_policy()
            ),
            Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
        ));
        assert!(pool.charged_for_test() > 0);
        drop(read);
        let _reopened = mount::NativeIndexStore::open_lifecycle3_for_test(
            dir.path(),
            SECRET,
            &mut NeverRng,
            current_policy(),
        )
        .unwrap();
        assert_eq!(pool.charged_for_test(), 0);
    }
    #[test]
    fn native_ui_state_io_pays_each_position_before_capture_and_refunds_busy_outside() {
        let dir = tempfile::tempdir().unwrap();
        let f = auth_fixture(dir.path(), 0xce20);
        let (budget, pool) = ui_io_budget();
        let mut credits = Vec::new();
        for _ in 0..4 {
            credits.push(budget.prepare().unwrap());
        }
        assert!(budget.prepare().is_err());
        assert_eq!(
            pool.charged_for_test(),
            crate::store::NativeUiStateBudget::required_capacity()
        );
        let held = f
            .vault
            .prepare_ui_state(credits.pop().unwrap())
            .read()
            .unwrap();
        let busy = f.vault.prepare_ui_state(credits.pop().unwrap());
        let before = pool.charged_for_test();
        assert_eq!(
            before,
            crate::store::NativeUiStateBudget::required_capacity(),
            "infallible capture must retain its caller's fee"
        );
        assert!(busy.read().is_err());
        assert!(pool.charged_for_test() < before);
        drop(held);
        drop(credits);
        assert_eq!(pool.charged_for_test(), 0);
    }
}
