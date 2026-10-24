//! Normal native V2 ownership/discovery over real blocked TCP edges. No private watcher or
//! pair state is injected. This test proves signalling custody, not audible WebRTC media.
use super::*;
use crate::voice_authority::SelectedHandoffOwner;
use catcoms_sync::voice_v2::presence::{PairSelector, VoicePairConfirmation};
use catcoms_sync::voice_v2::{Body, CallGrant};

/// Two actual browser brokers, not fixed AES providers or framed endpoint mocks. This
/// proves a single-pair Node/native cycle; it does not establish WebView policy or live RTC.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_webcrypto_confirmation_uses_real_http_and_blocked_tcp_member_bridge() {
    native_webcrypto_cycle(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_webcrypto_end_closes_automatic_work_before_first_browser_input() {
    native_webcrypto_cycle(true).await;
}

async fn native_webcrypto_cycle(end_before_input: bool) {
    use crate::voice_handoff::integration::webcrypto::{catch_fixture, Endpoint, Failure};
    async fn observation(
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
    async fn endpoint(scenario: &Scenario, index: usize) -> Result<Endpoint, Failure> {
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
        match catch_fixture(Endpoint::prepare(state, owner)).await {
            Ok(result) => result,
            Err(error) => Err(error),
        }
    }
    let mut scenario = Scenario::new(false);
    scenario.admit().await;
    scenario.close(&[3, 4, 5]).await;
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    for index in [0, 1, 2] {
        enable(&scenario, index).await;
    }
    // B owns no controller, endpoint key, call owner or crypto broker. Actual TCP policy
    // forbids A-C; only the original member helper path can deliver the signed envelopes.
    let endpoints = tokio::join!(endpoint(&scenario, 1), endpoint(&scenario, 2));
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
        let (ap, cp) = tokio::join!(a.derive(discovered[0].1), c.derive(discovered[1].1));
        let a_handshake = ap.confirmation_handshake().unwrap();
        let c_handshake = cp.confirmation_handshake().unwrap();
        let a_clock = crate::voice_runtime(&scenario.online(1).state).clock();
        let c_clock = crate::voice_runtime(&scenario.online(2).state).clock();
        let observations = || async {
            let (a, c) = tokio::join!(
                observation(
                    &scenario.online(1).actor,
                    a_clock.as_ref(),
                    &ag,
                    discovered[0].1,
                    a_handshake
                ),
                observation(
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
    let mut scenario = Scenario::new(false);
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
    confirmation_over_blocked_tcp(false, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_confirmation_framed_result_uses_actual_actor_body_and_original_phase() {
    confirmation_over_blocked_tcp(true, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_confirmation_rejected_result_remains_original_history_after_return() {
    confirmation_over_blocked_tcp(true, true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_confirmation_known_incoming_retry_survives_permanent_input_loss() {
    confirmation_over_blocked_tcp(true, false, true).await;
}

async fn confirmation_over_blocked_tcp(framed: bool, reject: bool, known_incoming: bool) {
    let mut scenario = Scenario::new(false);
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
    ap.retire_plan().unwrap();
    cp.retire_plan().unwrap();
    drop((ap, cp, ag, cg));
    scenario.close(&[0, 1, 2]).await;
    scenario.rules.assert_graph();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_v2_confirmation_alias_claims_refuse_before_publication_in_both_barrier_orderings() {
    for (wrong_first, lose_input) in [(true, false), (false, false), (true, true), (false, true)] {
        let mut scenario = Scenario::new(false);
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
