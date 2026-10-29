//! Actual authenticated TCP capacity Control. Replies prove native custody only;
//! no browser claim, ledger slot, signed Ready, SDP or microphone is fabricated.
use super::*;

fn request(version: u8, mode: u8, generation: u64) -> [u8; 10] {
    let mut bytes = [0; 10];
    bytes[0] = version;
    bytes[1] = mode;
    bytes[2..].copy_from_slice(&generation.to_be_bytes());
    bytes
}

async fn call(
    port: u16,
    client: &Client,
    sequence: &mut u64,
    mode: u8,
    generation: u64,
) -> Vec<u8> {
    *sequence += 1;
    let (header, admission, cipher) = client.packet(
        *sequence,
        30,
        0,
        client.session,
        &request(11, mode, generation),
    );
    let response = exchange(port, &wire(port, &admission, &cipher), false).await;
    let opened = client.action_response(&response, &header, 0);
    assert_eq!(opened.len(), 59);
    assert_eq!(opened[0], 11);
    assert_eq!(&opened[2..10], &generation.to_be_bytes());
    opened
}

async fn pin_and_ack(port: u16, client: &Client, sequence: &mut u64) {
    *sequence += 1;
    let (header, admission, cipher) =
        client.packet(*sequence, 29, 0, client.session, &request(10, 0, 1));
    let response = exchange(port, &wire(port, &admission, &cipher), false).await;
    assert_eq!(
        &client.action_response(&response, &header, 0)[..2],
        &[10, 1]
    );
    // Capability support must be observable BEFORE ACK without minting a Root
    // registration, task, capacity or ordinal. Mode29-only native is not enough.
    let probe = call(port, client, sequence, 1, 1).await;
    assert_eq!(probe[1], 0);
    assert_eq!(&probe[10..], &[0; 49]);
    *sequence += 1;
    let (header, admission, cipher) = client.status(*sequence);
    let response = exchange(port, &wire(port, &admission, &cipher), false).await;
    assert_eq!(client.response(&response, &header).1, [1, 1, 1]);
}

async fn state(
    port: u16,
    client: &Client,
    sequence: &mut u64,
    generation: u64,
    wanted: u8,
) -> Vec<u8> {
    bounded(async {
        loop {
            let observed = call(port, client, sequence, 1, generation).await;
            if observed[1] == wanted
                && (wanted != 1 || observed[10..18] == generation.to_be_bytes())
            {
                break observed;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
}

#[tokio::test]
async fn real_tcp_capacity_control_fences_header_counters_and_recovers_original_canceled_generations(
) {
    let (f, owner, _public, runtime, client) = setup().await;
    let route = f.state.voice_channels.route(client.slot).unwrap();
    let bound = BoundServer::bind_application(
        f.state.voice_channels.clone(),
        runtime,
        Origin::PackagedHttp,
        super::super::super::context::Context::new(&f.state),
    )
    .unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    for (kind, logical, body) in [
        (1, client.session, request(11, 0, 1).to_vec()),
        (2, client.session, request(11, 0, 1).to_vec()),
        (0, [77; 16], request(11, 0, 1).to_vec()),
        (0, client.session, request(11, 0, 1)[..9].to_vec()),
        (
            0,
            client.session,
            [request(11, 0, 1).as_slice(), &[0]].concat(),
        ),
    ] {
        let (_, admission, cipher) = client.packet(1, 30, kind, logical, &body);
        assert!(exchange(port, &wire(port, &admission, &cipher), false)
            .await
            .is_empty());
        assert_eq!(route.session().fixture_counts(), (false, 0, 0));
    }
    // This is a valid operation30 header but an encrypted mode29 version. No
    // fallback is tried, and admitted nonces cannot be refunded on rejection.
    let (_, admission, cipher) = client.packet(1, 30, 0, client.session, &request(10, 0, 1));
    assert!(exchange(port, &wire(port, &admission, &cipher), false)
        .await
        .is_empty());
    assert_eq!(route.session().fixture_counts(), (false, 1, 1));
    let mut sequence = 1;
    pin_and_ack(port, &client, &mut sequence).await;
    let before = route.selected().resources().counts();
    let empty = call(port, &client, &mut sequence, 1, 1).await;
    assert_eq!(&empty[..2], &[11, 0]);
    assert_eq!(&empty[10..], &[0; 49]);
    assert_eq!(route.selected().resources().counts(), before);
    let parked = crate::voice_handoff::factory::tests::park(&f.actor).await;
    let admitted = call(port, &client, &mut sequence, 0, 1).await;
    assert_eq!(admitted[1], 1);
    let preparing = state(port, &client, &mut sequence, 1, 1).await;
    assert_eq!(preparing[26], 0);
    let canceled = call(port, &client, &mut sequence, 2, 1).await;
    drop(parked);
    assert_eq!(
        canceled[1], 4,
        "Root work cannot return while its actor is parked"
    );
    let returned = state(port, &client, &mut sequence, 1, 5).await;
    assert_eq!(
        returned[26], 1,
        "late mint is retained as the same stable registration"
    );
    let incarnation = returned[27..].to_vec();
    let _ = call(port, &client, &mut sequence, 0, 2).await;
    let reserved = state(port, &client, &mut sequence, 2, 3).await;
    assert_eq!(&reserved[10..18], &2_u64.to_be_bytes());
    assert_eq!(&reserved[27..], &incarnation);
    for mode in [0, 1, 2] {
        // Simulate a lost old reply while the successor is actually reserved.
        let old = call(port, &client, &mut sequence, mode, 1).await;
        assert_eq!(old[1], 8);
        assert_eq!(&old[10..18], &2_u64.to_be_bytes());
        assert_eq!(&old[18..26], &1_u64.to_be_bytes());
        assert_eq!(&old[27..], &incarnation);
    }
    crate::end_voice_owner_inner(&f.state, &owner).await;
    let closed = state(port, &client, &mut sequence, 2, 6).await;
    assert_eq!(&closed[26..], &[0; 33]);
    assert_eq!(
        route.session().fixture_counts(),
        (false, sequence, sequence)
    );
    drop(running.finish().await.unwrap());
    f.close().await;
}

#[tokio::test]
async fn real_tcp_capacity_observe_and_never_started_cancel_need_no_fresh_group_credit() {
    use catcoms_sync::voice_v2::resources::{ResourceKind, GROUP_BYTES};
    let (f, _owner, _public, runtime, client) = setup().await;
    let route = f.state.voice_channels.route(client.slot).unwrap();
    let bound = BoundServer::bind_application(
        f.state.voice_channels.clone(),
        runtime,
        Origin::PackagedHttp,
        super::super::super::context::Context::new(&f.state),
    )
    .unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    let mut sequence = 0;
    pin_and_ack(port, &client, &mut sequence).await;
    let resources = route.selected().resources();
    let filler = resources
        .charge(
            route.selected().binding().scope.group,
            GROUP_BYTES
                - resources
                    .group_counts(route.selected().binding().scope.group)
                    .unwrap()
                    .0,
            ResourceKind::Retained,
        )
        .unwrap();
    let full = resources.counts();
    for mode in [0, 1] {
        let snapshot = call(port, &client, &mut sequence, mode, 1).await;
        assert_eq!(snapshot[1], 0);
        assert_eq!(&snapshot[10..], &[0; 49]);
    }
    let canceled = call(port, &client, &mut sequence, 2, 1).await;
    assert_eq!(canceled[1], 5);
    assert_eq!(&canceled[10..18], &1_u64.to_be_bytes());
    assert_eq!(&canceled[18..26], &1_u64.to_be_bytes());
    assert_eq!(&canceled[26..], &[0; 33]);
    assert_eq!(resources.counts(), full);
    drop(filler);
    drop(running.finish().await.unwrap());
    f.close().await;
}
