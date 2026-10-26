//! Actual framed retention-only adoption after actor-committed Finish on the TCP fixture.
use super::*;
use catcoms_sync::voice_v2::VoiceError;

pub(super) fn body(
    parent: &ledger::ResolvedRow,
    claim: [u8; 32],
    release: Option<[u8; 32]>,
) -> Vec<u8> {
    let mut bytes = super::transition::body(parent, None);
    bytes[0] = 4;
    bytes[185] = u8::from(release.is_some());
    bytes.extend_from_slice(&claim);
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
        11,
        0,
        parent.id().logical(),
    );
    let reply = crate::voice_handoff::key_session::apply(
        &context::Context::new(state),
        route,
        opened_frame(public, route, header, body, None),
    )
    .await
    .unwrap();
    opened_reply(public, header, body, &reply)
}
pub(super) async fn adopt(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    claim: [u8; 32],
) -> [u8; 32] {
    let body = body(parent, claim, None);
    let capacity = replies::refused(factory::Refused::Protocol(VoiceError::Capacity));
    let unauthorized = replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized));
    let commit = state.ui_session_commit.clone().lock_owned().await;
    assert_eq!(
        exchange(state, public, route, parent, &body).await,
        capacity,
        "real final UI admission contention cannot publish key custody"
    );
    drop(commit);
    for offset in [1, 33, 65, 104, 105, 113, 152, 184, 186, 217] {
        let mut wrong = body.clone();
        wrong[offset] ^= 1;
        assert_eq!(
            exchange(state, public, route, parent, &wrong).await,
            unauthorized,
            "changed original key adoption binding at {offset}"
        );
    }
    let clock = crate::voice_runtime(state).clock();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let reply = exchange(state, public, route, parent, &body).await;
            if reply == capacity {
                clock.sleep(std::time::Duration::from_millis(10)).await;
                continue;
            }
            break reply;
        }
    })
    .await
    .expect("original established-key adoption stalled");
    assert_eq!(first.len(), crate::voice_handoff::key_session::REPLY_BYTES);
    assert_eq!(&first[..2], &[4, 1]);
    assert_eq!(&first[2..18], &parent.id().logical());
    assert_eq!(&first[170..202], &claim);
    let proof = first[202..234].try_into().unwrap();
    // Model an ignored authenticated reply: exact retry returns the one original receipt.
    assert_eq!(exchange(state, public, route, parent, &body).await, first);
    assert!(matches!(parent.retire_plan(), Err(VoiceError::Capacity)));
    // Plan intent now closes first, even while this obligation awaits Release. It is
    // therefore destructive, not a harmless readiness probe. Without that intent no
    // final proof may be issued; dedicated fence scenarios exercise early Pending.
    assert!(matches!(
        parent.issue_final(route.session().identity()),
        Err(VoiceError::Unauthorized)
    ));
    proof
}
pub(super) fn foreign_refuses(
    parent: &ledger::ResolvedRow,
    foreign: &bootstrap::Route,
    claim: [u8; 32],
    proof: [u8; 32],
) {
    let (serial, version, input_tag) = parent.status().unwrap().action.unwrap();
    let action = requests::Action {
        serial,
        version,
        input_tag,
    };
    let handshake = parent.confirmation_handshake().unwrap();
    assert!(matches!(
        parent.recover_key_session(foreign.session().identity(), action, handshake, claim),
        Err(VoiceError::Unauthorized)
    ));
    assert!(matches!(
        parent.release_key_session(
            foreign.session().identity(),
            action,
            handshake,
            claim,
            proof
        ),
        Err(VoiceError::Unauthorized)
    ));
}
pub(super) async fn release(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    claim: [u8; 32],
    proof: [u8; 32],
    ended: bool,
) {
    let original = parent.key_custody_for_test();
    let current_before = original.is_current();
    let recovered = exchange(state, public, route, parent, &body(parent, claim, None)).await;
    assert_eq!(&recovered[..2], &[4, if ended { 2 } else { 1 }]);
    assert_eq!(
        &recovered[202..234],
        &proof,
        "End cannot replace adoption history"
    );
    let mut wrong = proof;
    wrong[0] ^= 1;
    assert_eq!(
        exchange(
            state,
            public,
            route,
            parent,
            &body(parent, claim, Some(wrong))
        )
        .await,
        replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized))
    );
    assert_eq!(
        original.is_current(),
        current_before,
        "an invalid release must not close the original Root successor binding"
    );
    for _ in 0..2 {
        let released = exchange(
            state,
            public,
            route,
            parent,
            &body(parent, claim, Some(proof)),
        )
        .await;
        assert_eq!(&released[..2], &[4, 3]);
        assert_eq!(&released[202..234], &proof);
        assert!(
            !original.is_current(),
            "native Released must fence the exact Root observer a parked worker retained"
        );
        assert!(matches!(
            original.prepare_sdp_handoff(),
            Err(VoiceError::Unauthorized)
        ));
    }
    let terminal = exchange(state, public, route, parent, &body(parent, claim, None)).await;
    assert_eq!(
        &terminal[..2],
        &[4, 3],
        "late adoption never reopens released key custody"
    );
}
