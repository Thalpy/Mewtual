//! Normal native V2 ownership/discovery over real blocked TCP edges. No private watcher or
//! pair state is injected. This test proves signalling custody, not audible WebRTC media.
use super::*;
use crate::voice_authority::SelectedHandoffOwner;
use catcoms_sync::voice_v2::presence::PairSelector;
use catcoms_sync::voice_v2::{Body, CallGrant};

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
