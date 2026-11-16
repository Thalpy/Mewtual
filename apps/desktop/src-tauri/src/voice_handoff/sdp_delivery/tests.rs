//! Real original native row/Actor and authenticated control frames. SDK success here is a
//! trusted fixture statement; this establishes no browser RTC execution or audible media.
use super::*;
use crate::voice_handoff::service::tests::{opened_frame, opened_reply};
use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;

#[test]
fn sdp_delivery_concrete_native_and_actor_graphs_fit_original_payments() {
    let native = task_layout();
    let actor = catcoms_app::voice_sdp_delivery_rpc_bytes();
    eprintln!(
        "SDP delivery native {native}/{TASK_BYTES}, actor {actor}/{VOICE_SDP_DELIVERY_RPC_BYTES}"
    );
    assert!(native <= TASK_BYTES);
    assert!(actor <= VOICE_SDP_DELIVERY_RPC_BYTES);
}

/// Test-only barrier inside the ACTUAL production Publisher's borrowed send tail.
/// This carries no Task, Work or physical custody; the entered publisher must pay itself.
pub(super) struct AfterSend {
    pub(super) sent: oneshot::Sender<()>,
    pub(super) release: std::sync::mpsc::Receiver<()>,
}
impl AfterSend {
    pub(super) fn park(self) {
        self.sent.send(()).unwrap();
        self.release
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("fixture did not release sender");
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::voice_handoff) async fn exercise(
    state: &crate::AppState,
    public: &super::super::bootstrap::SetupOutput,
    route: &Route,
    parent: &ResolvedRow,
    pc_frame: &[u8],
    child: super::super::ledger::RowId,
    action: Action,
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
    assert!(
        original.input().is_err(),
        "delivery must not require already-compacted input"
    );
    let delivery = original
        .recover_sdp_open_history()
        .unwrap()
        .unwrap()
        .delivery
        .unwrap();
    let owners = row()
        .sdp_input_owners(route.session().identity(), action)
        .unwrap();
    let funding = owners.funding.clone();
    let mut body = vec![8];
    body.extend_from_slice(&parent.id().logical());
    body.push(1);
    body.extend_from_slice(&action.serial.to_be_bytes());
    body.extend_from_slice(&action.version.to_be_bytes());
    body.extend_from_slice(&action.input_tag);
    body.push(0);
    let mut applied = body.clone();
    *applied.last_mut().unwrap() = 1;
    let header = || {
        (
            route.session().fixture_counts().1 + 1,
            26,
            0,
            child.logical(),
        )
    };
    let capacity = super::super::sdp_replies::refused(VoiceError::Capacity.into());
    let unauthorized = super::super::sdp_replies::refused(VoiceError::Unauthorized.into());

    // Concrete two-part construction must roll back its Task when the original finite
    // native row admits no further RPC Copy. No Actor/RTC transition may have occurred.
    let mut pressure = Vec::new();
    for _ in 0..15 {
        let mut copy = owners.phase.copy(1).unwrap();
        copy.bind_native_history(funding.clone()).unwrap();
        pressure.push(copy);
    }
    let h = header();
    let reply = apply(&context, route, opened_frame(public, route, h, &body, None))
        .await
        .unwrap();
    assert_eq!(opened_reply(public, h, &body, &reply), capacity);
    drop(reply);
    assert_eq!(funding.in_flight(), 15);
    drop(pressure);
    assert!(!delivery.was_admitted());

    // Real finite mailbox refusal, before any command handoff. Every synchronous failure
    // remains paid through the consuming callback even when its receiver is immediately ready.
    let parked = super::super::integration::pc::park(&actor).await;
    let mut queries = Vec::new();
    loop {
        match actor.try_queue_voice_v2_context(route.selected().binding().scope.room) {
            Ok(query) => queries.push(query),
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity)) => break,
            Err(error) => panic!("unexpected mailbox {error:?}"),
        }
    }
    let h = header();
    let reply = apply(&context, route, opened_frame(public, route, h, &body, None))
        .await
        .unwrap();
    assert_eq!(opened_reply(public, h, &body, &reply), capacity);
    drop((reply, queries, parked));
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);

    // Final native UI admission loses AFTER a positive mailbox admission. The queued
    // command is not permission, and the original native owner remains unadmitted.
    let parked = super::super::integration::pc::park(&actor).await;
    let h = header();
    let incoming = opened_frame(public, route, h, &body, None);
    let waiter = queue(
        &context,
        route,
        row(),
        action,
        false,
        incoming.hold_position().unwrap(),
    )
    .unwrap();
    assert!(waiter.work.is_some());
    let commit = state.ui_session_commit.clone().lock_owned().await;
    drop(parked);
    let output = waiter.complete().await.unwrap();
    assert!(matches!(
        output.result,
        Err(Refused::Protocol(VoiceError::Capacity))
    ));
    assert!(!delivery.was_admitted());
    drop((output, incoming, commit));
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);

    // Applied cannot be invented from an accepted Open or from SDK settlement. Those
    // complete crypto; they do not prove a remote description was delivered to this PC.
    assert_eq!(
        super::super::sdp_result::tests::exchange(
            state,
            public,
            route,
            child,
            &applied,
            26,
            &owners.intent
        )
        .await,
        unauthorized
    );
    assert!(!delivery.was_applied());
    let mut foreign = body.clone();
    foreign[34] ^= 1;
    assert_eq!(
        super::super::sdp_result::tests::exchange(
            state,
            public,
            route,
            child,
            &foreign,
            26,
            &owners.intent
        )
        .await,
        unauthorized
    );

    // Lose the request observer after real enqueue. Header exclusivity and Task/RPC
    // fees survive until actual callback collection, regardless of the missing waiter.
    let parked = super::super::integration::pc::park(&actor).await;
    let h = header();
    let incoming = opened_frame(public, route, h, &body, None);
    let waiter = queue(
        &context,
        route,
        row(),
        action,
        false,
        incoming.hold_position().unwrap(),
    )
    .unwrap();
    assert!(waiter.work.is_some());
    drop((waiter, incoming));
    assert_eq!(funding.in_flight(), 2);
    assert!(matches!(
        route.session().header(),
        Err(channel::Refused::Capacity)
    ));
    drop(parked);
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    route.session().fixture_wait_physical_idle().await;
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);
    assert!(delivery.was_admitted() && !delivery.was_applied());
    let admitted = super::super::sdp_result::tests::exchange(
        state,
        public,
        route,
        child,
        &body,
        26,
        &owners.intent,
    )
    .await;
    assert_eq!(&admitted[..2], &[8, 0]);
    assert_eq!(admitted[219], 2);

    // This exact trusted fixture represents successful SDK completion on the original PC.
    // No timeout, lost reply or old Admitted history is allowed to make that statement.
    let completed = super::super::sdp_result::tests::exchange(
        state,
        public,
        route,
        child,
        &applied,
        26,
        &owners.intent,
    )
    .await;
    assert_eq!(&completed[..2], &[8, 0]);
    assert_eq!(completed[219], 3);
    assert!(delivery.was_applied());
    assert!(state.ui_session_commit.try_lock().is_ok());

    // Test both real current publication and App's UNUSED-hook historical branch.
    // Drop even the successful receiver and caller Work while the sender is parked.
    for is_applied in [false, true] {
        let task = owners.phase.task_copy(TASK_BYTES).unwrap();
        let target = Box::new(
            row()
                .sdp_input_target(route.session().identity(), action, &owners, &task)
                .unwrap(),
        );
        let mut task = task;
        task.bind_native_history(funding.clone()).unwrap();
        let mut rpc = owners.phase.copy(VOICE_SDP_DELIVERY_RPC_BYTES).unwrap();
        rpc.bind_native_history(funding.clone()).unwrap();
        let h = header();
        let incoming = opened_frame(public, route, h, &body, None);
        let custody = Arc::new(Custody {
            _physical: incoming.hold_position().unwrap(),
            _task: task,
        });
        let (reply, receiver) = oneshot::channel();
        let (sent, observed) = oneshot::channel();
        let (release, wait) = std::sync::mpsc::sync_channel(1);
        let publisher = Box::new(Publisher {
            context: context.clone(),
            route: route.clone(),
            target: Some(target),
            delivery: delivery.clone(),
            applied: is_applied,
            reply: Some(reply),
            after_send: Some(AfterSend {
                sent,
                release: wait,
            }),
            custody: custody.clone(),
        });
        let (_, grant) = factory::native::observation_actor(&context, route).unwrap();
        let work = actor
            .try_queue_voice_sdp_delivery(
                grant,
                owners.intent.clone(),
                original.clone(),
                delivery.clone(),
                is_applied,
                rpc,
                publisher,
            )
            .unwrap();
        drop(custody);
        observed.await.unwrap();
        let output = receiver.await.unwrap();
        // A concurrent maintenance try-lock may refuse fresh publication; the historical
        // branch always succeeds. Either output remains paid through sender return.
        assert!(
            output.result.is_ok()
                || (!is_applied
                    && matches!(output.result, Err(Refused::Protocol(VoiceError::Capacity))))
        );
        drop((output, work, incoming));
        assert_eq!(
            funding.in_flight(),
            2,
            "immediate receiver cannot refund entered sender"
        );
        assert!(matches!(
            route.session().header(),
            Err(channel::Refused::Capacity)
        ));
        assert!(
            state.ui_session_commit.try_lock().is_ok(),
            "callback return does not retain UI"
        );
        release.send(()).unwrap();
        actor
            .voice_v2_context(route.selected().binding().scope.room)
            .await
            .unwrap();
        route.session().fixture_wait_physical_idle().await;
        actor
            .voice_v2_context(route.selected().binding().scope.room)
            .await
            .unwrap();
        assert_eq!(funding.in_flight(), 0);
    }

    // Another genuine authenticated channel cannot borrow the old channel's row, even
    // when its serialized selector happens to be identical. Refuse before queue funding.
    let foreign = super::super::channel::tests::FixtureChannel::new(
        crate::voice_runtime(state).resources(),
        route.selected().binding().scope.group,
    );
    assert_eq!(
        row()
            .sdp_result_action(foreign.session.identity(), action)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    let wire = foreign.wire(
        child.logical(),
        1,
        super::super::channel::Operation::SdpDelivery,
        super::super::channel::BodyKind::Control,
        &applied,
    );
    let mut incoming = foreign.authenticate(&wire).unwrap().admit(None).unwrap();
    let count = incoming.remaining_mut().len();
    incoming
        .remaining_mut()
        .copy_from_slice(&wire[wire.len() - count..]);
    incoming.advance(count).unwrap();
    incoming.open().unwrap();
    assert!(matches!(
        resolve(&context, route, &incoming),
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    drop((incoming, foreign));

    // A previously successful false26 is NOT a reusable execution permit. Close the
    // actual PC while another false26 is queued; current refusal must win over old history.
    let parked = super::super::integration::pc::park(&actor).await;
    let h = header();
    let incoming = opened_frame(public, route, h, &body, None);
    let waiter = queue(
        &context,
        route,
        row(),
        action,
        false,
        incoming.hold_position().unwrap(),
    )
    .unwrap();
    let pc = super::super::requests::pc::Request::decode(parent.id().logical(), pc_frame).unwrap();
    parent.close_pc(route.session().identity(), &pc).unwrap();
    drop(parked);
    let output = waiter.complete().await.unwrap();
    assert!(matches!(
        output.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    assert!(matches!(
        project(route, Ok(Prepared::Completion(output))),
        Projection::Refused(_)
    ));
    drop(incoming);
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert_eq!(funding.in_flight(), 0);
    assert_eq!(
        super::super::sdp_result::tests::exchange(
            state,
            public,
            route,
            child,
            &body,
            26,
            &owners.intent
        )
        .await,
        unauthorized
    );
    let recovered = super::super::sdp_result::tests::exchange(
        state,
        public,
        route,
        child,
        &applied,
        26,
        &owners.intent,
    )
    .await;
    // Retirement is a timestamped observation and legitimately changes on PC close.
    // Assert the immutable identity/outcome/digest and Applied fact, not that old
    // currentness metadata is frozen forever.
    assert_eq!(&recovered[..35], &completed[..35]); // version/tag, row, parent, purpose
    assert_eq!(recovered[37], completed[37]); // Action-present discriminator
    assert_eq!(&recovered[38..185], &completed[38..185]);
    assert_eq!(&recovered[186..], &completed[186..]);
    assert_eq!(recovered[185], 1);

    // Return ends progress custody; historical Applied still uses the actual original
    // Session/Action cache and cannot allocate a replacement Phase/SDK/RTC owner.
    let cleanup = &body[..18];
    let h = (
        route.session().fixture_counts().1 + 1,
        24,
        0,
        child.logical(),
    );
    let reply = super::super::sdp_http::apply(
        &context,
        route,
        opened_frame(public, route, h, cleanup, None),
    )
    .await
    .unwrap();
    assert_eq!(&opened_reply(public, h, cleanup, &reply)[..2], &[8, 0]);
    drop(reply);
    let history = super::super::sdp_result::tests::exchange(
        state,
        public,
        route,
        child,
        &applied,
        26,
        &owners.intent,
    )
    .await;
    assert_eq!(history[219], 3);
    assert_eq!(funding.in_flight(), 0);
}
