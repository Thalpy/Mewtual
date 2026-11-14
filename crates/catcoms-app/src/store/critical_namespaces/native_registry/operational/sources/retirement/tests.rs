//! Actual original cells and real vault exclusion; UI credentials remain
//! synthetic. These tests establish no native command authorization.
use super::super::tests::{command, inputs, policy, request, selected, sources};
use super::*;
use catcoms_rt::MemNetwork;
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use std::sync::atomic::AtomicUsize;

thread_local! {
    static AFTER_KEYS: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
pub(in super::super) fn after_key_preparation() {
    let hook = AFTER_KEYS.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

#[test]
fn native_sources_retirement_one_prepaid_capture_drop_never_reopens_original() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    let old = command(&vault, &budget);
    let before = owner.policy.pool.charged_for_test();
    let capture = owner.prepare_retirement().unwrap();
    assert_eq!(
        owner.policy.pool.charged_for_test(),
        before,
        "capture uses its prepaid slot"
    );
    assert!(matches!(
        owner.prepare_retirement(),
        Err(NativeTextSourceRetirementRefusal::Busy)
    ));
    drop(capture); // completely unrun; the canonical original is still closed
    assert!(owner.require_command(&old).is_err());
    let mut retry = owner.prepare_retirement().unwrap();
    assert_eq!(
        retry.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
    assert!(vault
        .capture_native_sources(policy().prepare::<MemNetwork, ChaCha20Rng>().unwrap())
        .finish()
        .is_err());
}

#[test]
fn native_sources_retirement_idle_keys_cannot_overtake_an_unentered_whole_loan() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    let job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let cell = job.loan.cell.clone();
    let mut capture = owner.prepare_retirement().unwrap();
    let row = capture.rows[0].as_ref().unwrap();
    assert!(row.keys.is_drained(), "no primitive was entered");
    assert_eq!(
        capture.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    assert_eq!(
        cell.state.lock().unwrap().retirement.destroyed,
        Destruction::NotDestroyed
    );
    drop(capture); // a lost/unpolled waiter must not discard the whole loan
    let mut retry = owner.prepare_retirement().unwrap();
    assert_eq!(
        retry.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    drop(job); // actual loan restoration; no success was inferred from absence
    assert_eq!(
        retry.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
    let state = cell.state.lock().unwrap();
    assert!(state.retiring && !state.running && state.body.is_none());
    assert!(matches!(
        state.retirement.tcp,
        Some(TcpDisposition::NotPrepared)
    ));
    assert_eq!(state.retirement.destroyed, Destruction::Returned);
}

#[test]
fn native_sources_retirement_original_key_end_is_distinct_from_a_returned_loan() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_device()
        .unwrap();
    let mut job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    job.keys.enter_device().unwrap();
    let NativeTextCreationJob {
        loan,
        keys,
        command: old,
    } = job;
    drop(loan);
    drop(old);
    let mut capture = owner.prepare_retirement().unwrap();
    assert!(!capture.rows[0].as_ref().unwrap().keys.is_drained());
    assert_eq!(
        capture.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    drop(keys);
    assert_eq!(
        capture.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
}

#[test]
fn native_sources_retirement_missing_body_and_missing_tcp_metadata_are_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = sources(&vault);
    drop(
        owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap(),
    );
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    let body = cell.state.lock().unwrap().body.take();
    drop(body); // deliberately no genuine destruction envelope
    let mut capture = owner.prepare_retirement().unwrap();
    assert_eq!(
        capture.try_retire(),
        Err(NativeTextSourceRetirementRefusal::Unknown)
    );
    assert_eq!(
        cell.state.lock().unwrap().retirement.destroyed,
        Destruction::NotDestroyed
    );
    // Even a contradictory body-destroyed bit cannot turn absent TCP evidence
    // into NotPrepared. This direct corruption is only a rejection regression.
    {
        let mut state = cell.state.lock().unwrap();
        state.retirement.sdk_closed = true;
        state.retirement.destroyed = Destruction::Returned;
    }
    assert_eq!(
        capture.try_retire(),
        Err(NativeTextSourceRetirementRefusal::Unknown)
    );
}

#[test]
fn native_sources_retirement_after_key_capture_fences_the_late_body_borrow() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = Arc::new(sources(&vault));
    drop(
        owner
            .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
            .unwrap(),
    );
    let closing = owner.clone();
    AFTER_KEYS.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            let mut capture = closing.prepare_retirement().unwrap();
            assert_eq!(
                capture.try_retire().unwrap(),
                NativeTextSourceRetirementStatus::Drained
            );
        }))
    });
    assert!(matches!(
        owner.prepare_creation(command(&vault, &budget), request(1, "Original"), inputs()),
        Err(NativeTextCreationRefusal::Unavailable)
    ));
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    assert!(cell.state.lock().unwrap().body.is_none());
    assert!(cell.state.lock().unwrap().retirement.complete());
}

struct PanicRng {
    rng: ChaCha20Rng,
    panic: Arc<AtomicBool>,
    drops: Arc<AtomicUsize>,
}
impl rand_core::CryptoRng for PanicRng {}
impl RngCore for PanicRng {
    fn next_u32(&mut self) -> u32 {
        self.rng.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        self.rng.fill_bytes(bytes)
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), rand_core::Error> {
        self.rng.try_fill_bytes(bytes)
    }
}
impl Drop for PanicRng {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        assert!(
            !self.panic.swap(false, Ordering::SeqCst),
            "actual retained RNG destructor unwind"
        );
    }
}

#[test]
fn native_sources_retirement_destructor_unwind_records_only_after_all_actual_body_fields() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = vault
        .capture_native_sources(policy().prepare::<MemNetwork, PanicRng>().unwrap())
        .finish()
        .unwrap();
    let panic = Arc::new(AtomicBool::new(false));
    let drops = Arc::new(AtomicUsize::new(0));
    let rng = || PanicRng {
        rng: ChaCha20Rng::seed_from_u64(1),
        panic: panic.clone(),
        drops: drops.clone(),
    };
    drop(
        owner
            .prepare_creation(
                command(&vault, &budget),
                request(1, "Original"),
                NativeTextCreationInputs::new(
                    rng(),
                    rng(),
                    Box::new(catcoms_rt::ManualClock::new(100)),
                ),
            )
            .unwrap(),
    );
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    let mut capture = owner.prepare_retirement().unwrap();
    panic.store(true, Ordering::SeqCst);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| capture.try_retire())).is_err()
    );
    assert_eq!(
        drops.load(Ordering::SeqCst),
        2,
        "both real fields dropped before acknowledgment"
    );
    {
        let state = cell.state.lock().unwrap();
        assert_eq!(state.retirement.destroyed, Destruction::Unwound);
        assert!(!state.running && state.body.is_none());
    }
    drop(capture); // lost reply after actual destruction, recover SAME evidence
    assert_eq!(
        owner.prepare_retirement().unwrap().try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
}

#[cfg(any(windows, target_os = "linux"))]
fn open(path: &std::path::Path) -> Result<(), AppError> {
    crate::store::NativeIndexStore::open_lifecycle3_for_test(
        path,
        crate::store::critical_namespaces::native_registry::preparation::tests::SECRET,
        &mut ChaCha20Rng::seed_from_u64(0xae01),
        crate::store::critical_namespaces::native_registry::preparation::tests::current_policy(),
    )
    .map(drop)
}

#[cfg(any(windows, target_os = "linux"))]
fn assert_busy(path: &std::path::Path) {
    assert!(matches!(
        open(path),
        Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
    ));
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn native_sources_retirement_last_control_payment_outlives_owner_without_pinning_physical() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, _) = selected(dir.path());
    let owner = sources(&vault);
    let pool = owner.policy.pool.clone();
    assert_eq!(pool.charged_for_test(), CONTROL_BYTES);
    let capture = owner.prepare_retirement().unwrap();
    drop(owner);
    drop(vault);
    // An empty capture retains its real control payment but no original
    // Physical. Neither dropping the collection early nor keeping a worker
    // envelope may silently release payment or keep an issuer alive.
    assert_eq!(pool.charged_for_test(), CONTROL_BYTES);
    open(dir.path()).unwrap();
    drop(capture);
    assert_eq!(pool.charged_for_test(), 0);
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test(flavor = "current_thread")]
async fn native_sources_retirement_lost_collection_after_body_drop_waits_same_original_tcp() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = mesh_owner(&vault, &budget, false);
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    let mut capture = owner.prepare_retirement().unwrap();
    // No async yield since construction: the REAL original Actor is unpolled,
    // not a fake completion flag. Body destruction closes it but cannot poll it.
    assert_eq!(
        capture.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    let tcp = {
        let state = cell.state.lock().unwrap();
        assert!(state.body.is_none() && !state.running);
        assert_eq!(state.retirement.destroyed, Destruction::Returned);
        let Some(TcpDisposition::Tracked(tcp)) = &state.retirement.tcp else {
            panic!("lost original TCP evidence")
        };
        assert!(!tcp.is_drained());
        tcp.clone()
    };
    drop(capture);
    let mut retry = owner.prepare_retirement().unwrap();
    assert_eq!(
        retry.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    drop(owner);
    drop(vault);
    assert_busy(dir.path());
    wait_tcp(&tcp).await;
    assert_eq!(
        retry.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
    drop(retry);
    drop(cell);
    open(dir.path()).unwrap(); // metadata-only TCP observer still alive
    assert!(tcp.is_drained());
}

#[cfg(any(windows, target_os = "linux"))]
#[tokio::test(flavor = "current_thread")]
async fn native_sources_retirement_member_end_preserves_busy_whole_sdk_then_same_body_closes() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(dir.path());
    let owner = Arc::new(mesh_owner(&vault, &budget, true));
    let mut job = owner
        .prepare_creation(command(&vault, &budget), request(1, "Original"), inputs())
        .unwrap();
    let body = job.loan.body.as_mut().unwrap();
    let end = body
        .work
        .hold_native_member_end_for_test(body.driver.as_mut().unwrap())
        .unwrap();
    let cell = job.loan.cell.clone();
    let tcp = body.network.as_ref().unwrap().observe_tcp().unwrap();
    drop(job);
    let closing = owner.clone();
    AFTER_KEYS.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            let mut capture = closing.prepare_retirement().unwrap();
            assert_eq!(
                capture.try_retire().unwrap(),
                NativeTextSourceRetirementStatus::Pending
            );
        }))
    });
    // Actual whole SDK close returned Busy and restored its original body. An
    // older prepared command still cannot borrow that restored closed body.
    assert!(matches!(
        owner.prepare_creation(command(&vault, &budget), request(1, "Original"), inputs()),
        Err(NativeTextCreationRefusal::Unavailable)
    ));
    let mut capture = owner.prepare_retirement().unwrap();
    assert!(capture.rows[0].as_ref().unwrap().keys.is_drained());
    {
        let state = cell.state.lock().unwrap();
        assert!(state.body.as_ref().unwrap().driver.is_some());
        assert_eq!(state.retirement.destroyed, Destruction::NotDestroyed);
    }
    assert_eq!(
        capture.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    drop(end); // real independent member source End, after key counters drained
    assert_eq!(
        capture.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    wait_tcp(&tcp).await;
    assert_eq!(
        capture.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
}

#[cfg(any(windows, target_os = "linux"))]
fn mesh_owner(
    vault: &NativeTextVault,
    budget: &crate::store::NativeUiInvocationBudget,
    founder: bool,
) -> NativeTextSources<catcoms_net::MeshService, ChaCha20Rng> {
    let owner = vault
        .capture_native_sources(policy().prepare().unwrap())
        .finish()
        .unwrap();
    owner
        .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_device()
        .unwrap();
    owner
        .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_network()
        .unwrap();
    owner
        .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
        .unwrap()
        .cover_pending_network()
        .unwrap();
    owner
        .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
        .unwrap()
        .prepare_transport()
        .unwrap();
    if founder {
        owner
            .prepare_creation(command(vault, budget), request(1, "Original"), inputs())
            .unwrap()
            .prepare_founder()
            .unwrap();
    }
    owner
}

#[cfg(any(windows, target_os = "linux"))]
async fn wait_tcp(tcp: &catcoms_net::NativeTcpDrain) {
    use catcoms_rt::Clock;
    tokio::select! {
        _ = tcp.wait() => {},
        _ = catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(10)) => panic!("original TCP did not finish")
    }
}
