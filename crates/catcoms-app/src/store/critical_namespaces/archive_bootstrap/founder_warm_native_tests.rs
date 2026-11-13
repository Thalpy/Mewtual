//! Actual whole warm Host with real SDK/current/outbox custody and synthetic UI
//! atomics. No native/index row, checked IPC or production save credential exists
//! in this fixture. Linux's bounded directory profile remains a separate gate.
use super::*;
use crate::store::MemberSaveJobReport;
use catcoms_replication::chat::Body;
use catcoms_rt::{NativeChatSourceOwner, NativeChatUiBinding};
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Arc,
};
use std::task::{Context, Poll, Waker};

#[test]
fn native_whole_warm_source_keeps_its_origin_through_current_host_and_member_save() {
    let directory = tempfile::tempdir().unwrap();
    let shared = pool();
    let mut rng = ChaCha20Rng::seed_from_u64(0x35a1);
    let layout = mount(directory.path(), &shared, &mut rng, true, true);
    let binding = NativeChatUiBinding::from_trusted_host(
        Arc::new(AtomicU64::new(7)),
        Arc::new(AtomicU64::new(7)),
        Arc::new(AtomicBool::new(false)),
    );
    let ui = binding.open_session(7).unwrap();
    let source = NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap();
    let mut keys = NativeFounderKeys::prepare(&source, &ui.observe()).unwrap();
    keys.enter_device().unwrap();
    let before = entries();
    let FreshV2TextFounderBegin::Original(mut driver) = layout
        .prepare_fresh_v2_text_founder_with_origin(
            [71; 32],
            "native whole warm source",
            policy(),
            inputs(0x35a1),
            FounderOrigin::NativeRequired(source.observe()),
        )
        .unwrap()
    else {
        panic!("genuine original")
    };
    assert!(driver.retry(&layout, &mut NeverRng).is_err());
    assert_eq!(entries(), before);
    driver
        .retry_with_access(&layout, &mut rng, FounderAccess::Native(&mut keys))
        .unwrap();
    assert!(driver.is_ready_data());
    let exact_source = *driver.binding_data().unwrap().source_data();
    assert_eq!(entries(), before + 1);
    assert!(driver
        .host
        .as_mut()
        .unwrap()
        .retry_empty_promotion(&layout, &mut NeverRng)
        .is_err());
    assert!(driver
        .host
        .as_mut()
        .unwrap()
        .retry_original_seed_settlement(&layout, &mut NeverRng)
        .is_err());
    // A genuine same-host controller for a different source allocation cannot
    // take over this retained whole product, even with identical UI labels.
    let foreign = NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap();
    let mut wrong = NativeFounderKeys::prepare(&foreign, &ui.observe()).unwrap();
    assert!(driver
        .retry_with_access(&layout, &mut NeverRng, FounderAccess::Native(&mut wrong))
        .is_err());
    driver
        .retry_with_access(&layout, &mut NeverRng, FounderAccess::Native(&mut keys))
        .unwrap();
    assert_eq!(driver.binding_data().unwrap().source_data(), &exact_source);
    assert_eq!(entries(), before + 1);

    let (_session, receiver) = tokio::sync::watch::channel(false);
    let negative = RequestCancellation::new(receiver, None);
    let body = Body::Create {
        text: "native whole ordinary member".into(),
        reply_to: None,
    };
    let request = || V2MemberSignRequest {
        token: [72; 32],
        channel: 7,
        body: &body,
        parents: &[],
        dependencies: &[],
    };
    assert!(driver
        .prepare_member_save(&layout, request(), &negative, &mut NeverRng)
        .is_err());
    assert_eq!(
        driver
            .prepare_member_save_with_access(
                &layout,
                request(),
                &negative,
                &mut rng,
                FounderAccess::Native(&mut keys)
            )
            .unwrap(),
        V2MemberSaveOutcome::Ready
    );
    assert!(driver
        .admit_member_save(
            &layout,
            [72; 32],
            MemberSaveNativeAdmission::new((), negative.clone())
        )
        .is_err());
    // TEST custody only. This checks Root source/mode/composition, not native
    // authorization; only the real native private producer may supply this box.
    let job = driver
        .admit_member_save_with_access(
            &layout,
            [72; 32],
            MemberSaveNativeAdmission::new((), negative.clone()),
            FounderAccess::Native(&mut keys),
        )
        .unwrap();
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    assert!(matches!(
        driver
            .prepare_member_save_with_access(
                &layout,
                request(),
                &negative,
                &mut NeverRng,
                FounderAccess::Native(&mut keys)
            )
            .unwrap(),
        V2MemberSaveOutcome::Saved { .. }
    ));
    let drain = ui.retire();
    assert!(driver
        .prepare_member_save_with_access(
            &layout,
            request(),
            &negative,
            &mut NeverRng,
            FounderAccess::Native(&mut keys)
        )
        .is_err());
    let mut waiting = Box::pin(drain.wait());
    assert!(matches!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    drop(keys);
    assert!(matches!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(())
    ));
    driver.close_and_drain().unwrap();
    assert!(!driver.is_ready_data());
}

#[test]
fn native_whole_warm_final_save_refuses_late_source_close_or_generation_publication() {
    use std::sync::atomic::Ordering::SeqCst;
    for close_source in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let shared = pool();
        let seed = 0x35b0 + u64::from(close_source);
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let layout = mount(directory.path(), &shared, &mut rng, true, true);
        let generation = Arc::new(AtomicU64::new(7));
        let authorized = Arc::new(AtomicU64::new(7));
        let binding = NativeChatUiBinding::from_trusted_host(
            generation.clone(),
            authorized.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        let ui = binding.open_session(7).unwrap();
        let mut source =
            Some(NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap());
        let mut keys = NativeFounderKeys::prepare(source.as_ref().unwrap(), &ui.observe()).unwrap();
        keys.enter_device().unwrap();
        let FreshV2TextFounderBegin::Original(mut driver) = layout
            .prepare_fresh_v2_text_founder_with_origin(
                [81; 32],
                "native save edge",
                policy(),
                inputs(seed),
                FounderOrigin::NativeRequired(source.as_ref().unwrap().observe()),
            )
            .unwrap()
        else {
            panic!("genuine original")
        };
        driver
            .retry_with_access(&layout, &mut rng, FounderAccess::Native(&mut keys))
            .unwrap();
        let (_session, receiver) = tokio::sync::watch::channel(false);
        let negative = RequestCancellation::new(receiver, None);
        let body = Body::Create {
            text: "original final-edge data".into(),
            reply_to: None,
        };
        let request = || V2MemberSignRequest {
            token: [82; 32],
            channel: 7,
            body: &body,
            parents: &[],
            dependencies: &[],
        };
        assert_eq!(
            driver
                .prepare_member_save_with_access(
                    &layout,
                    request(),
                    &negative,
                    &mut rng,
                    FounderAccess::Native(&mut keys)
                )
                .unwrap(),
            V2MemberSaveOutcome::Ready
        );
        let host = driver.host.as_ref().unwrap();
        let original = host.member_save_original_for_test(&layout).unwrap();
        let completed = host.member_save_ends_completed_for_test().unwrap();
        let observed = Arc::new(AtomicBool::new(false));
        let hook_seen = observed.clone();
        let hook: Box<dyn FnOnce()> = if close_source {
            let closing = source.take().unwrap();
            Box::new(move || {
                drop(closing);
                hook_seen.store(true, SeqCst);
            })
        } else {
            let generation = generation.clone();
            Box::new(move || {
                generation.fetch_add(1, SeqCst);
                hook_seen.store(true, SeqCst);
            })
        };
        let (returned, refusal) =
            PreparedV2TextHost::<MemNetwork, ChaCha20Rng>::with_native_member_save_edge_for_test(
                hook,
                || {
                    driver.admit_member_save_with_access(
                        &layout,
                        [82; 32],
                        MemberSaveNativeAdmission::new((), negative.clone()),
                        FounderAccess::Native(&mut keys),
                    )
                },
            )
            .unwrap_err();
        assert!(
            observed.load(SeqCst),
            "the actual final edge must be reached"
        );
        assert!(
            !negative.is_cancelled(),
            "legacy session cancellation deliberately stayed live"
        );
        assert_eq!(refusal, crate::store::V2MemberSaveRefusal::Unavailable);
        drop(returned);
        let host = driver.host.as_ref().unwrap();
        assert_eq!(host.member_save_owner_idle_for_test(&layout), Some(true));
        assert_eq!(
            host.member_save_original_for_test(&layout).unwrap(),
            original
        );
        assert!(host
            .member_save_custody_for_test(&layout)
            .unwrap()
            .starts_with("Prepared"));
        assert_eq!(
            host.member_save_ends_completed_for_test(),
            Some(completed + 1)
        );
        drop(keys);
        drop(ui);
        if !close_source {
            authorized.store(generation.load(SeqCst), SeqCst);
            let fresh_ui = binding.open_session(generation.load(SeqCst)).unwrap();
            let mut fresh =
                NativeFounderKeys::prepare(source.as_ref().unwrap(), &fresh_ui.observe()).unwrap();
            assert_eq!(
                driver
                    .prepare_member_save_with_access(
                        &layout,
                        request(),
                        &negative,
                        &mut NeverRng,
                        FounderAccess::Native(&mut fresh)
                    )
                    .unwrap(),
                V2MemberSaveOutcome::Ready
            );
            assert_eq!(
                driver
                    .host
                    .as_ref()
                    .unwrap()
                    .member_save_original_for_test(&layout)
                    .unwrap(),
                original
            );
            let job = driver
                .admit_member_save_with_access(
                    &layout,
                    [82; 32],
                    MemberSaveNativeAdmission::new((), negative.clone()),
                    FounderAccess::Native(&mut fresh),
                )
                .unwrap();
            assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
        }
        driver.close_and_drain().unwrap();
    }
}
