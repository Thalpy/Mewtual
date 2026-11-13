//! Real logical reservations with synthetic host atomics. These prove UI
//! bookkeeping/lifetime, not a checked Tauri command, row or cryptographic source.
use super::*;
use catcoms_rt::{NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatSourceOwner};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::task::{Context, Poll, Wake, Waker};

struct Fixture {
    owner: UiAdmissionOwner,
    generation: Arc<AtomicU64>,
    authorized: Arc<AtomicU64>,
    requested: Arc<AtomicBool>,
    budget: NativeUiInvocationBudget,
    pool: CriticalCatalogReadPool,
}
impl Fixture {
    fn new() -> Self {
        let pool = CriticalCatalogReadPool::new(MINIMUM_CAPACITY).unwrap();
        let budget = NativeUiInvocationBudget::new(pool.clone()).unwrap();
        let generation = Arc::new(AtomicU64::new(7));
        let authorized = Arc::new(AtomicU64::new(7));
        let requested = Arc::new(AtomicBool::new(false));
        let owner = UiAdmissionOwner::new(
            generation.clone(),
            authorized.clone(),
            requested.clone(),
            budget.clone(),
        )
        .unwrap();
        Self {
            owner,
            generation,
            authorized,
            requested,
            budget,
            pool,
        }
    }
    fn candidate(&self) -> UiCandidate {
        self.owner
            .prepare_capture(self.generation.load(SeqCst), self.budget.prepare().unwrap())
            .finish()
            .unwrap()
    }
    fn capture(&self) -> UiCapture {
        let mut candidate = self.candidate();
        self.owner.admit(&mut candidate).unwrap()
    }
    fn authorize(&self, generation: u64) {
        self.generation.store(generation, SeqCst);
        self.authorized.store(generation, SeqCst);
        self.requested.store(false, SeqCst);
    }
}
struct NoWake;
impl Wake for NoWake {
    fn wake(self: Arc<Self>) {}
}
fn poll(future: std::pin::Pin<&mut impl Future<Output = ()>>) -> Poll<()> {
    future.poll(&mut Context::from_waker(&Waker::from(Arc::new(NoWake))))
}
fn retired(owner: &UiAdmissionOwner, generation: u64) -> RetirementBatch {
    let result = owner.begin_retirement(generation).unwrap();
    assert_eq!(result.state(), UiRetirementState::Started);
    result
        .into_batch()
        .expect("actual original owner must move into the retirement batch")
}

#[test]
fn independently_paid_competitors_use_one_canonical_generation_and_keep_the_loser_outside() {
    let f = Fixture::new();
    let baseline = f.pool.charged_for_test();
    let mut first = f.candidate();
    let mut second = f.candidate();
    assert_eq!(f.pool.charged_for_test(), baseline + 2 * INVOCATION_BYTES);
    let a = f.owner.admit(&mut first).unwrap();
    let b = f.owner.admit(&mut second).unwrap();
    assert!(first.owner.is_none());
    assert!(
        second.owner.is_some(),
        "unused close owner must leave the guard with its caller"
    );
    assert!(Arc::ptr_eq(&a.record, &b.record));
    assert!(
        f.owner.admit(&mut second).is_err(),
        "an affine candidate cannot publish twice"
    );
    let source = NativeChatSourceOwner::for_trusted_source(&a.command, Arc::new(())).unwrap();
    let mut cut = source
        .prepare_cut(&b.command, NativeChatKeyPurpose::FounderSdk)
        .unwrap();
    let mut ends = NativeChatKeyEnds::new();
    cut.enter(
        &source.observe(),
        NativeChatKeyPurpose::FounderSdk,
        &mut ends,
    )
    .unwrap();
    drop(first);
    drop(second);
    drop(b);
    assert_eq!(f.pool.charged_for_test(), baseline + INVOCATION_BYTES);
    let batch = retired(&f.owner, 7);
    assert!(matches!(
        f.owner.begin_retirement(7).unwrap().state(),
        UiRetirementState::AlreadyRetiring
    ));
    let mut wait = Box::pin(batch.retire());
    assert!(poll(wait.as_mut()).is_pending());
    drop(ends);
    assert!(poll(wait.as_mut()).is_ready());
    assert!(matches!(
        f.owner.begin_retirement(7).unwrap().state(),
        UiRetirementState::KeyWorkDrained
    ));
    drop(wait);
    drop(a);
    drop(source);
    drop(cut);
    drop(f.owner.collect_drained().unwrap());
    assert_eq!(f.pool.charged_for_test(), baseline);
}

#[test]
fn final_canonical_admission_rejects_late_generation_foreign_registry_and_foreign_payment() {
    let f = Fixture::new();
    let other = Fixture::new();
    let mut stale = f.candidate();
    f.authorize(8);
    assert!(f.owner.admit(&mut stale).is_err());
    assert!(stale.owner.is_some());
    assert!(f.owner.registry.state.lock().unwrap().slots.is_empty());
    let mut wrong_registry = other.candidate();
    assert!(f.owner.admit(&mut wrong_registry).is_err());
    let mut wrong_payment = f
        .owner
        .prepare_capture(8, other.budget.prepare().unwrap())
        .finish()
        .unwrap();
    assert!(f.owner.admit(&mut wrong_payment).is_err());
    assert!(wrong_payment.owner.is_some());
    let capture = f.capture();
    f.owner.require_capture(&capture).unwrap();
    assert!(other.owner.require_capture(&capture).is_err());
    f.owner.close_first();
    assert!(f.owner.require_capture(&capture).is_err());
}

#[test]
fn every_transient_candidate_has_its_own_charge_and_exhaustion_precedes_construction() {
    let f = Fixture::new();
    let baseline = f.pool.charged_for_test();
    let mut candidates = Vec::new();
    while let Ok(credit) = f.budget.prepare() {
        candidates.push(f.owner.prepare_capture(7, credit).finish().unwrap());
    }
    assert_eq!(candidates.len(), 2 * GENERATIONS);
    assert_eq!(
        f.pool.charged_for_test(),
        baseline + candidates.len() * INVOCATION_BYTES
    );
    assert!(f.budget.prepare().is_err());
    assert!(f.owner.registry.state.lock().unwrap().slots.is_empty());
    drop(candidates.pop());
    let replacement = f.budget.prepare().unwrap();
    assert!(f.budget.prepare().is_err());
    drop(replacement);
    drop(candidates);
    assert_eq!(f.pool.charged_for_test(), baseline);
}

#[test]
fn full_generation_and_memory_caps_still_allow_retirement_and_unentered_pins_keep_payment() {
    let f = Fixture::new();
    let mut captures = Vec::new();
    for generation in 7..7 + GENERATIONS as u64 {
        f.authorize(generation);
        captures.push(f.capture());
    }
    f.authorize(7 + GENERATIONS as u64);
    let mut over_cap = f.candidate();
    assert!(f.owner.admit(&mut over_cap).is_err());
    drop(over_cap);
    let mut credits = Vec::new();
    while let Ok(credit) = f.budget.prepare() {
        credits.push(credit);
    }
    let charged = f.pool.charged_for_test();
    let batch = retired(&f.owner, u64::MAX);
    assert_eq!(
        f.pool.charged_for_test(),
        charged,
        "lock needs no new fee or retirement slot"
    );
    let mut wait = Box::pin(batch.retire());
    assert!(poll(wait.as_mut()).is_ready());
    drop(wait);
    drop(f.owner.collect_drained().unwrap());
    assert!(
        f.budget.prepare().is_err(),
        "captured records still own the old generation payments"
    );
    drop(credits);
    drop(captures);
    assert_eq!(f.pool.charged_for_test(), HOST_BYTES + REGISTRY_BYTES);

    // Separately pin a genuinely unentered cut through idle retirement and GC.
    f.authorize(100);
    let capture = f.capture();
    let source = NativeChatSourceOwner::for_trusted_source(&capture.command, Arc::new(())).unwrap();
    let cut = source
        .prepare_cut(&capture.command, NativeChatKeyPurpose::FounderSdk)
        .unwrap();
    let batch = retired(&f.owner, 100);
    let mut wait = Box::pin(batch.retire());
    assert!(poll(wait.as_mut()).is_ready());
    drop(wait);
    drop(f.owner.collect_drained().unwrap());
    drop(capture);
    drop(f.owner);
    assert_eq!(f.pool.charged_for_test(), HOST_BYTES + INVOCATION_BYTES);
    drop(source);
    assert_eq!(f.pool.charged_for_test(), HOST_BYTES + INVOCATION_BYTES);
    drop(cut);
    assert_eq!(f.pool.charged_for_test(), 0);
}

#[test]
fn retired_generation_cannot_reopen_after_gc_even_with_unchanged_host_atomics() {
    let f = Fixture::new();
    let original = f.capture();
    let batch = retired(&f.owner, 7);
    let mut wait = Box::pin(batch.retire());
    assert!(poll(wait.as_mut()).is_ready());
    drop(wait);
    assert!(f.owner.require_capture(&original).is_err());
    let mut before_gc = f.candidate();
    assert!(f.owner.admit(&mut before_gc).is_err());
    drop(f.owner.collect_drained().unwrap());
    let mut after_gc = f.candidate();
    assert!(f.owner.admit(&mut after_gc).is_err());
    f.authorize(8);
    let fresh = f.capture();
    f.owner.require_capture(&fresh).unwrap();
}

#[test]
fn affine_owner_drop_closes_unfinished_preparation_without_reviving_or_refunding_its_pins() {
    let f = Fixture::new();
    let preparation = f.owner.prepare_capture(7, f.budget.prepare().unwrap());
    drop(f.owner);
    assert_eq!(f.pool.charged_for_test(), HOST_BYTES + INVOCATION_BYTES);
    assert!(preparation.finish().is_err());
    assert_eq!(f.pool.charged_for_test(), 0);
}

struct GuardWake {
    registry: Weak<Registry>,
    wakes: AtomicUsize,
}
impl Wake for GuardWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        assert!(self.registry.upgrade().unwrap().state.try_lock().is_ok());
        self.wakes.fetch_add(1, SeqCst);
    }
}
#[test]
fn retirement_moves_actual_close_owners_out_before_synchronously_waking_observers() {
    let f = Fixture::new();
    let capture = f.capture();
    let notice = Arc::new(GuardWake {
        registry: Arc::downgrade(&f.owner.registry),
        wakes: AtomicUsize::new(0),
    });
    let waker = Waker::from(notice.clone());
    let mut observed = Box::pin(capture.record.drain.wait());
    assert!(observed
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_pending());
    let batch = retired(&f.owner, 7);
    assert_eq!(notice.wakes.load(SeqCst), 0);
    let mut wait = Box::pin(batch.retire());
    assert!(poll(wait.as_mut()).is_ready());
    assert_eq!(notice.wakes.load(SeqCst), 1);
    assert!(observed
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_ready());
}

#[test]
fn repeated_retirement_cannot_become_the_last_payer_after_concurrent_final_end_and_gc() {
    let f = Fixture::new();
    let refunded = Arc::new(AtomicBool::new(false));
    NEXT_FEE_NOTICE.with(|slot| {
        assert!(slot.borrow_mut().replace(refunded.clone()).is_none());
    });
    let capture = f.capture();
    let source = NativeChatSourceOwner::for_trusted_source(&capture.command, Arc::new(())).unwrap();
    let mut cut = source
        .prepare_cut(&capture.command, NativeChatKeyPurpose::FounderSdk)
        .unwrap();
    let mut ends = NativeChatKeyEnds::new();
    cut.enter(
        &source.observe(),
        NativeChatKeyPurpose::FounderSdk,
        &mut ends,
    )
    .unwrap();
    drop(capture);
    let first = retired(&f.owner, 7);
    let mut waiting = Box::pin(first.retire());
    assert!(poll(waiting.as_mut()).is_pending());
    // Leave only the canonical record and actual admitted key/pin ownership.
    // The repeated call must not acquire another hidden record/fee reference.
    drop(waiting);
    assert!(!refunded.load(SeqCst));
    let registry = f.owner.registry.clone();
    let notice = refunded.clone();
    AFTER_RETIREMENT_UNLOCK.with(|slot| {
        assert!(slot
            .borrow_mut()
            .replace(Box::new(move || {
                // Another native-owned task completes and collects outside ALL its
                // guards while this caller still owns an outer native/state guard.
                std::thread::spawn(move || {
                    drop(ends);
                    drop(cut);
                    drop(source);
                    drop(collect_drained(&registry).unwrap());
                    assert!(
                        notice.load(SeqCst),
                        "a repeated reply must hold no last-payer custody"
                    );
                })
                .join()
                .unwrap();
            }))
            .is_none());
    });
    let outer_native_state = Mutex::new(());
    let outer_guard = outer_native_state.lock().unwrap();
    assert!(matches!(
        f.owner.begin_retirement(7).unwrap().state(),
        UiRetirementState::AlreadyRetiring
    ));
    assert!(
        refunded.load(SeqCst),
        "the real reservation retired in the collector before this reply"
    );
    drop(outer_guard);
    assert!(AFTER_RETIREMENT_UNLOCK.with(|slot| slot.borrow().is_none()));
    assert_eq!(f.pool.charged_for_test(), HOST_BYTES + REGISTRY_BYTES);
}

#[test]
fn cached_ui_check_uses_only_original_pins_and_atomics_while_registry_is_held() {
    let f = Fixture::new();
    let capture = f.capture();
    let other = Fixture::new();
    let guard = f.owner.registry.state.lock().unwrap();
    assert!(f.owner.is_current_cached(&capture));
    assert!(!other.owner.is_current_cached(&capture));
    // The full registry/payment check is deliberately NOT the final edge.
    assert!(f.owner.require_capture(&capture).is_err());
    f.generation.store(8, SeqCst);
    assert!(!f.owner.is_current_cached(&capture));
    f.generation.store(7, SeqCst);
    assert!(f.owner.is_current_cached(&capture));
    f.requested.store(true, SeqCst);
    assert!(!f.owner.is_current_cached(&capture));
    drop(guard);
}

#[test]
fn cached_ui_check_sees_canonical_withdrawal_before_outside_guard_counter_retirement() {
    let f = Fixture::new();
    let capture = f.capture();
    assert!(f.owner.is_current_cached(&capture));
    // Hold native atomics unchanged to isolate the canonical record's fence.
    // Production lock publishes them first; cached withdrawal must still stand
    // on its own before the moved close owner can notify outside the guard.
    let retiring = f.owner.begin_retirement(7).unwrap();
    assert!(capture.current.is_current());
    assert!(!f.owner.is_current_cached(&capture));
    futures::executor::block_on(retiring.into_batch().unwrap().retire());
    assert!(!capture.current.is_current());
    assert!(!f.owner.is_current_cached(&capture));
}
