//! Actual native/Actor admission after the real browser HTTP/TCP Offer/Answer round.
//! The fixture does not move a browser peer: its admitted media offer is explicitly
//! abandoned before the original browser SDK closes. This is native scope/race evidence,
//! not op28 HTTP, physical media or browser ownership-transfer acceptance.
use super::*;
use crate::voice_handoff::requests::media::{ExistingKind, OriginalKind};

pub(super) struct AfterSend {
    sent: oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}
impl AfterSend {
    pub(super) fn park(self) {
        self.sent.send(()).unwrap();
        self.release
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("fixture did not release original media sender");
    }
}

#[test]
fn complete_native_media_callback_graph_and_original_actor_payment_fit_without_cap_changes() {
    let native = task_layout();
    let actor = catcoms_app::voice_initial_media_rpc_bytes();
    eprintln!("native media task {native}/{TASK_BYTES}, actor {actor}/{VOICE_INITIAL_MEDIA_RPC_BYTES}, control {}",layout_bytes());
    assert!(native <= TASK_BYTES);
    assert!(actor <= VOICE_INITIAL_MEDIA_RPC_BYTES);
}

pub(in crate::voice_handoff) async fn exercise(
    state: &crate::AppState,
    route: &Route,
    parent: &ResolvedRow,
) -> Arc<catcoms_sync::voice_v2::CallGrant> {
    let context = Context::new(state);
    let (actor, grant) = factory::native::observation_actor(&context, route).unwrap();
    let room = route.selected().binding().scope.room;
    let original = || parent.media_test_original(route.session().identity());
    let row = || {
        state
            .voice_handoff
            .resolve_sdp_parent(route.selected(), parent.id())
            .unwrap()
    };
    // Real physical exclusivity, without consuming HTTP nonces owned by the still-live
    // browser broker. Shipping requests obtain this only from authenticated Incoming.
    let physical = || {
        let header = route.session().header().unwrap();
        header.fixture_scope_hold().unwrap()
    };
    let history = || {
        parent
            .media_original_history(
                route.session().identity(),
                &original(),
                OriginalKind::Resolve,
            )
            .unwrap()
            .unwrap()
    };

    // A new same-byte grant must not be accepted as the original native actor authority.
    // This goes through the PUBLIC ChannelSync wrapper, not just LocalJoin's private gate.
    let foreign = Arc::new(catcoms_sync::voice_v2::CallGrant::from_lease(
        grant.lease(),
        grant.scope(),
    ));
    let waiter = queue_impl(
        &context,
        route,
        row(),
        original(),
        physical(),
        // CallGrant is affine: dropping even this forged wrapper revokes its shared
        // lease. Keep it through the scenario so rejection alone doesn't End the call.
        QueueOptions {
            grant: Some(foreign.clone()),
            after_send: None,
        },
    )
    .unwrap();
    let output = waiter.complete().await.unwrap();
    assert!(matches!(
        output.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    drop(output);
    let child = parent
        .media_test_root()
        .expect("retry must retain the irreversible child");
    assert!(!child.was_admitted());

    // Full mailbox fails locally, with the exact same child retained for another attempt.
    let parked = super::super::integration::pc::park(&actor).await;
    let mut queries = Vec::new();
    loop {
        match actor.try_queue_voice_v2_context(room) {
            Ok(query) => queries.push(query),
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity)) => break,
            Err(error) => panic!("unexpected media mailbox error {error:?}"),
        }
    }
    let output = queue(&context, route, row(), original(), physical())
        .unwrap()
        .complete()
        .await
        .unwrap();
    assert!(matches!(
        output.result,
        Err(Refused::Protocol(VoiceError::Capacity))
    ));
    drop((output, queries, parked));
    actor.voice_v2_context(room).await.unwrap();
    assert!(Arc::ptr_eq(&child, &parent.media_test_root().unwrap()));
    assert!(!child.was_admitted());

    // Queuing isn't permission: UI lock wins AFTER enqueue and BEFORE final admission.
    let parked = super::super::integration::pc::park(&actor).await;
    let waiter = queue(&context, route, row(), original(), physical()).unwrap();
    assert!(waiter.work.is_some());
    let commit = state.ui_session_commit.clone().lock_owned().await;
    drop(parked);
    let output = waiter.complete().await.unwrap();
    assert!(matches!(
        output.result,
        Err(Refused::Protocol(VoiceError::Capacity))
    ));
    drop((output, commit));
    assert!(!history().admitted);
    assert!(Arc::ptr_eq(&child, &parent.media_test_root().unwrap()));

    // Lose the observer while an admitted command is parked. The actual producer owns
    // physical exclusivity and its fee until callback return, independently of this waiter.
    let parked = super::super::integration::pc::park(&actor).await;
    let (sent, observed) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    let waiter = queue_impl(
        &context,
        route,
        row(),
        original(),
        physical(),
        QueueOptions {
            grant: None,
            after_send: Some(AfterSend {
                sent,
                release: wait,
            }),
        },
    )
    .unwrap();
    assert!(waiter.work.is_some());
    assert!(matches!(
        route.session().header(),
        Err(channel::Refused::Capacity)
    ));
    drop(parked);
    observed.await.unwrap();
    let Waiter {
        reply,
        work,
        _custody,
    } = waiter;
    let output = reply.await.unwrap();
    assert!(
        output.result.is_ok()
            || matches!(output.result, Err(Refused::Protocol(VoiceError::Capacity)))
    );
    // Drop every receiving owner while the actual sender is still returning.
    drop((output, work, _custody));
    assert!(matches!(
        route.session().header(),
        Err(channel::Refused::Capacity)
    ));
    assert!(
        state.ui_session_commit.try_lock().is_ok(),
        "sender tail must hold no native final guard"
    );
    release.send(()).unwrap();
    actor.voice_v2_context(room).await.unwrap();
    route.session().fixture_wait_physical_idle().await;
    actor.voice_v2_context(room).await.unwrap();
    // Maintenance may temporarily own a try-only final gate. Retry ONLY a known local
    // refusal, on the same stored child and original deadline, never a fresh reservation.
    for _ in 0..32 {
        if history().admitted {
            break;
        }
        let output = queue(&context, route, row(), original(), physical())
            .unwrap()
            .complete()
            .await
            .unwrap();
        assert!(
            output.result.is_ok()
                || matches!(output.result, Err(Refused::Protocol(VoiceError::Capacity)))
        );
        drop(output);
    }
    let recovered = history();
    assert!(recovered.admitted && !recovered.current);
    assert!(child.was_admitted());
    assert!(Arc::ptr_eq(&child, &parent.media_test_root().unwrap()));
    let (_, mono) = observe_time(&context).unwrap();
    let current = super::super::sdp::current(&context, route, |admission| {
        parent.media_current(admission, original().pc.base.action, recovered.proof, mono)
    })
    .unwrap();
    assert!(current.current && current.retire_in.is_some());
    assert!(
        current.retire_in.unwrap() > 0,
        "guard test must precede the ORIGINAL Source deadline"
    );
    let session = route.session().identity();
    // These are the ACTUAL cleanup methods, with their genuine original bindings. None
    // may cancel still-authorized Answer retries just because local RTC already succeeded.
    assert!(matches!(
        parent.close_pc(session, &original().pc),
        Err(VoiceError::Capacity)
    ));
    let pc = original().pc;
    let sdk = super::super::requests::sdk::Request {
        base: pc.base,
        storage: pc.storage,
        kind: super::super::requests::sdk::Kind::Release {
            generation: pc.sdk_generation,
            proof: pc.sdk,
        },
    };
    assert!(matches!(
        parent.release_sdk(session, &sdk, pc.sdk_generation, pc.sdk),
        Err(VoiceError::Capacity)
    ));
    let pc = original().pc;
    assert!(matches!(
        parent.release_sdp(session, &pc.base, pc.storage),
        Err(VoiceError::Capacity)
    ));
    let pc = original().pc;
    assert!(matches!(
        parent.release_key_session(
            session,
            pc.base.action,
            pc.base.handshake,
            parent.media_test_transition_proof(),
            pc.base.key
        ),
        Err(VoiceError::Capacity)
    ));
    assert!(current.current && child.is_current());

    // This test has NOT transferred the original browser PC. Abandon this offered media
    // ownership explicitly; its original SDK obligation still pays the actual browser PC.
    let closed = parent
        .media_existing_history(
            route.session().identity(),
            original().pc.base.action,
            recovered.proof,
            ExistingKind::Close,
        )
        .unwrap();
    assert!(closed.admitted && closed.closed && !closed.current && !closed.released);
    let released = parent
        .media_existing_history(
            route.session().identity(),
            original().pc.base.action,
            recovered.proof,
            ExistingKind::Release,
        )
        .unwrap();
    assert!(released.admitted && released.closed && released.released && !released.current);
    assert!(parent.media_test_root().is_none());
    assert!(!child.is_current());
    assert!(
        grant.lease().is_live(),
        "negative identity test must not revoke the original call"
    );
    // Endpoint holds this affine duplicate until actual End, also on fixture failure.
    foreign
}

/// No first media effect occurred. A lost/negative media command must not introduce a
/// separate SDK release debt. Endpoint's actual End -> SDK drain -> Final/GC proves cleanup.
pub(in crate::voice_handoff) fn negative_before_admission(route: &Route, parent: &ResolvedRow) {
    let original = parent.media_test_original(route.session().identity());
    assert!(parent
        .media_original_history(route.session().identity(), &original, OriginalKind::Resolve)
        .unwrap()
        .is_none());
    let closed = parent
        .media_original_history(route.session().identity(), &original, OriginalKind::Close)
        .unwrap()
        .unwrap();
    assert!(closed.closed && !closed.admitted && !closed.released);
    assert!(parent.media_test_root().is_none());
    let recovered = parent
        .media_original_history(route.session().identity(), &original, OriginalKind::Resolve)
        .unwrap()
        .unwrap();
    assert!(recovered.closed && !recovered.admitted && !recovered.released);
}
