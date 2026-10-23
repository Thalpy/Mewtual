use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn session(resources: Arc<Resources>, group: [u8; 32]) -> (Arc<Session>, Keys) {
    use catcoms_rt::RngCore;
    let mut rng = ChaCha20Rng::from_seed([37; 32]);
    let mut browser_rng = rng.clone();
    let mut seed = [0; 32];
    browser_rng.fill_bytes(&mut seed);
    (
        Session::new(resources, group, 1, &mut rng).unwrap(),
        Keys::new(&seed),
    )
}

/// Test-only actual codec endpoint shared by native ledger admission regressions. It creates
/// deterministic real MAC/AEAD bytes, never an asserted authentication boolean or a forged
/// AuthenticatedRequest. Its allocations are fixtures, not application copy-bound evidence.
pub(in crate::voice_handoff) struct FixtureChannel {
    pub(in crate::voice_handoff) session: Arc<Session>,
    browser: Keys,
}
impl FixtureChannel {
    pub(in crate::voice_handoff) fn new(resources: Arc<Resources>, group: [u8; 32]) -> Self {
        let (session, browser) = session(resources, group);
        Self { session, browser }
    }
    pub(in crate::voice_handoff) fn wire(
        &self,
        logical: [u8; 16],
        sequence: u64,
        operation: Operation,
        kind: BodyKind,
        plaintext: &[u8],
    ) -> Vec<u8> {
        let header = Header::new(
            Direction::Request,
            operation,
            kind,
            1,
            sequence,
            logical,
            plaintext.len() + 16,
        )
        .unwrap();
        let (admission, body) = self.browser.request(header, plaintext);
        let mut wire = format!("POST /voice/v2 HTTP/1.1\r\nHost: 127.0.0.1:41891\r\nOrigin: http://tauri.localhost\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nx-voice-admission: {}\r\n\r\n", body.len(), std::str::from_utf8(&admission).unwrap()).into_bytes();
        wire.extend_from_slice(&body);
        wire
    }
    pub(in crate::voice_handoff) fn authenticate(
        &self,
        wire: &[u8],
    ) -> Result<AuthenticatedRequest, Refused> {
        let mut worker = self.session.header()?;
        worker.scratch_mut()[..wire.len()].copy_from_slice(wire);
        worker.set_filled(wire.len())?;
        worker.authenticate(41891, "http://tauri.localhost")
    }
}
fn request(
    keys: &Keys,
    sequence: u64,
    operation: Operation,
    kind: BodyKind,
    plaintext: &[u8],
) -> (Header, Vec<u8>) {
    let header = Header::new(
        Direction::Request,
        operation,
        kind,
        1,
        sequence,
        [9; 16],
        plaintext.len() + 16,
    )
    .unwrap();
    let (admission, body) = keys.request(header, plaintext);
    let mut wire = format!("POST /voice/v2 HTTP/1.1\r\nHost: 127.0.0.1:41891\r\nOrigin: http://tauri.localhost\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nx-voice-admission: {}\r\n\r\n",
        body.len(), std::str::from_utf8(&admission).unwrap()).into_bytes();
    wire.extend_from_slice(&body);
    (header, wire)
}
fn incoming(
    session: &Arc<Session>,
    bytes: &[u8],
    phase: Option<&Arc<Phase>>,
) -> Result<Incoming, Refused> {
    let mut worker = session.header()?;
    worker.scratch_mut()[..bytes.len()].copy_from_slice(bytes);
    worker.set_filled(bytes.len()).unwrap();
    worker.admit(41891, "http://tauri.localhost", phase)
}

#[test]
fn paid_fixed_storage_and_original_control_lane_cover_codec_peak() {
    use aes_gcm::Aes256Gcm;
    use hkdf::Hkdf;
    use hmac::Hmac;
    use sha2::Sha256;
    let adapters = [
        super::super::pending_close::layout_bytes(),
        super::super::history::layout_bytes(),
        super::super::finalization::layout_bytes(),
        super::super::input::layout_bytes(),
        super::super::admission::layout_bytes(),
        super::super::settlement::layout_bytes(),
    ];
    eprintln!("adapter graphs pending/history/final/input/admission/settlement: {adapters:?}");
    let state = std::mem::size_of::<HeaderWorker>()
        .max(std::mem::size_of::<AuthenticatedRequest>())
        .max(std::mem::size_of::<Incoming>())
        .max(std::mem::size_of::<PreparedReply>())
        .max(std::mem::size_of::<CompletedReply>());
    // One physical stage at a time, one synchronous AES/HMAC operation at a time. Include
    // known live codec temporaries, canonical encoding and a reserved 1 KiB for the future IO
    // wrapper. That wrapper still requires its own actual layout/copy/collection proof.
    let peak = std::mem::size_of::<Session>()
        + std::mem::size_of::<SessionIdentity>() + 2 * std::mem::size_of::<usize>()
        + std::mem::size_of::<PhysicalPosition>()
        + 2 * std::mem::size_of::<usize>()
        + std::mem::size_of::<PhysicalHold>()
        + std::mem::size_of::<WorkLane>()
        + 2 * std::mem::size_of::<usize>()
        + 2 * std::mem::size_of::<usize>()
        + state
        + http::HEADER_BYTES // FULL fixed Box allocation, not merely its inline pointer
        // Only one adapter owns this original physical position. Count
        // its largest complete handler graph, not a sum of mutually exclusive dispatch arms.
        + adapters.into_iter().max().unwrap()
        + std::mem::size_of::<Aes256Gcm>()
        + 2 * std::mem::size_of::<Hmac<Sha256>>()
        + crypto::HEADER * 4
        + crypto::ADMISSION * 2
        + 72
        + 32
        + 12
        + 16
        + 1024;
    assert!(
        peak <= CONTROL_BYTES,
        "physical codec peak {peak} exceeds {CONTROL_BYTES}"
    );
    let setup = std::mem::size_of::<Session>()
        + std::mem::size_of::<Hkdf<Sha256>>()
        + 32 * 8
        + 2 * std::mem::size_of::<usize>();
    assert!(setup <= BOOTSTRAP_BYTES);
    assert_eq!(crypto::ADMISSION, 144);
    eprintln!(
        "physical codec peak {peak}; fixed scratch {}",
        http::HEADER_BYTES
    );
}

#[test]
fn fixed_header_scratch_is_one_original_allocation_through_auth_body_and_reply_moves() {
    let (session, browser) = session(Arc::new(Resources::default()), [8; 32]);
    let (_, bytes) = request(&browser, 1, Operation::Input, BodyKind::Control, b"query");
    let mut worker = session.header().unwrap();
    let pointer = worker.scratch.as_ptr();
    worker.scratch_mut()[..bytes.len()].copy_from_slice(&bytes);
    worker.set_filled(bytes.len()).unwrap();
    let authenticated = worker
        .authenticate(41891, "http://tauri.localhost")
        .unwrap();
    assert_eq!(authenticated.worker.scratch.as_ptr(), pointer);
    let mut incoming = authenticated.admit(None).unwrap();
    assert_eq!(incoming.worker.scratch.as_ptr(), pointer);
    assert_eq!(incoming.open().unwrap(), b"query");
    let hold = incoming.hold_position().unwrap();
    let mut prepared = incoming.reply(BodyKind::Control, 2, None).unwrap();
    assert_eq!(prepared.worker.scratch.as_ptr(), pointer);
    prepared.plain_mut().copy_from_slice(b"ok");
    let completed = prepared.seal().unwrap();
    assert_eq!(completed.worker.scratch.as_ptr(), pointer);
    drop(completed);
    assert_eq!(session.fixture_counts(), (true, 1, 1));
    drop(hold);
    assert_eq!(session.fixture_counts(), (false, 1, 1));
}

#[tokio::test]
async fn shutdown_waits_for_both_physical_owners_and_never_reopens_original_counters() {
    use std::future::{poll_fn, Future};
    use std::task::Poll;
    for worker_first in [false, true] {
        let (session, _) = session(Arc::new(Resources::default()), [8; 32]);
        let worker = session.header().unwrap();
        let hold = worker.fixture_hold().unwrap();
        session.close_admission();
        assert!(matches!(session.header(), Err(Refused::Binding)));
        let mut idle = Box::pin(session.wait_idle());
        assert!(poll_fn(|cx| Poll::Ready(idle.as_mut().poll(cx).is_pending())).await);
        if worker_first {
            drop(worker);
            assert!(poll_fn(|cx| Poll::Ready(idle.as_mut().poll(cx).is_pending())).await);
            drop(hold);
        } else {
            drop(hold);
            assert!(poll_fn(|cx| Poll::Ready(idle.as_mut().poll(cx).is_pending())).await);
            drop(worker);
        }
        idle.await;
        assert_eq!(session.fixture_counts(), (false, 0, 0));
        assert!(matches!(session.header(), Err(Refused::Binding)));
        // Last-drop-before-registration must be equally safe; no remembered wake is needed
        // when the actual counter already establishes that the original position is idle.
        session.wait_idle().await;
    }
}

#[test]
fn same_logical_retry_has_fresh_physical_sequences_and_response_binds_original_request() {
    let pool = Arc::new(Resources::default());
    let (session, browser) = session(pool.clone(), [8; 32]);
    let counts = pool.counts();
    let mut responses = Vec::new();
    for sequence in [1, 2] {
        let (request, wire) = request(
            &browser,
            sequence,
            Operation::Input,
            BodyKind::Control,
            b"query",
        );
        let mut admitted = incoming(&session, &wire, None).unwrap();
        assert_eq!(admitted.open().unwrap(), b"query");
        let mut reply = admitted.reply(BodyKind::Control, 7, None).unwrap();
        reply.plain_mut().copy_from_slice(b"receipt");
        let completed = reply.seal().unwrap();
        assert_eq!(
            session.header().err().map(|_| Refused::Capacity),
            Some(Refused::Capacity)
        );
        let mut bytes = completed.body().to_vec();
        let response = browser
            .response(
                std::str::from_utf8(completed.admission()).unwrap(),
                &request,
                &mut bytes,
            )
            .unwrap();
        assert_eq!(&bytes[..7], b"receipt");
        assert_eq!(response.sequence(), sequence);
        responses.push((request, *completed.admission(), completed.body().to_vec()));
        drop(completed);
        assert_eq!(pool.counts(), counts);
    }
    let (first, _admission, _bytes) = &responses[0];
    let (_second, admission, bytes) = &responses[1];
    assert_eq!(
        browser
            .response(
                std::str::from_utf8(admission).unwrap(),
                first,
                &mut bytes.clone()
            )
            .err(),
        Some(Refused::Authentication)
    );
}

#[test]
fn failed_body_spends_admission_but_capacity_before_admission_does_not() {
    let (session, browser) = session(Arc::new(Resources::default()), [3; 32]);
    let (_header, mut wire) = request(&browser, 1, Operation::Status, BodyKind::Control, b"status");
    let busy = session.header().unwrap();
    assert_eq!(
        incoming(&session, &wire, None).err(),
        Some(Refused::Capacity)
    );
    assert_eq!(session.counters.lock().unwrap().request_floor, 0);
    drop(busy);
    *wire.last_mut().unwrap() ^= 1;
    let mut admitted = incoming(&session, &wire, None).unwrap();
    assert_eq!(admitted.open().err(), Some(Refused::Body));
    drop(admitted);
    assert_eq!(incoming(&session, &wire, None).err(), Some(Refused::Replay));
    let counters = session.counters.lock().unwrap();
    assert_eq!((counters.request_floor, counters.response_sequence), (1, 1));
}

#[test]
fn header_mutation_cannot_spend_a_counter_and_unknown_fields_refuse() {
    let (session, browser) = session(Arc::new(Resources::default()), [3; 32]);
    let (header, _wire) = request(&browser, 1, Operation::Status, BodyKind::Control, b"x");
    let (admission, _body) = browser.request(header, b"x");
    for index in 0..admission.len() {
        let mut changed = admission;
        changed[index] = if changed[index] == b'0' { b'1' } else { b'0' };
        assert!(
            session
                .keys
                .authenticate_request(std::str::from_utf8(&changed).unwrap())
                .is_err(),
            "mutation at {index}"
        );
        assert_eq!(session.counters.lock().unwrap().request_floor, 0);
    }
}

#[test]
fn completed_output_keeps_the_original_credit_after_all_outer_session_waiters_drop() {
    let pool = Arc::new(Resources::default());
    let (session, browser) = session(pool.clone(), [2; 32]);
    let (_header, wire) = request(&browser, 1, Operation::Status, BodyKind::Control, b"x");
    let mut admitted = incoming(&session, &wire, None).unwrap();
    admitted.open().unwrap();
    let mut reply = admitted.reply(BodyKind::Control, 1, None).unwrap();
    reply.plain_mut()[0] = 7;
    let completed = reply.seal().unwrap();
    let held = pool.counts();
    assert_eq!(
        held,
        (CONTROL_BYTES + BOOTSTRAP_BYTES + BROWSER_CONTROL_BYTES, 1),
        "the original control lane stays prepaid through actual output collection"
    );
    let weak = Arc::downgrade(&session);
    drop(session);
    assert!(weak.upgrade().is_some());
    assert_eq!(pool.counts(), held);
    // A ready JoinHandle/output tuple is still actual custody until collection.
    let ready_but_uncollected = Some(completed);
    assert_eq!(pool.counts(), held);
    drop(ready_but_uncollected);
    assert!(weak.upgrade().is_none());
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn nonce_exhaustion_never_wraps_and_refuses_before_spending_the_request() {
    let (session, browser) = session(Arc::new(Resources::default()), [4; 32]);
    session.counters.lock().unwrap().response_sequence = u64::MAX;
    let (_header, wire) = request(&browser, 1, Operation::Status, BodyKind::Control, b"x");
    assert_eq!(
        incoming(&session, &wire, None).err(),
        Some(Refused::Exhausted)
    );
    assert_eq!(session.counters.lock().unwrap().request_floor, 0);
    assert!(!session.counters.lock().unwrap().busy);
}

#[test]
fn control_replies_remain_possible_after_the_original_data_phase_is_closed() {
    let (coordinator, _join, action, phase) = crate::voice_authority::tests::phase_fixture();
    let group = action.protected_binding().unwrap().scope.group;
    let (session, browser) = session(coordinator.runtime().resources(), group);
    let (_header, wire) = request(&browser, 1, Operation::Result, BodyKind::Data, b"result");
    let mut admitted = incoming(&session, &wire, Some(&phase)).unwrap();
    assert_eq!(admitted.open().unwrap(), b"result");
    action.settle_engine();
    phase.close();
    assert!(!phase.is_drained());
    let mut reply = admitted.reply(BodyKind::Control, 4, None).unwrap();
    assert!(
        phase.is_drained(),
        "the actual old data body dropped before control reply allocation"
    );
    reply.plain_mut().copy_from_slice(b"done");
    let completed = reply.seal().unwrap();
    assert_eq!(completed.body().len(), 20);
    drop(completed);
}

#[test]
fn actual_opened_result_moves_to_actor_custody_without_reusing_its_response_nonce_or_position() {
    let (coordinator, _join, action, phase) = crate::voice_authority::tests::phase_fixture();
    let group = action.protected_binding().unwrap().scope.group;
    let (session, browser) = session(coordinator.runtime().resources(), group);
    let (request, wire) = request(
        &browser,
        1,
        Operation::Result,
        BodyKind::Data,
        b"metaoutput",
    );
    let mut admitted = incoming(&session, &wire, Some(&phase)).unwrap();
    assert_eq!(
        admitted.result_body(&phase, 4..10).err(),
        Some(Refused::Body)
    );
    assert_eq!(admitted.open().unwrap(), b"metaoutput");
    let physical = admitted.hold_position().unwrap();
    let body = admitted.result_body(&phase, 4..10).unwrap();
    assert_eq!(
        admitted.result_body(&phase, 4..10).err(),
        Some(Refused::Bounds)
    );
    assert!(admitted.open().is_err());
    action.settle_engine();
    phase.close_confirmation().unwrap();
    assert!(!phase.is_drained());
    let mut prepared = admitted.reply(BodyKind::Control, 2, None).unwrap();
    prepared.plain_mut().copy_from_slice(b"ok");
    let completed = prepared.seal().unwrap();
    let mut response = completed.body().to_vec();
    browser
        .response(
            std::str::from_utf8(completed.admission()).unwrap(),
            &request,
            &mut response,
        )
        .unwrap();
    assert_eq!(&response[..2], b"ok");
    drop(completed);
    assert_eq!(
        session.fixture_counts(),
        (true, 1, 1),
        "detached worker still owns original physical flight"
    );
    assert!(
        !phase.is_drained(),
        "the actual actor body still owns the only input buffer"
    );
    drop(body);
    assert!(phase.is_drained());
    drop(physical);
    assert_eq!(session.fixture_counts(), (false, 1, 1));
}

#[test]
fn result_transfer_local_capacity_and_foreign_phase_refusals_keep_the_exact_opened_buffer() {
    let (coordinator, _join, action, phase) = crate::voice_authority::tests::phase_fixture();
    let (_foreign_coordinator, _foreign_join, _foreign_action, foreign) =
        crate::voice_authority::tests::phase_fixture();
    let group = action.protected_binding().unwrap().scope.group;
    let (session, browser) = session(coordinator.runtime().resources(), group);
    let (_, wire) = request(
        &browser,
        1,
        Operation::Result,
        BodyKind::Data,
        b"metaoutput",
    );
    let mut admitted = incoming(&session, &wire, Some(&phase)).unwrap();
    assert_eq!(admitted.open().unwrap(), b"metaoutput");
    let pointer = admitted.body.plain().unwrap().as_ptr();
    let original = (admitted.filled, admitted.body.cipher_bytes);
    assert_eq!(
        admitted.result_body(&foreign, 4..10).err(),
        Some(Refused::Bounds)
    );
    let pressure = phase
        .buffer(
            16 * 1024
                - admitted.body.cipher_bytes
                - catcoms_app::VOICE_CONFIRMATION_BODY_METADATA_BYTES
                + 1,
        )
        .unwrap();
    assert_eq!(
        admitted.result_body(&phase, 4..10).err(),
        Some(Refused::Capacity)
    );
    assert_eq!(admitted.body.plain().unwrap(), b"metaoutput");
    assert_eq!(admitted.body.plain().unwrap().as_ptr(), pointer);
    assert_eq!((admitted.filled, admitted.body.cipher_bytes), original);
    assert_eq!(session.fixture_counts(), (true, 1, 1));
    for range in [11..11, Range { start: 10, end: 4 }, 0..11] {
        assert_eq!(
            admitted.result_body(&phase, range).err(),
            Some(Refused::Bounds)
        );
        assert_eq!(admitted.body.plain().unwrap().as_ptr(), pointer);
    }
    drop(pressure);
    let body = admitted.result_body(&phase, 4..10).unwrap();
    drop(admitted);
    action.settle_engine();
    phase.close_confirmation().unwrap();
    assert!(!phase.is_drained());
    drop(body);
    assert!(phase.is_drained());
}

#[test]
fn correctly_authenticated_noncanonical_headers_and_illegal_body_classes_never_debit() {
    let (session, browser) = session(Arc::new(Resources::default()), [5; 32]);
    let base = Header::new(
        Direction::Request,
        Operation::Status,
        BodyKind::Control,
        1,
        1,
        [9; 16],
        17,
    )
    .unwrap()
    .fixture_raw();
    let mut cases = Vec::new();
    for (index, value) in [
        (0, 0),
        (4, 2),
        (5, 1),
        (5, 2),
        (6, 255),
        (7, 0),
        (7, 65),
        (36, 1),
        (36, 2),
        (37, 1),
        (38, 1),
        (39, 1),
    ] {
        let mut raw = base;
        raw[index] = value;
        cases.push(raw);
    }
    let mut result_control = base;
    result_control[6] = Operation::Result as u8;
    cases.push(result_control);
    for length in [0u32, 15, 273, 2065] {
        let mut raw = base;
        raw[32..36].copy_from_slice(&length.to_be_bytes());
        cases.push(raw);
    }
    let mut zero_sequence = base;
    zero_sequence[8..16].fill(0);
    cases.push(zero_sequence);
    // This header is itself legal/authenticated, but conflicts with HTTP Content-Length.
    let mut mismatched_length = base;
    mismatched_length[35] = 18;
    cases.push(mismatched_length);
    for raw in cases {
        let admission = browser.fixture_raw_admission(raw);
        let wire = format!("POST /voice/v2 HTTP/1.1\r\nHost: 127.0.0.1:41891\r\nOrigin: http://tauri.localhost\r\nContent-Type: application/octet-stream\r\nContent-Length: 17\r\nx-voice-admission: {}\r\n\r\n",
            std::str::from_utf8(&admission).unwrap());
        assert!(
            incoming(&session, wire.as_bytes(), None).is_err(),
            "accepted authenticated raw header {raw:?}"
        );
        let counters = session.counters.lock().unwrap();
        assert_eq!(
            (
                counters.request_floor,
                counters.response_sequence,
                counters.busy
            ),
            (0, 0, false)
        );
    }
}

#[test]
fn wrong_pool_group_and_actual_phase_capacity_refuse_without_spending_and_retry_exact_request() {
    let (coordinator, _join, action, phase) = crate::voice_authority::tests::phase_fixture();
    let pool = coordinator.runtime().resources();
    let group = action.protected_binding().unwrap().scope.group;
    let (same, browser) = session(pool.clone(), group);
    let (foreign_pool, _) = session(Arc::new(Resources::default()), group);
    let (foreign_group, _) = session(pool, [99; 32]);
    let (_header, wire) = request(&browser, 1, Operation::Result, BodyKind::Data, b"x");
    for foreign in [foreign_pool, foreign_group] {
        assert_eq!(
            incoming(&foreign, &wire, Some(&phase)).err(),
            Some(Refused::Binding)
        );
        let counters = foreign.counters.lock().unwrap();
        assert_eq!((counters.request_floor, counters.response_sequence), (0, 0));
    }
    let full = phase.buffer(16 * 1024).unwrap();
    assert_eq!(
        incoming(&same, &wire, Some(&phase)).err(),
        Some(Refused::Capacity)
    );
    {
        let counters = same.counters.lock().unwrap();
        assert_eq!(
            (
                counters.request_floor,
                counters.response_sequence,
                counters.busy
            ),
            (0, 0, false)
        );
    }
    drop(full);
    let mut admitted = incoming(&same, &wire, Some(&phase)).unwrap();
    assert_eq!(admitted.open().unwrap(), b"x");
    drop(admitted);
    action.settle_engine();
    phase.close();
    assert!(
        phase.is_drained(),
        "refusals leaked no original phase contribution"
    );
}

#[test]
fn constructor_second_payment_refusal_restores_credit_and_never_calls_rng() {
    use catcoms_rt::RngCore;
    let pool = Arc::new(Resources::default());
    let group = [7; 32];
    let pressure = pool
        .charge(
            group,
            catcoms_sync::voice_v2::resources::GROUP_BYTES - CONTROL_BYTES - BOOTSTRAP_BYTES / 2,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    let mut rng = ChaCha20Rng::from_seed([49; 32]);
    let mut untouched = rng.clone();
    assert_eq!(
        Session::new(pool.clone(), group, 1, &mut rng)
            .err()
            .map(|_| Refused::Capacity),
        Some(Refused::Capacity)
    );
    assert_eq!(rng.next_u64(), untouched.next_u64());
    assert_eq!(pool.counts(), before);
    drop(pressure);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn dropping_partial_admission_or_unsealed_reply_releases_position_without_refunding_nonce() {
    let (session, browser) = session(Arc::new(Resources::default()), [2; 32]);
    let (_header, wire) = request(&browser, 1, Operation::Status, BodyKind::Control, b"status");
    let admitted = incoming(&session, &wire[..wire.len() - 3], None).unwrap();
    assert!(session.header().is_err());
    drop(admitted);
    assert_eq!(incoming(&session, &wire, None).err(), Some(Refused::Replay));
    let (_header, next) = request(&browser, 2, Operation::Status, BodyKind::Control, b"status");
    let mut admitted = incoming(&session, &next, None).unwrap();
    admitted.open().unwrap();
    let prepared = admitted.reply(BodyKind::Control, 0, None).unwrap();
    assert!(session.header().is_err());
    drop(prepared);
    let counters = session.counters.lock().unwrap();
    assert_eq!(
        (
            counters.request_floor,
            counters.response_sequence,
            counters.busy
        ),
        (2, 2, false)
    );
}

#[test]
fn native_matches_independent_webcrypto_fixed_vectors_at_data_and_u64_boundaries() {
    // These public bytes are independently checked with actual WebCrypto in the frontend
    // suite. Neither side may replace fixed expected bytes with only its own round trip.
    let fixture: serde_json::Value = serde_json::from_str(include_str!("vectors.json")).unwrap();
    let seed: [u8; 32] = hex::decode(fixture["seed"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let keys = Keys::new(&seed);
    let frame = |value: &serde_json::Value| {
        let raw = hex::decode(value["header"].as_str().unwrap()).unwrap();
        Header::new(
            if raw[5] == 0 {
                Direction::Request
            } else {
                Direction::Response
            },
            match raw[6] {
                1 => Operation::Input,
                3 => Operation::Result,
                9 => Operation::Status,
                _ => panic!("fixture operation"),
            },
            if raw[36] == 0 {
                BodyKind::Control
            } else {
                BodyKind::Data
            },
            raw[7],
            u64::from_be_bytes(raw[8..16].try_into().unwrap()),
            raw[16..32].try_into().unwrap(),
            u32::from_be_bytes(raw[32..36].try_into().unwrap()) as usize,
        )
        .unwrap()
    };
    for case in fixture["cases"].as_array().unwrap() {
        let request = frame(&case["request"]);
        let response = frame(&case["response"]);
        let request_text = case["request"]["admission"].as_str().unwrap();
        let request_plain = hex::decode(case["request"]["plaintext"].as_str().unwrap()).unwrap();
        let (admission, cipher) = keys.request(request, &request_plain);
        assert_eq!(std::str::from_utf8(&admission).unwrap(), request_text);
        assert_eq!(
            hex::encode(request.fixture_raw()),
            case["request"]["header"].as_str().unwrap()
        );
        assert_eq!(
            hex::encode(&cipher),
            case["request"]["ciphertext"].as_str().unwrap()
        );
        assert_eq!(keys.authenticate_request(request_text).unwrap(), request);
        let mut opened = cipher;
        keys.open_request(&request, &mut opened).unwrap();
        assert_eq!(&opened[..request_plain.len()], request_plain);

        let response_plain = hex::decode(case["response"]["plaintext"].as_str().unwrap()).unwrap();
        let mut response_cipher = response_plain.clone();
        response_cipher.resize(response.length(), 0);
        keys.seal_response(&response, &request, &mut response_cipher)
            .unwrap();
        assert_eq!(
            hex::encode(&response_cipher),
            case["response"]["ciphertext"].as_str().unwrap()
        );
        assert_eq!(
            hex::encode(response.fixture_raw()),
            case["response"]["header"].as_str().unwrap()
        );
        let admission = keys.response_admission(&response, &request);
        let text = std::str::from_utf8(&admission).unwrap();
        assert_eq!(text, case["response"]["admission"].as_str().unwrap());
        assert_eq!(
            keys.response(text, &request, &mut response_cipher).unwrap(),
            response
        );
        assert_eq!(&response_cipher[..response_plain.len()], response_plain);
    }
}

#[test]
fn concurrent_header_claims_share_one_position_until_the_actual_winner_drops() {
    let (session, _browser) = session(Arc::new(Resources::default()), [3; 32]);
    let ready = Arc::new(std::sync::Barrier::new(9));
    let retire = Arc::new(std::sync::Barrier::new(9));
    let winners = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let (session, ready, retire, winners) = (
                session.clone(),
                ready.clone(),
                retire.clone(),
                winners.clone(),
            );
            scope.spawn(move || {
                ready.wait();
                let worker = session.header();
                if worker.is_ok() {
                    winners.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                }
                retire.wait(); // no winner can retire before every contender has made its claim
                drop(worker);
            });
        }
        ready.wait();
        retire.wait();
    });
    assert_eq!(winners.load(std::sync::atomic::Ordering::Acquire), 1);
    assert!(session.header().is_ok());
}

#[test]
fn actual_detached_hold_keeps_the_original_session_busy_after_header_cancellation() {
    let pool = Arc::new(Resources::default());
    let (session, _browser) = session(pool.clone(), [3; 32]);
    let worker = session.header().unwrap();
    let hold = worker.hold().unwrap();
    assert!(hold.belongs_to(&session));
    assert!(matches!(worker.hold(), Err(Refused::Capacity)));
    drop(worker);
    assert!(matches!(session.header(), Err(Refused::Capacity)));
    assert_eq!(session.fixture_counts(), (true, 0, 0));
    let paid = pool.counts();
    let weak = Arc::downgrade(&session);
    drop(session);
    assert!(weak.upgrade().is_some());
    assert_eq!(pool.counts(), paid);
    drop(hold);
    assert!(weak.upgrade().is_none());
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn admitted_opened_request_and_completed_reply_share_the_same_unique_detached_hold() {
    let (session, browser) = session(Arc::new(Resources::default()), [3; 32]);
    let (_, wire) = request(&browser, 1, Operation::Input, BodyKind::Control, &[1]);
    let mut frame = incoming(&session, &wire, None).unwrap();
    assert!(matches!(frame.hold_position(), Err(Refused::Body)));
    frame.open().unwrap();
    let hold = frame.hold_position().unwrap();
    assert!(matches!(frame.hold_position(), Err(Refused::Capacity)));
    let reply = frame
        .reply(BodyKind::Control, 0, None)
        .unwrap()
        .seal()
        .unwrap();
    drop(reply);
    assert!(matches!(session.header(), Err(Refused::Capacity)));
    assert_eq!(session.fixture_counts(), (true, 1, 1));
    drop(hold);
    assert_eq!(session.fixture_counts(), (false, 1, 1));
    assert!(session.header().is_ok());
}

#[test]
fn concurrent_detached_hold_claims_cannot_multiply_the_prepaid_physical_owner() {
    let (session, _browser) = session(Arc::new(Resources::default()), [3; 32]);
    let worker = session.header().unwrap();
    let ready = std::sync::Barrier::new(9);
    let held = std::sync::Barrier::new(9);
    let winners = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                ready.wait();
                let hold = worker.hold();
                if hold.is_ok() {
                    winners.fetch_add(1, Ordering::SeqCst);
                }
                held.wait();
                drop(hold);
            });
        }
        ready.wait();
        held.wait();
    });
    assert_eq!(winners.load(Ordering::SeqCst), 1);
    assert!(matches!(session.header(), Err(Refused::Capacity)));
    drop(worker);
    assert!(session.header().is_ok());
}
