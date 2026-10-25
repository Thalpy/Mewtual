//! Actual recorded Actor ownership under a genuine blocked-TCP PC intent. These tests
//! exercise typed Rust RPCs; no production SDP endpoint, browser SDK or RTC is claimed.
use super::*;
use catcoms_app::{ServerActor, VoiceSdpBody, VOICE_SDP_RPC_METADATA_BYTES};
use catcoms_sync::voice_v2::chain::sdp_handoff::{NativeRowFunding, NativeRowPurpose};
use catcoms_sync::voice_v2::sdp::publication::{History, Outcome, Prepared};
use catcoms_sync::voice_v2::sdp::{plaintext_commitment, InitialSdpIntent};
use catcoms_sync::voice_v2::{CallGrant, VoiceError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context as TaskContext, Poll, Wake, Waker};

struct CollectionOracle {
    fee: Arc<NativeRowFunding>,
    slot: Arc<Outcome>,
    observed: AtomicBool,
}
impl Wake for CollectionOracle {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if self.fee.is_collected() {
            assert!(
                matches!(
                    self.slot.history(),
                    History::NeverEntered | History::Refused(_)
                ),
                "last Copy notified collection before retiring its actual Writer"
            );
            self.observed.store(true, Ordering::Release);
        }
    }
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
) -> Option<Arc<Outcome>> {
    let phase = parent.sdp_test_phase().unwrap();
    let absent = [0; 32];
    let tag = Some(plaintext_commitment(b"v=0").unwrap());
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let slot = Prepared::new(intent.clone(), fee.clone(), tag)
        .unwrap()
        .install();
    // A fresh authenticated Control14 position supplies actual native publication custody.
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
    let owner = super::sdp_actor::publication(
        state,
        route,
        &phase,
        intent,
        incoming.hold_position().unwrap(),
    );
    let weak = Arc::downgrade(&owner);
    let buffer = phase
        .copy(3 + VOICE_SDP_RPC_METADATA_BYTES)
        .unwrap()
        .adopt(b"v=0".to_vec())
        .unwrap();
    let mut body = VoiceSdpBody::from_buffer(buffer, 0..3, intent.clone()).unwrap();
    body.attach_publication(owner.clone()).unwrap();

    let future = actor.queue_voice_sdp_offer_recorded(
        grant.clone(),
        absent,
        intent.clone(),
        &body,
        slot.try_writer().unwrap(),
    );
    assert!(matches!(slot.history(), History::InFlight));
    drop(future); // Never polled: the eagerly constructed whole Command owns the Writer.
    assert!(matches!(slot.history(), History::NeverEntered));

    // A real stopped Actor must drop the failed Command and Writer, not leave InFlight.
    let stopped_fixture = Box::pin(crate::voice_activation_tests::Fixture::new()).await;
    let stopped = stopped_fixture.actor.clone();
    stopped_fixture.close().await;
    assert!(matches!(
        stopped
            .queue_voice_sdp_offer_recorded(
                grant.clone(),
                absent,
                intent.clone(),
                &body,
                slot.try_writer().unwrap()
            )
            .await,
        Err(catcoms_app::VoiceEndpointError::Stopped)
    ));
    assert!(matches!(slot.history(), History::NeverEntered));

    let parked = super::pc::park(actor).await;
    let pending = actor
        .queue_voice_sdp_offer_recorded(
            grant.clone(),
            absent,
            intent.clone(),
            &body,
            slot.try_writer().unwrap(),
        )
        .await
        .unwrap();
    drop(pending); // Lost waiter cannot certify absence while the actual Actor owns work.
    assert!(matches!(slot.history(), History::InFlight));
    drop(parked);
    actor
        .voice_v2_context(route.selected().binding().scope.room)
        .await
        .unwrap();
    assert!(matches!(slot.history(), History::NeverEntered));

    let pending = actor
        .queue_voice_sdp_offer_recorded(
            grant.clone(),
            absent,
            intent.clone(),
            &body,
            slot.try_writer().unwrap(),
        )
        .await
        .unwrap();
    let completion = pending.complete().await.unwrap();
    let reason = completion.result().err().unwrap();
    assert!(matches!(
        reason,
        VoiceError::Unauthorized | VoiceError::Capacity
    ));
    assert!(matches!(slot.history(), History::Refused(actual) if actual == reason));
    drop(completion);
    drop(body);
    drop(owner);
    drop(incoming);
    assert!(weak.upgrade().is_none());
    assert!(state.ui_session_commit.try_lock().is_ok());

    // The final original Open Copy is owned only by an UNPOLLED recorded queue future.
    // A synchronous real Notify waker must already observe the negative when IO hits zero.
    let open_fee = phase.native_history(NativeRowPurpose::Open).unwrap();
    let open_slot = Prepared::new(intent.clone(), open_fee.clone(), None)
        .unwrap()
        .install();
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
    let owner = super::sdp_actor::publication(
        state,
        route,
        &phase,
        intent,
        incoming.hold_position().unwrap(),
    );
    let mut copy = phase.copy(VOICE_SDP_RPC_METADATA_BYTES).unwrap();
    copy.bind_native_history(open_fee.clone()).unwrap();
    copy.attach_keepalive(owner.clone()).unwrap();
    let future = actor.queue_voice_sdp_open_recorded(
        grant.clone(),
        absent,
        intent.clone(),
        copy,
        open_slot.try_writer().unwrap(),
    );
    drop(open_fee.close_io());
    assert!(!open_fee.is_collected());
    let oracle = Arc::new(CollectionOracle {
        fee: open_fee.clone(),
        slot: open_slot.clone(),
        observed: AtomicBool::new(false),
    });
    let waker = Waker::from(oracle.clone());
    let mut wait = std::pin::pin!(open_fee.wait_collected());
    assert!(matches!(
        std::future::Future::poll(wait.as_mut(), &mut TaskContext::from_waker(&waker)),
        Poll::Pending
    ));
    drop(future);
    assert!(open_fee.is_collected());
    assert!(
        oracle.observed.load(Ordering::Acquire),
        "actual collection notification missing"
    );
    assert!(matches!(open_slot.history(), History::NeverEntered));
    drop(owner);
    drop(incoming);
    assert!(state.ui_session_commit.try_lock().is_ok());

    let retained = if intent.local_is_low() {
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
        let owner = super::sdp_actor::publication(
            state,
            route,
            &phase,
            intent,
            incoming.hold_position().unwrap(),
        );
        let mut buffer = phase
            .copy(3 + VOICE_SDP_RPC_METADATA_BYTES)
            .unwrap()
            .adopt(b"v=0".to_vec())
            .unwrap();
        buffer.bind_native_history(fee.clone()).unwrap();
        let mut body = VoiceSdpBody::from_buffer(buffer, 0..3, intent.clone()).unwrap();
        body.attach_publication(owner.clone()).unwrap();
        let request = requests::pc::Request::decode(parent.id().logical(), frame).unwrap();
        let clock = crate::voice_runtime(state).clock();
        let remaining = intent
            .remaining_at(clock.now_ms(), clock.monotonic_ms())
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_millis(remaining), async {
            for attempt in 0..32 {
                let pending = actor
                    .queue_voice_sdp_offer_recorded(
                        grant.clone(),
                        request.base.handshake,
                        intent.clone(),
                        &body,
                        slot.try_writer().unwrap(),
                    )
                    .await
                    .unwrap();
                // Actual Actor ordering puts its publication/complete before this command.
                // Never collect or inspect the successful RPC reply: recover only the slot.
                actor
                    .voice_v2_context(route.selected().binding().scope.room)
                    .await
                    .unwrap();
                let history = slot.history();
                drop(pending);
                assert!(intent.current_at(clock.now_ms(), clock.monotonic_ms()));
                match history {
                    History::Published(_) => break,
                    History::Refused(VoiceError::Capacity) if attempt < 31 => {
                        clock.sleep(std::time::Duration::from_millis(10)).await;
                    }
                    _ => panic!("actual LOW Actor publication did not reach its original slot"),
                }
            }
        })
        .await
        .expect("recorded publication observer exceeded original Ready deadline");
        assert!(matches!(slot.history(), History::Published(_)));
        drop(body);
        drop(owner);
        drop(incoming);
        Some(slot.clone())
    } else {
        None
    };
    drop(fee.close_io());
    assert!(fee.is_collected());
    // No Row was installed here. Production must still inventory its complete Row+slot
    // graph and install exactly one slot under Index before using negative compaction.
    assert!(
        Outcome::allocation_bytes() < catcoms_sync::voice_v2::chain::sdp_handoff::NATIVE_ROW_BYTES
    );
    retained
}
