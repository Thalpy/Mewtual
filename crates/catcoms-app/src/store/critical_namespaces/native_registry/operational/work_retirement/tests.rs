//! Actual metadata/auth/UI capsules and real vault exclusion. These are Root
//! storage-lifetime tests, not native IPC/close-prelude or physical-network proof.
use super::super::sources::tests::{selected, sources};
use super::*;
use crate::store::{NativeTextSourceRetirementStatus, NativeUiStateBudget};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::atomic::{AtomicBool, AtomicU64};

fn ui_budget() -> NativeUiStateBudget {
    NativeUiStateBudget::new(
        CriticalCatalogReadPool::new(NativeUiStateBudget::required_capacity()).unwrap(),
    )
    .unwrap()
}
fn invoke_budget() -> NativeUiInvocationBudget {
    NativeUiInvocationBudget::new(
        CriticalCatalogReadPool::new(NativeUiInvocationBudget::required_capacity()).unwrap(),
    )
    .unwrap()
}
#[cfg(any(windows, target_os = "linux"))]
fn open(path: &std::path::Path) -> Result<(), AppError> {
    crate::store::NativeIndexStore::open_lifecycle3_for_test(
        path,
        crate::store::critical_namespaces::native_registry::preparation::tests::SECRET,
        &mut ChaCha20Rng::seed_from_u64(0xda20),
        crate::store::critical_namespaces::native_registry::preparation::tests::current_policy(),
    )
    .map(drop)
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn native_work_retirement_source_zero_does_not_clear_queued_auth_and_original_ui_preparation() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let sources = sources(&vault);
    let check = Arc::new(
        vault
            .prepare_native_authentication(budget.prepare().unwrap())
            .verify(crate::store::critical_namespaces::native_registry::preparation::tests::SECRET)
            .unwrap(),
    );
    let auth = vault.prepare_native_authentication(budget.prepare().unwrap());
    let ui = vault.prepare_ui_state_for_original(ui_budget().prepare().unwrap(), check.clone());
    let mut source_close = sources.capture_retirement().finish().unwrap();
    assert_eq!(
        source_close.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
    let work = vault
        .capture_work_retirement(budget.prepare().unwrap())
        .finish()
        .unwrap();
    assert!(!work.is_drained_cached());
    drop(auth); // actual queued authentication capsule, no simulated done flag
    assert!(
        !work.is_drained_cached(),
        "independent UI slot still owns its original work"
    );
    drop(ui);
    assert!(work.is_drained_cached());
    assert!(
        matches!(
            open(dir.path()),
            Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
        ),
        "affine originals still retain Physical after work drainage"
    );
    drop(source_close);
    drop(sources);
    drop(vault);
    open(dir.path()).unwrap();
    assert!(!check.is_live_cached()); // check/target/drain metadata still alive
    assert!(work.is_drained_cached());
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn native_work_retirement_actual_read_and_saved_outputs_each_retain_the_original_ui_slot() {
    for saved in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (vault, budget) = selected(dir.path());
        let ui = ui_budget();
        let output = vault
            .prepare_ui_state(ui.prepare().unwrap())
            .save(b"{\"version\":1}", &mut ChaCha20Rng::seed_from_u64(0xda21))
            .unwrap();
        let output: Box<dyn Send> = if saved {
            Box::new(output)
        } else {
            drop(output);
            let read = vault
                .prepare_ui_state(ui.prepare().unwrap())
                .read()
                .unwrap();
            assert_eq!(read.bytes(), Some(b"{\"version\":1}".as_slice()));
            Box::new(read)
        };
        let work = vault
            .capture_work_retirement(budget.prepare().unwrap())
            .finish()
            .unwrap();
        assert!(!work.is_drained_cached());
        drop(vault);
        assert!(matches!(
            open(dir.path()),
            Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
        ));
        drop(output); // actual response ownership, independently of IO success
        assert!(work.is_drained_cached());
        open(dir.path()).unwrap(); // metadata-only work observer cannot pin files
    }
}

#[test]
fn native_work_retirement_lost_observer_reissues_the_same_closed_original_controls() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let auth = vault.prepare_native_authentication(budget.prepare().unwrap());
    let first = vault
        .capture_work_retirement(budget.prepare().unwrap())
        .finish()
        .unwrap();
    assert!(!first.is_drained_cached());
    drop(first);
    let retry = vault
        .capture_work_retirement(budget.prepare().unwrap())
        .finish()
        .unwrap();
    assert!(!retry.is_drained_cached());
    // Closure is permanent: no newly captured authentication can reopen work.
    let refused = vault.prepare_native_authentication(budget.prepare().unwrap());
    assert!(refused.verify(b"wrong secret").is_err());
    assert!(!retry.is_drained_cached());
    drop(auth);
    assert!(retry.is_drained_cached());
}

#[test]
fn native_work_retirement_a_replacement_empty_vault_cannot_clear_an_old_capsule() {
    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let (old, old_budget) = selected(old_dir.path());
    let (new, new_budget) = selected(new_dir.path());
    let original = old.prepare_native_authentication(old_budget.prepare().unwrap());
    let old_close = old
        .capture_work_retirement(old_budget.prepare().unwrap())
        .finish()
        .unwrap();
    let new_close = new
        .capture_work_retirement(new_budget.prepare().unwrap())
        .finish()
        .unwrap();
    assert!(new_close.is_drained_cached());
    assert!(!old_close.is_drained_cached());
    drop(original);
    assert!(old_close.is_drained_cached());
}

#[test]
fn native_work_retirement_version2_and_foreign_credit_are_explicit_refusals() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let foreign = invoke_budget();
    assert!(vault
        .capture_work_retirement(foreign.prepare().unwrap())
        .finish()
        .is_err());
    // The same original can still yield the correct negative observation with
    // its genuine credit; refusing a foreign credit never invents missing work.
    assert!(vault
        .capture_work_retirement(budget.prepare().unwrap())
        .finish()
        .unwrap()
        .is_drained_cached());
    let dir = tempfile::tempdir().unwrap();
    let index = super::super::facade::upgrade::tests::index2(
        dir.path(),
        &mut ChaCha20Rng::seed_from_u64(0xda22),
    );
    let budget = invoke_budget();
    let version2 = NativeTextVault::from_trusted_host(
        index,
        Arc::new(AtomicU64::new(7)),
        Arc::new(AtomicU64::new(7)),
        Arc::new(AtomicBool::new(false)),
        budget.clone(),
    )
    .unwrap();
    assert!(version2
        .capture_work_retirement(budget.prepare().unwrap())
        .finish()
        .is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn native_work_retirement_wait_registers_before_actual_final_capsule_drop() {
    use catcoms_rt::Clock;
    use std::future::{poll_fn, Future};
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let auth = vault.prepare_native_authentication(budget.prepare().unwrap());
    let work = vault
        .capture_work_retirement(budget.prepare().unwrap())
        .finish()
        .unwrap();
    let wait = work.wait();
    tokio::pin!(wait);
    poll_fn(|cx| {
        assert!(
            wait.as_mut().poll(cx).is_pending(),
            "real original slot remains ACTIVE"
        );
        std::task::Poll::Ready(())
    })
    .await;
    drop(auth); // actual final End wakes the registered original observation
    tokio::select! {
        _=wait=>{},
        _=catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(5))=>panic!("lost final original work wake"),
    }
    assert!(work.is_drained_cached());
}
