//! Actual native command entries, including lifecycle after one result ACK.
use super::*;
use crate::voice_activation_tests::Fixture;
use catcoms_rt::{Clock, ManualClock};

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::select! {
        value = future => value,
        _ = SystemClock.sleep(Duration::from_secs(30)) => panic!("admitted call entry timed out"),
    }
}
async fn controller(f: &Fixture) -> String {
    let old_candidate = f.candidate().await;
    let authority = f.state.voice_authority.lock().unwrap();
    let voice_authority::CandidateKind::Call { controller, .. } = authority
        .candidate(&old_candidate, SystemClock.monotonic_ms())
        .unwrap()
    else {
        panic!("fixture candidate");
    };
    controller
}

#[tokio::test]
async fn actual_admitted_prepare_entry_completes_without_recursive_coordinator_lock() {
    let f = Fixture::with_voice_clock(Arc::new(ManualClock::new(1000))).await;
    let controller = controller(&f).await;
    let (send, receive) = tokio::sync::oneshot::channel();
    // Run the actual entry on a separate native executor. A recursive blocking
    // mutex cannot starve the outer watchdog on the fixture's original executor.
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(prepare_inner(
            &f.state,
            controller,
            7,
            channel_id("general").to_string(),
        ));
        let _ = send.send((f, result));
    });
    let (f, result) = bounded(receive).await.unwrap();
    thread.join().unwrap();
    let candidate = result.unwrap();
    assert!(matches!(
        f.state
            .voice_authority
            .lock()
            .unwrap()
            .candidate(&candidate, 1000),
        Some(voice_authority::CandidateKind::AdmittedCall { .. })
    ));
    f.close().await;
}

#[tokio::test]
async fn accepted_entry_recovers_same_result_and_one_ack_is_collected_after_normal_end() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let controller = controller(&f).await;
    for _ in 0..(voice_call_activation::MAX_ACTIVATIONS + 2) {
        let candidate = prepare_inner(
            &f.state,
            controller.clone(),
            7,
            channel_id("general").to_string(),
        )
        .await
        .unwrap();
        assert!(
            activate_voice_owner_inner(&f.state, candidate.clone())
                .await
                .is_err(),
            "Existing entry must stay closed"
        );
        let first = bounded(activate_inner(&f.state, candidate.clone()))
            .await
            .unwrap();
        let handle = retained(&f.state, &candidate).unwrap();
        let again = bounded(activate_inner(&f.state, candidate.clone()))
            .await
            .unwrap();
        assert_eq!(first.join, again.join);
        assert_eq!(first.owner, again.owner);
        // The native boundary receives a deterministic valid public point;
        // this does not claim a browser SDK operation or audible RTC acceptance.
        // Exercise key ACK/settlement BEFORE End, so maintenance must revisit a
        // taskless Generation later rather than relying on a second settlement.
        let executor = [9; 32];
        let admission = crate::voice_admitted_key::admit_inner(&f.state, &candidate, executor)
            .await
            .unwrap();
        let point: [u8; 65] = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap();
        let publication = bounded(crate::voice_admitted_key::publish_inner(
            &f.state,
            &candidate,
            executor,
            &admission.permit,
            point,
        ))
        .await
        .unwrap();
        assert_eq!(publication.join, first.join.as_ref().unwrap().as_str());
        crate::voice_admitted_key::settle_inner(&f.state, &candidate, executor)
            .await
            .unwrap();
        crate::voice_admitted_key::acknowledge_inner(&f.state, &candidate, executor).unwrap();
        acknowledge_inner(&f.state, &candidate).await.unwrap(); // exactly once, while active
        assert!(retained(&f.state, &candidate).is_some());
        handle.close();
        clock.advance_ms(250);
        bounded(handle.wait_execution_return()).await;
        assert!(handle.collectible());
        collect_retired(&f.state); // same bounded production maintenance used by next prepare
        assert!(retained(&f.state, &candidate).is_none());
    }
    f.close().await;
}

#[tokio::test]
async fn rejected_entry_drives_cleanup_without_a_second_activation_request() {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let controller = controller(&f).await;
    let healthy = f.candidate().await;
    activate_voice_owner_inner(&f.state, healthy.clone())
        .await
        .unwrap();
    for _ in 0..3 {
        let candidate = prepare_inner(
            &f.state,
            controller.clone(),
            7,
            channel_id("general").to_string(),
        )
        .await
        .unwrap();
        let first = activate_inner(&f.state, candidate.clone())
            .await
            .err()
            .unwrap();
        let handle = retained(&f.state, &candidate).unwrap();
        // Do not re-invoke activation: the rejected first request owes cleanup.
        bounded(handle.wait_execution_return()).await;
        assert_eq!(handle.reply().await.result().err().unwrap(), first);
        acknowledge_inner(&f.state, &candidate).await.unwrap();
        assert!(retained(&f.state, &candidate).is_none());
        assert!(
            f.state
                .voice_authority
                .lock()
                .unwrap()
                .owner(&healthy)
                .is_some(),
            "loser must preserve the healthy owner"
        );
    }
    end_voice_owner_inner(&f.state, &healthy).await;
    f.close().await;
}
