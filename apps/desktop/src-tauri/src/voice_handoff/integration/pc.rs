//! Actual native PC-intent/SDK/Copy races after real blocked-TCP Finish and Ready.
//! The actor is parked by its production durable-send lease, never a timing sleep or
//! literal admission bool. No browser/PC/SDP/physical media claim is made here.
use super::*;
use catcoms_sync::voice_v2::{chain::sdp_handoff::Status, VoiceError};
async fn exchange(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    body: &[u8],
) -> Vec<u8> {
    let clock = crate::voice_runtime(state).clock();
    let capacity = replies::refused(factory::Refused::Protocol(VoiceError::Capacity));
    for attempt in 0..32 {
        let header = (
            route.session().fixture_counts().1 + 1,
            14,
            0,
            parent.id().logical(),
        );
        let reply = crate::voice_handoff::pc::apply(
            &context::Context::new(state),
            route,
            opened_frame(public, route, header, body, None),
        )
        .await
        .unwrap();
        let opened = opened_reply(public, header, body, &reply);
        if opened != capacity || attempt == 31 {
            return opened;
        }
        // Maintenance can hold the same native current fence for an admitted turn.
        // Retry ONLY its authenticated local Capacity, with the identical original PC
        // request. Original Ready and the outer fixture deadline are never extended.
        clock.sleep(std::time::Duration::from_millis(10)).await;
    }
    unreachable!("last bounded attempt returns its exact response")
}
pub(super) async fn park(
    actor: &catcoms_app::ServerActor,
) -> catcoms_app::durable_chat::DurableSendReady {
    let expected_context = actor.durable_send_context().await.unwrap();
    actor
        .prepare_durable_send(catcoms_app::durable_chat::DurableSendRequest {
            token: [71; 16],
            expected_context,
            channel: catcoms_app::channel_id("general"),
            text: "not committed".into(),
            reply_to: String::new(),
        })
        .await
        .unwrap()
}
fn release_sdk(parent: &ledger::ResolvedRow, token: &super::sdk::Token) {
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
}
pub(super) async fn exercise(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    token: super::sdk::Token,
    case: u8,
) {
    let mut body = token.body[..153].to_vec();
    body[0] = 7;
    body.extend_from_slice(&token.generation.to_be_bytes());
    body.extend_from_slice(&token.proof);
    body.push(0);
    let request = requests::pc::Request::decode(parent.id().logical(), &body).unwrap();
    let context = context::Context::new(state);
    let phase = parent.sdp_test_phase().unwrap();
    let unauthorized = replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized));
    for offset in [1, 9, 17, 49, 81, 113, 121, 153, 161, 192] {
        let mut wrong = body.clone();
        wrong[offset] ^= 1;
        assert_eq!(
            exchange(state, public, route, parent, &wrong).await,
            unauthorized,
            "changed native PC obligation {offset}"
        );
    }
    if case == 0 {
        release_sdk(parent, &token);
        let incoming = opened_frame(
            public,
            route,
            (
                route.session().fixture_counts().1 + 1,
                14,
                0,
                parent.id().logical(),
            ),
            &body,
            None,
        );
        let physical = incoming.hold_position().unwrap();
        assert!(
            matches!(
                crate::voice_handoff::sdp::current(&context, route, |admission| parent.begin_pc(
                    admission,
                    &physical,
                    route.session(),
                    &request
                )),
                Err(factory::Refused::Protocol(VoiceError::Unauthorized))
            ),
            "fresh begin independently refuses an already released SDK"
        );
        drop(physical);
        drop(incoming);
        assert_eq!(
            exchange(state, public, route, parent, &body).await,
            vec![7, 2, 0],
            "SDK Release wins before PC intent without creating a PC identity"
        );
        body[193] = 1;
        assert_eq!(
            exchange(state, public, route, parent, &body).await,
            vec![7, 2, 0]
        );
        assert_eq!(phase.status(), Status::Drained);
        return;
    }
    factory::preflight_binding(&context, route).await.unwrap();
    if matches!(case, 1 | 4 | 5 | 6 | 7) {
        let first = exchange(state, public, route, parent, &body).await;
        assert_eq!(
            first.len(),
            85,
            "actual native PC endpoint admits after genuine Ready; response {first:?}"
        );
        assert_eq!(&first[..3], &[7, 1, 1]);
        let second = exchange(state, public, route, parent, &body).await;
        assert_eq!(
            &first[..77],
            &second[..77],
            "lost reply recovers exact PC/Ready identity"
        );
        assert!(
            u64::from_be_bytes(second[77..85].try_into().unwrap())
                <= u64::from_be_bytes(first[77..85].try_into().unwrap())
        );
    }
    let incoming = opened_frame(
        public,
        route,
        (
            route.session().fixture_counts().1 + 1,
            14,
            0,
            parent.id().logical(),
        ),
        &body,
        None,
    );
    let physical = incoming.hold_position().unwrap();
    let preparation = if matches!(case, 1 | 4 | 5 | 6 | 7) {
        None
    } else {
        Some(
            crate::voice_handoff::sdp::current(&context, route, |admission| {
                parent.begin_pc(admission, &physical, route.session(), &request)
            })
            .unwrap()
            .unwrap(),
        )
    };
    let (actor, grant) = factory::native::observation_actor(&context, route).unwrap();
    let custody = if let Some(preparation) = &preparation {
        preparation.custody.clone()
    } else {
        parent
            .pc_test_keys(route.session().identity(), &request)
            .unwrap()
    };
    let work = phase.copy(4096).unwrap();
    let paused = if matches!(case, 2 | 3) {
        Some(park(&actor).await)
    } else {
        None
    };
    let pending = actor
        .queue_voice_initial_sdp_intent(
            grant.clone(),
            request.base.handshake,
            custody,
            phase.effect(),
            1,
            work,
        )
        .await
        .unwrap();
    let mut unstarted_sdp = None;
    // Successful queue return is the positive barrier: the original command, rather than
    // an unpolled future, owns the same phase contribution behind the paused actor.
    if matches!(case, 1 | 4 | 5 | 6 | 7) {
        let completion = pending.complete().await.unwrap();
        assert!(completion.result().is_ok());
        // A new authenticated frame supplies its own physical hold for actual typed SDP
        // RPC tests. The original PC-intent request is already collected here.
        drop(physical);
        drop(incoming);
        if case == 7 {
            let retained = Box::pin(super::sdp_outcome::exercise(
                state,
                public,
                route,
                parent,
                &body,
                &actor,
                &grant,
                completion.result().unwrap(),
            ))
            .await;
            parent
                .close_pc(route.session().identity(), &request)
                .unwrap();
            release_sdk(parent, &token);
            assert_eq!(phase.status(), Status::Closing);
            drop(completion);
            assert_eq!(phase.status(), Status::Drained);
            assert!(parent.sdp_test_whole_drained());
            if let Some(slot) = retained {
                use catcoms_sync::voice_v2::sdp::publication::History;
                let History::Published(action) = slot.history() else {
                    panic!("lost original publication after PC/SDK teardown");
                };
                assert!(action.matches_sdp_plaintext_commitment(
                    &catcoms_sync::voice_v2::sdp::plaintext_commitment(b"v=0").unwrap()
                ));
                action.retire_if_unstarted();
                assert!(action.is_settled());
            }
            return;
        }
        // Keep the test-only nested stopped-Actor setup out of every ancestor TCP future.
        super::sdp_intake::exercise(state, public, route, parent, &body);
        unstarted_sdp = Some(
            Box::pin(super::sdp_rows::exercise(
                state,
                public,
                route,
                parent,
                &body,
                &actor,
                &grant,
                completion.result().unwrap(),
                match case {
                    4 => 1,
                    5 => 2,
                    6 => 3,
                    _ => 0,
                },
            ))
            .await,
        );
        // This is fixture heap storage, not production task payment or a larger stack.
        let held = Box::pin(super::sdp_actor::exercise(
            state,
            public,
            route,
            parent,
            &body,
            &actor,
            &grant,
            completion.result().unwrap(),
        ))
        .await;
        let proof = parent
            .close_pc(route.session().identity(), &request)
            .unwrap()
            .proof;
        assert!(proof.is_some());
        release_sdk(parent, &token);
        assert_eq!(phase.status(), Status::Closing);
        assert!(!completion.result().unwrap().current_at(1_000, 1_000));
        drop(completion);
        assert_eq!(
            phase.status(),
            Status::Closing,
            "actual SDP body/result still owns publication"
        );
        drop(held.0);
        assert_eq!(
            phase.status(),
            Status::Closing,
            "returned SDP completion is still uncollected"
        );
        drop(held.1);
    } else {
        let proof = parent
            .close_pc(route.session().identity(), &request)
            .unwrap()
            .proof;
        assert!(proof.is_some());
        assert_eq!(
            phase.status(),
            Status::Closing,
            "actual queued RPC still owns its Copy"
        );
        release_sdk(parent, &token);
        assert!(!phase.effect().is_live());
        assert_eq!(phase.status(), Status::Closing);
        if case == 3 {
            drop(pending); // Lost waiter does not destroy the queued Command's Copy.
            drop(paused);
            actor
                .voice_v2_context(route.selected().binding().scope.room)
                .await
                .unwrap();
        } else {
            assert_eq!(case, 2);
            drop(paused);
            let late = pending.complete().await.unwrap();
            assert_eq!(
                late.result().err(),
                Some(VoiceError::Unauthorized),
                "late unknown Root intent cannot cross persisted native Phase close"
            );
            assert_eq!(phase.status(), Status::Closing);
            drop(late);
        }
        drop(physical);
        drop(incoming);
    }
    assert_eq!(phase.status(), Status::Drained);
    if let Some(retained) = unstarted_sdp {
        retained.after_parent_close(state, route, parent);
        assert_eq!(
            phase.status(),
            Status::Drained,
            "historical child must not retain variable reserve/Copies"
        );
    }
    if preparation.is_some() {
        assert!(!parent.sdp_test_whole_drained());
    }
    drop(preparation); // Actual native finally, distinct from queued/result Copy drainage.
    assert!(parent.sdp_test_whole_drained());
}
