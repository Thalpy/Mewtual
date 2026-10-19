//! Communication Package 0: pin the legacy direct-only signalling gap on actual TCP actors.
//! This is a reproduced limitation, not acceptance of bridged room discovery or audible voice.
use super::*;

async fn wait_for_bridge(scenario: &Scenario, bridge: usize) {
    let bridge_label = fingerprint(&scenario.clients[bridge].identity.as_ref().unwrap().1);
    periodic_bounded("only the permitted voice bridge is online", async {
        loop {
            let left = scenario.online(1).actor.online_members().await;
            let right = scenario.online(2).actor.online_members().await;
            if left == [bridge_label.clone()] && right == [bridge_label.clone()] {
                break;
            }
            SystemClock.sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
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
    let start = SystemClock.now_ms();
    assert!(bounded(
        "submit legacy call signal to connected bridge",
        scenario
            .online(1)
            .actor
            .send_call_signal(helper, payload.clone()),
    )
    .await
    .unwrap());
    bounded(
        "connected bridge receives the original authenticated identity",
        async {
            loop {
                if scenario
                    .online(bridge)
                    .call_signals
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|row| row.0 == author && row.1 == payload)
                {
                    break;
                }
                SystemClock.sleep(Duration::from_millis(20)).await;
            }
        },
    )
    .await;
    let bridge_receipt_ms = SystemClock.now_ms().saturating_sub(start);

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
    let control_start = SystemClock.now_ms();
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
        "voice baseline: bridge={} direct_submitted={} bridge_receipt_ms={} control_window_ms={} payload_bytes={} endpoint_receipts=0",
        bridge,
        submitted,
        bridge_receipt_ms,
        SystemClock.now_ms().saturating_sub(control_start),
        payload.len(),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_voice_signals_have_no_member_bridge_fallback_on_tcp() {
    let mut scenario = Scenario::new(false);
    scenario.admit().await;
    // Native fixture blocks endpoint 1--2 in the actual swarm throughout admission/restart.
    // B=0 and D=3 were originally admitted authorized members; neither joins a frontend call.
    scenario.close(&[3, 4, 5]).await;
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    observe_legacy_gap(&scenario, 0).await;
    scenario.close(&[0]).await;
    scenario.reopen(&[3]).await;
    observe_legacy_gap(&scenario, 3).await;
    scenario.close(&[1, 2, 3]).await;
    scenario.rules.assert_graph();
}
