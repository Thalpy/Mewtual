//! Normal native V2 ownership/discovery over real blocked TCP edges. No private watcher or
//! pair state is injected. This test proves signalling custody, not audible WebRTC media.
use super::*;
use crate::voice_authority::SelectedHandoffOwner;
use crate::voice_handoff::integration::webcrypto::{catch_fixture, Endpoint, Failure, Round};
use catcoms_sync::voice_v2::presence::{PairSelector, VoicePairConfirmation};
use catcoms_sync::voice_v2::{Body, CallGrant};
use serde_json::json;

/// Two actual browser brokers, not fixed AES providers or framed endpoint mocks. This
/// proves a single-pair Node/native cycle; it does not establish WebView policy or live RTC.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_webcrypto_confirmation_uses_real_http_and_blocked_tcp_member_bridge() {
    native_webcrypto_cycle(false, false, false, None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_webcrypto_end_closes_automatic_work_before_first_browser_input() {
    native_webcrypto_cycle(true, false, false, None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_and_browser_automatically_confirm_over_blocked_tcp_member_bridge() {
    native_webcrypto_cycle(false, true, false, None).await;
}

/// Native Ready now traverses normal dispatch after both original browser/confirmation
/// Returns. No Ready frame, receipt or private Pair state is injected by this scenario.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_ready_is_automatic_over_blocked_tcp_member_bridge_after_browser_confirmation_returns(
) {
    native_webcrypto_cycle(false, true, true, None).await;
}

async fn confirmation_observation(
    actor: &catcoms_app::ServerActor,
    clock: &dyn catcoms_rt::Clock,
    grant: &Arc<CallGrant>,
    selector: PairSelector,
    handshake: [u8; 32],
) -> VoicePairConfirmation {
    bounded("exact native pair confirmation observation", async {
        loop {
            match actor
                .voice_pair_confirmation(grant.clone(), selector, handshake)
                .await
            {
                Ok(state) => return state,
                // Real final Group/Policy contention is a proven pre-observation refusal.
                // Retry ONLY that same identity; neither wrong state nor a stale grant is
                // success. The outer bound also covers an Actor reply which never arrives.
                Err(catcoms_app::VoiceEndpointError::Protocol(
                    catcoms_sync::voice_v2::VoiceError::Capacity,
                )) => clock.sleep(std::time::Duration::from_millis(20)).await,
                Err(error) => panic!("native pair observation refused: {error:?}"),
            }
        }
    })
    .await
}
async fn browser_endpoint(
    scenario: &Scenario,
    index: usize,
    paused: bool,
) -> Result<Endpoint, Failure> {
    browser_endpoint_mode(scenario, index, paused, false).await
}
async fn browser_endpoint_mode(
    scenario: &Scenario,
    index: usize,
    paused: bool,
    deferred: bool,
) -> Result<Endpoint, Failure> {
    let state = scenario.online(index).state.clone();
    let owner = catch_fixture(async {
        let controller = prepare_voice_controller_inner(&state).await.unwrap();
        activate_voice_owner_inner(&state, controller.clone())
            .await
            .unwrap();
        let owner =
            prepare_voice_owner_inner(&state, controller, 1, channel_id("general").to_string())
                .await
                .unwrap();
        activate_voice_owner_inner(&state, owner.clone())
            .await
            .unwrap();
        owner
    })
    .await?;
    match catch_fixture(async {
        if deferred {
            Endpoint::prepare_deferred_discovery(state, owner).await
        } else if paused {
            Endpoint::prepare_paused(state, owner).await
        } else {
            Endpoint::prepare(state, owner).await
        }
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(error),
    }
}

async fn native_webcrypto_cycle(
    end_before_input: bool,
    automatic_browser: bool,
    exercise_ready: bool,
    exercise_sdp: Option<Round>,
) {
    let mut scenario = Scenario::new(false, isolate().await);
    scenario.admit().await;
    scenario.close(&[3, 4, 5]).await;
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    for index in [0, 1, 2] {
        enable(&scenario, index).await;
    }
    // B owns no controller, endpoint key, call owner or crypto broker. Actual TCP policy
    // forbids A-C; only the original member helper path can deliver the signed envelopes.
    let endpoints = tokio::join!(
        browser_endpoint(&scenario, 1, false),
        browser_endpoint(&scenario, 2, false)
    );
    let (mut a, mut c) = match endpoints {
        (Ok(a), Ok(c)) => (a, c),
        (Ok(mut a), Err(error)) => {
            a.collect().await;
            std::panic::resume_unwind(error)
        }
        (Err(error), Ok(mut c)) => {
            c.collect().await;
            std::panic::resume_unwind(error)
        }
        (Err(error), Err(_)) => std::panic::resume_unwind(error),
    };
    let cycle = catch_fixture(async {
        let a_selected = a.selected();
        let c_selected = c.selected();
        let ag = a_selected.grant().unwrap();
        let cg = c_selected.grant().unwrap();
        let discovered = both_fresh(&scenario, &a_selected, &c_selected, &ag, &cg, None, 0).await;
        if end_before_input {
            let (ap, cp) = tokio::join!(
                a.wait_native_derivation(discovered[0].1),
                c.wait_native_derivation(discovered[1].1)
            );
            assert!(!ap.status().unwrap().engine.settled && !cp.status().unwrap().engine.settled);
            // Neither Node has received a Derive/child control step. The normal close
            // below must discover native work and use original authenticated cleanup only.
            return;
        }
        if automatic_browser {
            // Only start the original consumers. Neither endpoint receives parent/child
            // identities or a scripted role sequence; native maintenance and broker work
            // discovery must compose independently through the real unjoined helper.
            tokio::join!(a.automatic_confirmation(), c.automatic_confirmation());
            let ap = scenario
                .online(1)
                .state
                .voice_handoff
                .lookup_original(
                    &a_selected,
                    crate::voice_handoff::ledger::Purpose::Derive(discovered[0].1),
                )
                .unwrap();
            let cp = scenario
                .online(2)
                .state
                .voice_handoff
                .lookup_original(
                    &c_selected,
                    crate::voice_handoff::ledger::Purpose::Derive(discovered[1].1),
                )
                .unwrap();
            let a_clock = crate::voice_runtime(&scenario.online(1).state).clock();
            let c_clock = crate::voice_runtime(&scenario.online(2).state).clock();
            let (a_state, c_state) = tokio::join!(
                confirmation_observation(
                    &scenario.online(1).actor,
                    a_clock.as_ref(),
                    &ag,
                    discovered[0].1,
                    ap.confirmation_handshake().unwrap()
                ),
                confirmation_observation(
                    &scenario.online(2).actor,
                    c_clock.as_ref(),
                    &cg,
                    discovered[1].1,
                    cp.confirmation_handshake().unwrap()
                )
            );
            assert_eq!([a_state, c_state], [VoicePairConfirmation::Established; 2]);
            if exercise_ready {
                // No explicit Ready preparation/queue/extraction API is called here.
                // Actual KeyClaim adoption and normal native maintenance must advance
                // both endpoints through the unjoined bridge with A-C still forbidden.
                tokio::select! {
                    _ = async {
                        while !ap.ready_test_complete() || !cp.ready_test_complete() {
                            a_clock.sleep(std::time::Duration::from_millis(20)).await;
                        }
                    } => {},
                    _ = SystemClock.sleep(Duration::from_secs(30)) => {
                        let transport = {
                            let trace = scenario.rules.trace.lock().unwrap();
                            format!("connections={:?}; received={:?}; submitted={:?}; Ready attempts/helpers={:?}",
                                trace.connections, trace.received_kinds, trace.connected_submissions, trace.ready_submissions)
                        }; // Never hold the transport trace while observing native ledger locks.
                        panic!("automatic Ready stalled: (LOW, progress, released, current) A={:?}, C={:?}; {transport}",
                            ap.ready_test_observation(), cp.ready_test_observation());
                    },
                }
                // Core192KiB + original HIGH Ready32KiB and both actual old Workflows
                // have drained/extracted before either new64KiB native sidecar is paid.
                // Actual browser64KiB custody uses HTTP12/13 and independently checks the
                // installed native SDK records; no RTC/SDP effect permission is fabricated.
                tokio::join!(a.sdp_custody(&ap), c.sdp_custody(&cp));
                // Original PC identity now traverses real HTTP14 and the retained broker.
                // This still creates no RTCPeerConnection or SDP/ICE/media effect.
                tokio::join!(a.pc_custody(&ap), c.pc_custody(&cp));
                if let Some(round) = exercise_sdp {
                    let lower=if matches!(round,Round::MediaHeldLower) {
                        let low=if scenario.clients[1].identity.as_ref().unwrap().1 < scenario.clients[2].identity.as_ref().unwrap().1 {1} else {2};
                        let (sent,received)=tokio::sync::oneshot::channel();
                        let (selected,parent)=if low==1 {(&a_selected,&ap)} else {(&c_selected,&cp)};
                        let mut slot=scenario.rules.voice_lower.lock().unwrap();
                        assert!(slot.is_none());
                        *slot=Some(super::transport::VoiceLowerCapture {endpoint:low,
                            issuer:scenario.clients[low].identity.as_ref().unwrap().1,join:selected.join(),
                            handshake:parent.confirmation_handshake().unwrap(),sent});
                        Some((low,received))
                    } else {None};
                    tokio::join!(a.sdp_round(&ap, round), c.sdp_round(&cp, round));
                    if matches!(round,Round::MediaHistory|Round::MediaHistoryCollector) {
                        let held=matches!(round,Round::MediaHistoryCollector);
                        tokio::join!(a.native_history_retirement(ap,discovered[0].1,held),
                            c.native_history_retirement(cp,discovered[1].1,held));
                        scenario.rules.assert_graph();
                        return;
                    }
                    if let Some((low,received))=lower {
                        let owner=bounded("original successful TCP Source lower owner capture",received).await.unwrap();
                        let blocked=if low==1 {a.block_media_index(&ap)} else {c.block_media_index(&cp)};
                        tokio::join!(a.media_retire_start(),c.media_retire_start());
                        let source_parent=if low==1 {&ap} else {&cp};
                        bounded("original runtime Source expiry under full native index",async {
                            while a_clock.monotonic_ms()<source_parent.media_test_until() {a_clock.sleep(Duration::from_millis(20)).await;}
                        }).await;
                        let capacity_observed=catch_fixture(async {
                            let endpoint=if low==1 {&mut a} else {&mut c};
                            assert_eq!(endpoint.media_retire_progress().await,json!({"done":false,"paid":true,"oldCurrent":true,"closed":0}));
                            assert!(!source_parent.media_test_migrated());
                        }).await;
                        drop(blocked); // Actual free-slot refusal leaves the same nested root usable.
                        if let Err(error)=capacity_observed {drop(owner);std::panic::resume_unwind(error);}
                        let observed=catch_fixture(async {
                            let (endpoint,parent,other,other_parent)=if low==1 {(&mut a,&ap,&mut c,&cp)} else {(&mut c,&cp,&mut a,&ap)};
                            bounded("original Source expiry and Pending Plan-final with actual lower still owned",async {
                                loop {
                                    if parent.media_test_migrated() && parent.media_test_final_progress().0 {break;}
                                    a_clock.sleep(Duration::from_millis(20)).await;
                                }
                            }).await;
                            assert_eq!(parent.media_test_final_progress(),(true,false,false));
                            assert_eq!(endpoint.media_retire_progress().await,json!({"done":false,"paid":true,"oldCurrent":false,"closed":0}));
                            // The other endpoint has no captured lower and must finish independently.
                            bounded("other endpoint retirement progresses while lower custody is held",async {
                                while !other_parent.media_test_final_progress().2 {a_clock.sleep(Duration::from_millis(20)).await;}
                            }).await;
                            assert_eq!(other.media_retire_progress().await,json!({"done":true,"paid":false,"oldCurrent":false,"closed":0}));
                        }).await;
                        drop(owner); // Release actual Source Flight OUTSIDE every native/rules guard, including failure cleanup.
                        if let Err(error)=observed {std::panic::resume_unwind(error);}
                        tokio::join!(a.media_retire_finish(),c.media_retire_finish());
                        assert_eq!(ap.media_test_final_progress(),(true,true,true));
                        assert_eq!(cp.media_test_final_progress(),(true,true,true));
                    }
                }
            }
            scenario.rules.assert_graph();
            return;
        }
        let (ap, cp) = tokio::join!(a.derive(discovered[0].1), c.derive(discovered[1].1));
        let a_handshake = ap.confirmation_handshake().unwrap();
        let c_handshake = cp.confirmation_handshake().unwrap();
        let a_clock = crate::voice_runtime(&scenario.online(1).state).clock();
        let c_clock = crate::voice_runtime(&scenario.online(2).state).clock();
        let observations = || async {
            let (a, c) = tokio::join!(
                confirmation_observation(
                    &scenario.online(1).actor,
                    a_clock.as_ref(),
                    &ag,
                    discovered[0].1,
                    a_handshake
                ),
                confirmation_observation(
                    &scenario.online(2).actor,
                    c_clock.as_ref(),
                    &cg,
                    discovered[1].1,
                    c_handshake
                ),
            );
            [a, c]
        };
        assert_eq!(observations().await, [VoicePairConfirmation::Confirming; 2]);
        let (low, low_parent, high, high_parent, low_index) =
            if scenario.clients[1].identity.as_ref().unwrap().1
                < scenario.clients[2].identity.as_ref().unwrap().1
            {
                (&mut a, &ap, &mut c, &cp, 0)
            } else {
                (&mut c, &cp, &mut a, &ap, 1)
            };
        low.start(low_parent).await;
        let hello = high.incoming(high_parent, 1).await;
        high.respond(high_parent, hello).await;
        let proof = low.incoming(low_parent, 2).await;
        low.respond(low_parent, proof).await;
        // Hold the final browser open even if transport has already delivered the signed
        // Finish. Sender signing is local establishment, not proof of receiver confirmation.
        let held = observations().await;
        assert_eq!(held[low_index], VoicePairConfirmation::Established);
        assert_eq!(held[1 - low_index], VoicePairConfirmation::Confirming);
        high.incoming(high_parent, 3).await;
        assert_eq!(
            observations().await,
            [VoicePairConfirmation::Established; 2]
        );
        scenario.rules.assert_graph();
        // Each step separately required its original native receipt. These independent pair
        // observations still grant no SDP/ICE/RTC authority or packaged WebView acceptance.
        drop((ap, cp, ag, cg));
    })
    .await;
    if let Err(error) = cycle {
        let (a, c) = tokio::join!(catch_fixture(a.collect()), catch_fixture(c.collect()));
        assert!(
            a.is_ok() && c.is_ok(),
            "both failed-cycle endpoints must finish resource collection"
        );
        std::panic::resume_unwind(error);
    }
    // Both collection attempts finish even if the other endpoint fails an assertion.
    let (a, c) = tokio::join!(catch_fixture(a.close()), catch_fixture(c.close()));
    for outcome in [a, c] {
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) | Err(error) => std::panic::resume_unwind(error),
        }
    }
    scenario.close(&[0, 1, 2]).await;
}

/// Start only each production browser lifecycle after native selected/setup. All further
/// discovery/confirmation/Ready/SDP/PC/media/history transitions run without test phase
/// hints. Actual TCP forbids the endpoint edge; helper0 owns no voice endpoint or key.
/// Controlled RTC validates custody, not ICE reachability or audible native media.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_browser_coordinator_completes_native_handoff_over_blocked_tcp_bridge() {
    Box::pin(coordinated_browser_cycle()).await;
}

async fn coordinated_browser_cycle() {
    let mut scenario = Scenario::new(false, isolate().await);
    scenario.admit().await;
    scenario.close(&[3, 4, 5]).await;
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    for index in [0, 1, 2] {
        enable(&scenario, index).await;
    }
    let endpoints = tokio::join!(
        Box::pin(browser_endpoint(&scenario, 1, false)),
        Box::pin(browser_endpoint(&scenario, 2, false))
    );
    let (mut a, mut c) = match endpoints {
        (Ok(a), Ok(c)) => (a, c),
        (Ok(mut a), Err(error)) => {
            a.collect().await;
            std::panic::resume_unwind(error)
        }
        (Err(error), Ok(mut c)) => {
            c.collect().await;
            std::panic::resume_unwind(error)
        }
        (Err(error), Err(_)) => std::panic::resume_unwind(error),
    };
    let cycle = catch_fixture(Box::pin(async {
        tokio::join!(
            Box::pin(a.coordinate_peers(1)),
            Box::pin(c.coordinate_peers(1))
        );
        // No prior native End may make the synchronous browser fence trivially true.
        a.end_coordinated_peers(1).await;
        c.end_coordinated_peers(1).await;
        scenario.rules.assert_graph();
        let trace = scenario.rules.trace.lock().unwrap();
        for edge in [(0, 1, 28), (1, 0, 28), (0, 2, 28), (2, 0, 28)] {
            assert!(
                trace.received_kinds.get(&edge).copied().unwrap_or(0) > 0,
                "actual automatic negotiation must traverse unjoined helper: {edge:?}"
            );
        }
    }))
    .await;
    if let Err(error) = cycle {
        let (ar, cr) = tokio::join!(
            Box::pin(catch_fixture(a.collect())),
            Box::pin(catch_fixture(c.collect()))
        );
        assert!(
            ar.is_ok() && cr.is_ok(),
            "both failed coordinated endpoints must collect"
        );
        std::panic::resume_unwind(error);
    }
    let (ar, cr) = tokio::join!(
        Box::pin(catch_fixture(a.close())),
        Box::pin(catch_fixture(c.close()))
    );
    for result in [ar, cr] {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) | Err(error) => std::panic::resume_unwind(error),
        }
    }
    scenario.close(&[0, 1, 2]).await;
}

/// Genuine second native peer on the matching existing endpoint's original selected key/HTTP Session. Only the RTC
/// provider is controlled. The first A-B path is blocked and uses unjoined helper0;
/// the newcomer then naturally selects one original endpoint over the permitted graph.
/// This checks composition on that endpoint, not automatic all-three call progress.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_two_peers_share_original_session_after_actual_history_collection() {
    // Three full endpoint lifecycle futures are substantially larger than their metadata.
    // Keep this test coordinator small on the standard harness stack; runtime budgets and
    // actual application/actor futures remain unchanged.
    Box::pin(two_browser_peer_cycle(SecondPeer::Complete)).await;
}

/// Actual native failure of C before storage/SDK/media admission must leave A-B's
/// original indexed media, keys/counters and Current usable. RTC remains controlled.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_failed_attached_peer_preserves_original_session_and_indexed_peer() {
    Box::pin(two_browser_peer_cycle(SecondPeer::FailedAttached)).await;
}

/// Actual local pre-snapshot contention advances C's normal cursor to its other
/// fresh peer. The SAME native-selected pair must satisfy all preservation assertions.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_local_discovery_refusal_selects_other_peer_and_preserves_indexed_sibling() {
    Box::pin(two_browser_peer_cycle(
        SecondPeer::FailedAttachedAfterDiscoveryRefusal,
    ))
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_failed_media_peer_preserves_original_session_and_indexed_sibling() {
    Box::pin(two_browser_peer_cycle(SecondPeer::FailedMedia)).await;
}

/// A real local discovery refusal must not leave the alternate native pair unable to
/// fund storage/SDK/media. This reaches the branch of the Linux failure deliberately;
/// no fixture-selected row, Ready flag or replacement key is supplied to production.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_local_discovery_refusal_selects_other_media_peer_and_preserves_indexed_sibling(
) {
    Box::pin(two_browser_peer_cycle(
        SecondPeer::FailedMediaAfterDiscoveryRefusal,
    ))
    .await;
}

#[derive(Clone, Copy)]
enum SecondPeer {
    Complete,
    FailedAttached,
    FailedAttachedAfterDiscoveryRefusal,
    FailedMedia,
    FailedMediaAfterDiscoveryRefusal,
}

async fn two_browser_peer_cycle(second: SecondPeer) {
    eprintln!("two-peer fixture: initial admission");
    let mut scenario = Scenario::new(false, isolate().await);
    scenario.admit().await;
    // Keep C an already-admitted MLS member, but offline for the first round so
    // helper0 is the ONLY permitted bridge. Reopen does not commit membership/epoch.
    scenario.close(&[3, 4, 5]).await;
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    for index in [0, 1, 2] {
        enable(&scenario, index).await;
    }
    // The refusal regression labels native identity order so cursor1 means the other
    // endpoint. This arranges observations only; no selector is supplied to a producer.
    // Ordinary cases follow whichever original pair C naturally selects.
    let forced_refusal = matches!(
        second,
        SecondPeer::FailedAttachedAfterDiscoveryRefusal
            | SecondPeer::FailedMediaAfterDiscoveryRefusal
    );
    let media_failure = matches!(
        second,
        SecondPeer::FailedMedia | SecondPeer::FailedMediaAfterDiscoveryRefusal
    );
    let (ai, bi) = if forced_refusal
        && scenario.clients[2].identity.as_ref().unwrap().1
            < scenario.clients[1].identity.as_ref().unwrap().1
    {
        (2, 1)
    } else {
        (1, 2)
    };
    let endpoints = tokio::join!(
        Box::pin(browser_endpoint(&scenario, ai, false)),
        Box::pin(browser_endpoint(&scenario, bi, false))
    );
    let (mut a, mut b) = match endpoints {
        (Ok(a), Ok(b)) => (a, b),
        (Ok(mut a), Err(error)) => {
            a.collect().await;
            std::panic::resume_unwind(error)
        }
        (Err(error), Ok(mut b)) => {
            b.collect().await;
            std::panic::resume_unwind(error)
        }
        (Err(error), Err(_)) => std::panic::resume_unwind(error),
    };
    let mut newcomer = None;
    let cycle = catch_fixture(Box::pin(async {
        let mut ab = discovered_browser_pair(&scenario, &a, ai, &b, bi).await;
        eprintln!("two-peer fixture: first complete pair");
        let mut old = Box::pin(complete_browser_media_pair(&mut a, &mut b, ab)).await;
        a.current_peers(1).await;
        b.current_peers(1).await;
        // Both native mode8 and browser compaction actually returned before next peer
        // admission. Late old Begin goes over actual authenticated HTTP in this step.
        eprintln!("two-peer fixture: first history collected; advance");
        // Both original browsers must be able to drive a naturally selected newcomer.
        // Their old indexed media stays live; advancing only A silently assumes native
        // C will never choose B, which is false after a local discovery refusal.
        tokio::join!(a.advance_pair(), b.advance_pair());
        scenario.reopen(&[3]).await;
        enable(&scenario, 3).await;
        eprintln!("two-peer fixture: newcomer setup");
        let c = match Box::pin(browser_endpoint_mode(&scenario, 3, true, forced_refusal)).await {
            Ok(c) => c,
            Err(error) => std::panic::resume_unwind(error),
        };
        newcomer = Some(c);
        let c = newcomer.as_mut().unwrap();
        let c_owner = c.selected();
        let cg = c_owner.grant().unwrap();
        let a_owner = a.selected();
        let b_owner = b.selected();
        let clock = crate::voice_runtime(&scenario.online(3).state).clock();
        bounded(
            "new endpoint proves both actual existing voice members before factory release",
            async {
                loop {
                    let (ca, cb) = tokio::join!(
                        fresh(
                            &scenario,
                            3,
                            &cg,
                            &a_owner,
                            scenario.clients[ai].identity.as_ref().unwrap().1
                        ),
                        fresh(
                            &scenario,
                            3,
                            &cg,
                            &b_owner,
                            scenario.clients[bi].identity.as_ref().unwrap().1
                        )
                    );
                    if ca.is_some() && cb.is_some() {
                        break;
                    }
                    clock.sleep(Duration::from_millis(20)).await;
                }
            },
        )
        .await;
        let (ac, bc) = tokio::join!(
            discovered_browser_pair(&scenario, &a, ai, c, 3),
            discovered_browser_pair(&scenario, &b, bi, c, 3)
        );
        eprintln!("two-peer fixture: newcomer discovered both; release");
        c.release_discovery();
        if forced_refusal {
            c.discover_after_local_refusal().await;
        }
        let choice = c.naturally_attached_pair([ac[1], bc[1]]).await;
        if forced_refusal {
            assert_eq!(
                choice, 1,
                "actual admitted local refusal must exercise alternate native pair remapping"
            );
        }
        // Rename the matching existing endpoint A for the shared preservation assertions.
        // This changes only the oracle/driver, not native selection, timers or authority.
        // Keep BOTH old peers current, including the endpoint not chosen by C.
        let ac = if choice == 1 {
            eprintln!("two-peer fixture: newcomer naturally selected the other original endpoint");
            std::mem::swap(&mut a, &mut b);
            old = (old.1, old.0);
            ab.swap(0, 1);
            bc
        } else {
            ac
        };
        assert_eq!(
            ab[0].epoch, ac[0].epoch,
            "reopening an existing member must not create a new MLS epoch"
        );
        if !matches!(second, SecondPeer::Complete) {
            eprintln!(
                "two-peer fixture: start newcomer confirmation without touching indexed peer"
            );
            let (ap, cp) = tokio::join!(
                Box::pin(a.automatic_pair(ac[0])),
                Box::pin(c.automatic_pair(ac[1]))
            );
            eprintln!("two-peer fixture: naturally selected newcomer pair confirmed");
            let (failed_id, _) = if media_failure {
                bounded(
                    "both genuine failed-peer Ready/core owners drain before RTC",
                    async {
                        while !ap.ready_test_complete() || !cp.ready_test_complete() {
                            clock.sleep(Duration::from_millis(20)).await;
                        }
                    },
                )
                .await;
                tokio::join!(Box::pin(a.sdp_custody(&ap)), Box::pin(c.sdp_custody(&cp)));
                tokio::join!(Box::pin(a.pc_custody(&ap)), Box::pin(c.pc_custody(&cp)));
                tokio::join!(
                    Box::pin(a.sdp_round(&ap, Round::Media)),
                    Box::pin(c.sdp_round(&cp, Round::Media))
                );
                tokio::join!(
                    Box::pin(a.fail_attached_media_pair(1, &ap)),
                    Box::pin(c.fail_attached_media_pair(0, &cp))
                )
            } else {
                tokio::join!(
                    Box::pin(a.fail_attached_pair(1, &ap)),
                    Box::pin(c.fail_attached_pair(0, &cp))
                )
            };
            // Retained failed rows still carry the exact original parent identities.
            assert_eq!(failed_id[..8], old.0[..8]);
            assert_ne!(failed_id, old.0);
            a.assert_collected_pair_known(ab[0]);
            if media_failure {
                a.surviving_peer(false).await;
            } else {
                a.current_peers(1).await;
            }
            b.current_peers(1).await;
            if media_failure {
                a.surviving_peer(true).await;
            } else {
                a.end_browser_peers(1).await;
            }
        } else {
            eprintln!("two-peer fixture: second complete pair");
            let next = Box::pin(complete_browser_media_pair(&mut a, c, ac)).await;
            assert_ne!(
                old.0, next.0,
                "genuine second native parent has a different full serial"
            );
            assert_eq!(
                &old.0[..8],
                &next.0[..8],
                "genuine next peer reuses a collected native slot"
            );
            a.assert_collected_pair_known(ab[0]);
            a.current_peers(2).await;
            b.current_peers(1).await;
            c.current_peers(1).await;
            a.end_browser_peers(2).await;
        }
        scenario.rules.assert_graph();
        let trace = scenario.rules.trace.lock().unwrap();
        for edge in [(0, 1, 28), (1, 0, 28), (0, 2, 28), (2, 0, 28)] {
            assert!(
                trace.received_kinds.get(&edge).copied().unwrap_or(0) > 0,
                "first actual pair must traverse the unjoined helper: {edge:?}"
            );
        }
    }))
    .await;
    if let Err(error) = cycle {
        // Every constructed endpoint is attempted even if a sibling cleanup panics. Its
        // collector releases any real scheduling hold before native End, outside guards.
        let (ar, br, cr) = tokio::join!(
            Box::pin(catch_fixture(a.collect())),
            Box::pin(catch_fixture(b.collect())),
            async {
                if let Some(c) = &mut newcomer {
                    Box::pin(catch_fixture(c.collect())).await
                } else {
                    Ok(true)
                }
            }
        );
        assert!(
            ar.is_ok() && br.is_ok() && cr.is_ok(),
            "all failed two-peer endpoints must collect"
        );
        std::panic::resume_unwind(error);
    }
    eprintln!("two-peer fixture: close all endpoints");
    let c = newcomer.unwrap();
    let (ar, br, cr) = tokio::join!(
        Box::pin(catch_fixture(a.close())),
        Box::pin(catch_fixture(b.close())),
        Box::pin(catch_fixture(c.close()))
    );
    for result in [ar, br, cr] {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) | Err(error) => std::panic::resume_unwind(error),
        }
    }
    scenario.close(&[0, 1, 2, 3]).await;
}

async fn discovered_browser_pair(
    scenario: &Scenario,
    a: &Endpoint,
    ai: usize,
    b: &Endpoint,
    bi: usize,
) -> [PairSelector; 2] {
    let ao = a.selected();
    let bo = b.selected();
    let ag = ao.grant().unwrap();
    let bg = bo.grant().unwrap();
    let clock = crate::voice_runtime(&scenario.online(ai).state).clock();
    bounded(
        "both exact original native watchers discover the next browser pair",
        async {
            loop {
                let (ab, ba) = tokio::join!(
                    fresh(
                        scenario,
                        ai,
                        &ag,
                        &bo,
                        scenario.clients[bi].identity.as_ref().unwrap().1
                    ),
                    fresh(
                        scenario,
                        bi,
                        &bg,
                        &ao,
                        scenario.clients[ai].identity.as_ref().unwrap().1
                    )
                );
                if let (Some(ab), Some(ba)) = (ab, ba) {
                    return [ab.1, ba.1];
                }
                clock.sleep(Duration::from_millis(20)).await;
            }
        },
    )
    .await
}

async fn complete_browser_media_pair(
    a: &mut Endpoint,
    b: &mut Endpoint,
    selectors: [PairSelector; 2],
) -> ([u8; 16], [u8; 16]) {
    let (ap, bp) = tokio::join!(
        Box::pin(a.automatic_pair(selectors[0])),
        Box::pin(b.automatic_pair(selectors[1]))
    );
    assert_eq!(
        ap.confirmation_handshake().unwrap(),
        bp.confirmation_handshake().unwrap()
    );
    eprintln!(
        "two-peer Ready start: A {}; other {}",
        a.ready_diagnostic(&ap),
        b.ready_diagnostic(&bp)
    );
    tokio::select! {
        _ = async {
            while !ap.ready_test_complete() || !bp.ready_test_complete() {
                SystemClock.sleep(Duration::from_millis(20)).await;
            }
        } => {},
        _ = SystemClock.sleep(Duration::from_secs(30)) => {
            panic!("scoped native Ready stalled: A {}; other {}", a.ready_diagnostic(&ap), b.ready_diagnostic(&bp));
        },
    }
    tokio::join!(Box::pin(a.sdp_custody(&ap)), Box::pin(b.sdp_custody(&bp)));
    tokio::join!(Box::pin(a.pc_custody(&ap)), Box::pin(b.pc_custody(&bp)));
    tokio::join!(
        Box::pin(a.sdp_round(&ap, Round::MediaHistory)),
        Box::pin(b.sdp_round(&bp, Round::MediaHistory))
    );
    // No synthetic changed-join reservation here: next slot reuse is asserted against
    // the real second Root factory, selected by normal discovery on the same Session.
    tokio::join!(
        Box::pin(a.collect_pair_history(ap, selectors[0])),
        Box::pin(b.collect_pair_history(bp, selectors[1]))
    )
}

async fn enable(scenario: &Scenario, index: usize) {
    let running = scenario.online(index);
    let generation = running.state.ui_session_generation.load(Ordering::Acquire);
    bounded(
        "actual native unlock/voice actor attachment",
        finalize_unlock_session(&running.state, generation, false),
    )
    .await
    .unwrap();
    bounded(
        "trusted native V2 forwarding switch",
        running
            .actor
            .set_voice_routing(catcoms_sync::VoiceRouting::Forwarding),
    )
    .await
    .unwrap();
}
async fn join(scenario: &Scenario, index: usize) -> Arc<SelectedHandoffOwner> {
    let state = &scenario.online(index).state;
    let controller = bounded(
        "native controller candidate",
        prepare_voice_controller_inner(state),
    )
    .await
    .unwrap();
    bounded(
        "native controller activation",
        activate_voice_owner_inner(state, controller.clone()),
    )
    .await
    .unwrap();
    let owner = bounded(
        "native call owner candidate",
        prepare_voice_owner_inner(state, controller, 1, channel_id("general").to_string()),
    )
    .await
    .unwrap();
    bounded(
        "native call activation",
        activate_voice_owner_inner(state, owner.clone()),
    )
    .await
    .unwrap();
    let executor = hex::encode([9; 32]);
    let key = bounded(
        "native selected-key admission",
        crate::voice_join_key::admit_inner(state, &owner, &executor),
    )
    .await
    .unwrap();
    let point: [u8; 65] = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap();
    bounded(
        "native endpoint public-key publication",
        crate::voice_join_key::publish_inner(state, &owner, &executor, &key.permit, point),
    )
    .await
    .unwrap();
    bounded(
        "native selected-key settlement",
        crate::voice_join_key::settle_inner(state, &owner, [9; 32]),
    )
    .await
    .unwrap();
    state
        .voice_authority
        .lock()
        .unwrap()
        .selected_handoff_owner(&owner, [9; 32])
        .unwrap()
}
async fn fresh(
    scenario: &Scenario,
    index: usize,
    grant: &Arc<CallGrant>,
    peer: &Arc<SelectedHandoffOwner>,
    device: DeviceId,
) -> Option<(u64, PairSelector)> {
    let peers = bounded(
        "participant's exact original watcher",
        scenario
            .online(index)
            .actor
            .voice_participant_peers(grant.clone()),
    )
    .await
    .unwrap();
    peers.into_iter().find_map(|evidence| {
        let object = evidence.announcement();
        let Body::Announcement { key, epoch, .. } = object.body() else {
            return None;
        };
        (object.issuer() == device
            && key.join == peer.join()
            && key.key_ref == catcoms_sync::voice_v2::key_reference(&peer.point()))
        .then_some((
            evidence.ordinal(),
            PairSelector {
                peer: device,
                join: key.join,
                key_ref: key.key_ref,
                epoch: *epoch,
            },
        ))
    })
}
async fn both_fresh(
    scenario: &Scenario,
    a: &Arc<SelectedHandoffOwner>,
    c: &Arc<SelectedHandoffOwner>,
    ag: &Arc<CallGrant>,
    cg: &Arc<CallGrant>,
    previous: Option<[u64; 2]>,
    bridge: usize,
) -> [(u64, PairSelector); 2] {
    let ad = scenario.clients[1].identity.as_ref().unwrap().1;
    let cd = scenario.clients[2].identity.as_ref().unwrap().1;
    let wait = async {
        loop {
            let (ap, cp) =
                tokio::join!(fresh(scenario, 1, ag, c, cd), fresh(scenario, 2, cg, a, ad));
            if let (Some(ap), Some(cp)) = (ap, cp) {
                if previous.is_none_or(|old| ap.0 > old[0] && cp.0 > old[1]) {
                    return [ap, cp];
                }
            }
            SystemClock.sleep(Duration::from_millis(50)).await;
        }
    };
    tokio::select! {
        result = wait => result,
        _ = SystemClock.sleep(Duration::from_secs(30)) => {
            let net = super::voice::bridge_diagnostic(scenario, bridge).await;
            let trace = scenario.rules.trace.lock().unwrap();
            panic!("V2 original implicit watchers did not prove both endpoints through bridge{bridge}: native={net:?}; received kind28={:?}; submissions={:?}", trace.received_kinds, trace.connected_submissions);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_discovers_blocked_endpoints_through_unjoined_tcp_bridge_then_restored_replacement(
) {
    let mut scenario = Scenario::new(false, isolate().await);
    scenario.admit().await;
    let saved = super::voice::wait_for_saved_bridge_endpoints(&scenario).await;
    scenario.close(&[3, 4, 5]).await;
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    for index in [0, 1, 2] {
        enable(&scenario, index).await;
    }
    // B is only an attached member/forwarder: no controller, owner or endpoint is created.
    let a = join(&scenario, 1).await;
    let c = join(&scenario, 2).await;
    let ag = a.grant().unwrap();
    let cg = c.grant().unwrap();
    let initial = both_fresh(&scenario, &a, &c, &ag, &cg, None, 0).await;
    scenario.rules.assert_graph();
    {
        let trace = scenario.rules.trace.lock().unwrap();
        for edge in [(0, 1, 28), (1, 0, 28), (0, 2, 28), (2, 0, 28)] {
            assert!(
                trace.received_kinds.get(&edge).copied().unwrap_or(0) > 0,
                "actual V2 ingress on permitted edge {edge:?}"
            );
        }
    }
    // Do not prepare a pair yet: an unestablished pair has an original15s deadline. A cached
    // PairReceipt cannot prove renewed discovery, and changing its selector would hide that.
    scenario.close(&[0]).await;
    let left = SystemClock.monotonic_ms();
    scenario.reopen(&[3]).await;
    assert!(super::voice::has_saved_bridge_endpoints(
        &network(&scenario.online(3).state).await,
        &saved
    ));
    enable(&scenario, 3).await;
    // Original observations must expire without B. D must renew the SAME joins/keys via the
    // actual source/helper scheduler, while ordinary actor commands continue to make progress.
    let load = scenario.command_load(&[1, 2, 3]);
    let remaining = (catcoms_sync::voice_v2::MAX_PACKET_LIFETIME_MS + 1)
        .saturating_sub(SystemClock.monotonic_ms().saturating_sub(left));
    SystemClock.sleep(Duration::from_millis(remaining)).await;
    let renewed = both_fresh(
        &scenario,
        &a,
        &c,
        &ag,
        &cg,
        Some([initial[0].0, initial[1].0]),
        3,
    )
    .await;
    drop(load);
    {
        let trace = scenario.rules.trace.lock().unwrap();
        for edge in [(3, 1, 28), (1, 3, 28), (3, 2, 28), (2, 3, 28)] {
            assert!(
                trace.received_kinds.get(&edge).copied().unwrap_or(0) > 0,
                "restored D actually served V2 edge {edge:?}"
            );
        }
    }
    let (ap, cp) = tokio::join!(
        scenario
            .online(1)
            .actor
            .prepare_voice_pair(ag, renewed[0].1),
        scenario
            .online(2)
            .actor
            .prepare_voice_pair(cg, renewed[1].1)
    );
    let ap = ap.unwrap();
    let cp = cp.unwrap();
    assert_eq!(ap.handshake, cp.handshake);
    assert_ne!(ap.local_is_low, cp.local_is_low);
    // The exact selected owners installed through native activation/publication now reach
    // real factory/Input/admission/Actor-result/ACK/settlement adapters, not a synthetic pair.
    tokio::join!(
        crate::voice_handoff::integration::derivation(&scenario.online(1).state, &a, renewed[0].1),
        crate::voice_handoff::integration::derivation(&scenario.online(2).state, &c, renewed[1].1),
    );
    scenario.close(&[1, 2, 3]).await;
    scenario.rules.assert_graph();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_confirmation_child_uses_the_returned_actual_parent_over_blocked_tcp_discovery() {
    confirmation_over_blocked_tcp(false, false, false, false, None, None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_confirmation_framed_result_uses_actual_actor_body_and_original_phase() {
    confirmation_over_blocked_tcp(true, false, false, false, None, None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_plan_closing_fence_recovers_claim_and_key_without_new_admission_or_end() {
    confirmation_over_blocked_tcp(true, false, false, false, None, Some(0)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_plan_closing_fence_keeps_storage_copy_and_sdk_debt_until_actual_release() {
    confirmation_over_blocked_tcp(true, false, false, false, None, Some(1)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_confirmation_rejected_result_remains_original_history_after_return() {
    confirmation_over_blocked_tcp(true, true, false, false, None, None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_confirmation_known_incoming_retry_survives_permanent_input_loss() {
    confirmation_over_blocked_tcp(true, false, true, false, None, None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_abandoned_custody_needs_exact_physical_collection_dead_lease_and_root_drain() {
    confirmation_over_blocked_tcp(true, false, false, true, None, None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_pc_intent_close_and_sdk_release_order_actual_queued_and_returned_actor_work() {
    confirmation_over_blocked_tcp(true, false, false, false, Some(0), None).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_pc_intent_sdk_release_before_begin_and_lost_queued_waiter_never_revive() {
    confirmation_over_blocked_tcp(true, false, false, false, Some(1), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_pc_intent_changed_sdp_input_refuses_before_publication_and_writer_blocks_close()
{
    confirmation_over_blocked_tcp(true, false, false, false, Some(2), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_pc_intent_recorded_sdp_keeps_admitted_engine_until_actual_settlement() {
    confirmation_over_blocked_tcp(true, false, false, false, Some(3), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_pc_intent_lost_publication_recovers_original_outcome_after_parent_close() {
    confirmation_over_blocked_tcp(true, false, false, false, Some(4), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_pc_intent_recorded_actor_outcomes_retire_before_actual_io_collection() {
    confirmation_over_blocked_tcp(true, false, false, false, Some(5), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_pc_intent_original_sdp_worker_retries_local_refusal_and_collects_after_waiter_loss(
) {
    confirmation_over_blocked_tcp(true, false, false, false, Some(6), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_pc_intent_original_sdp_completion_retires_body_but_keeps_task_and_ui_until_collection(
) {
    confirmation_over_blocked_tcp(true, false, false, false, Some(7), None).await;
}

async fn confirmation_over_blocked_tcp(
    framed: bool,
    reject: bool,
    known_incoming: bool,
    orphan: bool,
    pc_cases: Option<u8>,
    fence_cases: Option<u8>,
) {
    let clock = (pc_cases == Some(8)).then(crate::voice_handoff::integration::ExpiryClock::new);
    if matches!(pc_cases, Some(10..=13)) {
        crate::voice_handoff::integration::with_result_completion(
            std::sync::Arc::new(tokio::sync::Barrier::new(2)),
            confirmation_over_blocked_tcp_inner(
                framed,
                reject,
                known_incoming,
                orphan,
                pc_cases,
                fence_cases,
                None,
            ),
        )
        .await;
    } else if let Some(clock) = clock {
        crate::voice_handoff::integration::with_expiry_clock(
            clock.clone(),
            confirmation_over_blocked_tcp_inner(
                framed,
                reject,
                known_incoming,
                orphan,
                pc_cases,
                fence_cases,
                Some(clock),
            ),
        )
        .await;
    } else {
        confirmation_over_blocked_tcp_inner(
            framed,
            reject,
            known_incoming,
            orphan,
            pc_cases,
            fence_cases,
            None,
        )
        .await;
    }
}
#[allow(clippy::too_many_arguments)]
async fn confirmation_over_blocked_tcp_inner(
    framed: bool,
    reject: bool,
    known_incoming: bool,
    orphan: bool,
    pc_cases: Option<u8>,
    fence_cases: Option<u8>,
    clock: Option<Arc<crate::voice_handoff::integration::ExpiryClock>>,
) {
    let mut scenario = Scenario::new(false, isolate().await);
    scenario.voice_clock = clock.map(|clock| clock as Arc<dyn catcoms_rt::Clock>);
    scenario.admit().await;
    scenario.close(&[3, 4, 5]).await;
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    for index in [0, 1, 2] {
        enable(&scenario, index).await;
    }
    let a = join(&scenario, 1).await;
    let c = join(&scenario, 2).await;
    let ag = a.grant().unwrap();
    let cg = c.grant().unwrap();
    let discovered = both_fresh(&scenario, &a, &c, &ag, &cg, None, 0).await;
    // A-C is forbidden by the real socket graph; unjoined B only serves discovery here.
    // Test-only AES keys exercise native confirmation; this does not claim RTC or browser keys.
    scenario.rules.assert_graph();
    let (ap, cp) = tokio::join!(
        crate::voice_handoff::integration::derivation_returned(
            &scenario.online(1).state,
            &a,
            discovered[0].1
        ),
        crate::voice_handoff::integration::derivation_returned(
            &scenario.online(2).state,
            &c,
            discovered[1].1
        ),
    );
    // This post-authentication fixture used to have no HTTP Host. Native key adoption
    // nudges once, but actual old-Lower drainage can require another paid Ready turn.
    // Start the real service cadence; never manufacture Ready/CoreDrained in the test.
    let services = if framed && !reject && !known_incoming {
        Some([1, 2].map(|index| {
            crate::voice_handoff::integration::NativeFixtureService::start(
                &scenario.online(index).state,
            )
        }))
    } else {
        None
    };
    if scenario.clients[1].identity.as_ref().unwrap().1
        < scenario.clients[2].identity.as_ref().unwrap().1
    {
        if known_incoming {
            crate::voice_handoff::integration::confirmation_known_incoming_retry(
                &scenario.online(1).state,
                &a,
                &ap,
                &scenario.online(2).state,
                &c,
                &cp,
            )
            .await;
        } else if reject {
            crate::voice_handoff::integration::confirmation_rejected_result(
                &scenario.online(1).state,
                &a,
                &ap,
                &scenario.online(2).state,
                &c,
                &cp,
            )
            .await;
        } else if framed {
            crate::voice_handoff::integration::confirmation_framed_round_trip(
                &scenario.online(1).state,
                &a,
                &ap,
                &scenario.online(2).state,
                &c,
                &cp,
                orphan,
                pc_cases,
                fence_cases,
            )
            .await;
        } else {
            crate::voice_handoff::integration::confirmation_round_trip(
                &scenario.online(1).state,
                &a,
                &ap,
                &scenario.online(2).state,
                &c,
                &cp,
            )
            .await;
        }
    } else if known_incoming {
        crate::voice_handoff::integration::confirmation_known_incoming_retry(
            &scenario.online(2).state,
            &c,
            &cp,
            &scenario.online(1).state,
            &a,
            &ap,
        )
        .await;
    } else if reject {
        crate::voice_handoff::integration::confirmation_rejected_result(
            &scenario.online(2).state,
            &c,
            &cp,
            &scenario.online(1).state,
            &a,
            &ap,
        )
        .await;
    } else if framed {
        crate::voice_handoff::integration::confirmation_framed_round_trip(
            &scenario.online(2).state,
            &c,
            &cp,
            &scenario.online(1).state,
            &a,
            &ap,
            orphan,
            pc_cases,
            fence_cases,
        )
        .await;
    } else {
        crate::voice_handoff::integration::confirmation_round_trip(
            &scenario.online(2).state,
            &c,
            &cp,
            &scenario.online(1).state,
            &a,
            &ap,
        )
        .await;
    }
    crate::end_voice_owner_inner(&scenario.online(1).state, a.id()).await;
    crate::end_voice_owner_inner(&scenario.online(2).state, c.id()).await;
    let purposes = [
        crate::voice_handoff::ledger::Purpose::Derive(discovered[0].1),
        crate::voice_handoff::ledger::Purpose::Derive(discovered[1].1),
    ];
    if orphan {
        assert_ne!(
            ap.retire_plan().is_ok(),
            cp.retire_plan().is_ok(),
            "only the abandoned Borrowed Claim refuses ordinary retirement"
        );
    } else {
        ap.retire_plan().unwrap();
        cp.retire_plan().unwrap();
    }
    drop((ap, cp, ag, cg));
    if orphan {
        let low = if scenario.clients[1].identity.as_ref().unwrap().1
            < scenario.clients[2].identity.as_ref().unwrap().1
        {
            1
        } else {
            2
        };
        let selected = if low == 1 { &a } else { &c };
        let state = &scenario.online(low).state;
        state.voice_channels.collect_retired(&state.voice_handoff);
        assert!(
            state
                .voice_handoff
                .lookup_original(selected, purposes[low - 1])
                .is_none(),
            "dead original history collects only after its actual last row owner drops"
        );
    }
    if let Some(services) = services {
        for service in services {
            service.finish().await;
        }
    }
    scenario.close(&[0, 1, 2]).await;
    scenario.rules.assert_graph();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_confirmation_alias_claims_refuse_before_publication_in_both_barrier_orderings() {
    for (wrong_first, lose_input) in [(true, false), (false, false), (true, true), (false, true)] {
        let mut scenario = Scenario::new(false, isolate().await);
        scenario.admit().await;
        scenario.close(&[3, 4, 5]).await;
        scenario.rules.drop_gossip.store(true, Ordering::Release);
        for index in [0, 1, 2] {
            enable(&scenario, index).await;
        }
        let a = join(&scenario, 1).await;
        let c = join(&scenario, 2).await;
        let ag = a.grant().unwrap();
        let cg = c.grant().unwrap();
        let discovered = both_fresh(&scenario, &a, &c, &ag, &cg, None, 0).await;
        let (ap, cp) = tokio::join!(
            crate::voice_handoff::integration::derivation_returned(
                &scenario.online(1).state,
                &a,
                discovered[0].1
            ),
            crate::voice_handoff::integration::derivation_returned(
                &scenario.online(2).state,
                &c,
                discovered[1].1
            ),
        );
        if scenario.clients[1].identity.as_ref().unwrap().1
            < scenario.clients[2].identity.as_ref().unwrap().1
        {
            crate::voice_handoff::integration::confirmation_alias_claims(
                &scenario.online(1).state,
                &a,
                &ap,
                wrong_first,
                lose_input,
            )
            .await;
        } else {
            crate::voice_handoff::integration::confirmation_alias_claims(
                &scenario.online(2).state,
                &c,
                &cp,
                wrong_first,
                lose_input,
            )
            .await;
        }
        crate::end_voice_owner_inner(&scenario.online(1).state, a.id()).await;
        crate::end_voice_owner_inner(&scenario.online(2).state, c.id()).await;
        ap.retire_plan().unwrap();
        cp.retire_plan().unwrap();
        drop((ap, cp, ag, cg));
        scenario.close(&[0, 1, 2]).await;
        scenario.rules.assert_graph();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_pc_intent_original_http_begin_history_return_and_premature_high_refusal() {
    confirmation_over_blocked_tcp(true, false, false, false, Some(8), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_original_sdp_engine_uses_actual_actor_and_recovers_lost_permit_after_queue_and_lock_races(
) {
    confirmation_over_blocked_tcp(true, false, false, false, Some(9), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_original_sdp_result_uses_actual_actor_and_survives_eager_cancellation_lock_and_history_retries(
) {
    confirmation_over_blocked_tcp(true, false, false, false, Some(10), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_sdp_result_lost_after_commit_recovers_original_seal_and_plaintext_open_through_tcp_bridge(
) {
    confirmation_over_blocked_tcp(true, false, false, false, Some(11), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_sdp_empty_result_cannot_replace_nonempty_original_open() {
    confirmation_over_blocked_tcp(true, false, false, false, Some(12), None).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_sdp_maximum_seal_and_open_cross_actual_native_result_and_tcp_bridge() {
    confirmation_over_blocked_tcp(true, false, false, false, Some(13), None).await;
}

/// Actual two-process retained brokers + authenticated HTTP/BYOB/WebCrypto and normal
/// native/Root dispatch over A-B-C TCP with A-C forbidden. B joins no call. The controlled
/// RTC provider proves ordering/composition only, not real SDP/ICE/media or WebView policy.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_maximum_offer_answer_round_uses_real_http_and_blocked_tcp_bridge() {
    native_webcrypto_cycle(false, true, true, Some(Round::Sdp)).await;
}

/// Each endpoint loses both actual native Result replies, the first completed action24
/// and successful Applied26, after independent canonical native observations. The same
/// browser keys/PC/actions must recover without repeating either controlled RTC method.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_original_round_retries_lost_http_results_applied_and_return() {
    native_webcrypto_cycle(false, true, true, Some(Round::LostSdp)).await;
}

/// Actual original PC transfers through authenticated HTTP28 after canonical maximum
/// Offer/Answer over A-B-C TCP (A-C forbidden, B joins no call). Controlled RTC only;
/// this proves native/browser ownership and End, not physical media/audio connectivity.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_media_adopts_same_original_peer_through_real_http_and_blocked_tcp() {
    native_webcrypto_cycle(false, true, true, Some(Round::Media)).await;
}

/// Lose only authenticated positive op28 Admit/Current/Release observations. Exact
/// recovery must not recreate PC/payment; native End closes the actual controlled SDK
/// peer while its browser owner remains live, then all original obligations collect.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_media_recovers_lost_authenticated_admit_current_release_and_native_end() {
    native_webcrypto_cycle(false, true, true, Some(Round::LostMedia)).await;
}

/// Native original expiry, index migration and old Plan-final/ACK happen while the same
/// browser peer and authenticated Session stay live. No test deadline or media grant is forged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_media_retires_original_negotiation_and_keeps_indexed_peer_usable() {
    native_webcrypto_cycle(false, true, true, Some(Round::MediaRetire)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_media_lost_detach_reply_recovers_index_before_old_plan_retirement() {
    native_webcrypto_cycle(false, true, true, Some(Round::LostMediaRetire)).await;
}

/// Retains the exact lower Source Flight delivered to the actual TCP driver, not a
/// fixture boolean or a delayed adapter future. Expiry cannot refund/finalize it early;
/// indexed media and the other endpoint still progress until actual owner Drop.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_media_keeps_same_peer_current_while_expired_original_lower_delays_final_ack(
) {
    native_webcrypto_cycle(false, true, true, Some(Round::MediaHeldLower)).await;
}

/// Native-scope unused candidate custody, after the real maximum HTTP/TCP SDP round.
/// No browser media transfer is claimed: Release abandons the offered native successor.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_sdp_index_candidate_release_wins_and_last_drop_follows_actual_ui_return() {
    native_webcrypto_cycle(false, true, true, Some(Round::NativeIndexDrop)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_media_native_history_collection_preserves_same_session_and_peer() {
    native_webcrypto_cycle(false, true, true, Some(Round::MediaHistory)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_browser_media_history_second_collector_cannot_overtake_actual_last_parent_drop() {
    native_webcrypto_cycle(false, true, true, Some(Round::MediaHistoryCollector)).await;
}
