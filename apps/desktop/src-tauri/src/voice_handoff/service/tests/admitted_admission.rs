//! Genuine admitted activation/publication and independently derived client keys
//! traverse the production TCP31 dispatcher. No private Session key is read.
use super::*;
use catcoms_rt::{Clock, SystemClock};

async fn guarded<T>(future: impl Future<Output = T>) -> T {
    tokio::select! { value=future=>value,_=SystemClock.sleep(Duration::from_secs(30))=>panic!("admitted31 HTTP watchdog") }
}
async fn setup_admitted() -> (
    Fixture,
    crate::voice_call_activation::Handle,
    Arc<SetupOutput>,
    Arc<Runtime>,
    Client,
) {
    let f = Fixture::new().await;
    let old = f.candidate().await;
    let controller = {
        let authority = f.state.voice_authority.lock().unwrap();
        let crate::voice_authority::CandidateKind::Call { controller, .. } = authority
            .candidate(&old, SystemClock.monotonic_ms())
            .unwrap()
        else {
            panic!("actual controller")
        };
        controller
    };
    let owner = crate::voice_admitted_call::prepare_inner(
        &f.state,
        controller,
        7,
        catcoms_app::channel_id("general").to_string(),
    )
    .await
    .unwrap();
    guarded(crate::voice_admitted_call::activate_inner(
        &f.state,
        owner.clone(),
    ))
    .await
    .unwrap();
    crate::voice_admitted_call::acknowledge_inner(&f.state, &owner)
        .await
        .unwrap();
    let admission = crate::voice_admitted_key::admit_inner(&f.state, &owner, [9; 32])
        .await
        .unwrap();
    guarded(crate::voice_admitted_key::publish_inner(
        &f.state,
        &owner,
        [9; 32],
        &admission.permit,
        hex::decode(POINT).unwrap().try_into().unwrap(),
    ))
    .await
    .unwrap();
    let pending = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_publication_task_for_cleanup(&owner, [9; 32])
        .unwrap();
    guarded(async {
        while !pending.task_returned() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    crate::voice_admitted_key::acknowledge_inner(&f.state, &owner, [9; 32]).unwrap();
    crate::voice_admitted_key::settle_inner(&f.state, &owner, [9; 32])
        .await
        .unwrap();
    drop(pending);
    let handle = f
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admitted_activation(&owner)
        .unwrap();
    let Status::Ready(public) = super::super::super::bootstrap::ensure_admitted(
        &f.state,
        &owner,
        [9; 32],
        &mut ChaCha20Rng::seed_from_u64(83),
    )
    .await
    .unwrap() else {
        panic!("actual admitted physical Row")
    };
    let runtime = f.state.voice_authority.lock().unwrap().runtime();
    let client = Client::with_label(
        public.public(),
        b"Mewtual admitted native voice bootstrap seed v2",
    );
    (f, handle, public, runtime, client)
}
fn body(kind: u8, g: u64) -> [u8; 10] {
    let mut bytes = [12, kind, 0, 0, 0, 0, 0, 0, 0, 0];
    bytes[2..].copy_from_slice(&g.to_be_bytes());
    bytes
}
async fn exchange31(port: u16, wire: &[u8]) -> Vec<u8> {
    guarded(async {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        stream.write_all(wire).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        response
    })
    .await
}
async fn control(port: u16, client: &Client, sequence: &mut u64, kind: u8, g: u64) -> Vec<u8> {
    *sequence += 1;
    let (header, admission, cipher) =
        client.packet(*sequence, 31, 0, client.session, &body(kind, g));
    let response = exchange31(port, &wire(port, &admission, &cipher)).await;
    let opened = client.action_response(&response, &header, 0);
    assert_eq!(opened.len(), 59);
    assert_eq!(opened[0], 12);
    assert_eq!(&opened[2..10], &g.to_be_bytes());
    opened
}
async fn state31(port: u16, client: &Client, sequence: &mut u64, g: u64, wanted: u8) -> Vec<u8> {
    guarded(async {
        loop {
            let response = control(port, client, sequence, 2, g).await;
            if response[1] == wanted {
                return response;
            }
            assert!(
                !matches!(response[1], 6 | 7),
                "unexpected terminal native state {}",
                response[1]
            );
            SystemClock.sleep(Duration::from_millis(1)).await;
        }
    })
    .await
}

#[tokio::test]
async fn actual_tcp_admitted31_rejects_old_profile_operations_and_reuses_canonical_root_registration(
) {
    let (f, handle, public, runtime, client) = guarded(setup_admitted()).await;
    let route = f.state.voice_channels.admitted_route(client.slot).unwrap();
    let context = super::super::super::context::Context::new(&f.state);
    let server = BoundServer::bind_application(
        f.state.voice_channels.clone(),
        runtime,
        Origin::PackagedHttp,
        context,
    )
    .unwrap();
    let port = server.port();
    let running = server.start().unwrap();
    for operation in [9, 29, 30] {
        let (_, admission, cipher) = client.packet(1, operation, 0, client.session, &body(2, 1));
        assert!(exchange31(port, &wire(port, &admission, &cipher))
            .await
            .is_empty());
        guarded(route.session().fixture_wait_physical_idle()).await;
        assert_eq!(
            route.session().fixture_counts(),
            (false, 0, 0),
            "wrong profile op must not spend counters"
        );
    }
    for (kind, logical, plain) in [
        (1, client.session, body(2, 1).to_vec()),
        (0, [77; 16], body(2, 1).to_vec()),
        (0, client.session, body(2, 1)[..9].to_vec()),
    ] {
        let (_, admission, cipher) = client.packet(1, 31, kind, logical, &plain);
        assert!(exchange31(port, &wire(port, &admission, &cipher))
            .await
            .is_empty());
        guarded(route.session().fixture_wait_physical_idle()).await;
        assert_eq!(route.session().fixture_counts(), (false, 0, 0));
    }
    // A valid31 header with an old encrypted body version is admitted once and
    // rejected by its one closed decoder; it never falls back or refunds nonce.
    let mut old = body(2, 1);
    old[0] = 11;
    let (_, admission, cipher) = client.packet(1, 31, 0, client.session, &old);
    assert!(exchange31(port, &wire(port, &admission, &cipher))
        .await
        .is_empty());
    guarded(route.session().fixture_wait_physical_idle()).await;
    assert_eq!(route.session().fixture_counts(), (false, 1, 1));
    let mut sequence = 1;
    let ack = control(port, &client, &mut sequence, 0, 0).await;
    assert_eq!(
        &ack[10..],
        &[0; 49],
        "SetupAck alone creates neither Slot nor Root mint"
    );
    control(port, &client, &mut sequence, 1, 1).await;
    let first = state31(port, &client, &mut sequence, 1, 3).await;
    assert_eq!(first[26], 1);
    // Lost observer/reply recovery uses the original generation and identity.
    for _ in 0..4 {
        let repeated = control(port, &client, &mut sequence, 1, 1).await;
        assert_eq!(&repeated[10..], &first[10..]);
    }
    control(port, &client, &mut sequence, 3, 1).await;
    let returned = state31(port, &client, &mut sequence, 1, 5).await;
    assert_eq!(&returned[27..], &first[27..]);
    control(port, &client, &mut sequence, 1, 2).await;
    let next = state31(port, &client, &mut sequence, 2, 3).await;
    assert_eq!(
        &next[27..],
        &first[27..],
        "successor reuses original affine registration/incarnation"
    );
    guarded(running.finish()).await.unwrap(); // real Slot/Work/native-hold shutdown
    drop(route);
    drop(public);
    crate::end_voice_owner_inner(&f.state, handle.id()).await;
    guarded(handle.wait_execution_return()).await;
    super::super::super::bootstrap::settle_admitted_setup(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    super::super::super::bootstrap::collect_admitted_channel(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    guarded(f.close()).await;
}
#[tokio::test]
async fn existing_physical_session_authenticates_but_rejects31_before_counter_debit() {
    let (f, _owner, public, _runtime, client) = setup().await;
    let route = f.state.voice_channels.route(client.slot).unwrap();
    let context = super::super::super::context::Context::new(&f.state);
    let (_, admission, cipher) = client.packet(1, 31, 0, client.session, &body(1, 1));
    let wire = wire(41891, &admission, &cipher);
    let mut worker = route.session().header().unwrap();
    worker.scratch_mut()[..wire.len()].copy_from_slice(&wire);
    worker.set_filled(wire.len()).unwrap();
    assert!(matches!(
        super::super::dispatch::admit(
            worker,
            41891,
            Origin::PackagedHttp.value(),
            &route,
            &context
        ),
        Err(Refused::Header)
    ));
    assert_eq!(route.session().fixture_counts(), (false, 0, 0));
    drop(public);
    guarded(f.close()).await;
}
