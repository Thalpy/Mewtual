//! Real authenticated storage control, after actual TCP/native Finish and KeyClaim.
//! Hold original preparation/copy owners at the real admission boundary; no sleeps or
//! literal unlocked/confirmed booleans stand in for the process publication fence.
use super::*;
use catcoms_sync::voice_v2::VoiceError;
pub(super) fn body(
    parent: &ledger::ResolvedRow,
    key: [u8; 32],
    release: Option<[u8; 32]>,
) -> Vec<u8> {
    let (serial, version, tag) = parent.status().unwrap().action.unwrap();
    let mut bytes = vec![5];
    bytes.extend_from_slice(&serial.to_be_bytes());
    bytes.extend_from_slice(&version.to_be_bytes());
    bytes.extend_from_slice(&tag);
    bytes.extend_from_slice(&parent.confirmation_handshake().unwrap());
    bytes.extend_from_slice(&key);
    bytes.extend_from_slice(&1u64.to_be_bytes());
    bytes.push(u8::from(release.is_some()));
    if let Some(proof) = release {
        bytes.extend_from_slice(&proof);
    }
    bytes
}
pub(super) async fn exchange(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    body: &[u8],
) -> Vec<u8> {
    let header = (
        route.session().fixture_counts().1 + 1,
        12,
        0,
        parent.id().logical(),
    );
    // Exercise the real authenticated adapter after framed native channel admission.
    let reply = crate::voice_handoff::sdp::apply(
        &context::Context::new(state),
        route,
        opened_frame(public, route, header, body, None),
    )
    .await
    .unwrap();
    opened_reply(public, header, body, &reply)
}
// Keep original endpoint/parent/proof owners explicit across these independent race modes.
#[allow(clippy::too_many_arguments)]
pub(super) async fn exercise(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    key: [u8; 32],
    attach: bool,
    sdk_race: bool,
    pc_case: Option<u8>,
) -> (
    Option<super::sdk::Token>,
    Option<super::sdp_http::RetiredProbe>,
) {
    let prepare = body(parent, key, None);
    let request = requests::sdp::Request::decode(parent.id().logical(), &prepare).unwrap();
    let capacity = replies::refused(factory::Refused::Protocol(VoiceError::Capacity));
    let unauthorized = replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized));
    let commit = state.ui_session_commit.clone().lock_owned().await;
    assert_eq!(
        exchange(state, public, route, parent, &prepare).await,
        capacity
    );
    drop(commit);
    for offset in [1, 9, 17, 49, 81, 113] {
        let mut wrong = prepare.clone();
        wrong[offset] ^= 1;
        assert_eq!(
            exchange(state, public, route, parent, &wrong).await,
            unauthorized,
            "changed SDP native identity {offset}"
        );
    }
    let mut zero = prepare.clone();
    zero[113..121].fill(0);
    assert_eq!(
        exchange(state, public, route, parent, &zero).await,
        replies::refused(factory::Refused::Protocol(VoiceError::Malformed))
    );
    if attach {
        let first = exchange(state, public, route, parent, &prepare).await;
        assert_eq!(first.len(), crate::voice_handoff::sdp::REPLY_BYTES);
        assert_eq!(&first[..2], &[5, 1]);
        assert_eq!(
            exchange(state, public, route, parent, &prepare).await,
            first,
            "lost reply recovers same original storage intent"
        );
        let proof = first[138..170].try_into().unwrap();
        let phase = parent.sdp_test_phase().unwrap();
        if let Some(case) = pc_case {
            let token = super::sdk::start(state, public, route, parent, &prepare, proof).await;
            let probe = super::pc::exercise(state, public, route, parent, token, case).await;
            assert!(parent.sdp_test_whole_drained());
            return (None, probe);
        }
        let mut copy = Some(phase.copy(8192).unwrap().adopt(vec![17; 8192]).unwrap());
        let sdk = if sdk_race {
            // Isolate SDK preparation custody: no native copy may mask a missing
            // sdk_preparing term in whole_drained during the candidate unwind test.
            drop(copy.take());
            super::sdk::close_preparation_then_unwind(
                state, public, route, parent, &prepare, proof,
            )
            .await;
            None
        } else {
            Some(super::sdk::start(state, public, route, parent, &prepare, proof).await)
        };
        let observer = phase.effect();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || parent.sdp_test_release_unwind(route.session().identity(), &request, proof)
        ))
        .is_err());
        // Quiet production collection must also resume a recorded closed intent while
        // the KeyClaim/call are still current. The held real buffer still blocks drainage.
        state.voice_handoff.collect_retired();
        let closing = exchange(
            state,
            public,
            route,
            parent,
            &body(parent, key, Some(proof)),
        )
        .await;
        assert_eq!(&closing[..2], &[5, if copy.is_some() { 2 } else { 3 }]);
        assert!(!observer.is_live());
        if let Some(copy) = &copy {
            assert!(!parent.sdp_test_drained());
            assert_eq!(
                copy.bytes()[0],
                17,
                "release cannot destroy a live native owner"
            );
        } else {
            assert!(parent.sdp_test_drained());
        }
        drop(copy);
        assert!(parent.sdp_test_drained());
        let closed = exchange(state, public, route, parent, &prepare).await;
        assert_eq!(&closed[..2], &[5, 3]);
        assert_eq!(&closed[138..170], &proof);
        assert!(phase.arm().is_err());
        assert_eq!(parent.sdp_test_whole_drained(), sdk.is_none());
        (sdk, None)
    } else {
        let context = context::Context::new(state);
        factory::preflight_binding(&context, route).await.unwrap();
        let mut pending = {
            let live = context.upgrade().unwrap();
            let commit = live.commit.clone().try_lock_owned().unwrap();
            let authority = live.authority.try_lock().unwrap();
            let admission =
                crate::voice_handoff::transition::Admission::new(&live, &authority, route, &commit);
            route
                .try_with_open(|| parent.begin_sdp(&admission, &request))
                .unwrap()
                .unwrap()
        };
        assert!(
            !parent.sdp_test_drained(),
            "intent is custody BEFORE64KiB allocation"
        );
        let preparing = exchange(state, public, route, parent, &prepare).await;
        assert_eq!(&preparing[..2], &[5, 0]);
        let proof = preparing[138..170].try_into().unwrap();
        let closing = exchange(
            state,
            public,
            route,
            parent,
            &body(parent, key, Some(proof)),
        )
        .await;
        assert_eq!(&closing[..2], &[5, 2]);
        // Simulate actual allocation returning AFTER release won; only final attachment
        // could arm it, and that original native fence must refuse irreversibly.
        pending.prepare().unwrap();
        {
            let live = context.upgrade().unwrap();
            let commit = live.commit.clone().try_lock_owned().unwrap();
            let authority = live.authority.try_lock().unwrap();
            let admission =
                crate::voice_handoff::transition::Admission::new(&live, &authority, route, &commit);
            assert!(matches!(
                route.try_with_open(|| parent.attach_sdp(&admission, &request, &mut pending)),
                Err(VoiceError::Unauthorized)
            ));
        }
        assert!(!parent.sdp_test_drained());
        drop(pending);
        assert!(parent.sdp_test_drained());
        let closed = exchange(state, public, route, parent, &prepare).await;
        assert_eq!(&closed[..2], &[5, 3]);
        assert_eq!(&closed[138..170], &proof);
        (None, None)
    }
}
