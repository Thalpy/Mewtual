//! Actual original selected3 owners; synthetic UI credentials do not prove
//! Tauri authorization. A short simulated native mutex contains affine Sources.
use super::super::tests::{command, inputs, policy, request, selected, sources};
use super::*;
use catcoms_rt::MemNetwork;
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// Always release and JOIN before checking a timeout result. A failing capture
// cannot strand the actual account/book locks or leak a background thread.
struct HeldWriters {
    release: Arc<(Mutex<bool>, std::sync::Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for HeldWriters {
    fn drop(&mut self) {
        {
            let mut released = self.release.0.lock().unwrap_or_else(|e| e.into_inner());
            *released = true;
            self.release.1.notify_all();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn native_sources_capture_avoids_actual_book_and_pool_writers_until_outside_finish() {
    use catcoms_rt::Clock;
    let dir = tempfile::tempdir().unwrap();
    let (vault, ui) = selected(dir.path());
    let owner = Arc::new(sources(&vault));
    let command = command(&vault, &ui);
    let credit = owner.policy.prepare_creation().unwrap();
    let book = owner.entries.clone();
    let pool = owner.policy.pool.clone();
    let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let signal = release.clone();
    let (ready, waiting) = tokio::sync::oneshot::channel();
    let thread = std::thread::spawn(move || {
        let _book = book.lock().unwrap();
        pool.with_counter_locked_for_test(|| {
            let _ = ready.send(());
            let mut released = signal.0.lock().unwrap();
            while !*released {
                released = signal.1.wait(released).unwrap();
            }
        });
    });
    let held = HeldWriters {
        release,
        thread: Some(thread),
    };
    let ready = tokio::select! {
        result=waiting => result.is_ok(),
        _=catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(5)) => false,
    };
    if !ready {
        drop(held);
        panic!("actual capture fixture writers did not become held");
    }
    let capturing = owner.clone();
    let (sent, received) = tokio::sync::oneshot::channel();
    let capture_thread = std::thread::spawn(move || {
        let preparation =
            capturing.capture_creation(command, request(1, "Original"), inputs(), credit);
        // Also test the matching retirement capture while both genuine writers
        // remain held. Its outside-guard finish must do the source close later.
        let retirement = capturing.capture_retirement();
        drop(capturing);
        let _ = sent.send((preparation, retirement));
    });
    let captured = tokio::select! {
        result=received => result.ok(),
        _=catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(5)) => None,
    };
    drop(held); // release/join real writers even if capture incorrectly blocked
    let joined = capture_thread.join();
    assert!(joined.is_ok());
    let (preparation, retirement) =
        captured.expect("guard-safe captures waited on book or pool writer");
    let mut worker = retirement.finish().unwrap();
    assert_eq!(
        worker.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    assert!(matches!(
        preparation.finish(),
        Err(NativeTextCreationRefusal::Unavailable)
    ));
    assert_eq!(
        worker.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
}

#[test]
fn native_sources_capture_native_owner_guard_leaves_before_actual_device_work() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, ui) = selected(dir.path());
    let owner = sources(&vault);
    let credit = owner.policy.prepare_creation().unwrap();
    let command = command(&vault, &ui);
    let native = Mutex::new(Some(owner));
    let preparation = {
        let guard = native.lock().unwrap();
        guard
            .as_ref()
            .unwrap()
            .capture_creation(command, request(1, "Original"), inputs(), credit)
    };
    assert!(native.try_lock().is_ok());
    let job = preparation.finish().unwrap();
    assert!(native.try_lock().is_ok());
    job.prepare_device().unwrap();
    let owner = native.lock().unwrap().take().unwrap();
    let mut retirement = owner.capture_retirement().finish().unwrap();
    assert_eq!(
        retirement.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
}

#[test]
fn native_sources_capture_two_permits_close_monotonically_and_empty_snapshot_waits_both() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, ui) = selected(dir.path());
    let owner = sources(&vault);
    let late_command = command(&vault, &ui);
    let late_credit = owner.policy.prepare_creation().unwrap();
    let capture = || {
        owner.capture_creation(
            command(&vault, &ui),
            request(1, "Original"),
            inputs(),
            owner.policy.prepare_creation().unwrap(),
        )
    };
    let first = capture();
    let second = capture();
    let third = capture();
    assert!(third.work.is_none());
    assert!(matches!(
        third.finish(),
        Err(NativeTextCreationRefusal::Busy)
    ));
    let mut retirement = owner.capture_retirement().finish().unwrap();
    assert!(owner.entries.lock().unwrap().is_empty());
    assert_eq!(
        retirement.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    drop(first);
    assert_eq!(
        retirement.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    drop(second);
    assert_eq!(
        retirement.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
    // These actual command/budget captures predate closure. Neither can reopen
    // original preparation admission after its CLOSED+zero observation.
    let late = owner.capture_creation(late_command, request(1, "Original"), inputs(), late_credit);
    assert!(late.work.is_none());
    assert!(matches!(
        late.finish(),
        Err(NativeTextCreationRefusal::Unavailable)
    ));
    assert_eq!(
        retirement.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
}

#[test]
fn native_sources_capture_foreign_equal_policy_refuses_before_any_physical_pin_or_io() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, ui) = selected(dir.path());
    let owner = sources(&vault);
    let foreign = policy();
    let foreign_pool = foreign.pool.clone();
    let credit = foreign.prepare_creation().unwrap();
    let before = owner.policy.pool.charged_for_test();
    let preparation = owner.capture_creation(
        command(&vault, &ui),
        request(1, "Original"),
        inputs(),
        credit,
    );
    assert!(preparation.work.is_none());
    assert_eq!(owner.policy.pool.charged_for_test(), before);
    assert_eq!(foreign_pool.charged_for_test(), ENTRY_BYTES);
    assert!(matches!(
        preparation.finish(),
        Err(NativeTextCreationRefusal::Unavailable)
    ));
    assert_eq!(foreign_pool.charged_for_test(), 0);
    assert!(owner.entries.lock().unwrap().is_empty());
}

#[test]
fn native_sources_capture_refused_weak_book_retains_original_control_after_sources_drop() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, ui) = selected(dir.path());
    let owner = sources(&vault);
    let pool = owner.policy.pool.clone();
    let command = command(&vault, &ui);
    let credit = owner.policy.prepare_creation().unwrap();
    owner.close_admission();
    let refused = owner.capture_creation(command, request(1, "Original"), inputs(), credit);
    assert!(refused.work.is_none());
    assert!(refused._end.is_none());
    drop(owner);
    drop(vault);
    assert_eq!(pool.charged_for_test(), CONTROL_BYTES + ENTRY_BYTES);
    assert!(matches!(
        refused.finish(),
        Err(NativeTextCreationRefusal::Unavailable)
    ));
    assert_eq!(pool.charged_for_test(), 0);
}

#[test]
fn native_sources_capture_retirement_preparation_is_owning_unpolled_and_missing_book_is_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, ui) = selected(dir.path());
    let owner = sources(&vault);
    drop(
        owner
            .prepare_creation(command(&vault, &ui), request(1, "Original"), inputs())
            .unwrap(),
    );
    let first = owner.capture_retirement();
    // Native capture has not falsely reported that RT source counters closed.
    assert!(!owner.entries.lock().unwrap()[0]
        .owner
        .observe_retirement()
        .is_drained());
    assert!(matches!(
        owner.capture_retirement().finish(),
        Err(NativeTextSourceRetirementRefusal::Busy)
    ));
    drop(first);
    let original = owner.capture_retirement();
    let mut worker = original.finish().unwrap();
    assert_eq!(
        worker.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
    drop(worker);
    let lost = owner.capture_retirement();
    drop(owner); // deliberately violate canonical retention to test rejection
    assert!(matches!(
        lost.finish(),
        Err(NativeTextSourceRetirementRefusal::Unknown)
    ));
}

struct DropRng {
    rng: ChaCha20Rng,
    guard: Arc<Mutex<()>>,
    drops: Arc<AtomicUsize>,
    outside: Arc<AtomicBool>,
}
impl rand_core::CryptoRng for DropRng {}
impl RngCore for DropRng {
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
impl Drop for DropRng {
    fn drop(&mut self) {
        if self.guard.try_lock().is_err() {
            self.outside.store(false, Ordering::SeqCst);
        }
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn native_sources_capture_refused_original_inputs_and_fee_leave_only_outside_native_guard() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, ui) = selected(dir.path());
    let owner = vault
        .capture_native_sources(policy().prepare::<MemNetwork, DropRng>().unwrap())
        .finish()
        .unwrap();
    let command = command(&vault, &ui);
    let credit = owner.policy.prepare_creation().unwrap();
    let guard = Arc::new(Mutex::new(()));
    let drops = Arc::new(AtomicUsize::new(0));
    let outside = Arc::new(AtomicBool::new(true));
    let rng = || DropRng {
        rng: ChaCha20Rng::seed_from_u64(7),
        guard: guard.clone(),
        drops: drops.clone(),
        outside: outside.clone(),
    };
    owner.close_admission();
    let capture = {
        let _held = guard.lock().unwrap();
        let capture = owner.capture_creation(
            command,
            request(1, "Original"),
            NativeTextCreationInputs::new(rng(), rng(), Box::new(catcoms_rt::ManualClock::new(1))),
            credit,
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        capture
    };
    assert_eq!(
        owner.policy.pool.charged_for_test(),
        CONTROL_BYTES + ENTRY_BYTES
    );
    assert!(matches!(
        capture.finish(),
        Err(NativeTextCreationRefusal::Unavailable)
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    assert!(outside.load(Ordering::SeqCst));
    assert_eq!(owner.policy.pool.charged_for_test(), CONTROL_BYTES);
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn native_sources_capture_empty_retirement_cannot_clear_unentered_original_physical_work() {
    let dir = tempfile::tempdir().unwrap();
    let (vault, ui) = selected(dir.path());
    let owner = sources(&vault);
    let capture = owner.capture_creation(
        command(&vault, &ui),
        request(1, "Original"),
        inputs(),
        owner.policy.prepare_creation().unwrap(),
    );
    assert!(capture.work.is_some());
    let mut retirement = owner.capture_retirement().finish().unwrap();
    assert_eq!(
        retirement.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Pending
    );
    drop(owner);
    drop(vault);
    let open =
        || {
            crate::store::NativeIndexStore::open_lifecycle3_for_test(dir.path(),
        crate::store::critical_namespaces::native_registry::preparation::tests::SECRET,
        &mut ChaCha20Rng::seed_from_u64(0xbb10),
        crate::store::critical_namespaces::native_registry::preparation::tests::current_policy())
        };
    assert!(matches!(
        open(),
        Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy))
    ));
    assert!(matches!(
        capture.finish(),
        Err(NativeTextCreationRefusal::Unavailable)
    ));
    assert_eq!(
        retirement.try_retire().unwrap(),
        NativeTextSourceRetirementStatus::Drained
    );
    drop(open().unwrap()); // negative fee-only retirement can remain alive
}
