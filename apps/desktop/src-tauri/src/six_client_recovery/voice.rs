//! Communication Package 0: pin the legacy direct-only signalling gap on actual TCP actors.
//! This is a reproduced limitation, not acceptance of bridged room discovery or audible voice.
use super::*;

/// Project original sealed routes and actor liveness without exposing network seeds/addresses.
/// Each independent probe has its own bound so diagnostics cannot strand a failed fixture.
pub(super) async fn bridge_diagnostic(scenario: &Scenario, bridge: usize) -> Vec<String> {
    async fn observe<F: Future>(future: F) -> Option<F::Output> {
        tokio::select! {
            value = future => Some(value),
            _ = SystemClock.sleep(Duration::from_millis(500)) => None,
        }
    }
    let mut observations = Vec::new();
    for index in [1, 2, bridge] {
        let running = scenario.online(index);
        let (net, online) = tokio::join!(
            observe(network(&running.state)),
            observe(running.actor.online_members()),
        );
        let saved = net.map(|net| {
            let targets: Vec<_> = net
                .reconnect_routes
                .iter()
                .map(|route| {
                    let target = scenario
                        .rules
                        .peers
                        .iter()
                        .position(|peer| peer.as_bytes() == &route.peer_id);
                    (
                        target,
                        target.is_some_and(|target| {
                            route.address == scenario.address(target).to_string()
                        }),
                    )
                })
                .collect();
            (net.reconnect_policy, targets)
        });
        observations.push(format!("client_index={index} saved_policy_targets_current_listener={saved:?} online={online:?} cadence_submitted_passes={}",
            running.passes.load(Ordering::Acquire)));
    }
    observations
}

/// Both exact peer/listener pairs need standing member-mesh consent; one saved direction,
/// duplicates or an obsolete listener cannot establish this replacement-bridge prerequisite.
pub(super) fn has_saved_bridge_endpoints(
    net: &ServerNet,
    expected: &[(PeerId, String); 2],
) -> bool {
    net.reconnect_policy == ReconnectPolicy::MemberMesh
        && expected.iter().all(|(peer, address)| {
            net.reconnect_routes
                .iter()
                .any(|route| route.peer_id == *peer.as_bytes() && route.address == *address)
        })
}

/// Original admission already requests both allowed outgoing directions. Wait for the ordinary
/// native worker to authenticate and seal them before destroying this bridge. Merely queueing
/// those dials (or retaining only its founder route) cannot establish this restart precondition.
pub(super) async fn wait_for_saved_bridge_endpoints(scenario: &Scenario) -> [(PeerId, String); 2] {
    let expected = [1, 2].map(|target| {
        (
            scenario.rules.peers[target],
            scenario.address(target).to_string(),
        )
    });
    tokio::select! {
        _ = async {
            loop {
                let net = network(&scenario.online(3).state).await;
                if has_saved_bridge_endpoints(&net, &expected) {
                    break;
                }
                SystemClock.sleep(Duration::from_millis(100)).await;
            }
        } => {},
        // Same bound as original native route capture; the reopened recovery bound stays intact.
        _ = SystemClock.sleep(Duration::from_secs(90)) => {
            let actual = bridge_diagnostic(scenario, 3).await;
            panic!("replacement bridge lacks sealed original endpoint routes: {actual:?}");
        }
    }
    expected
}

#[test]
fn saved_bridge_endpoints_require_both_exact_peer_listener_pairs_and_member_policy() {
    let expected = [
        (PeerId::new([1; 32]), "/ip4/127.0.0.1/tcp/45001".to_owned()),
        (PeerId::new([2; 32]), "/ip4/127.0.0.1/tcp/45002".to_owned()),
    ];
    let routes: Vec<_> = expected
        .iter()
        .map(|(peer, address)| ReconnectRoute {
            peer_id: *peer.as_bytes(),
            address: address.clone(),
        })
        .collect();
    let mut net = new_server_net("", "", "");
    net.reconnect_policy = ReconnectPolicy::MemberMesh;
    for missing in 0..2 {
        net.reconnect_routes = vec![routes[1 - missing].clone()];
        assert!(!has_saved_bridge_endpoints(&net, &expected));
    }
    net.reconnect_routes = routes.clone();
    net.reconnect_routes[1].address = routes[0].address.clone();
    assert!(
        !has_saved_bridge_endpoints(&net, &expected),
        "wrong listener"
    );
    net.reconnect_routes = vec![routes[0].clone(), routes[0].clone()];
    assert!(
        !has_saved_bridge_endpoints(&net, &expected),
        "duplicate peer"
    );
    net.reconnect_routes = routes;
    assert!(has_saved_bridge_endpoints(&net, &expected));
    net.reconnect_policy = ReconnectPolicy::AuthorizedPeer(*expected[0].0.as_bytes());
    assert!(
        !has_saved_bridge_endpoints(&net, &expected),
        "singleton consent"
    );
}

async fn wait_for_bridge(scenario: &Scenario, bridge: usize) {
    let bridge_label = fingerprint(&scenario.clients[bridge].identity.as_ref().unwrap().1);
    tokio::select! {
        _ = async {
            loop {
                let left = scenario.online(1).actor.online_members().await;
                let right = scenario.online(2).actor.online_members().await;
                if left == [bridge_label.clone()] && right == [bridge_label.clone()] {
                    break;
                }
                SystemClock.sleep(Duration::from_millis(100)).await;
            }
        } => {},
        _ = SystemClock.sleep(Duration::from_millis(PERIODIC_RECOVERY_MS)) => {
            let actual = bridge_diagnostic(scenario, bridge).await;
            panic!("only voice bridge index={bridge} must be online within {PERIODIC_RECOVERY_MS}ms: {actual:?}");
        }
    }
}

async fn observe_legacy_gap(scenario: &Scenario, bridge: usize) {
    wait_for_bridge(scenario, bridge).await;
    let target_device = scenario.clients[2].identity.as_ref().unwrap().1;
    // The online roster proves the live bridge, not discovery of the blocked target's signed
    // descriptor. Wait for the real PEX path before asserting the narrower local-submission gap;
    // never inject a descriptor or open the forbidden A-C connection to prepare this baseline.
    bounded(
        "blocked endpoint descriptor reaches source through normal discovery",
        async {
            let mut passes = 0;
            loop {
                if scenario
                    .online(1)
                    .actor
                    .member_transport_peer(target_device)
                    .await
                    == Some(scenario.rules.peers[2])
                {
                    break;
                }
                if passes % 10 == 0 {
                    eprintln!("voice descriptor readiness: bridge={} helper_has_target={} source_has_target=false source_passes={} helper_passes={}",
                        bridge,
                        scenario.online(bridge).actor.member_transport_peer(target_device).await == Some(scenario.rules.peers[2]),
                        scenario.online(1).passes.load(Ordering::Acquire),
                        scenario.online(bridge).passes.load(Ordering::Acquire));
                }
                // Drive only the requesting side. Enqueuing reciprocal discovery commands at
                // both actors here aligns their RPC waits and tests a separate scheduling issue.
                scenario.online(1).actor.drive_discovery().await.unwrap();
                passes += 1;
                SystemClock.sleep(Duration::from_millis(100)).await;
            }
        },
    )
    .await;
    let author = scenario.clients[1].identity.as_ref().unwrap().1.to_string();
    let target = scenario.clients[2].identity.as_ref().unwrap().1.to_string();
    let helper = scenario.clients[bridge]
        .identity
        .as_ref()
        .unwrap()
        .1
        .to_string();
    let payload = format!(
        "{{\"callId\":\"{}\",\"type\":\"voice-ping\"}}",
        channel_id("general")
    )
    .into_bytes();
    let start = SystemClock.monotonic_ms();
    let mut submissions = 0;
    // The real App repeats this best-effort heartbeat every five seconds. A local queue ACK
    // cannot promise that a single packet survived a connection race. Exercise that same
    // retry contract, with fresh native authentication and one original deadline covering
    // every send/wait. Never retry arbitrary legacy offers/ICE on the strength of this test.
    let receipt = tokio::select! {
        received = async {
            for _ in 0..6 {
                submissions += 1;
                assert!(scenario.online(1).actor
                    .send_call_signal(helper.clone(), payload.clone()).await.unwrap());
                let next_send = SystemClock.monotonic_ms().saturating_add(5_000);
                loop {
                    if scenario.online(bridge).call_signals.lock().unwrap().iter()
                        .any(|row| row.0 == author && row.1 == payload) {
                        return true;
                    }
                    let remaining = next_send.saturating_sub(SystemClock.monotonic_ms());
                    if remaining == 0 {
                        break;
                    }
                    SystemClock.sleep(Duration::from_millis(remaining.min(20))).await;
                }
            }
            false
        } => received,
        _ = SystemClock.sleep(Duration::from_secs(30)) => false,
    };
    if !receipt {
        // These bounded fixture maps distinguish local queueing/injected loss from actual
        // receiver ingress. They contain neither identity seeds, routes nor payload bytes.
        let trace = scenario.rules.trace.lock().unwrap();
        eprintln!("legacy voice receipt missing: bridge={bridge} attempts={submissions} dropped={:?} queued={:?} received_kinds={:?} connected_submissions={:?}",
            trace.dropped_call_signals, trace.submitted_call_signals,
            trace.received_kinds, trace.connected_submissions);
        drop(trace);
        panic!("connected bridge did not receive the original authenticated legacy voice identity");
    }
    assert!(
        (2..=6).contains(&submissions),
        "receipt must follow a fresh send after the first deliberately lost heartbeat"
    );
    let bridge_receipt_ms = SystemClock.monotonic_ms().saturating_sub(start);

    // A true result is local submission to the network worker. The blocked physical A-C edge
    // still cannot deliver, and B has no application forwarding path for this legacy kind.
    let submitted = bounded(
        "submit legacy signal addressed to the blocked endpoint",
        scenario
            .online(1)
            .actor
            .send_call_signal(target, payload.clone()),
    )
    .await
    .unwrap();
    assert!(
        submitted,
        "the baseline must reproduce accepted local submission"
    );
    let control_start = SystemClock.monotonic_ms();
    for _ in 0..10 {
        assert_eq!(
            bounded(
                "control during unavailable voice route",
                scenario.online(2).actor.member_count()
            )
            .await,
            6
        );
        SystemClock.sleep(Duration::from_millis(100)).await;
    }
    assert!(
        scenario.online(2).call_signals.lock().unwrap().is_empty(),
        "a legacy signal must not acquire an unauthenticated forwarding fallback"
    );
    scenario.rules.assert_graph();
    eprintln!(
        "voice baseline: bridge={} direct_submitted={} heartbeat_submissions={} bridge_receipt_ms={} control_window_ms={} payload_bytes={} endpoint_receipts=0",
        bridge,
        submitted,
        submissions,
        bridge_receipt_ms,
        SystemClock.monotonic_ms().saturating_sub(control_start),
        payload.len(),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_voice_signals_have_no_member_bridge_fallback_on_tcp() {
    let mut scenario = Scenario::new(false, isolate().await);
    scenario.admit().await;
    let original_endpoints = wait_for_saved_bridge_endpoints(&scenario).await;
    // Native fixture blocks endpoint 1--2 in the actual swarm throughout admission/restart.
    // B=0 and D=3 were originally admitted authorized members; neither joins a frontend call.
    scenario.close(&[3, 4, 5]).await;
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    scenario
        .rules
        .drop_next_call_signal
        .store(true, Ordering::Release);
    observe_legacy_gap(&scenario, 0).await;
    scenario.close(&[0]).await;
    scenario.reopen(&[3]).await;
    let reopened = bounded(
        "load replacement bridge's sealed original routes after reopen",
        network(&scenario.online(3).state),
    )
    .await;
    assert!(
        has_saved_bridge_endpoints(&reopened, &original_endpoints),
        "both original saved directions must survive the actual native close/reopen"
    );
    scenario
        .rules
        .drop_next_call_signal
        .store(true, Ordering::Release);
    observe_legacy_gap(&scenario, 3).await;
    scenario.close(&[1, 2, 3]).await;
    scenario.rules.assert_graph();
    assert_eq!(
        scenario.rules.trace.lock().unwrap().dropped_call_signals,
        BTreeMap::from([((1, 0), 1), ((1, 3), 1)]),
        "each phase must exercise heartbeat recovery from exactly one injected loss"
    );
}
