//! Lifecycle/counter tests only. These test atomics are not a native UI factory,
//! a mounted source, or evidence that an actual SDK/signing primitive is gated.
use super::*;
use std::future::Future;
use std::task::{Context, Poll, Wake, Waker};

struct Fixture {
    binding: NativeChatUiBinding,
    generation: Arc<AtomicU64>,
    authorized: Arc<AtomicU64>,
    requested: Arc<AtomicBool>,
}
impl Fixture {
    fn new() -> Self {
        let generation = Arc::new(AtomicU64::new(7));
        let authorized = Arc::new(AtomicU64::new(7));
        let requested = Arc::new(AtomicBool::new(false));
        Self {
            binding: NativeChatUiBinding::from_trusted_host(
                generation.clone(),
                authorized.clone(),
                requested.clone(),
                Arc::new(()),
            ),
            generation,
            authorized,
            requested,
        }
    }
    fn session(&self) -> NativeChatUiSessionOwner {
        self.binding
            .open_session(self.generation.load(Ordering::SeqCst), Arc::new(()))
            .unwrap()
    }
    fn source(&self, session: &NativeChatUiSessionOwner) -> NativeChatSourceOwner {
        NativeChatSourceOwner::for_trusted_source(&session.observe(), Arc::new(())).unwrap()
    }
    fn lock(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.requested.store(true, Ordering::SeqCst);
    }
    fn unlock(&self) {
        self.authorized
            .store(self.generation.load(Ordering::SeqCst), Ordering::SeqCst);
        self.requested.store(false, Ordering::SeqCst);
    }
}
struct NoWake;
impl Wake for NoWake {
    fn wake(self: Arc<Self>) {}
}
fn pending(future: std::pin::Pin<&mut impl Future<Output = ()>>) -> bool {
    future
        .poll(&mut Context::from_waker(&Waker::from(Arc::new(NoWake))))
        .is_pending()
}

#[test]
fn native_chat_key_distinct_purposes_are_bound_to_the_original_source() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let wrong = fixture.source(&ui);
    let mut cut = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::FounderSdk)
        .unwrap();
    let mut ends = NativeChatKeyEnds::new();
    assert_eq!(
        cut.preflight(&wrong.observe(), NativeChatKeyPurpose::FounderSdk),
        Err(NativeChatKeyRefusal::Foreign)
    );
    assert_eq!(
        cut.preflight(&source.observe(), NativeChatKeyPurpose::BootstrapHeader),
        Err(NativeChatKeyRefusal::WrongPurpose)
    );
    assert!(cut
        .preflight(&source.observe(), NativeChatKeyPurpose::FounderSdk)
        .is_ok());
    assert_eq!(
        cut.enter(
            &wrong.observe(),
            NativeChatKeyPurpose::FounderSdk,
            &mut ends
        ),
        Err(NativeChatKeyRefusal::Foreign)
    );
    assert_eq!(
        cut.enter(
            &source.observe(),
            NativeChatKeyPurpose::BootstrapHeader,
            &mut ends
        ),
        Err(NativeChatKeyRefusal::WrongPurpose)
    );
    cut.enter(
        &source.observe(),
        NativeChatKeyPurpose::FounderSdk,
        &mut ends,
    )
    .unwrap();
    assert_eq!(
        cut.enter(
            &source.observe(),
            NativeChatKeyPurpose::FounderSdk,
            &mut ends
        ),
        Err(NativeChatKeyRefusal::AlreadyEntered)
    );
    let mut profile = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::BootstrapProfile)
        .unwrap();
    profile
        .enter(
            &source.observe(),
            NativeChatKeyPurpose::BootstrapProfile,
            &mut ends,
        )
        .unwrap();
    assert_eq!(ui.original.counter.state.load(Ordering::SeqCst), 2);
    assert_eq!(source.original.counter.state.load(Ordering::SeqCst), 2);
    drop(ends);
    assert_eq!(ui.original.counter.state.load(Ordering::SeqCst), 0);
    assert_eq!(source.original.counter.state.load(Ordering::SeqCst), 0);
}

#[test]
fn native_chat_key_generation_publication_fences_before_registry_retirement() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let mut cut = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::Device)
        .unwrap();
    // Stop the lock immediately after its real generation publication: its
    // flag and later registry retirement have deliberately not happened yet.
    fixture.generation.fetch_add(1, Ordering::SeqCst);
    assert!(!fixture.requested.load(Ordering::SeqCst));
    assert!(ui.original.counter.open());
    assert_eq!(
        cut.enter(
            &source.observe(),
            NativeChatKeyPurpose::Device,
            &mut NativeChatKeyEnds::new()
        ),
        Err(NativeChatKeyRefusal::Unavailable)
    );
    assert_eq!(ui.original.counter.state.load(Ordering::SeqCst), 0);
}

#[test]
fn native_chat_key_final_checks_refuse_each_close_after_both_counter_entries() {
    for action in 0..6 {
        let fixture = Fixture::new();
        let ui = fixture.session();
        let source = fixture.source(&ui);
        let observation = source.observe();
        let ui_counter = ui.original.counter.clone();
        let mut cut = source
            .prepare_cut(&ui.observe(), NativeChatKeyPurpose::FounderSdk)
            .unwrap();
        let hook: Box<dyn FnOnce()> = match action {
            0 => {
                let generation = fixture.generation.clone();
                Box::new(move || {
                    generation.fetch_add(1, Ordering::SeqCst);
                })
            }
            1 => {
                let authorized = fixture.authorized.clone();
                Box::new(move || {
                    authorized.store(9, Ordering::SeqCst);
                })
            }
            2 => {
                let requested = fixture.requested.clone();
                Box::new(move || {
                    requested.store(true, Ordering::SeqCst);
                })
            }
            3 => Box::new(move || drop(source)),
            4 => Box::new(move || drop(ui)),
            5 => Box::new(move || drop(fixture.binding)),
            _ => unreachable!(),
        };
        CUT_BOUNDARY.with(|slot| *slot.borrow_mut() = Some((CutBoundary::BeforeFinal, hook)));
        let mut ends = NativeChatKeyEnds::new();
        assert_eq!(
            cut.enter(&observation, NativeChatKeyPurpose::FounderSdk, &mut ends),
            Err(NativeChatKeyRefusal::Unavailable),
            "negative action {action}"
        );
        assert!(CUT_BOUNDARY.with(|slot| slot.borrow().is_none()));
        assert_eq!(ui_counter.state.load(Ordering::SeqCst) & !CLOSED, 1);
        // Refusal retains BOTH increments outside the simulated primitive frame.
        // Removing the final checks would return Ok above for every action.
        ui_counter.close_and_notify();
        let drain = NativeChatKeyDrain {
            original: ui_counter,
        };
        let mut waiting = Box::pin(drain.wait());
        assert!(pending(waiting.as_mut()));
        drop(ends);
        assert!(!pending(waiting.as_mut()));
    }
}

#[test]
fn native_chat_key_source_close_between_counters_retains_only_the_original_ui_end() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let observation = source.observe();
    let source_counter = source.original.counter.clone();
    let mut cut = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::BootstrapProfile)
        .unwrap();
    CUT_BOUNDARY.with(|slot| {
        *slot.borrow_mut() = Some((CutBoundary::AfterUiCounter, Box::new(move || drop(source))))
    });
    let mut ends = NativeChatKeyEnds::new();
    assert_eq!(
        cut.enter(
            &observation,
            NativeChatKeyPurpose::BootstrapProfile,
            &mut ends
        ),
        Err(NativeChatKeyRefusal::Unavailable)
    );
    assert!(CUT_BOUNDARY.with(|slot| slot.borrow().is_none()));
    assert_eq!(source_counter.state.load(Ordering::SeqCst), CLOSED);
    assert_eq!(ui.original.counter.state.load(Ordering::SeqCst), 1);
    let drain = ui.retire();
    let mut waiting = Box::pin(drain.wait());
    assert!(pending(waiting.as_mut()));
    drop(ends);
    assert!(!pending(waiting.as_mut()));
    assert_eq!(source_counter.state.load(Ordering::SeqCst), CLOSED);
}

#[test]
fn native_chat_key_lock_unlock_never_rebinds_an_old_command() {
    let fixture = Fixture::new();
    let old = fixture.session();
    let source = fixture.source(&old);
    let mut cut = source
        .prepare_cut(&old.observe(), NativeChatKeyPurpose::FounderSdk)
        .unwrap();
    fixture.lock();
    fixture.unlock();
    let fresh = fixture.session();
    assert_eq!(
        cut.enter(
            &source.observe(),
            NativeChatKeyPurpose::FounderSdk,
            &mut NativeChatKeyEnds::new()
        ),
        Err(NativeChatKeyRefusal::Unavailable)
    );
    let mut current = source
        .prepare_cut(&fresh.observe(), NativeChatKeyPurpose::FounderSdk)
        .unwrap();
    let mut ends = NativeChatKeyEnds::new();
    current
        .enter(
            &source.observe(),
            NativeChatKeyPurpose::FounderSdk,
            &mut ends,
        )
        .unwrap();
    // A later authenticated command can use the SAME retained source, while
    // neither the old captured generation nor its endpoint is changed.
    assert_eq!(old.original.expected, 7);
    assert_eq!(fresh.original.expected, 8);
}

#[test]
fn native_chat_key_foreign_host_with_equal_generation_is_refused() {
    let first = Fixture::new();
    let second = Fixture::new();
    let ui = first.session();
    let other = second.session();
    let source = first.source(&ui);
    assert!(matches!(
        source.prepare_cut(&other.observe(), NativeChatKeyPurpose::MemberAction),
        Err(NativeChatKeyRefusal::Foreign)
    ));
}

#[test]
fn native_chat_key_weak_observations_never_keep_close_owners_open() {
    for remove_source in [false, true] {
        let fixture = Fixture::new();
        let ui = fixture.session();
        let source = fixture.source(&ui);
        let observation = source.observe();
        let mut cut = source
            .prepare_cut(&ui.observe(), NativeChatKeyPurpose::BootstrapHeader)
            .unwrap();
        if remove_source {
            drop(source);
        } else {
            drop(ui);
        }
        assert_eq!(
            cut.enter(
                &observation,
                NativeChatKeyPurpose::BootstrapHeader,
                &mut NativeChatKeyEnds::new()
            ),
            Err(NativeChatKeyRefusal::Unavailable)
        );
    }
}

#[test]
fn native_chat_key_admitted_end_drains_before_retired_owner_completes() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let observation = source.observe();
    let mut cut = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::FounderSdk)
        .unwrap();
    let mut ends = NativeChatKeyEnds::new();
    cut.enter(&observation, NativeChatKeyPurpose::FounderSdk, &mut ends)
        .unwrap();
    let source_drain = source.retire();
    let ui_drain = ui.retire();
    let mut a = Box::pin(source_drain.wait());
    let mut b = Box::pin(ui_drain.wait());
    assert!(pending(a.as_mut()));
    assert!(pending(b.as_mut()));
    drop(ends);
    assert!(!pending(a.as_mut()));
    assert!(!pending(b.as_mut()));
    assert_eq!(
        cut.enter(
            &observation,
            NativeChatKeyPurpose::FounderSdk,
            &mut NativeChatKeyEnds::new()
        ),
        Err(NativeChatKeyRefusal::AlreadyEntered)
    );
}

#[test]
fn native_chat_key_close_first_admits_no_end() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let observation = source.observe();
    let mut cut = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::MemberAction)
        .unwrap();
    let drain = source.retire();
    let mut ends = NativeChatKeyEnds::new();
    assert_eq!(
        cut.enter(&observation, NativeChatKeyPurpose::MemberAction, &mut ends),
        Err(NativeChatKeyRefusal::Unavailable)
    );
    assert_eq!(ui.original.counter.state.load(Ordering::SeqCst), 0);
    assert!(!pending(Box::pin(drain.wait()).as_mut()));
}

#[test]
fn native_chat_key_source_capacity_refusal_retains_partial_ui_end() {
    let fixture = Fixture::new();
    let old = fixture.session();
    let source = fixture.source(&old);
    let mut entered = Vec::new();
    for _ in 0..MAX_ENTRIES {
        let mut ends = NativeChatKeyEnds::new();
        source
            .prepare_cut(&old.observe(), NativeChatKeyPurpose::MemberAction)
            .unwrap()
            .enter(
                &source.observe(),
                NativeChatKeyPurpose::MemberAction,
                &mut ends,
            )
            .unwrap();
        entered.push(ends);
    }
    // Old admitted calls can still be draining after a new unlock. This gives
    // the new UI counter room while the SAME source remains fully occupied.
    fixture.lock();
    fixture.unlock();
    let fresh = fixture.session();
    let mut cut = source
        .prepare_cut(&fresh.observe(), NativeChatKeyPurpose::MemberAction)
        .unwrap();
    let mut refused = NativeChatKeyEnds::new();
    assert_eq!(
        cut.enter(
            &source.observe(),
            NativeChatKeyPurpose::MemberAction,
            &mut refused
        ),
        Err(NativeChatKeyRefusal::Busy)
    );
    assert_eq!(fresh.original.counter.state.load(Ordering::SeqCst), 1);
    assert_eq!(
        source.original.counter.state.load(Ordering::SeqCst),
        MAX_ENTRIES
    );
    let fresh_drain = fresh.retire();
    let mut waiting = Box::pin(fresh_drain.wait());
    assert!(pending(waiting.as_mut()));
    drop(refused);
    assert!(!pending(waiting.as_mut()));
    drop(entered);
    assert_eq!(source.original.counter.state.load(Ordering::SeqCst), 0);
}

#[test]
fn native_chat_key_end_wake_is_deferred_beyond_the_outer_guard_on_unwind() {
    struct WakeCheck {
        inside: Arc<AtomicBool>,
        violation: Arc<AtomicBool>,
        woke: Arc<AtomicBool>,
    }
    impl Wake for WakeCheck {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.woke.store(true, Ordering::SeqCst);
            if self.inside.load(Ordering::SeqCst) {
                self.violation.store(true, Ordering::SeqCst);
            }
        }
    }
    struct Guard(Arc<AtomicBool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let mut cut = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::FounderSdk)
        .unwrap();
    let mut ends = NativeChatKeyEnds::new();
    cut.enter(
        &source.observe(),
        NativeChatKeyPurpose::FounderSdk,
        &mut ends,
    )
    .unwrap();
    let drain = ui.retire();
    let inside = Arc::new(AtomicBool::new(false));
    let violation = Arc::new(AtomicBool::new(false));
    let woke = Arc::new(AtomicBool::new(false));
    let waker = Waker::from(Arc::new(WakeCheck {
        inside: inside.clone(),
        violation: violation.clone(),
        woke: woke.clone(),
    }));
    let mut waiting = Box::pin(drain.wait());
    assert!(matches!(
        waiting.as_mut().poll(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        inside.store(true, Ordering::SeqCst);
        let _guard = Guard(inside.clone());
        // The batch is borrowed by this frame, never owned by an inner guard.
        assert!(ends.slots[NativeChatKeyPurpose::FounderSdk.slot()].is_some());
        panic!("injected outer key call unwind");
    }));
    assert!(unwind.is_err());
    assert!(!woke.load(Ordering::SeqCst));
    drop(ends);
    assert!(woke.load(Ordering::SeqCst));
    assert!(!violation.load(Ordering::SeqCst));
    assert!(!pending(waiting.as_mut()));
}

#[test]
fn native_chat_key_paid_custody_survives_owner_until_the_last_end_and_drain() {
    struct Custody(Arc<AtomicBool>);
    impl Drop for Custody {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let fixture = Fixture::new();
    let ui = fixture.session();
    let dropped = Arc::new(AtomicBool::new(false));
    let source = NativeChatSourceOwner::for_trusted_source(
        &ui.observe(),
        Arc::new(Custody(dropped.clone())),
    )
    .unwrap();
    let mut ends = NativeChatKeyEnds::new();
    source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::Device)
        .unwrap()
        .enter(&source.observe(), NativeChatKeyPurpose::Device, &mut ends)
        .unwrap();
    let drain = source.retire();
    assert!(!dropped.load(Ordering::SeqCst));
    drop(ends);
    assert!(!dropped.load(Ordering::SeqCst));
    drop(drain);
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn native_chat_key_early_refusal_keeps_last_metadata_destructor_outside_the_guard() {
    struct Custody {
        inside: Arc<AtomicBool>,
        violation: Arc<AtomicBool>,
        dropped: Arc<AtomicBool>,
    }
    impl Drop for Custody {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
            if self.inside.load(Ordering::SeqCst) {
                self.violation.store(true, Ordering::SeqCst);
            }
        }
    }
    struct Guard(Arc<AtomicBool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    for full_ui in [false, true] {
        let fixture = Fixture::new();
        let ui = fixture.session();
        let occupied_source = fixture.source(&ui);
        let mut occupied = Vec::new();
        if full_ui {
            for _ in 0..MAX_ENTRIES {
                let mut ends = NativeChatKeyEnds::new();
                occupied_source
                    .prepare_cut(&ui.observe(), NativeChatKeyPurpose::MemberAction)
                    .unwrap()
                    .enter(
                        &occupied_source.observe(),
                        NativeChatKeyPurpose::MemberAction,
                        &mut ends,
                    )
                    .unwrap();
                occupied.push(ends);
            }
        }
        let inside = Arc::new(AtomicBool::new(false));
        let violation = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let source = NativeChatSourceOwner::for_trusted_source(
            &ui.observe(),
            Arc::new(Custody {
                inside: inside.clone(),
                violation: violation.clone(),
                dropped: dropped.clone(),
            }),
        )
        .unwrap();
        let original = source.observe();
        // Both metadata pins and End storage belong to the OUTER frame.
        let mut cut = source
            .prepare_cut(&ui.observe(), NativeChatKeyPurpose::FounderSdk)
            .unwrap();
        let mut ends = NativeChatKeyEnds::new();
        CUT_BOUNDARY.with(|slot| {
            *slot.borrow_mut() = Some((
                if full_ui {
                    CutBoundary::BeforeUiCounter
                } else {
                    CutBoundary::BeforePrecheck
                },
                Box::new(move || drop(source)),
            ))
        });
        {
            inside.store(true, Ordering::SeqCst);
            let _guard = Guard(inside.clone());
            assert_eq!(
                cut.enter(&original, NativeChatKeyPurpose::FounderSdk, &mut ends),
                Err(if full_ui {
                    NativeChatKeyRefusal::Busy
                } else {
                    NativeChatKeyRefusal::Unavailable
                })
            );
            assert!(CUT_BOUNDARY.with(|slot| slot.borrow().is_none()));
            assert!(!dropped.load(Ordering::SeqCst));
            assert!(!violation.load(Ordering::SeqCst));
        }
        // No successful or partial entry happened. The refused cut's pin, not
        // a fabricated End, must be what retains the exact physical/payer Arc.
        assert!(ends.slots.iter().all(Option::is_none));
        drop(ends);
        assert!(!dropped.load(Ordering::SeqCst));
        drop(cut);
        assert!(dropped.load(Ordering::SeqCst));
        assert!(!violation.load(Ordering::SeqCst));
        drop(occupied);
    }
}

#[test]
fn native_chat_key_contended_final_counter_makes_only_one_cas_attempt() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let original = source.observe();
    let mut first = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::FounderSdk)
        .unwrap();
    let mut competing = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::BootstrapProfile)
        .unwrap();
    let competing_original = original.clone();
    let retained = Arc::new(std::sync::Mutex::new(NativeChatKeyEnds::new()));
    let other_ends = retained.clone();
    CUT_BOUNDARY.with(|slot| {
        *slot.borrow_mut() = Some((
            CutBoundary::BeforeCounterCas,
            Box::new(move || {
                competing
                    .enter(
                        &competing_original,
                        NativeChatKeyPurpose::BootstrapProfile,
                        &mut other_ends.lock().unwrap(),
                    )
                    .unwrap()
            }),
        ))
    });
    let mut refused = NativeChatKeyEnds::new();
    // The competing real cut enters after our original load, invalidating this
    // exact CAS. A retry loop would incorrectly return Ok instead of Busy.
    assert_eq!(
        first.enter(&original, NativeChatKeyPurpose::FounderSdk, &mut refused),
        Err(NativeChatKeyRefusal::Busy)
    );
    assert!(CUT_BOUNDARY.with(|slot| slot.borrow().is_none()));
    assert!(refused.slots.iter().all(Option::is_none));
    assert_eq!(ui.original.counter.state.load(Ordering::SeqCst), 1);
    assert_eq!(source.original.counter.state.load(Ordering::SeqCst), 1);
    drop(retained);
    assert_eq!(ui.original.counter.state.load(Ordering::SeqCst), 0);
    // No count/key unit was spent by the refused cut; a fresh attempt now has
    // room, using the same original source and command.
    first
        .enter(&original, NativeChatKeyPurpose::FounderSdk, &mut refused)
        .unwrap();
}

struct CountWake(AtomicUsize);
impl Wake for CountWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn native_chat_key_preissued_drain_waits_for_close_and_wakes_on_idle_retirement() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let drain = ui.observe_retirement();
    let other = drain.clone();
    assert!(!drain.is_drained(), "open+zero is not retirement");
    let wakes = Arc::new(CountWake(AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut wait = Box::pin(drain.wait());
    assert!(wait
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_pending());
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    let retired = ui.retire();
    assert_eq!(
        wakes.0.load(Ordering::SeqCst),
        1,
        "the preissued waiter wakes once across retirement and repeated Drop"
    );
    assert!(other.is_drained() && retired.is_drained());
    assert!(wait
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_ready());
}

#[test]
fn native_chat_key_source_sc_close_defers_idle_wake_until_outside_guard_retirement() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let mut cut = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::MemberAction)
        .unwrap();
    let observation = source.observe();
    let drain = source.observe_retirement();
    let wakes = Arc::new(CountWake(AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut wait = Box::pin(drain.wait());
    assert!(wait
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_pending());
    let guard = std::sync::Mutex::new(());
    {
        let _held = guard.lock().unwrap();
        source.close();
        source.close(); // Repeated pre-fencing is still only atomics.
        assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
        assert!(cut.is_cancelled());
        assert_eq!(
            cut.enter(
                &observation,
                NativeChatKeyPurpose::MemberAction,
                &mut NativeChatKeyEnds::new()
            ),
            Err(NativeChatKeyRefusal::Unavailable)
        );
    }
    let retired = source.retire();
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert!(retired.is_drained());
    assert!(wait
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_ready());
}

#[test]
fn native_chat_key_source_sc_close_keeps_admitted_ends_and_other_sources_independent() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let first = fixture.source(&ui);
    let other = fixture.source(&ui);
    let mut first_cut = first
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::MemberAction)
        .unwrap();
    let mut other_cut = other
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::MemberAction)
        .unwrap();
    let mut first_ends = NativeChatKeyEnds::new();
    first_cut
        .enter(
            &first.observe(),
            NativeChatKeyPurpose::MemberAction,
            &mut first_ends,
        )
        .unwrap();
    let drain = first.observe_retirement();
    let wakes = Arc::new(CountWake(AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut wait = Box::pin(drain.wait());
    assert!(wait
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_pending());
    first.close();
    let retired = first.retire();
    assert!(!retired.is_drained());
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    assert!(!other_cut.is_cancelled());
    let mut other_ends = NativeChatKeyEnds::new();
    other_cut
        .enter(
            &other.observe(),
            NativeChatKeyPurpose::MemberAction,
            &mut other_ends,
        )
        .unwrap();
    drop(first_ends);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert!(wait
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_ready());
    assert!(!other_cut.is_cancelled());
    drop(other_ends);
}

#[test]
fn native_chat_key_current_metadata_pin_observes_owner_close_without_becoming_an_issuer() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let current = ui.observe_current();
    let command = ui.observe();
    let guard = std::sync::Mutex::new(());
    {
        let _held = guard.lock().unwrap();
        assert!(current.is_current());
        fixture.generation.store(8, Ordering::SeqCst);
        assert!(!current.is_current());
    }
    fixture.generation.store(7, Ordering::SeqCst);
    assert!(current.is_current());
    drop(ui);
    assert!(!current.is_current());
    // A strong metadata pin can keep the weak command's allocation present;
    // it cannot keep the owner open or authorize another source/primitive.
    assert!(NativeChatSourceOwner::for_trusted_source(&command, Arc::new(())).is_err());
}

#[test]
fn native_chat_key_idle_retirement_keeps_actual_generation_and_host_custody_in_unentered_pins() {
    struct Charge(Arc<AtomicUsize>);
    impl Drop for Charge {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let host_dropped = Arc::new(AtomicUsize::new(0));
    let generation_dropped = Arc::new(AtomicUsize::new(0));
    let binding = NativeChatUiBinding::from_trusted_host(
        Arc::new(AtomicU64::new(7)),
        Arc::new(AtomicU64::new(7)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(Charge(host_dropped.clone())),
    );
    let ui = binding
        .open_session(7, Arc::new(Charge(generation_dropped.clone())))
        .unwrap();
    let counter = Arc::downgrade(&ui.original.counter);
    let source = NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap();
    let original = source.observe();
    let mut cut = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::FounderSdk)
        .unwrap();
    let drain = ui.retire();
    assert!(drain.is_drained(), "no primitive was entered");
    drop(drain); // Canonical retirement may now reclaim its slot/record.
    binding.close();
    drop(binding);
    drop(source);
    assert!(counter.upgrade().is_some());
    assert_eq!(host_dropped.load(Ordering::SeqCst), 0);
    assert_eq!(generation_dropped.load(Ordering::SeqCst), 0);
    assert_eq!(
        cut.enter(
            &original,
            NativeChatKeyPurpose::FounderSdk,
            &mut NativeChatKeyEnds::new()
        ),
        Err(NativeChatKeyRefusal::Unavailable)
    );
    drop(cut);
    assert!(counter.upgrade().is_none());
    assert_eq!(host_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(generation_dropped.load(Ordering::SeqCst), 1);
}

#[test]
fn native_chat_key_whole_binding_close_is_sc_only_before_separate_owner_retirement() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let original = source.observe();
    let mut cut = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::FounderSdk)
        .unwrap();
    let drain = ui.observe_retirement();
    let wakes = Arc::new(CountWake(AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut wait = Box::pin(drain.wait());
    assert!(wait
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_pending());
    fixture.binding.close();
    fixture.binding.close();
    assert_eq!(
        wakes.0.load(Ordering::SeqCst),
        0,
        "host fence must run no Wakers under its caller's guard"
    );
    assert!(
        !drain.is_drained(),
        "binding close is distinct from owner drainage"
    );
    assert_eq!(
        cut.enter(
            &original,
            NativeChatKeyPurpose::FounderSdk,
            &mut NativeChatKeyEnds::new()
        ),
        Err(NativeChatKeyRefusal::Unavailable)
    );
    assert!(fixture.binding.open_session(7, Arc::new(())).is_err());
    assert!(source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::BootstrapHeader)
        .is_err());
    drop(ui); // Actual owner batch closes/notifies outside every writer.
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert!(wait
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_ready());
}

#[test]
fn native_chat_key_session_preparation_never_owns_close_or_retargets_after_capture() {
    for close_host in [false, true] {
        let fixture = Fixture::new();
        let preparation = fixture.binding.prepare_session(7, Arc::new(()));
        if close_host {
            fixture.binding.close();
        } else {
            fixture.lock();
            fixture.unlock();
        }
        assert!(preparation.finish().is_err());
        // The old captured generation never becomes a newly authorized one.
        // A live original binding accepts only a genuinely new preparation.
        let fresh = fixture.binding.prepare_session(8, Arc::new(())).finish();
        assert_eq!(fresh.is_ok(), !close_host);
        drop(fresh);
    }
    let fixture = Fixture::new();
    let preparation = fixture.binding.prepare_session(7, Arc::new(()));
    drop(fixture); // A metadata pin must not keep the affine binding open.
    assert!(preparation.finish().is_err());
}
