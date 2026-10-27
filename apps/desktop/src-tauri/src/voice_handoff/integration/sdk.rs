//! Native SDK obligation after real TCP Finish/key/storage admission. No browser SDK or
//! audible result is fabricated here: this tests the native post-authentication boundary.
use super::*;
use catcoms_sync::voice_v2::VoiceError;
pub(super) struct Token {
    pub(super) session: Arc<channel::SessionIdentity>,
    pub(super) request: requests::sdk::Request,
    pub(super) body: Vec<u8>,
    pub(super) generation: u64,
    pub(super) proof: [u8; 32],
}
async fn exchange(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    body: &[u8],
) -> Vec<u8> {
    let header = (
        route.session().fixture_counts().1 + 1,
        13,
        0,
        parent.id().logical(),
    );
    let reply = crate::voice_handoff::sdk::apply(
        &context::Context::new(state),
        route,
        opened_frame(public, route, header, body, None),
    )
    .await
    .unwrap();
    opened_reply(public, header, body, &reply)
}
pub(super) async fn start(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    storage_body: &[u8],
    storage: [u8; 32],
) -> Token {
    let mut body = storage_body[..121].to_vec();
    body[0] = 6;
    body.extend_from_slice(&storage);
    body.push(0);
    let request = requests::sdk::Request::decode(parent.id().logical(), &body).unwrap();
    let capacity = replies::refused(factory::Refused::Protocol(VoiceError::Capacity));
    let unauthorized = replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized));
    let commit = state.ui_session_commit.clone().lock_owned().await;
    assert_eq!(
        exchange(state, public, route, parent, &body).await,
        capacity
    );
    drop(commit);
    for offset in [1, 9, 17, 49, 81, 113, 121, 152] {
        let mut wrong = body.clone();
        wrong[offset] ^= 1;
        assert_eq!(
            exchange(state, public, route, parent, &wrong).await,
            unauthorized,
            "changed native SDK binding {offset}"
        );
    }
    // Use the actual affine physical position. Even trusted code cannot allocate another
    // candidate under it while the first original preparation is retained.
    let context = context::Context::new(state);
    factory::preflight_binding(&context, route).await.unwrap();
    let incoming = opened_frame(
        public,
        route,
        (
            route.session().fixture_counts().1 + 1,
            13,
            0,
            parent.id().logical(),
        ),
        &body,
        None,
    );
    let physical = incoming.hold_position().unwrap();
    let mut preparation = crate::voice_handoff::sdp::current(&context, route, |admission| {
        parent.begin_sdk(admission, &physical, route.session(), &request)
    })
    .unwrap()
    .unwrap();
    assert!(matches!(
        crate::voice_handoff::sdp::current(&context, route, |admission| parent.begin_sdk(
            admission,
            &physical,
            route.session(),
            &request
        )),
        Err(factory::Refused::Protocol(VoiceError::Capacity))
    ));
    preparation.allocate().unwrap();
    assert!(preparation.allocate().is_err());
    drop(preparation);
    drop(physical);
    drop(incoming);
    let clock = crate::voice_runtime(state).clock();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let reply = exchange(state, public, route, parent, &body).await;
            if reply != capacity {
                break reply;
            }
            clock.sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("original SDK custody admission stalled");
    assert_eq!(first.len(), crate::voice_handoff::sdk::REPLY_BYTES);
    assert_eq!(&first[..2], &[6, 1]);
    assert_eq!(
        exchange(state, public, route, parent, &body).await,
        first,
        "lost reply recovers one immutable SDK obligation"
    );
    let generation = u64::from_be_bytes(first[170..178].try_into().unwrap());
    assert_eq!(generation, 1);
    Token {
        session: route.session().identity().clone(),
        request,
        body,
        generation,
        proof: first[178..210].try_into().unwrap(),
    }
}
pub(super) async fn release_after_key_release(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    token: &Token,
) {
    assert!(
        parent.sdp_test_drained(),
        "storage state3 remains independent of SDK release"
    );
    assert!(
        !parent.sdp_test_whole_drained(),
        "KeyRelease does not release SDK custody"
    );
    assert!(matches!(parent.retire_plan(), Err(VoiceError::Capacity)));
    let closing = exchange(state, public, route, parent, &token.body).await;
    assert_eq!(&closing[..2], &[6, 2]);
    assert_eq!(&closing[178..210], &token.proof);
    let mut release = token.body.clone();
    release[153] = 1;
    release.extend_from_slice(&token.generation.to_be_bytes());
    release.extend_from_slice(&token.proof);
    for offset in [154, 162, 193] {
        let mut wrong = release.clone();
        wrong[offset] ^= 1;
        assert_eq!(
            exchange(state, public, route, parent, &wrong).await,
            replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized))
        );
    }
    for _ in 0..2 {
        let reply = exchange(state, public, route, parent, &release).await;
        assert_eq!(&reply[..2], &[6, 3]);
        assert_eq!(&reply[178..210], &token.proof);
    }
    assert!(parent.sdp_test_whole_drained());
    assert_eq!(
        &exchange(state, public, route, parent, &token.body).await[..2],
        &[6, 3],
        "late SDK Admit cannot reopen released custody"
    );
}
pub(super) fn release_after_physical_collection(parent: &ledger::ResolvedRow, token: Token) {
    // This is explicitly the native post-authentication API, not evidence that a retired
    // HTTP Session accepts another request. A destroyed realm needs an independent native
    // destruction proof; without one its abandoned SDK claim can remain until process exit.
    assert!(
        !parent.sdk_test_orphan_gate(),
        "dead lease + physical collection + Root drain do not prove SDK completion"
    );
    assert!(!parent.sdp_test_whole_drained());
    assert_eq!(
        parent
            .recover_sdk(&token.session, &token.request)
            .unwrap()
            .unwrap()
            .state,
        2
    );
    let mut foreign = token.proof;
    foreign[0] ^= 1;
    assert!(matches!(
        parent.release_sdk(&token.session, &token.request, token.generation, foreign),
        Err(VoiceError::Unauthorized)
    ));
    assert_eq!(
        parent
            .release_sdk(
                &token.session,
                &token.request,
                token.generation,
                token.proof
            )
            .unwrap()
            .state,
        3
    );
    assert!(parent.sdk_test_orphan_gate());
    assert!(parent.sdp_test_whole_drained());
}

pub(super) fn foreign_refuses(
    parent: &ledger::ResolvedRow,
    foreign: &bootstrap::Route,
    token: &Token,
) {
    assert!(matches!(
        parent.recover_sdk(foreign.session().identity(), &token.request),
        Err(VoiceError::Unauthorized)
    ));
    assert!(matches!(
        parent.release_sdk(
            foreign.session().identity(),
            &token.request,
            token.generation,
            token.proof
        ),
        Err(VoiceError::Unauthorized)
    ));
}

pub(super) async fn close_preparation_then_unwind(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    storage_body: &[u8],
    storage: [u8; 32],
) {
    let mut body = storage_body[..121].to_vec();
    body[0] = 6;
    body.extend_from_slice(&storage);
    body.push(0);
    let request = requests::sdk::Request::decode(parent.id().logical(), &body).unwrap();
    let context = context::Context::new(state);
    factory::preflight_binding(&context, route).await.unwrap();
    let incoming = opened_frame(
        public,
        route,
        (
            route.session().fixture_counts().1 + 1,
            13,
            0,
            parent.id().logical(),
        ),
        &body,
        None,
    );
    let physical = incoming.hold_position().unwrap();
    let mut preparation = crate::voice_handoff::sdp::current(&context, route, |admission| {
        parent.begin_sdk(admission, &physical, route.session(), &request)
    })
    .unwrap()
    .unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        preparation.allocate().unwrap();
        // The exact native post-auth storage Release wins while the original SDK
        // candidate is allocated but has not crossed final admission. The caller already
        // destroyed its real storage copy, so ONLY sdk_preparing blocks whole drainage.
        parent
            .release_sdp(route.session().identity(), &request.base, storage)
            .unwrap();
        assert!(matches!(
            crate::voice_handoff::sdp::current(&context, route, |admission| parent.attach_sdk(
                admission,
                &request,
                &mut preparation
            )),
            Err(factory::Refused::Protocol(VoiceError::Unauthorized))
        ));
        assert!(
            parent.sdp_test_drained(),
            "storage really drained before candidate Drop"
        );
        assert!(
            !parent.sdp_test_whole_drained(),
            "the allocated SDK candidate alone blocks retirement"
        );
        // Own the actual preparation INSIDE the unwinding closure so its destructor is
        // part of this boundary. A mere borrowed panic would leave its flag live here.
        let _owned = preparation;
        panic!("allocated SDK candidate unwinds after storage close");
    }))
    .is_err());
    assert!(parent
        .recover_sdk(route.session().identity(), &request)
        .unwrap()
        .is_none());
    drop(physical);
    drop(incoming);
}

/// The actual SDK candidate is parked across an early Plan fence. Direct trusted-native
/// cleanup below intentionally retains the original physical position; a second HTTP
/// request could not enter it. This tests final admission/destruction, not another wire.
pub(super) async fn fence_preparation_then_unwind(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    storage_body: &[u8],
    storage: [u8; 32],
    claim: [u8; 32],
) {
    let mut body = storage_body[..121].to_vec();
    body[0] = 6;
    body.extend_from_slice(&storage);
    body.push(0);
    let request = requests::sdk::Request::decode(parent.id().logical(), &body).unwrap();
    let context = context::Context::new(state);
    factory::preflight_binding(&context, route).await.unwrap();
    let incoming = opened_frame(
        public,
        route,
        (
            route.session().fixture_counts().1 + 1,
            13,
            0,
            parent.id().logical(),
        ),
        &body,
        None,
    );
    let physical = incoming.hold_position().unwrap();
    let mut preparation = crate::voice_handoff::sdp::current(&context, route, |admission| {
        parent.begin_sdk(admission, &physical, route.session(), &request)
    })
    .unwrap()
    .unwrap();
    preparation.allocate().unwrap();
    let original_key = parent.key_custody_for_test();
    assert!(plan_final::request(&context, route, parent)
        .await
        .unwrap()
        .proof
        .is_none());
    assert!(!original_key.is_current());
    assert!(matches!(
        crate::voice_handoff::sdp::current(&context, route, |admission| {
            parent.attach_sdk(admission, &request, &mut preparation)
        }),
        Err(factory::Refused::Protocol(VoiceError::Unauthorized))
    ));
    assert!(
        matches!(
            parent.recover_sdk(route.session().identity(), &request),
            Err(VoiceError::Unauthorized)
        ),
        "unattached frozen SDK cannot enter Actor preflight"
    );
    parent
        .release_sdp(route.session().identity(), &request.base, storage)
        .unwrap();
    assert!(
        parent.sdp_test_drained(),
        "no copy/input may mask the original SDK worker"
    );
    parent
        .release_key_session(
            route.session().identity(),
            request.base.action,
            request.base.handshake,
            claim,
            request.base.key,
        )
        .unwrap();
    parent
        .release_transition(
            route.session().identity(),
            request.base.action,
            request.base.handshake,
            claim,
            crate::voice_runtime(state).clock().monotonic_ms(),
        )
        .unwrap();
    assert!(
        !parent.sdp_test_whole_drained(),
        "actual SDK candidate alone still owns its paid obligation"
    );
    assert!(
        plan_final::request(&context, route, parent)
            .await
            .unwrap()
            .proof
            .is_none(),
        "released Claim/key/storage cannot certify a parked candidate's destruction"
    );
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _owned = preparation;
        panic!("actual failed candidate unwinds after its Plan fence");
    }))
    .is_err());
    assert!(parent.sdp_test_whole_drained());
    drop(physical);
    drop(incoming);
    let clock = crate::voice_runtime(state).clock();
    let receipt = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let receipt = plan_final::request(&context, route, parent).await.unwrap();
            if receipt.proof.is_some() {
                break receipt;
            }
            clock.sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("original parked SDK did not drain after its real destruction");
    parent
        .acknowledge_final(route.session().identity(), receipt.proof.unwrap())
        .unwrap();
}
