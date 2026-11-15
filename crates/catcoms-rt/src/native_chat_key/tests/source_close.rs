//! Counter-only row fences. Synthetic host atoms prove neither AppState caller
//! authentication nor a real signing primitive; Root/native tests own those.
use super::*;

#[test]
fn source_close_handle_refuses_every_prepared_purpose_and_unlock_cannot_revive_it() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let purposes = [
        NativeChatKeyPurpose::Device,
        NativeChatKeyPurpose::FounderSdk,
        NativeChatKeyPurpose::BootstrapProfile,
        NativeChatKeyPurpose::BootstrapHeader,
        NativeChatKeyPurpose::MemberAction,
        NativeChatKeyPurpose::AddSdk,
        NativeChatKeyPurpose::GoverningHeader,
    ];
    let mut cuts: Vec<_> = purposes
        .iter()
        .map(|purpose| source.prepare_cut(&ui.observe(), *purpose).unwrap())
        .collect();
    let closer = source.close_handle();
    closer.close();
    closer.close();
    let mut ends = NativeChatKeyEnds::new();
    for (cut, purpose) in cuts.iter_mut().zip(purposes) {
        assert_eq!(
            cut.enter(&source.observe(), purpose, &mut ends),
            Err(NativeChatKeyRefusal::Unavailable)
        );
    }
    assert!(ends.slots.iter().all(Option::is_none));
    fixture.lock();
    fixture.unlock();
    let fresh = fixture.session();
    assert!(matches!(
        source.prepare_cut(&fresh.observe(), NativeChatKeyPurpose::MemberAction),
        Err(NativeChatKeyRefusal::Unavailable)
    ));
}

#[test]
fn source_close_handle_preserves_actual_ends_and_another_sources_admission() {
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
    let mut other_ends = NativeChatKeyEnds::new();
    first_cut
        .enter(
            &first.observe(),
            NativeChatKeyPurpose::MemberAction,
            &mut first_ends,
        )
        .unwrap();
    let drain = first.observe_retirement();
    first.close_handle().close();
    assert!(
        !drain.is_drained(),
        "a close is not retirement of an already admitted End"
    );
    other_cut
        .enter(
            &other.observe(),
            NativeChatKeyPurpose::MemberAction,
            &mut other_ends,
        )
        .unwrap();
    assert!(!other_cut.is_cancelled());
    assert_eq!(other.original.counter.state.load(Ordering::SeqCst), 1);
    drop(first_ends);
    assert!(
        drain.is_drained(),
        "only original key work drained, never SDK/physical proof"
    );
    assert!(
        !other.observe_retirement().is_drained(),
        "the other source remains open"
    );
    drop(other_ends);
}

#[test]
fn source_close_handle_drop_and_clone_grant_no_owner_or_implicit_retirement() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let close = source.close_handle();
    let another = close.clone();
    drop(close);
    drop(another);
    let mut cut = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::MemberAction)
        .unwrap();
    let mut ends = NativeChatKeyEnds::new();
    cut.enter(
        &source.observe(),
        NativeChatKeyPurpose::MemberAction,
        &mut ends,
    )
    .unwrap();
    assert!(!source.observe_retirement().is_drained());
}

#[test]
fn source_close_handle_retains_original_payer_without_source_or_ui_host() {
    struct Payer(Arc<AtomicBool>);
    impl Drop for Payer {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let fixture = Fixture::new();
    let ui = fixture.session();
    let dropped = Arc::new(AtomicBool::new(false));
    let source =
        NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(Payer(dropped.clone())))
            .unwrap();
    let observation = source.observe();
    let host = Arc::downgrade(&ui.original.host);
    let close = source.close_handle();
    let another = close.clone();
    drop(source);
    drop(ui);
    drop(fixture);
    assert!(
        observation.original.upgrade().is_none(),
        "the fence is not an issuer pin"
    );
    assert!(
        host.upgrade().is_none(),
        "counter custody has no Source/Host backlink"
    );
    assert!(!dropped.load(Ordering::SeqCst));
    close.close();
    drop(close);
    assert!(!dropped.load(Ordering::SeqCst));
    drop(another);
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn source_close_handle_at_actual_source_counter_cas_never_notifies_under_registry_guard() {
    let fixture = Fixture::new();
    let ui = fixture.session();
    let source = fixture.source(&ui);
    let mut cut = source
        .prepare_cut(&ui.observe(), NativeChatKeyPurpose::MemberAction)
        .unwrap();
    let drain = source.observe_retirement();
    let wakes = Arc::new(CountWake(AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut wait = Box::pin(drain.wait());
    assert!(wait
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_pending());
    let closer = source.close_handle();
    CUT_BOUNDARY.with(|slot| {
        *slot.borrow_mut() = Some((
            CutBoundary::AfterUiCounter,
            Box::new(move || {
                // The next CAS is the source counter, after the actual UI debit. Close
                // precisely after its state load and before its real SC compare_exchange.
                CUT_BOUNDARY.with(|slot| {
                    *slot.borrow_mut() = Some((
                        CutBoundary::BeforeCounterCas,
                        Box::new(move || closer.close()),
                    ))
                });
            }),
        ))
    });
    let mut ends = NativeChatKeyEnds::new();
    let registry = std::sync::Mutex::new(());
    {
        let _guard = registry.lock().unwrap();
        assert_eq!(
            cut.enter(
                &source.observe(),
                NativeChatKeyPurpose::MemberAction,
                &mut ends
            ),
            Err(NativeChatKeyRefusal::Unavailable)
        );
        assert!(CUT_BOUNDARY.with(|slot| slot.borrow().is_none()));
        assert_eq!(source.original.counter.state.load(Ordering::SeqCst), CLOSED);
        assert_eq!(
            ui.original.counter.state.load(Ordering::SeqCst),
            1,
            "the original refused UI End stays owned until outside guard"
        );
        assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    }
    drop(ends);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    let _retired = source.retire();
    assert_eq!(
        wakes.0.load(Ordering::SeqCst),
        1,
        "outside-guard original retirement notifies the registered waiter"
    );
    assert!(wait
        .as_mut()
        .poll(&mut Context::from_waker(&waker))
        .is_ready());
}
