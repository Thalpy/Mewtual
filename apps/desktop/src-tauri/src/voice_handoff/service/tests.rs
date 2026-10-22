//! Real TCP checks plus deterministic parked-IO ownership tests. Known scalar one is a public
//! test key, not a production key export. The independent client codec uses the shipping wire
//! layout already pinned by shared WebCrypto vectors; no native Session key is inspected.
use super::*;
use crate::voice_activation_tests::Fixture;
use crate::voice_handoff::bootstrap::{ensure, SetupOutput, Status};
use aes_gcm::{
    aead::{AeadInPlace, KeyInit},
    Aes256Gcm,
};
use catcoms_rt::ManualClock;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use p256::{PublicKey, SecretKey};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use sha2::Sha256;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const POINT: &str = "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
const DOMAIN: &[u8] = b"Mewtual native voice handoff v1";

// Wall time only bounds a broken test harness. All service/protocol time comes from Clock.
async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(15), future)
        .await
        .expect("status harness stalled")
}
async fn until(mut condition: impl FnMut() -> bool) {
    bounded(async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await;
}
async fn setup() -> (Fixture, String, Arc<SetupOutput>, Arc<Runtime>, Client) {
    let f = Fixture::new().await;
    let owner = f.candidate().await;
    crate::activate_voice_owner_inner(&f.state, owner.clone())
        .await
        .unwrap();
    let executor = hex::encode([9; 32]);
    let admitted = crate::voice_join_key::admit_inner(&f.state, &owner, &executor)
        .await
        .unwrap();
    crate::voice_join_key::publish_inner(
        &f.state,
        &owner,
        &executor,
        &admitted.permit,
        hex::decode(POINT).unwrap().try_into().unwrap(),
    )
    .await
    .unwrap();
    crate::voice_join_key::settle_inner(&f.state, &owner, [9; 32])
        .await
        .unwrap();
    let Status::Ready(public) = ensure(
        &f.state,
        &owner,
        [9; 32],
        &mut ChaCha20Rng::seed_from_u64(83),
    )
    .await
    .unwrap() else {
        panic!("setup not ready")
    };
    let runtime = f.state.voice_authority.lock().unwrap().runtime();
    let client = Client::new(public.public());
    (f, owner, public, runtime, client)
}

struct Client {
    seed: [u8; 32],
    slot: u8,
    session: [u8; 16],
    hash: [u8; 32],
}

/// Actual native bootstrap + independent client MAC/AEAD for sibling executor regressions.
/// Fixture allocations are not application custody evidence; returned Incoming is the real
/// original Session owner and has already passed header authentication and body opening.
pub(in crate::voice_handoff) fn opened_frame(
    public: &SetupOutput,
    route: &super::super::bootstrap::Route,
    header: (u64, u8, u8, [u8; 16]),
    plaintext: &[u8],
    phase: Option<&Arc<super::super::custody::Phase>>,
) -> Incoming {
    let client = Client::new(public.public());
    let (_, admission, body) = client.packet(header.0, header.1, header.2, header.3, plaintext);
    let submitted = wire(41891, &admission, &body);
    let mut worker = route.session().header().unwrap();
    worker.scratch_mut()[..submitted.len()].copy_from_slice(&submitted);
    worker.set_filled(submitted.len()).unwrap();
    let mut incoming = worker
        .admit(41891, Origin::PackagedHttp.value(), phase)
        .unwrap();
    assert_eq!(incoming.open().unwrap(), plaintext);
    incoming
}
impl Client {
    fn new(public: &super::super::channel::bootstrap::PublicSetup) -> Self {
        let mut scalar = [0; 32];
        scalar[31] = 1;
        let key = SecretKey::from_slice(&scalar).unwrap();
        let transcript = public.transcript();
        let native = PublicKey::from_sec1_bytes(&transcript[transcript.len() - 65..]).unwrap();
        let shared = p256::ecdh::diffie_hellman(key.to_nonzero_scalar(), native.as_affine());
        let mut seed = [0; 32];
        Hkdf::<Sha256>::new(Some(&public.hash), shared.raw_secret_bytes())
            .expand(b"Mewtual native voice bootstrap seed v1", &mut seed)
            .unwrap();
        Self {
            seed,
            slot: public.slot,
            session: public.session,
            hash: public.hash,
        }
    }
    fn key(&self, label: &[u8]) -> [u8; 32] {
        let mut result = [0; 32];
        Hkdf::<Sha256>::new(Some(DOMAIN), &self.seed)
            .expand(label, &mut result)
            .unwrap();
        result
    }
    fn plain(&self) -> Vec<u8> {
        [
            b"\x01".as_slice(),
            self.session.as_slice(),
            self.hash.as_slice(),
        ]
        .concat()
    }
    fn packet(
        &self,
        sequence: u64,
        operation: u8,
        kind: u8,
        logical: [u8; 16],
        plain: &[u8],
    ) -> ([u8; 40], String, Vec<u8>) {
        let mut header = [0; 40];
        header[..8].copy_from_slice(&[b'M', b'W', b'V', b'H', 1, 0, operation, self.slot]);
        header[8..16].copy_from_slice(&sequence.to_be_bytes());
        header[16..32].copy_from_slice(&logical);
        header[32..36].copy_from_slice(&((plain.len() + 16) as u32).to_be_bytes());
        header[36] = kind;
        let mut nonce = [0; 12];
        nonce[..4].copy_from_slice(b"VHQ1");
        nonce[4..].copy_from_slice(&header[8..16]);
        let mut body = plain.to_vec();
        let tag = Aes256Gcm::new_from_slice(&self.key(b"v1 c2n body AES-256-GCM"))
            .unwrap()
            .encrypt_in_place_detached((&nonce).into(), &header, &mut body)
            .unwrap();
        body.extend_from_slice(&tag);
        let mut mac =
            <Hmac<Sha256> as Mac>::new_from_slice(&self.key(b"v1 c2n header HMAC-SHA256")).unwrap();
        mac.update(&header);
        let admission =
            hex::encode([header.as_slice(), mac.finalize().into_bytes().as_slice()].concat());
        (header, admission, body)
    }
    fn status(&self, sequence: u64) -> ([u8; 40], String, Vec<u8>) {
        self.packet(sequence, 9, 0, self.session, &self.plain())
    }
    fn response(&self, bytes: &[u8], request: &[u8; 40]) -> (u64, [u8; 3]) {
        let end = bytes.windows(4).position(|x| x == b"\r\n\r\n").unwrap() + 4;
        let text = std::str::from_utf8(&bytes[..end]).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Cache-Control: no-store\r\n"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(text.contains("Content-Length: 163\r\n"));
        let admission = std::str::from_utf8(&bytes[end..end + 144]).unwrap();
        let decoded = hex::decode(admission).unwrap();
        assert_eq!(decoded.len(), 72);
        let response = &decoded[..40];
        assert_eq!(
            &response[..8],
            &[b'M', b'W', b'V', b'H', 1, 1, 9, self.slot]
        );
        assert_eq!(&response[16..32], &self.session);
        assert_eq!(&response[32..], &[0, 0, 0, 19, 0, 0, 0, 0]);
        let aad = [response, request.as_slice()].concat();
        let mut mac =
            <Hmac<Sha256> as Mac>::new_from_slice(&self.key(b"v1 n2c header HMAC-SHA256")).unwrap();
        mac.update(&aad);
        mac.verify_slice(&decoded[40..]).unwrap();
        let mut nonce = [0; 12];
        nonce[..4].copy_from_slice(b"VHP1");
        nonce[4..].copy_from_slice(&response[8..16]);
        let mut body = bytes[end + 144..].to_vec();
        assert_eq!(body.len(), 19);
        let tag: [u8; 16] = body[3..].try_into().unwrap();
        Aes256Gcm::new_from_slice(&self.key(b"v1 n2c body AES-256-GCM"))
            .unwrap()
            .decrypt_in_place_detached((&nonce).into(), &aad, &mut body[..3], (&tag).into())
            .unwrap();
        (
            u64::from_be_bytes(response[8..16].try_into().unwrap()),
            body[..3].try_into().unwrap(),
        )
    }
    fn expected_response(&self, request: &[u8; 40], sequence: u64, snapshot: [u8; 3]) -> Vec<u8> {
        let mut header = *request;
        header[5] = 1;
        header[8..16].copy_from_slice(&sequence.to_be_bytes());
        header[32..36].copy_from_slice(&19u32.to_be_bytes());
        let aad = [header.as_slice(), request.as_slice()].concat();
        let mut nonce = [0; 12];
        nonce[..4].copy_from_slice(b"VHP1");
        nonce[4..].copy_from_slice(&header[8..16]);
        let mut body = snapshot.to_vec();
        let tag = Aes256Gcm::new_from_slice(&self.key(b"v1 n2c body AES-256-GCM"))
            .unwrap()
            .encrypt_in_place_detached((&nonce).into(), &aad, &mut body)
            .unwrap();
        body.extend_from_slice(&tag);
        let mut mac =
            <Hmac<Sha256> as Mac>::new_from_slice(&self.key(b"v1 n2c header HMAC-SHA256")).unwrap();
        mac.update(&aad);
        let admission: [u8; 144] =
            hex::encode([header.as_slice(), mac.finalize().into_bytes().as_slice()].concat())
                .into_bytes()
                .try_into()
                .unwrap();
        let mut output = Output::new();
        output.reply(Origin::PackagedHttp, body.len()).unwrap();
        [output.bytes(), admission.as_slice(), body.as_slice()].concat()
    }
}
fn wire(port: u16, admission: &str, body: &[u8]) -> Vec<u8> {
    let mut wire = format!("POST /voice/v2 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://tauri.localhost\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nx-voice-admission: {admission}\r\n\r\n", body.len()).into_bytes();
    wire.extend_from_slice(body);
    wire
}
async fn exchange(port: u16, wire: &[u8], eof: bool) -> Vec<u8> {
    bounded(async {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        stream.write_all(wire).await.unwrap();
        if eof {
            stream.shutdown().await.unwrap();
        }
        let mut received = Vec::new();
        if let Err(error) = (&mut stream).take(1024).read_to_end(&mut received).await {
            assert!(matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ));
        }
        received
    })
    .await
}
fn empty(clock: Arc<dyn Clock>) -> (Arc<Registry>, Arc<Runtime>) {
    (Arc::new(Registry::default()), Runtime::new(clock))
}

#[tokio::test]
async fn real_tcp_status_needs_no_request_eof_and_closed_history_cannot_revive_a_call() {
    let (f, owner, _public, runtime, client) = setup().await;
    let before = runtime.resources().counts();
    let bound = BoundServer::bind(
        f.state.voice_channels.clone(),
        runtime.clone(),
        Origin::PackagedHttp,
    )
    .unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    assert_eq!(
        runtime.resources().counts().0,
        before.0 + INFRASTRUCTURE_BYTES
    );
    let (header, admission, body) = client.status(1);
    let reply = exchange(port, &wire(port, &admission, &body), false).await;
    assert_eq!(client.response(&reply, &header), (1, [1, 1, 1]));
    f.state.voice_authority.lock().unwrap().end(&owner);
    let (header, admission, body) = client.status(2);
    let reply = exchange(port, &wire(port, &admission, &body), false).await;
    assert_eq!(client.response(&reply, &header), (2, [1, 2, 1]));
    let charged = runtime.resources().counts().0;
    let completed = running.finish().await.unwrap();
    assert!(!completed.io_failed);
    assert_eq!(runtime.resources().counts().0, charged);
    drop(completed);
    assert_eq!(
        runtime.resources().counts().0,
        charged - INFRASTRUCTURE_BYTES
    );
    f.close().await;
}

#[tokio::test]
async fn prepaid_status_makes_progress_with_every_crypto_slot_and_process_byte_occupied() {
    use catcoms_sync::voice_v2::resources::ResourceKind;
    let (f, owner, _public, runtime, client) = setup().await;
    let resources = runtime.resources();
    let group = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .owner(&owner)
        .unwrap()
        .binding
        .scope
        .group;
    let bound = BoundServer::bind(
        f.state.voice_channels.clone(),
        runtime,
        Origin::PackagedHttp,
    )
    .unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    let mut lanes = Vec::new();
    for _ in 0..2 {
        lanes.push(resources.reserve_crypto_lane(group).unwrap());
    }
    for _ in 0..4 {
        lanes.push(resources.reserve_crypto_lane([97; 32]).unwrap());
    }
    assert_eq!(resources.counts().1, 8);
    assert!(resources.reserve_crypto_lane([98; 32]).is_err());
    // This isolated fixture has one group before pressure, plus the separately measured
    // 4KiB process index and production infrastructure envelope. Fill its remaining bytes,
    // then the other bounded test groups, without increasing any configured limit.
    let group_used = resources.counts().0 - 4096 - INFRASTRUCTURE_BYTES;
    let mut pressure = vec![resources
        .charge(group, 256 * 1024 - group_used, ResourceKind::Retained)
        .unwrap()];
    for value in 97..=104 {
        let left = 2 * 1024 * 1024 - resources.counts().0;
        if left == 0 {
            break;
        }
        pressure.push(
            resources
                .charge([value; 32], left.min(256 * 1024), ResourceKind::Retained)
                .unwrap(),
        );
    }
    assert_eq!(resources.counts(), (2 * 1024 * 1024, 8));
    let (header, admission, body) = client.status(1);
    let reply = exchange(port, &wire(port, &admission, &body), false).await;
    assert_eq!(client.response(&reply, &header), (1, [1, 1, 1]));
    assert_eq!(
        resources.counts().1,
        7,
        "only completed setup retires; original control slot survives"
    );
    let refill = resources.reserve_crypto_lane(group).unwrap();
    assert_eq!(resources.counts().1, 8);
    f.state.voice_authority.lock().unwrap().end(&owner);
    let (header, admission, body) = client.status(2);
    let reply = exchange(port, &wire(port, &admission, &body), false).await;
    assert_eq!(client.response(&reply, &header), (2, [1, 2, 1]));
    assert_eq!(
        resources.counts().1,
        8,
        "historical status obtains no fresh work slot"
    );
    running.finish().await.unwrap();
    refill.close();
    for lane in lanes {
        lane.close();
    }
    drop(pressure);
    f.close().await;
}

#[tokio::test]
async fn authenticated_wrong_operation_class_length_and_logical_id_refuse_before_debit() {
    let (f, _, _public, runtime, client) = setup().await;
    let bound = BoundServer::bind(
        f.state.voice_channels.clone(),
        runtime,
        Origin::PackagedHttp,
    )
    .unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    for (operation, kind, logical, plain) in [
        (1, 0, client.session, client.plain()),
        (9, 1, client.session, client.plain()),
        (9, 0, [4; 16], client.plain()),
        (9, 0, client.session, vec![1; 48]),
    ] {
        let (_, admission, body) = client.packet(1, operation, kind, logical, &plain);
        assert!(exchange(port, &wire(port, &admission, &body), false)
            .await
            .is_empty());
        assert_eq!(
            f.state
                .voice_channels
                .route(client.slot)
                .unwrap()
                .session()
                .fixture_counts(),
            (false, 0, 0)
        );
    }
    let (header, admission, body) = client.status(1);
    assert_eq!(
        client
            .response(
                &exchange(port, &wire(port, &admission, &body), false).await,
                &header
            )
            .0,
        1
    );
    running.finish().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn admitted_bad_plaintext_bad_tag_truncation_and_lost_response_never_refund_counters() {
    let (f, _, _public, runtime, client) = setup().await;
    let bound = BoundServer::bind(
        f.state.voice_channels.clone(),
        runtime.clone(),
        Origin::PackagedHttp,
    )
    .unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    for sequence in 1..=5 {
        let mut plain = client.plain();
        if sequence == 1 {
            plain[0] = 2;
        } else if sequence == 4 {
            plain[1] ^= 1; // Valid AEAD, wrong body Session.
        } else if sequence == 5 {
            plain[17] ^= 1; // Valid AEAD, wrong immutable transcript hash.
        }
        let (_, admission, mut body) = client.packet(sequence, 9, 0, client.session, &plain);
        if sequence == 2 {
            body[0] ^= 1;
        }
        let mut submitted = wire(port, &admission, &body);
        if sequence == 3 {
            submitted.pop();
        }
        assert!(exchange(port, &submitted, true).await.is_empty());
        assert_eq!(
            runtime.resources().counts().1,
            2,
            "invalid/truncated bodies do not settle browser setup"
        );
        let (_, admission, body) = client.status(sequence);
        assert!(exchange(port, &wire(port, &admission, &body), false)
            .await
            .is_empty());
        assert_eq!(
            f.state
                .voice_channels
                .route(client.slot)
                .unwrap()
                .session()
                .fixture_counts(),
            (false, sequence, sequence)
        );
    }
    let (header, admission, body) = client.status(6);
    let reply = exchange(port, &wire(port, &admission, &body), false).await;
    assert_eq!(client.response(&reply, &header).0, 6);
    assert_eq!(
        runtime.resources().counts().1,
        1,
        "only the complete authenticated Status settles setup, preserving control capacity"
    );
    // Throw away that valid response. Repeating the original request is still a replay.
    assert!(exchange(port, &wire(port, &admission, &body), false)
        .await
        .is_empty());
    running.finish().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn origins_unknown_slots_and_oversize_refuse_before_admission() {
    let (f, _, _public, runtime, client) = setup().await;
    let bound = BoundServer::bind(
        f.state.voice_channels.clone(),
        runtime,
        Origin::PackagedHttp,
    )
    .unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    let (header, admission, body) = client.status(1);
    let valid = wire(port, &admission, &body);
    let mut unknown = admission.clone();
    unknown.replace_range(14..16, "40");
    let bad_origin = String::from_utf8(valid[..valid.len() - body.len()].to_vec())
        .unwrap()
        .replace(
            "Origin: http://tauri.localhost",
            "Origin: https://attacker.invalid",
        );
    for invalid in [
        [bad_origin.as_bytes(), body.as_slice()].concat(),
        wire(port, &unknown, &body),
        vec![b'a'; http::HEADER_BYTES],
    ] {
        assert!(exchange(port, &invalid, false).await.is_empty());
    }
    assert_eq!(
        f.state
            .voice_channels
            .route(client.slot)
            .unwrap()
            .session()
            .fixture_counts(),
        (false, 0, 0)
    );
    assert_eq!(
        client
            .response(&exchange(port, &valid, false).await, &header)
            .1,
        [1, 1, 1]
    );
    running.finish().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn real_tcp_pipeline_can_never_admit_or_answer_the_second_request() {
    let (f, _, _public, runtime, client) = setup().await;
    let bound = BoundServer::bind(
        f.state.voice_channels.clone(),
        runtime,
        Origin::PackagedHttp,
    )
    .unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    let (first, admission, body) = client.status(1);
    let mut submitted = wire(port, &admission, &body);
    let (_, admission, body) = client.status(2);
    submitted.extend_from_slice(&wire(port, &admission, &body));
    let received = exchange(port, &submitted, false).await;
    // TCP may present both requests in one read (refuse), or exactly the first one
    // (answer once and close). A write call is not a transport message boundary.
    let route = f.state.voice_channels.route(client.slot).unwrap();
    until(|| !route.session().fixture_counts().0).await;
    let counters = route.session().fixture_counts();
    assert!(matches!(counters, (false, 0, 0) | (false, 1, 1)));
    // Closing with unread pipelined bytes can reset/truncate the first reply. Delivery loss
    // says nothing about admission: inspect the actual counters independently. Whatever
    // arrived must be a prefix of the ONE permitted authenticated response, never a second.
    let expected = client.expected_response(&first, 1, [1, 1, 1]);
    assert!(expected.starts_with(&received));
    if received.len() == expected.len() {
        assert_eq!(client.response(&received, &first), (1, [1, 1, 1]));
    }
    if counters.1 == 0 {
        assert!(received.is_empty());
    }
    running.finish().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn deterministically_coalesced_excess_refuses_before_either_counter_is_spent() {
    let (f, _, _public, runtime, client) = setup().await;
    let _credit = runtime
        .resources()
        .charge_process_retained(INFRASTRUCTURE_BYTES)
        .unwrap();
    let (_, admission, body) = client.status(1);
    let mut submitted = wire(41891, &admission, &body);
    let (_, admission, body) = client.status(2);
    submitted.extend_from_slice(&wire(41891, &admission, &body));
    assert!(submitted.len() < http::HEADER_BYTES);
    let session = f
        .state
        .voice_channels
        .route(client.slot)
        .unwrap()
        .session()
        .clone();
    let dropped = Arc::new(AtomicBool::new(false));
    let stream = Parked {
        read: std::io::Cursor::new(submitted),
        at: usize::MAX,
        writes: 0,
        dropped: dropped.clone(),
        session: session.clone(),
        expected_busy_on_drop: false,
    };
    let mut connection = Connection::new(stream);
    assert_eq!(
        connection
            .serve(
                &f.state.voice_channels,
                41891,
                Origin::PackagedHttp,
                &ManualClock::new(1000),
                6000
            )
            .await,
        Err(Refused::Header)
    );
    assert_eq!(session.fixture_counts(), (false, 0, 0));
    drop(connection);
    assert!(dropped.load(Ordering::Acquire));
    f.close().await;
}

#[tokio::test]
async fn preflight_is_fixed_cors_without_allocating_a_group_or_advancing_a_session() {
    let (registry, runtime) = empty(Arc::new(ManualClock::new(1000)));
    let bound = BoundServer::bind(registry.clone(), runtime.clone(), Origin::PackagedHttp).unwrap();
    let before = runtime.resources().counts();
    let port = bound.port();
    let running = bound.start().unwrap();
    let wire = format!("OPTIONS /voice/v2 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://tauri.localhost\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: X-Voice-Admission, Content-Type\r\n\r\n");
    let reply = String::from_utf8(exchange(port, wire.as_bytes(), false).await).unwrap();
    assert!(reply.starts_with("HTTP/1.1 204 No Content\r\n"));
    assert!(reply.contains("Access-Control-Allow-Origin: http://tauri.localhost\r\n"));
    assert!(!reply.contains("Allow-Credentials"));
    assert!(!reply.contains("Content-Length:"));
    assert!(!reply.contains("x-voice-admission:"));
    assert!(reply.ends_with("\r\n\r\n"));
    assert_eq!(runtime.resources().counts(), before);
    assert!(registry.route(1).is_err());
    running.finish().await.unwrap();
}

#[test]
fn fixed_bound_and_wrong_pool_refusal_do_not_allocate_crypto_or_a_group() {
    let (registry, runtime) = empty(Arc::new(ManualClock::new(1000)));
    let resources = runtime.resources();
    let bound = BoundServer::bind(registry.clone(), runtime.clone(), Origin::PackagedHttp).unwrap();
    assert_ne!(bound.port(), 0);
    assert_eq!(resources.counts(), (4096 + INFRASTRUCTURE_BYTES, 0));
    drop(bound);
    assert_eq!(resources.counts(), (4096, 0));
    let other = Runtime::new(Arc::new(ManualClock::new(1000)));
    assert!(matches!(
        BoundServer::bind(registry.clone(), other.clone(), Origin::PackagedHttp),
        Err(Refused::Binding)
    ));
    assert_eq!(other.resources().counts(), (0, 0));
    let held = resources
        .charge_process_retained(2 * 1024 * 1024 - 4096 - INFRASTRUCTURE_BYTES + 1)
        .unwrap();
    // Same original pool, but the infrastructure payment must refuse BEFORE socket bind.
    assert!(matches!(
        BoundServer::bind(registry, runtime, Origin::PackagedHttp),
        Err(Refused::Capacity)
    ));
    drop(held);
}

#[test]
fn starting_outside_the_native_runtime_refuses_without_spawning_or_leaking_payment() {
    let (registry, runtime) = empty(Arc::new(ManualClock::new(1000)));
    let resources = runtime.resources();
    let bound = BoundServer::bind(registry, runtime, Origin::PackagedHttp).unwrap();
    assert!(matches!(bound.start(), Err(Refused::Io)));
    assert_eq!(resources.counts(), (0, 0));
}

#[tokio::test]
async fn completed_uncollected_and_aborted_idle_or_active_tasks_keep_their_process_payment() {
    for mode in 0..3 {
        // Graceful process shutdown now permanently closes its original Registry. Each
        // independent stop/abort ordering therefore needs its own actual process fixture;
        // never restore a stopped Registry just to exercise the next test mode.
        let (f, _, _public, runtime, client) = setup().await;
        let baseline = runtime.resources().counts().0;
        let bound = BoundServer::bind(
            f.state.voice_channels.clone(),
            runtime.clone(),
            Origin::PackagedHttp,
        )
        .unwrap();
        let port = bound.port();
        let mut running = bound.start().unwrap();
        until(|| running.phase.load(Ordering::Acquire) == 1).await;
        let mut stream = None;
        if mode == 2 {
            let (_, admission, body) = client.status(1);
            let mut wire = wire(port, &admission, &body);
            wire.pop();
            let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
            socket.write_all(&wire).await.unwrap();
            stream = Some(socket);
            until(|| {
                f.state
                    .voice_channels
                    .route(client.slot)
                    .unwrap()
                    .session()
                    .fixture_counts()
                    .1
                    == 1
            })
            .await;
        }
        if mode == 0 {
            running.stop();
        } else {
            running.task.as_ref().unwrap().abort();
        }
        until(|| running.task.as_ref().unwrap().is_finished()).await;
        assert_eq!(
            runtime.resources().counts().0,
            baseline + INFRASTRUCTURE_BYTES
        );
        if mode == 0 {
            let completed = running.finish().await.unwrap();
            assert_eq!(
                runtime.resources().counts().0,
                baseline + INFRASTRUCTURE_BYTES
            );
            drop(completed);
            assert!(
                matches!(
                    BoundServer::bind(
                        f.state.voice_channels.clone(),
                        runtime.clone(),
                        Origin::PackagedHttp
                    ),
                    Err(Refused::Binding)
                ),
                "the original stopped Registry cannot restart"
            );
        } else {
            assert!(matches!(running.finish().await, Err(Refused::Io)));
        }
        assert_eq!(runtime.resources().counts().0, baseline);
        if let Some(mut stream) = stream {
            let mut bytes = [0; 1];
            assert!(matches!(
                bounded(stream.read(&mut bytes)).await,
                Ok(0) | Err(_)
            ));
            assert_eq!(
                f.state
                    .voice_channels
                    .route(client.slot)
                    .unwrap()
                    .session()
                    .fixture_counts(),
                (false, 1, 1)
            );
        }
        f.close().await;
    }
}

#[tokio::test]
async fn partial_reads_do_not_renew_the_absolute_connection_deadline() {
    let clock = Arc::new(ManualClock::new(1000));
    let (registry, runtime) = empty(clock.clone());
    let bound = BoundServer::bind(registry, runtime, Origin::PackagedHttp).unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    socket.write_all(b"P").await.unwrap();
    until(|| running.phase.load(Ordering::Acquire) == 2).await;
    clock.advance_ms(CONNECTION_MS - 1);
    socket.write_all(b"O").await.unwrap();
    clock.advance_ms(1);
    let mut bytes = [0; 1];
    assert!(matches!(
        bounded(socket.read(&mut bytes)).await,
        Ok(0) | Err(_)
    ));
    until(|| running.phase.load(Ordering::Acquire) == 1).await;
    running.finish().await.unwrap();
}

#[tokio::test]
async fn dropping_the_observer_detaches_and_stops_instead_of_refunding_a_live_task() {
    let (registry, runtime) = empty(Arc::new(ManualClock::new(1000)));
    let bound = BoundServer::bind(registry.clone(), runtime.clone(), Origin::PackagedHttp).unwrap();
    let credit = Arc::downgrade(&bound.credit);
    let port = bound.port();
    let running = bound.start().unwrap();
    let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    socket.write_all(b"POST").await.unwrap();
    until(|| running.phase.load(Ordering::Acquire) == 2).await;
    drop(running);
    until(|| credit.upgrade().is_none()).await;
    let mut byte = [0; 1];
    assert!(matches!(
        bounded(socket.read(&mut byte)).await,
        Ok(0) | Err(_)
    ));
    assert_eq!(runtime.resources().counts(), (4096, 0));
}

#[tokio::test]
async fn cancelling_finish_keeps_the_actual_socket_task_paid_until_it_drains() {
    let (f, _, _public, runtime, client) = setup().await;
    let baseline = runtime.resources().counts().0;
    let bound = BoundServer::bind(
        f.state.voice_channels.clone(),
        runtime.clone(),
        Origin::PackagedHttp,
    )
    .unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    let session = f
        .state
        .voice_channels
        .route(client.slot)
        .unwrap()
        .session()
        .clone();
    let credit = Arc::downgrade(&running._credit);
    let (_, admission, body) = client.status(1);
    let mut submitted = wire(port, &admission, &body);
    submitted.pop();
    let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    socket.write_all(&submitted).await.unwrap();
    until(|| {
        f.state
            .voice_channels
            .route(client.slot)
            .unwrap()
            .session()
            .fixture_counts()
            .1
            == 1
    })
    .await;
    // This single-threaded test polls finish exactly once, sending stop and moving its private
    // handle into the await, then loses that observer before the actual runner can be polled.
    let mut finish = Box::pin(running.finish());
    std::future::poll_fn(|cx| {
        assert!(finish.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(finish);
    assert!(credit.upgrade().is_some());
    assert_eq!(
        runtime.resources().counts().0,
        baseline + INFRASTRUCTURE_BYTES
    );
    until(|| credit.upgrade().is_none()).await;
    let mut byte = [0; 1];
    assert!(matches!(
        bounded(socket.read(&mut byte)).await,
        Ok(0) | Err(_)
    ));
    assert_eq!(session.fixture_counts(), (false, 1, 1));
    assert!(f.state.voice_channels.route(client.slot).is_err());
    assert!(matches!(
        session.header(),
        Err(super::super::channel::Refused::Binding)
    ));
    assert_eq!(runtime.resources().counts().0, baseline);
    f.close().await;
}

#[test]
fn fixed_response_headers_fit_for_every_allowed_origin() {
    let origins = [Origin::PackagedHttp, Origin::PackagedScheme].into_iter();
    #[cfg(debug_assertions)]
    let origins = origins.chain([Origin::Development]);
    for origin in origins {
        let mut head = Output::new();
        head.reply(origin, 19).unwrap();
        assert!(head.filled < RESPONSE_HEAD_BYTES);
        let mut head = Output::new();
        head.preflight(origin).unwrap();
        assert!(head.filled < RESPONSE_HEAD_BYTES);
    }
}

// Park at one of the actual writer calls without timing a kernel buffer. Generated request
// bytes flow through the same Connection implementation as TCP. A drop probe checks that
// the stream is destroyed BEFORE its admitted Session position can be reused.
struct Parked {
    read: std::io::Cursor<Vec<u8>>,
    at: usize,
    writes: usize,
    dropped: Arc<AtomicBool>,
    session: Arc<super::super::channel::Session>,
    expected_busy_on_drop: bool,
}
impl AsyncRead for Parked {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let position = self.read.position() as usize;
        let take = buf.remaining().min(self.read.get_ref().len() - position);
        buf.put_slice(&self.read.get_ref()[position..position + take]);
        self.read.set_position((position + take) as u64);
        Poll::Ready(Ok(()))
    }
}
impl AsyncWrite for Parked {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.writes += 1;
        if self.writes >= self.at {
            Poll::Pending
        } else {
            Poll::Ready(Ok(data.len()))
        }
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Pending
    }
}
impl Drop for Parked {
    fn drop(&mut self) {
        assert_eq!(
            self.session.fixture_counts().0,
            self.expected_busy_on_drop,
            "unexpected Session custody when stream is destroyed"
        );
        self.dropped.store(true, Ordering::Release);
    }
}
#[tokio::test]
async fn cancellation_while_writing_header_prefix_body_or_shutdown_closes_stream_before_reply_custody(
) {
    let (f, _, _public, runtime, client) = setup().await;
    // Direct Connection tests still prepay the production infrastructure envelope.
    let _credit = runtime
        .resources()
        .charge_process_retained(INFRASTRUCTURE_BYTES)
        .unwrap();
    let clock = ManualClock::new(1000);
    for at in 1..=4 {
        let (_, admission, body) = client.status(at as u64);
        let session = f
            .state
            .voice_channels
            .route(client.slot)
            .unwrap()
            .session()
            .clone();
        let dropped = Arc::new(AtomicBool::new(false));
        let stream = Parked {
            read: std::io::Cursor::new(wire(41891, &admission, &body)),
            at,
            writes: 0,
            dropped: dropped.clone(),
            session: session.clone(),
            expected_busy_on_drop: true,
        };
        let mut connection = Connection::new(stream);
        let mut served = Box::pin(connection.serve(
            &f.state.voice_channels,
            41891,
            Origin::PackagedHttp,
            &clock,
            6000,
        ));
        std::future::poll_fn(|cx| {
            assert!(served.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(session.fixture_counts(), (true, at as u64, at as u64));
        drop(served);
        drop(connection);
        assert!(dropped.load(Ordering::Acquire));
        assert_eq!(session.fixture_counts(), (false, at as u64, at as u64));
    }
    f.close().await;
}

#[tokio::test]
async fn fixed_runner_future_and_output_fit_the_prepaid_infrastructure_peak() {
    let (registry, runtime) = empty(Arc::new(ManualClock::new(1000)));
    let bound = BoundServer::bind(registry, runtime, Origin::PackagedHttp).unwrap();
    let (_stop, cancelled) = oneshot::channel();
    let state = ServerState {
        listener: TcpListener::from_std(bound.listener).unwrap(),
        registry: bound.registry,
        clock: bound.clock,
        origin: bound.origin,
        port: bound.port,
        context: bound.context,
        credit: bound.credit,
        phase: Arc::new(std::sync::atomic::AtomicU8::new(0)),
    };
    let future = state.run(cancelled);
    // Include the concrete nested async frame (router + Session header coexist), the
    // public handle/completed output and pinned Tokio task/Arc/oneshot bookkeeping allowance.
    let peak = worker_layout(&future);
    eprintln!(
        "native IO peak {peak}; weak context footprint {}",
        super::super::context::Context::retained_layout_bytes()
    );
    assert!(
        peak <= INFRASTRUCTURE_BYTES,
        "native status peak needs {peak}"
    );
}

#[tokio::test]
async fn service_shutdown_collects_an_authenticated_detached_position_after_socket_owner_loss() {
    let (f, _owner, output, runtime, client) = setup().await;
    let bound = BoundServer::bind(
        f.state.voice_channels.clone(),
        runtime,
        Origin::PackagedHttp,
    )
    .unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    let route = f.state.voice_channels.route(client.slot).unwrap();
    let (_, admission, body) = client.status(1);
    let submitted = wire(port, &admission, &body);
    let mut worker = route.session().header().unwrap();
    worker.scratch_mut()[..submitted.len()].copy_from_slice(&submitted);
    worker.set_filled(submitted.len()).unwrap();
    let mut incoming = worker
        .admit_status(port, Origin::PackagedHttp.value(), client.session)
        .unwrap();
    assert_eq!(incoming.open().unwrap(), client.plain());
    let physical = incoming.hold_position().unwrap();
    drop(incoming); // the socket/header owner disappeared; detached work is still real.
    let mut finish = Box::pin(running.finish());
    assert!(std::future::poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx).is_pending())).await);
    until(|| route.require_open().is_err()).await;
    assert!(f.state.voice_channels.route(client.slot).is_err());
    assert!(matches!(
        route.session().header(),
        Err(super::super::channel::Refused::Binding)
    ));
    assert!(std::future::poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx).is_pending())).await);
    // The listener has closed even though native work must still be collected.
    assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .is_err());
    drop(physical);
    assert!(!bounded(finish).await.unwrap().io_failed);
    assert_eq!(route.session().fixture_counts(), (false, 1, 1));
    assert!(matches!(
        route.session().header(),
        Err(super::super::channel::Refused::Binding)
    ));
    drop(output);
    f.close().await;
}

#[tokio::test]
async fn shutdown_finds_a_retired_session_owned_only_by_an_uncollected_reply() {
    let (f, owner, output, runtime, client) = setup().await;
    let bound = BoundServer::bind(
        f.state.voice_channels.clone(),
        runtime,
        Origin::PackagedHttp,
    )
    .unwrap();
    let port = bound.port();
    let running = bound.start().unwrap();
    let route = f.state.voice_channels.route(client.slot).unwrap();
    let (_, admission, body) = client.status(1);
    let submitted = wire(port, &admission, &body);
    let mut worker = route.session().header().unwrap();
    worker.scratch_mut()[..submitted.len()].copy_from_slice(&submitted);
    worker.set_filled(submitted.len()).unwrap();
    let mut incoming = worker
        .admit_status(port, Origin::PackagedHttp.value(), client.session)
        .unwrap();
    incoming.open().unwrap();
    let mut prepared = incoming.reply(BodyKind::Control, 3, None).unwrap();
    prepared.plain_mut().copy_from_slice(&route.snapshot());
    let reply = prepared.seal().unwrap();
    drop(route);
    f.state.voice_authority.lock().unwrap().end(&owner);
    super::super::bootstrap::collect_channel(&f.state, &owner, [9; 32])
        .await
        .unwrap();
    f.state
        .voice_channels
        .collect_retired(&f.state.voice_handoff); // actual reply prevents reclamation.
    let mut finish = Box::pin(running.finish());
    assert!(std::future::poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx).is_pending())).await);
    // Observe service reaching its drain through the closed socket, not a timer.
    bounded(async {
        while let Ok(stream) = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
            drop(stream);
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(std::future::poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx).is_pending())).await);
    drop(reply);
    assert!(!bounded(finish).await.unwrap().io_failed);
    drop(output);
    f.close().await;
}

#[tokio::test]
async fn completed_reply_without_a_row_reference_keeps_native_control_paid_through_final_close() {
    use crate::voice_handoff::bootstrap::collect_channel;
    use crate::voice_handoff::channel::{BOOTSTRAP_BYTES, BROWSER_CONTROL_BYTES, CONTROL_BYTES};
    let (f, owner, output, runtime, client) = setup().await;
    let route = f.state.voice_channels.route(client.slot).unwrap();
    let session = Arc::downgrade(route.session());
    let (_, admission, body) = client.status(1);
    let submitted = wire(41891, &admission, &body);
    let mut worker = route.session().header().unwrap();
    worker.scratch_mut()[..submitted.len()].copy_from_slice(&submitted);
    worker.set_filled(submitted.len()).unwrap();
    let mut incoming = worker
        .admit_status(41891, Origin::PackagedHttp.value(), client.session)
        .unwrap();
    assert_eq!(incoming.open().unwrap(), client.plain());
    let mut prepared = incoming.reply(BodyKind::Control, 3, None).unwrap();
    prepared.plain_mut().copy_from_slice(&route.snapshot());
    let completed = prepared.seal().unwrap();
    drop(route); // CompletedReply directly owns Session, not Row.
    f.state.voice_authority.lock().unwrap().end(&owner);
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert!(f.state.voice_channels.route(client.slot).is_err());
    assert_eq!(session.strong_count(), 1);
    assert_eq!(runtime.resources().counts().1, 1);
    assert_eq!(completed.body().len(), 19);
    let paid = runtime.resources().counts().0;
    drop(completed);
    assert_eq!(session.strong_count(), 0);
    assert_eq!(
        runtime.resources().counts(),
        (
            paid - CONTROL_BYTES - BOOTSTRAP_BYTES - BROWSER_CONTROL_BYTES,
            0
        )
    );
    drop(output);
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    let collected = runtime.resources().counts();
    collect_channel(&f.state, &owner, [9; 32]).await.unwrap();
    assert_eq!(runtime.resources().counts(), collected);
    f.close().await;
}
