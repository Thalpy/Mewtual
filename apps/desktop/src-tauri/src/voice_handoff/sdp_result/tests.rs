//! Real original Result row/Phase/Actor and private authenticated class2 frames after
//! blocked-TCP Finish/Ready. AEAD output is mocked; no browser SDK or RTC claim.
use super::*;
use crate::voice_handoff::service::tests::{
    authenticated_header, opened_frame, opened_reply, signed_frame,
};
use catcoms_app::VoiceEndpointError;
use catcoms_sync::voice_v2::chain::sdp_handoff::{NativeRowFunding, NativeRowPurpose};
use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn sdp_result_concrete_worker_publisher_and_observer_fit_original_task_payment() {
    let bytes = task_layout();
    eprintln!("actual native SDP Result graph {bytes}/{TASK_BYTES}");
    assert!(bytes <= TASK_BYTES);
}
fn body(
    parent: super::super::ledger::RowId,
    action: Action,
    tag: [u8; 32],
    payload: &[u8],
) -> Vec<u8> {
    let mut body = vec![8];
    body.extend_from_slice(&parent.logical());
    body.push(0);
    body.extend_from_slice(&action.serial.to_be_bytes());
    body.extend_from_slice(&action.version.to_be_bytes());
    body.extend_from_slice(&action.input_tag);
    body.extend_from_slice(&tag);
    body.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    body.extend_from_slice(payload);
    body
}
fn opened(
    state: &crate::AppState,
    public: &super::super::bootstrap::SetupOutput,
    route: &Route,
    child: super::super::ledger::RowId,
    body: &[u8],
) -> Incoming {
    opened_operation(state, public, route, child, body, 19)
}
fn opened_operation(
    state: &crate::AppState,
    public: &super::super::bootstrap::SetupOutput,
    route: &Route,
    child: super::super::ledger::RowId,
    body: &[u8],
    operation: u8,
) -> Incoming {
    let header = (
        route.session().fixture_counts().1 + 1,
        operation,
        2,
        child.logical(),
    );
    let wire = signed_frame(public, header, body);
    let authenticated = authenticated_header(route, &wire);
    let permit = state
        .voice_handoff
        .sdp_result_intake(
            route.selected(),
            route.session().identity(),
            &authenticated,
            crate::voice_runtime(state).clock(),
        )
        .unwrap();
    let mut incoming = authenticated.admit_sdp(permit).unwrap();
    let count = incoming.remaining_mut().len();
    incoming
        .remaining_mut()
        .copy_from_slice(&wire[wire.len() - count..]);
    incoming.advance(count).unwrap();
    assert_eq!(incoming.open().unwrap(), body);
    incoming
}

/// Retry only an authenticated prepublication Capacity with the SAME original output
/// and action under one original intent deadline. Real maintenance may hold a short
/// native gate; a legal refusal is not failed terminal acceptance or SDK reexecution.
#[allow(clippy::too_many_arguments)]
async fn exchange(
    state: &crate::AppState,
    public: &super::super::bootstrap::SetupOutput,
    route: &Route,
    child: super::super::ledger::RowId,
    encoded: &[u8],
    operation: u8,
    intent: &Arc<catcoms_sync::voice_v2::sdp::InitialSdpIntent>,
) -> Vec<u8> {
    let clock = crate::voice_runtime(state).clock();
    let remaining = intent
        .remaining_at(clock.now_ms(), clock.monotonic_ms())
        .unwrap();
    let deadline = clock.monotonic_ms().saturating_add(remaining);
    let capacity = super::super::sdp_replies::refused(VoiceError::Capacity.into());
    let attempts = async {
        loop {
            let h = (
                route.session().fixture_counts().1 + 1,
                operation,
                if matches!(operation, 19 | 20) { 2 } else { 0 },
                child.logical(),
            );
            let incoming = if matches!(operation, 19 | 20) {
                opened_operation(state, public, route, child, encoded, operation)
            } else {
                opened_frame(public, route, h, encoded, None)
            };
            let context = Context::new(state);
            let reply = if operation == 18 {
                super::super::sdp_engine::admit(&context, route, incoming).await
            } else {
                apply(&context, route, incoming).await
            }
            .unwrap();
            let result = opened_reply(public, h, encoded, &reply);
            drop(reply);
            assert!(
                clock.monotonic_ms() < deadline,
                "native response exceeded original intent bound"
            );
            if result != capacity {
                return result;
            }
            assert!(
                clock.monotonic_ms() < deadline,
                "exact original native Result/engine stayed busy"
            );
            clock.sleep(std::time::Duration::from_millis(10)).await;
        }
    };
    // Cover the WHOLE enqueue/Actor receive/response/retry loop, including an Actor
    // which never returns. The first deadline is never renewed by exact retries.
    tokio::select! {
        biased;
        _ = clock.sleep(std::time::Duration::from_millis(remaining)) => panic!("original native Result/engine observer bound exceeded"),
        result = attempts => result,
    }
}
struct NeverApplied {
    funding: Arc<NativeRowFunding>,
    dropped: Arc<AtomicBool>,
}
impl Drop for NeverApplied {
    fn drop(&mut self) {
        assert!(
            self.funding.in_flight() > 0,
            "affine publisher outlived its original payment"
        );
        self.dropped.store(true, Ordering::Release);
    }
}
impl VoiceSdpResultPublisher for NeverApplied {
    fn publish(self: Box<Self>, _: SdpResultAdmission<'_>) -> Result<(), VoiceError> {
        panic!("abandoned/historical request must not invoke a final publisher")
    }
}

/// Park AFTER actual native guards released and the terminal committed. Losing the
/// real reply at this point cannot establish prepublication refusal.
struct AfterCommit {
    original: Option<Box<Publisher>>,
    committed: Option<tokio::sync::oneshot::Sender<()>>,
    release: std::sync::mpsc::Receiver<()>,
    _custody: Arc<Custody>,
}
impl VoiceSdpResultPublisher for AfterCommit {
    fn publish(mut self: Box<Self>, step: SdpResultAdmission<'_>) -> Result<(), VoiceError> {
        self.original.take().unwrap().publish(step)?;
        self.committed.take().unwrap().send(()).unwrap();
        self.release
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("fixture did not release committed publisher");
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::voice_handoff) async fn exercise(
    state: &crate::AppState,
    public: &super::super::bootstrap::SetupOutput,
    route: &Route,
    parent_row: &ResolvedRow,
    _pc_frame: &[u8],
    child: super::super::ledger::RowId,
    action: Action,
    lose_reply: bool,
) {
    let context = Context::new(state);
    let (actor, _) = factory::native::observation_actor(&context, route).unwrap();
    let parent = parent_row.id();
    let row = || {
        state
            .voice_handoff
            .resolve(
                route.selected(),
                super::super::ledger::Purpose::Sdp {
                    parent,
                    purpose: NativeRowPurpose::Seal,
                },
                child,
            )
            .unwrap()
    };
    let original = row()
        .sdp_result_action(route.session().identity(), action)
        .unwrap();
    let owners = row()
        .sdp_input_owners(route.session().identity(), action)
        .unwrap();
    let funding = owners.funding.clone();
    let payload = [42; 19];
    let tag = original.output_tag(&payload).unwrap();
    let encoded = body(parent, action, tag, &payload);
    let expected = Expected {
        action,
        identity: SdpResultIdentity::Bytes(tag),
        seal: true,
    };

    // Header-only intake refuses foreign Session/namespace/purpose BEFORE allocating
    // any class2 backing or spending either physical nonce.
    for (logical, operation) in [(parent.logical(), 19), (child.logical(), 20)] {
        let header = (
            route.session().fixture_counts().1 + 1,
            operation,
            2,
            logical,
        );
        let wire = signed_frame(public, header, &encoded);
        let authenticated = authenticated_header(route, &wire);
        let before = route.session().fixture_counts();
        assert!(state
            .voice_handoff
            .sdp_result_intake(
                route.selected(),
                route.session().identity(),
                &authenticated,
                crate::voice_runtime(state).clock()
            )
            .is_err());
        assert_eq!(route.session().fixture_counts(), before);
    }
    {
        let header = (
            route.session().fixture_counts().1 + 1,
            19,
            2,
            child.logical(),
        );
        let wire = signed_frame(public, header, &encoded);
        let authenticated = authenticated_header(route, &wire);
        let foreign = Arc::new(channel::SessionIdentity([77; 32]));
        assert!(state
            .voice_handoff
            .sdp_result_intake(
                route.selected(),
                &foreign,
                &authenticated,
                crate::voice_runtime(state).clock()
            )
            .is_err());
    }

    // The actual opened allocation survives a Body-constructor Capacity refusal.
    let mut incoming = opened(state, public, route, child, &encoded);
    incoming.fixture_underpay_sdp_rpc(&owners.phase, &funding);
    let refused = construct(&context, route, &mut incoming, row(), expected, &original);
    assert!(matches!(
        refused,
        Err(Refused::Protocol(VoiceError::Capacity))
    ));
    assert_eq!(incoming.plain().unwrap(), encoded);
    assert_eq!(
        funding.in_flight(),
        1,
        "only identical restored buffer remains"
    );
    drop(incoming);
    assert_eq!(funding.in_flight(), 0);

    // Eager unpolled, capacity-wait cancellation and queued-but-abandoned paths all
    // retain the original publisher before the actual Body/Task payment can retire.
    for mode in 0..3 {
        let mut incoming = opened(state, public, route, child, &encoded);
        let task = construct(&context, route, &mut incoming, row(), expected, &original).unwrap();
        let (_, grant) = factory::native::observation_actor(&context, route).unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let parked = (mode != 0).then_some(());
        let parked_actor = if parked.is_some() {
            Some(super::super::integration::pc::park(&actor).await)
        } else {
            None
        };
        let mut queries = Vec::new();
        if mode == 1 {
            loop {
                match actor.try_queue_voice_v2_context(route.selected().binding().scope.room) {
                    Ok(query) => queries.push(query),
                    Err(VoiceEndpointError::Protocol(VoiceError::Capacity)) => break,
                    Err(error) => panic!("unexpected mailbox error {error:?}"),
                }
            }
        }
        let mut sender = Box::pin(actor.queue_voice_sdp_result_scoped(
            grant,
            task.target.intent.clone(),
            original.clone(),
            VoiceSdpResultInput::Sealed {
                body: task.body.as_ref().unwrap(),
                tag,
            },
            Box::new(NeverApplied {
                funding: funding.clone(),
                dropped: dropped.clone(),
            }),
        ));
        if mode == 1 {
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(sender.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
        } else if mode == 2 {
            let pending = (&mut sender).await.unwrap();
            drop(pending); // Actor has not applied it; closing reply is authoritative here.
        }
        drop(task);
        assert!(!dropped.load(Ordering::Acquire));
        drop(sender);
        drop(incoming);
        drop(queries);
        drop(parked_actor);
        actor
            .voice_v2_context(route.selected().binding().scope.room)
            .await
            .unwrap();
        route.session().fixture_wait_physical_idle().await;
        assert!(dropped.load(Ordering::Acquire));
        assert_eq!(funding.in_flight(), 0);
        assert!(original.terminal_status().receipt.is_none());
    }

    // Admit the ORIGINAL engine through its real native hook and actual Actor.
    let mut engine = encoded[..66].to_vec();
    engine.extend_from_slice(&(original.engine_bytes() as u32).to_be_bytes());
    assert_eq!(
        &exchange(state, public, route, child, &engine, 18, &owners.intent).await[..2],
        &[8, 4]
    );
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();

    // UI wins AFTER actual mailbox admission. Root preparation runs, final native
    // try-lock refuses, and the exact original Action remains pending and unpaid SDK
    // custody is retained. No stale unlocked preflight is an admission grant.
    let parked = super::super::integration::pc::park(&actor).await;
    let mut incoming = opened(state, public, route, child, &encoded);
    let mut task = construct(&context, route, &mut incoming, row(), expected, &original).unwrap();
    let mut executing = Box::pin(task.execute());
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(executing.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let held = state.ui_session_commit.clone().lock_owned().await;
    drop(parked);
    assert!(matches!(
        executing.await,
        Err(Refused::Protocol(VoiceError::Capacity))
    ));
    drop(held);
    drop(task);
    drop(incoming);
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert!(original.terminal_status().receipt.is_none());
    assert!(!original.terminal_status().settled);
    assert_eq!(funding.in_flight(), 0);

    // Actual authenticated class2 request -> exact original Body -> actual Actor ->
    // lexical native fence -> signed Root terminal -> canonical Control history.
    if lose_reply {
        let clock = crate::voice_runtime(state).clock();
        let remaining = owners
            .intent
            .remaining_at(clock.now_ms(), clock.monotonic_ms())
            .unwrap();
        let losing = async {
            loop {
                let mut incoming = opened(state, public, route, child, &encoded);
                let task =
                    construct(&context, route, &mut incoming, row(), expected, &original).unwrap();
                let (_, grant) = factory::native::observation_actor(&context, route).unwrap();
                let (committed, observed) = tokio::sync::oneshot::channel();
                let (release, wait) = std::sync::mpsc::sync_channel(1);
                let publisher = Box::new(AfterCommit {
                    original: Some(Box::new(Publisher {
                        context: context.clone(),
                        route: route.clone(),
                        target: task.target.clone(),
                        identity: expected.identity,
                        _custody: task.custody.clone(),
                    })),
                    committed: Some(committed),
                    release: wait,
                    _custody: task.custody.clone(),
                });
                let pending = actor
                    .queue_voice_sdp_result_scoped(
                        grant,
                        task.target.intent.clone(),
                        original.clone(),
                        VoiceSdpResultInput::Sealed {
                            body: task.body.as_ref().unwrap(),
                            tag,
                        },
                        publisher,
                    )
                    .await
                    .unwrap();
                if observed.await.is_err() {
                    // No positive barrier: collect only a proven exact Capacity refusal,
                    // then retry this SAME output/row under the one enclosing deadline.
                    let completion = pending.complete().await.unwrap();
                    assert_eq!(completion.result().err(), Some(VoiceError::Capacity));
                    drop((completion, task, incoming));
                    actor
                        .voice_v2_context(route.selected().binding().scope.room)
                        .await
                        .unwrap();
                    clock.sleep(std::time::Duration::from_millis(10)).await;
                    continue;
                }
                let later_lock = state.ui_session_commit.clone().lock_owned().await;
                drop(later_lock); // Clock-bearing history must run outside this native fence.
                assert!(original.terminal_status().receipt.is_some());
                drop((pending, task, incoming));
                assert_eq!(
                    funding.in_flight(),
                    2,
                    "still-entered sender pays Body and native custody"
                );
                release.send(()).unwrap();
                actor
                    .voice_v2_context(route.selected().binding().scope.room)
                    .await
                    .unwrap();
                route.session().fixture_wait_physical_idle().await;
                assert_eq!(funding.in_flight(), 0);
                break;
            }
        };
        tokio::select! {
            biased;
            _ = clock.sleep(std::time::Duration::from_millis(remaining)) => panic!("original postcommit observer bound exceeded"),
            () = losing => (),
        }
    }
    let history = exchange(state, public, route, child, &encoded, 19, &owners.intent).await;
    assert_eq!(&history[..2], &[8, 0]);
    assert!(
        state.ui_session_commit.try_lock().is_ok(),
        "Result UI must be lexical"
    );
    let past = original.terminal_status().receipt.unwrap();
    assert!(matches!(past.outcome, CryptoOutcome::Sealed { .. }));
    assert!(
        !original.terminal_status().settled,
        "a Result never settles SDK work"
    );
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);

    let h = (
        route.session().fixture_counts().1 + 1,
        19,
        2,
        child.logical(),
    );
    let reply = apply(
        &context,
        route,
        opened(state, public, route, child, &encoded),
    )
    .await
    .unwrap();
    assert_eq!(&opened_reply(public, h, &encoded, &reply)[..2], &[8, 0]);
    assert!(
        matches!(route.session().header(), Err(channel::Refused::Capacity)),
        "fixed output retains its original physical position"
    );
    assert!(
        state.ui_session_commit.try_lock().is_ok(),
        "held Control output does not retain UI"
    );
    drop(reply);

    // Real App duplicate result bypasses its native hook. An UNUSED consuming hook
    // drops normally while SAME Body/Task/RPC still pays it; no invented refusal.
    let mut incoming = opened(state, public, route, child, &encoded);
    let task = construct(&context, route, &mut incoming, row(), expected, &original).unwrap();
    let (_, grant) = factory::native::observation_actor(&context, route).unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let pending = actor
        .queue_voice_sdp_result_scoped(
            grant,
            task.target.intent.clone(),
            original.clone(),
            VoiceSdpResultInput::Sealed {
                body: task.body.as_ref().unwrap(),
                tag,
            },
            Box::new(NeverApplied {
                funding: funding.clone(),
                dropped: dropped.clone(),
            }),
        )
        .await
        .unwrap();
    let completion = pending.complete().await.unwrap();
    assert!(completion
        .result()
        .unwrap()
        .receipt()
        .same_transition(&past));
    assert!(dropped.load(Ordering::Acquire));
    drop((completion, task, incoming));
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);

    // Changed actual bytes cannot pose as the claimed exact result, even when history
    // exists and would otherwise bypass every current native/Root publication hook.
    let mut changed = encoded.clone();
    *changed.last_mut().unwrap() ^= 1;
    let h = (
        route.session().fixture_counts().1 + 1,
        19,
        2,
        child.logical(),
    );
    let reply = apply(
        &context,
        route,
        opened(state, public, route, child, &changed),
    )
    .await
    .unwrap();
    assert_eq!(
        opened_reply(public, h, &changed, &reply),
        super::super::sdp_replies::refused(VoiceError::RetryMismatch.into())
    );
    drop(reply);
    assert!(original
        .terminal_status()
        .receipt
        .unwrap()
        .same_transition(&past));

    // Fixture-only actual SDK completion/ACK releases the original engine lane so
    // production Source can carry this signed Offer through the real TCP bridge.
    // The HIGH fixture waits for that actual packet, never injected Offer state.
    let saved = row().sdp_history(route.session().identity()).unwrap();
    assert!(saved
        .status()
        .engine
        .receipt
        .as_ref()
        .unwrap()
        .same_transition(&past));
    assert!(!saved.status().engine.settled);
    // Fixture-only SDK completion/ACK for later cleanup; no browser execution claim.
    original.acknowledge_result(&past).unwrap();
    original.settle_engine();
    assert!(
        row()
            .sdp_history(route.session().identity())
            .unwrap()
            .status()
            .engine
            .settled
    );
}

/// The HIGH native row is created only after the real signed Offer traversed the
/// permitted TCP topology. Exercise metadata custody AND an actual rejected Open.
pub(in crate::voice_handoff) async fn exercise_open(
    state: &crate::AppState,
    public: &super::super::bootstrap::SetupOutput,
    route: &Route,
    parent: &ResolvedRow,
    child: super::super::ledger::RowId,
    action: Action,
    authentication_failed: bool,
) {
    let context = Context::new(state);
    let (actor, _) = factory::native::observation_actor(&context, route).unwrap();
    let row = || {
        state
            .voice_handoff
            .resolve(
                route.selected(),
                super::super::ledger::Purpose::Sdp {
                    parent: parent.id(),
                    purpose: NativeRowPurpose::Open,
                },
                child,
            )
            .unwrap()
    };
    let original = row()
        .sdp_result_action(route.session().identity(), action)
        .unwrap();
    let owners = row()
        .sdp_input_owners(route.session().identity(), action)
        .unwrap();
    let funding = owners.funding.clone();
    let mut encoded = body(parent.id(), action, [0; 32], &[])[..66].to_vec();
    encoded[17] = 1;
    let expected = Expected {
        action,
        identity: SdpResultIdentity::AuthenticationFailed,
        seal: false,
    };
    let header = || {
        (
            route.session().fixture_counts().1 + 1,
            21,
            0,
            child.logical(),
        )
    };

    // Real metadata Copy is attached to the one-way Task custody before enqueue.
    // Drop the outer Task and unpolled sender: both native/Actor work must stay paid
    // through the affine publisher's Drop, then physically collect without a cycle.
    let h = header();
    let mut incoming = opened_frame(public, route, h, &encoded, None);
    let mut task = construct(&context, route, &mut incoming, row(), expected, &original).unwrap();
    assert!(task.metadata.as_ref().unwrap().has_keepalive());
    let (_, grant) = factory::native::observation_actor(&context, route).unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let sender = actor.queue_voice_sdp_result_scoped(
        grant,
        task.target.intent.clone(),
        original.clone(),
        VoiceSdpResultInput::AuthenticationFailed(task.metadata.take().unwrap()),
        Box::new(NeverApplied {
            funding: funding.clone(),
            dropped: dropped.clone(),
        }),
    );
    drop(task);
    drop(incoming);
    assert!(!dropped.load(Ordering::Acquire));
    assert_eq!(funding.in_flight(), 2);
    drop(sender);
    assert!(dropped.load(Ordering::Acquire));
    route.session().fixture_wait_physical_idle().await;
    assert_eq!(funding.in_flight(), 0);
    assert!(original.terminal_status().receipt.is_none());

    let mut engine = encoded.clone();
    engine.extend_from_slice(&(original.engine_bytes() as u32).to_be_bytes());
    assert_eq!(
        &exchange(state, public, route, child, &engine, 18, &owners.intent).await[..2],
        &[8, 4]
    );
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();

    let operation = if authentication_failed {
        21
    } else {
        let plaintext = b"v=0";
        encoded.extend_from_slice(&original.output_tag(plaintext).unwrap());
        encoded.extend_from_slice(&(plaintext.len() as u16).to_be_bytes());
        encoded.extend_from_slice(plaintext);
        20
    };
    assert_eq!(
        &exchange(
            state,
            public,
            route,
            child,
            &encoded,
            operation,
            &owners.intent
        )
        .await[..2],
        &[8, 0]
    );
    assert!(state.ui_session_commit.try_lock().is_ok());
    let first = original.recover_sdp_open_history().unwrap().unwrap();
    assert_eq!(
        first.receipt.outcome,
        if authentication_failed {
            CryptoOutcome::Rejected(VoiceError::Malformed)
        } else {
            CryptoOutcome::Opened
        }
    );
    if authentication_failed {
        assert!(first.delivery.is_none());
    } else {
        assert!(
            !first.delivery.as_ref().unwrap().was_admitted(),
            "accepted Open is not RTC authority"
        );
    }
    assert!(
        !original.terminal_status().settled,
        "rejection alone cannot settle actual SDK work"
    );
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);
    // Exact native retry uses canonical original rejection, no new engine/Task/Copy.
    assert_eq!(
        &exchange(
            state,
            public,
            route,
            child,
            &encoded,
            operation,
            &owners.intent
        )
        .await[..2],
        &[8, 0]
    );
    let retry = original.recover_sdp_open_history().unwrap().unwrap();
    assert!(retry.receipt.same_transition(&first.receipt));
    if !authentication_failed {
        assert!(
            Arc::ptr_eq(
                first.delivery.as_ref().unwrap(),
                retry.delivery.as_ref().unwrap()
            ),
            "exact native retry returns SAME delivery allocation"
        );
    }
    original.acknowledge_result(&first.receipt).unwrap();
    original.settle_engine();
}
