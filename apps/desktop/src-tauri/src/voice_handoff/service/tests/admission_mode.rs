//! Actual TCP/MAC/AEAD/dispatch checks for the mode-only boundary. No native Root
//! admission or microphone/RTC success is inferred from these protocol-selection replies.
use super::*;

fn body(mode: u8, generation: u64) -> [u8; 10] {
    let mut bytes = [10, mode, 0, 0, 0, 0, 0, 0, 0, 0];
    bytes[2..].copy_from_slice(&generation.to_be_bytes());
    bytes
}

async fn control(port: u16, client: &Client, sequence: u64, mode: u8) -> Vec<u8> {
    let (header, admission, cipher) =
        client.packet(sequence, 29, 0, client.session, &body(mode, 1));
    let response = exchange(port, &wire(port, &admission, &cipher), false).await;
    client.action_response(&response, &header, 0)
}

#[tokio::test]
async fn real_tcp_admission_mode_authenticates_before_counters_and_recovers_the_same_pin() {
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
    let before = route.selected().resources().counts();
    // These carry an authentic MAC from the exact original client. Rejection is
    // the actual header grammar/session/class gate, not a bad-MAC shortcut.
    for (kind, logical, plain) in [
        (1, client.session, body(0, 1).to_vec()),
        (2, client.session, body(0, 1).to_vec()),
        (0, [77; 16], body(0, 1).to_vec()),
        (0, client.session, body(0, 1)[..9].to_vec()),
        (0, client.session, [body(0, 1).as_slice(), &[0]].concat()),
    ] {
        let (_, admission, cipher) = client.packet(1, 29, kind, logical, &plain);
        assert!(exchange(port, &wire(port, &admission, &cipher), false)
            .await
            .is_empty());
        assert_eq!(route.session().fixture_counts(), (false, 0, 0));
    }
    let (_, admission, cipher) = client.packet(1, 29, 0, client.session, &body(0, 1));
    let mut invalid_mac = admission.into_bytes();
    let last = invalid_mac.last_mut().unwrap();
    *last = if *last == b'0' { b'1' } else { b'0' };
    assert!(exchange(
        port,
        &wire(port, std::str::from_utf8(&invalid_mac).unwrap(), &cipher),
        false
    )
    .await
    .is_empty());
    assert_eq!(route.session().fixture_counts(), (false, 0, 0));
    // Exact header + bad encrypted version DOES spend both its admitted request
    // and reserved response nonce, even though no reply is sealed. Neither is
    // refunded on malformed plaintext. There is no old-version decoder fallback.
    let mut invalid = body(0, 1);
    invalid[0] = 9;
    let (_, admission, cipher) = client.packet(1, 29, 0, client.session, &invalid);
    assert!(exchange(port, &wire(port, &admission, &cipher), false)
        .await
        .is_empty());
    assert_eq!(route.session().fixture_counts(), (false, 1, 1));
    let observed = control(port, &client, 2, 1).await;
    assert_eq!(
        observed,
        [10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    let pinned = control(port, &client, 3, 0).await;
    assert_eq!(pinned.len(), 18);
    assert_eq!(&pinned[..2], &[10, 1]);
    assert_eq!(&pinned[2..10], &1u64.to_be_bytes());
    assert_eq!(&pinned[10..], &0u64.to_be_bytes());
    // Simulate losing the reply after native publication: exact content recovers
    // under a fresh physical request sequence, with no second registration/factory.
    assert_eq!(control(port, &client, 4, 0).await, pinned);
    assert_eq!(route.selected().resources().counts(), before);
    let (header, admission, cipher) = client.status(5);
    let response = exchange(port, &wire(port, &admission, &cipher), false).await;
    assert_eq!(client.response(&response, &header).1, [1, 1, 1]);
    assert!(!route.native_ready());
    assert!(route.require_pilot_open().is_err());
    let cancelled = control(port, &client, 6, 2).await;
    assert_eq!(&cancelled[..2], &[10, 2]);
    assert_eq!(&cancelled[10..], &1u64.to_be_bytes());
    assert_eq!(control(port, &client, 7, 0).await, cancelled);
    crate::end_voice_owner_inner(&f.state, &owner).await;
    let closed = control(port, &client, 8, 1).await;
    assert_eq!(&closed[..2], &[10, 3]);
    assert_eq!(&closed[2..], &cancelled[2..]);
    assert_eq!(route.session().fixture_counts(), (false, 8, 8));
    drop(running.finish().await.unwrap());
    f.close().await;
}
