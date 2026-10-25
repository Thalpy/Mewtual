//! Real Actor/mailbox/custody tests under a genuine native PC intent. The frame carrying
//! the lifetime hold is the existing authenticated Control14 fixture, NOT a new SDP endpoint
//! or browser/RTC acceptance. No literal admission bool or timing sleep substitutes for an
//! actual queued command, UI guard, physical position or original Phase contribution.
use super::*;
use catcoms_app::{voice_sdp_rpc_bytes, VOICE_SDP_RPC_METADATA_BYTES};
use catcoms_app::{ServerActor, VoiceEndpointError, VoiceSdpBody, VoiceSdpCompletion};
use catcoms_sync::voice_v2::chain::sdp_handoff::Phase;
use catcoms_sync::voice_v2::crypto_actions::CryptoAction;
use catcoms_sync::voice_v2::sdp::InitialSdpIntent;
use catcoms_sync::voice_v2::{CallGrant, VoiceError};

type HeldResult = (VoiceSdpBody, VoiceSdpCompletion<Arc<CryptoAction>>);

fn constructor_refusals(phase: &Arc<Phase>, intent: &Arc<InitialSdpIntent>, other: &Arc<Phase>) {
    // Capacity is the real backing allocation, not visible length. Every refusal returns
    // the identical original allocation, including the native publication owner if attached.
    let bytes = vec![9; 32];
    let pointer = bytes.as_ptr();
    let buffer = phase.copy(bytes.capacity()).unwrap().adopt(bytes).unwrap();
    let (error, buffer) = VoiceSdpBody::from_buffer(buffer, 0..32, intent.clone())
        .unwrap_err()
        .into_parts();
    assert_eq!(error, VoiceError::Capacity);
    assert_eq!(buffer.bytes().as_ptr(), pointer);
    assert_eq!(buffer.bytes(), &[9; 32]);
    drop(buffer);
    // An invalid slice descriptor, not an attempted reverse iteration.
    for range in [std::ops::Range { start: 33, end: 32 }, 0..33] {
        let buffer = phase
            .copy(32 + VOICE_SDP_RPC_METADATA_BYTES)
            .unwrap()
            .adopt(vec![8; 32])
            .unwrap();
        let pointer = buffer.bytes().as_ptr();
        let (error, buffer) = VoiceSdpBody::from_buffer(buffer, range, intent.clone())
            .unwrap_err()
            .into_parts();
        assert_eq!(error, VoiceError::Bounds);
        assert_eq!(buffer.bytes().as_ptr(), pointer);
        drop(buffer);
    }
    // Equal original key/plan labels do not make a separately allocated Phase the one
    // installed in the original PC intent. A paid foreign buffer is still unauthorized.
    let buffer = other
        .copy(32 + VOICE_SDP_RPC_METADATA_BYTES)
        .unwrap()
        .adopt(vec![7; 32])
        .unwrap();
    let pointer = buffer.bytes().as_ptr();
    let (error, buffer) = VoiceSdpBody::from_buffer(buffer, 0..32, intent.clone())
        .unwrap_err()
        .into_parts();
    assert_eq!(error, VoiceError::Unauthorized);
    assert_eq!(buffer.bytes().as_ptr(), pointer);
    drop(buffer);
}

pub(super) fn publication(
    state: &AppState,
    route: &bootstrap::Route,
    phase: &Arc<Phase>,
    intent: &Arc<InitialSdpIntent>,
    physical: channel::PhysicalHold,
) -> Arc<sdp_publication::Publication> {
    let task = phase.task_copy(sdp_publication::TASK_BYTES).unwrap();
    let mut owner =
        sdp_publication::Publication::preparing(physical, route.session(), intent, task).unwrap();
    // The enclosing genuine PC fixture has already passed current native admission. This
    // test installs the actual UI commit, never a claimed generation or cached open bool.
    owner.admit(state.ui_session_commit.clone().try_lock_owned().unwrap());
    Arc::new(owner)
}

fn assert_blocked(state: &AppState, route: &bootstrap::Route) {
    assert!(
        state.ui_session_commit.try_lock().is_err(),
        "lock completion must wait for actual queued/result collection"
    );
    assert!(
        route.session().fixture_counts().0,
        "actual physical position must survive the submitting worker"
    );
    assert!(matches!(
        route.session().header(),
        Err(channel::Refused::Capacity)
    ));
}
fn assert_collected(state: &AppState, route: &bootstrap::Route) {
    assert!(state.ui_session_commit.try_lock().is_ok());
    assert!(!route.session().fixture_counts().0);
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn exercise(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    frame: &[u8],
    actor: &ServerActor,
    grant: &Arc<CallGrant>,
    intent: &Arc<InitialSdpIntent>,
) -> HeldResult {
    let phase = parent.sdp_test_phase().unwrap();
    let request = requests::pc::Request::decode(parent.id().logical(), frame).unwrap();
    let same_key = parent
        .pc_test_keys(route.session().identity(), &request)
        .unwrap();
    let other = same_key.prepare_sdp_handoff().unwrap();
    other.arm().unwrap();
    constructor_refusals(&phase, intent, &other);
    other.close();
    drop(other);
    // Unknown handshake deliberately exercises the native-paid Actor rejection without
    // issuing another Root SDP action. Happy Offer/Answer semantics have separate Root
    // tests; these scenarios pin real mailbox/collection lifetimes on both endpoint roles.
    let absent = [0; 32];
    assert_ne!(
        absent,
        requests::pc::Request::decode(parent.id().logical(), frame)
            .unwrap()
            .base
            .handshake
    );

    // A body cannot be queued before a native publication owner is installed.
    let buffer = phase
        .copy(32 + VOICE_SDP_RPC_METADATA_BYTES)
        .unwrap()
        .adopt(vec![6; 32])
        .unwrap();
    let unadmitted = VoiceSdpBody::from_buffer(buffer, 0..32, intent.clone()).unwrap();
    assert!(matches!(
        actor
            .queue_voice_sdp_offer(grant.clone(), absent, intent.clone(), &unadmitted)
            .await,
        Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
    ));
    drop(unadmitted);

    let incoming = opened_frame(
        public,
        route,
        (
            route.session().fixture_counts().1 + 1,
            14,
            0,
            parent.id().logical(),
        ),
        frame,
        None,
    );
    let owner = publication(
        state,
        route,
        &phase,
        intent,
        incoming.hold_position().unwrap(),
    );
    let weak = Arc::downgrade(&owner);
    let mut buffer = phase.copy(32).unwrap().adopt(vec![4; 32]).unwrap();
    let pointer = buffer.bytes().as_ptr();
    buffer.attach_keepalive(owner.clone()).unwrap();
    let (error, buffer) = VoiceSdpBody::from_buffer(buffer, 0..32, intent.clone())
        .unwrap_err()
        .into_parts();
    assert_eq!(error, VoiceError::Capacity);
    assert_eq!(buffer.bytes().as_ptr(), pointer);
    assert!(buffer.has_keepalive());
    drop(owner);
    drop(incoming);
    assert_blocked(state, route);
    assert!(weak.upgrade().is_some());
    drop(buffer);
    assert!(weak.upgrade().is_none());
    assert_collected(state, route);

    // A separate real native fixture has a large construction future. Pin it on the
    // test heap rather than embedding it throughout this nested real-TCP observer graph.
    // Production SDP task layout/payment remains a separate not-yet-implemented boundary.
    let stopped_fixture = Box::pin(crate::voice_activation_tests::Fixture::new()).await;
    let stopped = stopped_fixture.actor.clone();
    stopped_fixture.close().await; // Actual command receiver/task has stopped.
    let incoming = opened_frame(
        public,
        route,
        (
            route.session().fixture_counts().1 + 1,
            14,
            0,
            parent.id().logical(),
        ),
        frame,
        None,
    );
    let owner = publication(
        state,
        route,
        &phase,
        intent,
        incoming.hold_position().unwrap(),
    );
    let buffer = phase
        .copy(32 + VOICE_SDP_RPC_METADATA_BYTES)
        .unwrap()
        .adopt(vec![6; 32])
        .unwrap();
    let mut body = VoiceSdpBody::from_buffer(buffer, 0..32, intent.clone()).unwrap();
    body.attach_publication(owner.clone()).unwrap();
    assert!(matches!(
        stopped
            .queue_voice_sdp_offer(grant.clone(), absent, intent.clone(), &body)
            .await,
        Err(VoiceEndpointError::Stopped)
    ));
    drop(owner);
    drop(incoming);
    assert_blocked(state, route); // A failed send is not collection of the caller's body.
    drop(body);
    assert_collected(state, route);

    // Positive queue admission parks the actual command behind the durable-send barrier.
    // A second claim is forbidden through both its Pending and completed/uncollected reply.
    for leave_held in [false, true] {
        let incoming = opened_frame(
            public,
            route,
            (
                route.session().fixture_counts().1 + 1,
                14,
                0,
                parent.id().logical(),
            ),
            frame,
            None,
        );
        let owner = publication(
            state,
            route,
            &phase,
            intent,
            incoming.hold_position().unwrap(),
        );
        let weak = Arc::downgrade(&owner);
        let buffer = phase
            .copy(32 + VOICE_SDP_RPC_METADATA_BYTES)
            .unwrap()
            .adopt(vec![6; 32])
            .unwrap();
        let mut body = VoiceSdpBody::from_buffer(buffer, 0..32, intent.clone()).unwrap();
        body.attach_publication(owner.clone()).unwrap();
        assert_eq!(
            body.attach_publication(owner.clone()),
            Err(VoiceError::RetryMismatch)
        );
        let parked = super::pc::park(actor).await;
        let future = actor.queue_voice_sdp_offer(grant.clone(), absent, intent.clone(), &body);
        let measured = voice_sdp_rpc_bytes(std::mem::size_of_val(&future));
        assert!(
            measured <= VOICE_SDP_RPC_METADATA_BYTES,
            "actual SDP RPC graph needs {measured}"
        );
        let pending = future.await.unwrap();
        assert!(matches!(
            actor
                .queue_voice_sdp_offer(grant.clone(), absent, intent.clone(), &body)
                .await,
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity))
        ));
        drop(owner); // Submitting native task disappears; the bytes/RPC now own publication.
        drop(incoming);
        assert_blocked(state, route);
        drop(parked);
        let complete = collect_rejection(state, actor, grant, intent, &body, absent, pending).await;
        assert_eq!(complete.result().err(), Some(VoiceError::Unauthorized));
        assert_blocked(state, route);
        assert!(matches!(
            actor
                .queue_voice_sdp_offer(grant.clone(), absent, intent.clone(), &body)
                .await,
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity))
        ));
        if leave_held {
            return (body, complete); // The caller closes PC/SDK while these real owners remain.
        }
        drop(complete);
        // The receiver can collect before the sender returns from send(). Wait for a
        // SAME Actor FIFO turn before asserting that its one RPC position is reusable.
        actor
            .voice_v2_context(route.selected().binding().scope.room)
            .await
            .unwrap();
        let pending = actor
            .queue_voice_sdp_offer(grant.clone(), absent, intent.clone(), &body)
            .await
            .unwrap();
        let retry = collect_rejection(state, actor, grant, intent, &body, absent, pending).await;
        assert_eq!(retry.result().err(), Some(VoiceError::Unauthorized));
        drop(body); // The original returned result is now the SOLE payload/publication owner.
        assert_blocked(state, route);
        assert!(weak.upgrade().is_some());
        drop(retry);
        actor
            .voice_v2_context(route.selected().binding().scope.room)
            .await
            .unwrap();
        assert!(weak.upgrade().is_none());
        assert_collected(state, route);

        // Metadata-only Open uses its own original Copy and the SAME native publication
        // graph. Losing both submitting owner and waiter cannot collect a queued command.
        let incoming = opened_frame(
            public,
            route,
            (
                route.session().fixture_counts().1 + 1,
                14,
                0,
                parent.id().logical(),
            ),
            frame,
            None,
        );
        let owner = publication(
            state,
            route,
            &phase,
            intent,
            incoming.hold_position().unwrap(),
        );
        let weak = Arc::downgrade(&owner);
        let mut work = phase.copy(VOICE_SDP_RPC_METADATA_BYTES).unwrap();
        work.attach_keepalive(owner.clone()).unwrap();
        let parked = super::pc::park(actor).await;
        let pending = actor
            .queue_voice_sdp_open(grant.clone(), absent, intent.clone(), work)
            .await
            .unwrap();
        drop(pending);
        drop(owner);
        drop(incoming);
        assert_blocked(state, route);
        assert!(weak.upgrade().is_some());
        drop(parked);
        actor
            .voice_v2_context(route.selected().binding().scope.room)
            .await
            .unwrap();
        assert!(
            weak.upgrade().is_none(),
            "actual Actor collection releases abandoned work"
        );
        assert_collected(state, route);
    }
    unreachable!()
}

/// Current Group/Policy locks may briefly refuse before the deliberate unknown-handshake
/// rejection. Collect that actual completion, then retry ONLY Capacity with identical
/// owners inside the ORIGINAL intent deadline. Never accept pressure as the required result.
async fn collect_rejection(
    state: &AppState,
    actor: &ServerActor,
    grant: &Arc<CallGrant>,
    intent: &Arc<InitialSdpIntent>,
    body: &VoiceSdpBody,
    absent: [u8; 32],
    mut pending: catcoms_app::VoiceSdpPending<Arc<CryptoAction>>,
) -> VoiceSdpCompletion<Arc<CryptoAction>> {
    let clock = crate::voice_runtime(state).clock();
    let remaining = intent
        .remaining_at(clock.now_ms(), clock.monotonic_ms())
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_millis(remaining), async {
        loop {
            let complete = pending.complete().await.unwrap();
            assert!(
                intent.current_at(clock.now_ms(), clock.monotonic_ms()),
                "same SDP rejection must complete inside original Ready deadline"
            );
            if complete.result().err() != Some(VoiceError::Capacity) {
                return complete;
            }
            drop(complete);
            clock.sleep(std::time::Duration::from_millis(10)).await;
            pending = actor
                .queue_voice_sdp_offer(grant.clone(), absent, intent.clone(), body)
                .await
                .unwrap();
        }
    })
    .await
    .expect("same SDP rejection observation exceeded original Ready bound")
}
