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
    let remaining = if matches!(operation, 22 | 23) {
        // Historical cleanup survives PC closure/expiry. This is only a broken-test
        // observer bound, never renewed protocol currentness or new action authority.
        15_000
    } else {
        intent
            .remaining_at(clock.now_ms(), clock.monotonic_ms())
            .unwrap()
    };
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
            } else if matches!(operation, 22 | 23) {
                super::super::sdp_http::apply(&context, route, incoming).await
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
/// The fixture has joined its real native Result flight and retained no SDK operation.
/// These are actual authenticated22/23 controls, not direct Root settlement. Both orders
/// retain input until ACK AND settlement; only browser integration can prove its SDK join.
#[allow(clippy::too_many_arguments)]
async fn finish_original(
    state: &crate::AppState,
    public: &super::super::bootstrap::SetupOutput,
    route: &Route,
    parent: super::super::ledger::RowId,
    child: super::super::ledger::RowId,
    action: Action,
    original: &Arc<CryptoAction>,
    receipt: &catcoms_sync::voice_v2::crypto_actions::CryptoResultReceipt,
    intent: &Arc<catcoms_sync::voice_v2::sdp::InitialSdpIntent>,
    open: bool,
) {
    let mut settle = body(parent, action, [0; 32], &[])[..66].to_vec();
    settle[17] = u8::from(open);
    let mut ack = settle.clone();
    ack.extend_from_slice(&super::super::requests::receipt_digest(receipt));
    let mut wrong_digest = ack.clone();
    *wrong_digest.last_mut().unwrap() ^= 1;
    assert_eq!(
        exchange(state, public, route, child, &wrong_digest, 22, intent).await,
        super::super::sdp_replies::refused(VoiceError::RetryMismatch.into())
    );
    let mut wrong_action = settle.clone();
    wrong_action[34] ^= 1; // valid grammar, foreign original input commitment
    assert_eq!(
        exchange(state, public, route, child, &wrong_action, 23, intent).await,
        super::super::sdp_replies::refused(VoiceError::Unauthorized.into())
    );
    assert!(!original.handoff_status().acknowledged);
    assert!(!original.terminal_status().settled);
    assert!(original.input().is_ok());
    // LOW uses ACK->settle; HIGH uses settle->ACK. Exact repeated controls model
    // losing either fixed reply, without inventing another provider execution.
    let order = if open {
        [(23, &settle), (22, &ack)]
    } else {
        [(22, &ack), (23, &settle)]
    };
    for (index, (operation, encoded)) in order.into_iter().enumerate() {
        let clock = crate::voice_runtime(state).clock();
        let lost = async {
            loop {
                let h = (
                    route.session().fixture_counts().1 + 1,
                    operation,
                    0,
                    child.logical(),
                );
                let incoming = opened_frame(public, route, h, encoded, None);
                let reply = super::super::sdp_http::apply(&Context::new(state), route, incoming)
                    .await
                    .unwrap();
                // Do not authenticate/decrypt/read this response. Observe actual Root
                // effect separately, then lose its successful fixed reply before retry.
                drop(reply);
                let observed = original.handoff_status();
                if if operation == 22 {
                    observed.acknowledged
                } else {
                    observed.engine.settled
                } {
                    break;
                }
                // A short native try-lock may refuse before effect. Keep the same exact
                // metadata under one whole observer bound, never blind-reexecute SDK.
                clock.sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        tokio::select! {
            biased;
            _ = clock.sleep(std::time::Duration::from_secs(15)) => panic!("native cleanup effect did not commit within harness bound"),
            _ = lost => {}
        }
        let reply = exchange(state, public, route, child, encoded, operation, intent).await;
        assert_eq!(&reply[..2], &[8, 0]);
        assert_eq!(reply[86], u8::from(open || index == 1));
        assert_eq!(reply[186], u8::from(!open || index == 1));
        let repeated = exchange(state, public, route, child, encoded, operation, intent).await;
        assert_eq!(&repeated[38..185], &reply[38..185]);
        assert_eq!(repeated[186], reply[186]);
        if index == 0 {
            assert!(original.input().is_ok());
        }
    }
    assert!(original.handoff_status().acknowledged);
    assert!(original.terminal_status().settled);
    assert!(original.input().is_err());
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
    maximum: bool,
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
    let payload = vec![42; if maximum { 8208 } else { 19 }];
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
        // Keep this foreign-purpose header within its OWN authenticated class maximum.
        // Maximum Seal output is16 bytes larger than maximum Open: a full-sized Seal
        // under opcode20 must refuse even earlier at header authentication, below.
        let header_body = if operation == 20 {
            &encoded[..encoded.len().min(100 + 8192)]
        } else {
            &encoded
        };
        let wire = signed_frame(public, header, header_body);
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
    if maximum {
        let before = route.session().fixture_counts();
        let header = (before.1 + 1, 20, 2, child.logical());
        let wire = signed_frame(public, header, &encoded);
        let mut worker = route.session().header().unwrap();
        let filled = wire.len().min(super::super::http::HEADER_BYTES);
        worker.scratch_mut()[..filled].copy_from_slice(&wire[..filled]);
        worker.set_filled(filled).unwrap();
        assert!(matches!(
            worker.authenticate(41891, "http://tauri.localhost"),
            Err(channel::Refused::Header)
        ));
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
    finish_original(
        state,
        public,
        route,
        parent,
        child,
        action,
        &original,
        &past,
        &owners.intent,
        false,
    )
    .await;
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
/// permitted TCP topology. Exercise metadata custody, exact output bounds and Open history.
#[allow(clippy::too_many_arguments)] // Original endpoint/row tuple plus bounded fixture variants.
pub(in crate::voice_handoff) async fn exercise_open(
    state: &crate::AppState,
    public: &super::super::bootstrap::SetupOutput,
    route: &Route,
    parent: &ResolvedRow,
    pc_frame: &[u8],
    child: super::super::ledger::RowId,
    action: Action,
    authentication_failed: bool,
    check_wrong_empty: bool,
    maximum: bool,
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

    if check_wrong_empty {
        // Class2 can represent an exact empty success, but THIS original signed Offer
        // has19 ciphertext bytes and requires exactly3 plaintext bytes. Its immutable
        // Root output bound must still refuse zero before any terminal/token effect.
        let mut empty = encoded.clone();
        empty.extend_from_slice(&[0; 32]);
        empty.extend_from_slice(&0u16.to_be_bytes());
        assert_eq!(
            exchange(state, public, route, child, &empty, 20, &owners.intent).await,
            super::super::sdp_replies::refused(VoiceError::Bounds.into())
        );
        assert!(original.terminal_status().receipt.is_none());
        assert!(!original.terminal_status().settled);
    }
    let operation = if authentication_failed {
        21
    } else {
        let plaintext: &[u8] = if maximum { &[b'x'; 8192] } else { b"v=0" };
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
    // Close the actual native PC before HIGH's cleanup. Its same Session/Action ACK
    // and settlement remain available without granting or reviving SDK/RTC currentness.
    let pc = super::super::requests::pc::Request::decode(parent.id().logical(), pc_frame).unwrap();
    parent.close_pc(route.session().identity(), &pc).unwrap();
    finish_original(
        state,
        public,
        route,
        parent.id(),
        child,
        action,
        &original,
        &first.receipt,
        &owners.intent,
        true,
    )
    .await;
}
