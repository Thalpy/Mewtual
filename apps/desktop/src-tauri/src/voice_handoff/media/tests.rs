//! Actual native/Actor admission after the real browser HTTP/TCP Offer/Answer round.
//! The fixture does not move a browser peer: its admitted media offer is explicitly
//! abandoned before the original browser SDK closes. This is native scope/race evidence,
//! not op28 HTTP, physical media or browser ownership-transfer acceptance.
use super::*;
use crate::voice_handoff::requests::media::{ExistingKind, OriginalKind};

type DropCheckpoint = Arc<std::sync::Mutex<Option<tokio::sync::OwnedMutexGuard<()>>>>;
type DropEvidence = Arc<std::sync::Mutex<Option<((usize, usize), bool)>>>;

fn candidate_drop_observer(
    checkpoint: DropCheckpoint,
    observation: DropEvidence,
) -> Box<dyn FnOnce((usize, usize))> {
    Box::new(move |counts| {
        // Drop observers must never assert: an earlier failed oracle may already be
        // unwinding. Record evidence, return every guard, and assert after cleanup.
        // The actual UI guard is acquired ONLY after sdp::current returns.
        let guard = checkpoint.lock().unwrap_or_else(|e| e.into_inner()).take();
        let after_ui_return = guard.is_some();
        drop(guard);
        *observation.lock().unwrap_or_else(|e| e.into_inner()) = Some((counts, after_ui_return));
    })
}

#[test]
fn unused_index_candidate_drop_oracle_records_missing_or_poisoned_checkpoint_without_panicking() {
    let checkpoint: DropCheckpoint = Arc::new(std::sync::Mutex::new(None));
    let observation: DropEvidence = Arc::new(std::sync::Mutex::new(None));
    // An unexpected commit/refusal can unwind before installing the guard. This exact
    // observer must report false rather than turn that test failure into process abort.
    candidate_drop_observer(checkpoint.clone(), observation.clone())((2, 3));
    assert_eq!(*observation.lock().unwrap(), Some(((2, 3), false)));
    for poison_checkpoint in [true, false] {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if poison_checkpoint {
                let _guard = checkpoint.lock().unwrap_or_else(|e| e.into_inner());
                panic!("simulate an earlier fixture assertion while observing checkpoint");
            } else {
                let _guard = observation.lock().unwrap_or_else(|e| e.into_inner());
                panic!("simulate an earlier fixture assertion while observing evidence");
            }
        }));
        assert!(result.is_err());
        candidate_drop_observer(checkpoint.clone(), observation.clone())((7, 11));
        assert_eq!(
            *observation.lock().unwrap_or_else(|e| e.into_inner()),
            Some(((7, 11), false))
        );
    }
}

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
    exercise_impl(state, route, parent, false).await
}
pub(in crate::voice_handoff) async fn exercise_index_drop(
    state: &crate::AppState,
    route: &Route,
    parent: &ResolvedRow,
) -> Arc<catcoms_sync::voice_v2::CallGrant> {
    exercise_impl(state, route, parent, true).await
}
async fn exercise_impl(
    state: &crate::AppState,
    route: &Route,
    parent: &ResolvedRow,
    index_drop: bool,
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
    let key = parent.key_custody_for_test();
    assert!(key.is_current());
    assert!(matches!(
        super::super::plan_final::request(&Context::new(state), route, parent).await,
        Err(super::super::factory::Refused::Protocol(
            VoiceError::Capacity
        ))
    ));
    assert!(
        key.is_current() && child.is_current(),
        "refused early Plan fence must not close the original key binding or live Answer Source"
    );

    let mut child = Some(child); // Preserve the previous fixture's intentional owner unless probing last Drop.
    let mut prepared = None;
    let checkpoint = Arc::new(std::sync::Mutex::new(None));
    let observation = Arc::new(std::sync::Mutex::new(None));
    let weak = Arc::downgrade(child.as_ref().unwrap());
    let mut physical_index = None;
    if index_drop {
        let clock = crate::voice_runtime(state).clock();
        while clock.monotonic_ms() < parent.media_test_until() {
            clock.sleep(std::time::Duration::from_millis(10)).await;
        }
        // Real same-Session physical exclusion spans preparation and unused candidate Drop.
        physical_index = Some(physical());
        let mut candidate = parent
            .prepare_media_index(
                route.selected().clone(),
                session.clone(),
                original().pc.base.action,
                recovered.proof,
                clock.monotonic_ms(),
            )
            .unwrap();
        candidate.observe_drop(candidate_drop_observer(
            checkpoint.clone(),
            observation.clone(),
        ));
        prepared = Some(candidate);
        drop(child.take());
    }
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
    assert!(child.as_ref().is_none_or(|child| !child.is_current()));
    if let Some(prepared) = prepared {
        let clock = crate::voice_runtime(state).clock();
        let limit = clock.monotonic_ms() + 1000;
        let refused = loop {
            let mono = clock.monotonic_ms();
            match super::super::sdp::current(&context, route, |admission| {
                prepared.commit(parent, admission, mono)
            }) {
                Err(Refused::Protocol(VoiceError::Capacity)) if mono < limit => {
                    clock.sleep(std::time::Duration::from_millis(5)).await
                }
                value => break value,
            }
        };
        let unauthorized = matches!(refused, Err(Refused::Protocol(VoiceError::Unauthorized)));
        let not_dropped = observation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none();
        // A retained UI guard is itself a failure. Bound its acquisition through the
        // sanctioned Clock so that the regression reports rather than hangs forever.
        let guard = tokio::select! {
            guard = state.ui_session_commit.clone().lock_owned() => Some(guard),
            _ = clock.sleep(std::time::Duration::from_secs(1)) => None,
        };
        *checkpoint.lock().unwrap_or_else(|e| e.into_inner()) = guard;
        drop(prepared);
        let actual = observation.lock().unwrap_or_else(|e| e.into_inner()).take();
        let root_dropped = weak.upgrade().is_none();
        // No assertion may unwind with the candidate or its physical position retained.
        drop(physical_index.take());
        assert!(
            unauthorized,
            "Release must defeat the prepared final admission"
        );
        assert!(
            not_dropped,
            "preparation must outlive its borrowed final callback"
        );
        assert_eq!(
            actual,
            Some(((1, 1), true)),
            "actual last Entry/media owner must drop after the final UI callback returns"
        );
        assert!(
            root_dropped,
            "last candidate dropped the actual media allocation's strong owner"
        );
    }
    drop(physical_index);
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
