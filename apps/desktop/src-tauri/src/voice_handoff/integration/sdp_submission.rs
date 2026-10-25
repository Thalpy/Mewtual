//! Original native submission under actual blocked-TCP Finish/Ready/PC custody. The
//! canonical encrypted SDP frame is independently authenticated, but production dispatch
//! remains closed. No browser SDK, SDP encryption or physical RTC/audio is claimed.
use super::*;
use crate::voice_handoff::sdp_submission::{Payload, Preparation};
use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;
use catcoms_sync::voice_v2::sdp::{publication::History, InitialSdpIntent};
use catcoms_sync::voice_v2::VoiceError;

fn opened(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    encoded: &[u8],
    seal: bool,
) -> channel::Incoming {
    let header = (
        route.session().fixture_counts().1 + 1,
        if seal { 15 } else { 16 },
        if seal { 2 } else { 0 },
        parent.id().logical(),
    );
    if !seal {
        return opened_frame(public, route, header, encoded, None);
    }
    let wire = service::tests::signed_frame(public, header, encoded);
    let authenticated = service::tests::authenticated_header(route, &wire);
    let permit = state
        .voice_handoff
        .sdp_begin_intake(
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
    assert_eq!(incoming.open().unwrap(), encoded);
    incoming
}
fn request<'a>(
    parent: &ledger::ResolvedRow,
    encoded: &'a [u8],
    seal: bool,
) -> requests::sdp_action::Begin<'a> {
    let requests::sdp_action::Request::Begin(request) = requests::sdp_action::Request::decode(
        if seal {
            channel::Operation::SdpSealBegin
        } else {
            channel::Operation::SdpOpenBegin
        },
        if seal {
            channel::BodyKind::Sdp
        } else {
            channel::BodyKind::Control
        },
        parent.id().logical(),
        encoded,
    )
    .unwrap() else {
        panic!("original Begin");
    };
    request
}
fn payload(
    incoming: &mut channel::Incoming,
    intent: &Arc<InitialSdpIntent>,
    funding: &Arc<catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowFunding>,
    seal: bool,
) -> Payload {
    if seal {
        let body = incoming.sdp_body(intent.clone(), funding).unwrap();
        assert!(body.uses_native_history(intent, funding));
        assert!(
            incoming.plain().unwrap().is_empty(),
            "one irreversible body transfer"
        );
        assert!(
            incoming.sdp_body(intent.clone(), funding).is_err(),
            "no second extraction"
        );
        Payload::Seal {
            body,
            delivery: None,
        }
    } else {
        Payload::Open
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn exercise(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    pc_frame: &[u8],
    intent: &Arc<InitialSdpIntent>,
    retain_output: bool,
) {
    let context = context::Context::new(state);
    let phase = parent.sdp_test_phase().unwrap();
    let clock = crate::voice_runtime(state).clock();
    let seal = intent.local_is_low();
    let purpose = if seal {
        NativeRowPurpose::Seal
    } else {
        NativeRowPurpose::Open
    };
    let mut encoded = pc_frame[..193].to_vec();
    encoded[0] = 8;
    encoded.extend_from_slice(&parent.pc_test_proof());
    if seal {
        encoded.extend_from_slice(&3u16.to_be_bytes());
        encoded.extend_from_slice(b"v=0");
    }
    let request = request(parent, &encoded, seal);
    let candidate = parent
        .sdp_action_candidate(route.session().identity(), &request)
        .unwrap();
    let funding = candidate.funding().clone();
    let wall = clock.now_ms();
    let mono = clock.monotonic_ms();
    let ledger::SdpBegin::Factory(ticket) =
        crate::voice_handoff::sdp::current(&context, route, |admission| {
            state.voice_handoff.begin_sdp_action(
                parent,
                route.selected(),
                admission,
                &candidate,
                wall,
                mono,
            )
        })
        .unwrap()
    else {
        panic!("one original row");
    };
    let row = ticket.resolved();
    let id = row.id();
    let slot = row.sdp_test_outcome();
    let original_remaining = intent
        .remaining_at(clock.now_ms(), clock.monotonic_ms())
        .unwrap();
    // One harness deadline covers ALL local retries and queue/closure observations. It
    // neither renews Ready nor substitutes a timeout for unknown-publication evidence.
    tokio::time::timeout(
        std::time::Duration::from_millis(original_remaining),
        async {
            let ticket = if !seal {
                // Leave ONE actual Phase Copy available: TaskCopy fits, the subsequent
                // metadata Copy refuses. No Incoming survives to mask premature idle.
                let mut held = Vec::new();
                for _ in 0..14 {
                    held.push(phase.copy(1).unwrap());
                }
                let incoming = opened(state, public, route, parent, &encoded, false);
                let physical = incoming.hold_position().unwrap();
                drop(incoming);
                let session = route.session().clone();
                let bound = std::time::Duration::from_secs(2);
                // Capacity-one signals never block destructor cleanup. A regression must
                // fail within a bound, even if construction unexpectedly succeeds.
                let (paid, paid_rx) = std::sync::mpsc::sync_channel(1);
                let (release, release_rx) = std::sync::mpsc::sync_channel(1);
                let (retiring, retiring_rx) = std::sync::mpsc::sync_channel(1);
                let ticket = ticket.fixture_retirement_signal(retiring);
                std::thread::scope(|scope| {
                    let work = scope.spawn(move || {
                        Preparation::new_after_payment(
                            ticket,
                            physical,
                            &session,
                            Payload::Open,
                            || {
                                let _ = paid.send(());
                                let _ = release_rx.recv_timeout(bound);
                            },
                        )
                    });
                    let paid_result = paid_rx.recv_timeout(bound);
                    let (release_result, retiring_result, busy, in_flight) = row
                        .sdp_test_hold_state(|| {
                            let released = release.try_send(());
                            let retiring = retiring_rx.recv_timeout(bound);
                            (
                                released,
                                retiring,
                                route.session().fixture_counts().0,
                                funding.in_flight(),
                            )
                        });
                    // Release State and every blocking channel BEFORE join/assert. If a
                    // bug returned Ok(Preparation), its eventual destructor remains safe.
                    drop(release);
                    drop(retiring_rx);
                    drop(paid_rx);
                    let result = work.join().unwrap();
                    assert!(paid_result.is_ok(), "actual task payment was not reached");
                    assert!(release_result.is_ok());
                    assert!(
                        retiring_result.is_ok(),
                        "actual Registration destructor was not reached"
                    );
                    assert!(busy, "physical idle cannot precede real worker retirement");
                    assert_eq!(
                        in_flight, 1,
                        "TaskCopy stays paid while Registration blocks on State"
                    );
                    assert!(matches!(result, Err(VoiceError::Capacity)));
                });
                drop(held);
                assert!(!route.session().fixture_counts().0);
                assert_eq!(funding.in_flight(), 0);
                let wall = clock.now_ms();
                let mono = clock.monotonic_ms();
                crate::voice_handoff::sdp::current(&context, route, |admission| {
                    row.resume_sdp_action(parent, admission, &candidate, wall, mono)
                })
                .unwrap()
                .unwrap()
            } else {
                ticket
            };
            let writer = ticket.publication_writer().unwrap();
            drop(ticket);
            let wall = clock.now_ms();
            let mono = clock.monotonic_ms();
            assert!(
                crate::voice_handoff::sdp::current(&context, route, |admission| row
                    .resume_sdp_action(parent, admission, &candidate, wall, mono))
                .unwrap()
                .is_none(),
                "a retained affine Writer excludes another actual worker"
            );
            drop(writer);
            let wall = clock.now_ms();
            let mono = clock.monotonic_ms();
            let ticket = crate::voice_handoff::sdp::current(&context, route, |admission| {
                row.resume_sdp_action(parent, admission, &candidate, wall, mono)
            })
            .unwrap()
            .unwrap();
            assert_eq!(ticket.id(), id);
            assert!(Arc::ptr_eq(&slot, &ticket.resolved().sdp_test_outcome()));

            let mut incoming = opened(state, public, route, parent, &encoded, seal);
            if seal {
                // Same Phase, wrong original collection counter: extraction must refuse with
                // the same opened allocation/content and no body/nonce duplication.
                let wrong = phase.native_history(NativeRowPurpose::Open).unwrap();
                let pointer = incoming.plain().unwrap().as_ptr();
                assert!(matches!(
                    incoming.sdp_body(intent.clone(), &wrong),
                    Err(channel::Refused::Binding)
                ));
                assert_eq!(incoming.plain().unwrap().as_ptr(), pointer);
                assert_eq!(incoming.plain().unwrap(), encoded);
            }
            let physical = incoming.hold_position().unwrap();
            let input = payload(&mut incoming, intent, &funding, seal);
            let preparation = Preparation::new(ticket, physical, route.session(), input).unwrap();
            let changed = row.sdp_test_notification();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(std::future::Future::poll(changed.as_mut(), &mut cx).is_pending());
            drop(preparation); // never launched: Writer must retire before Registration notifies
            assert!(std::future::Future::poll(changed.as_mut(), &mut cx).is_ready());
            assert!(matches!(slot.history(), History::NeverEntered));
            assert_eq!(funding.in_flight(), 0);
            drop(incoming);

            if seal {
                let mut incoming = opened(state, public, route, parent, &encoded, true);
                incoming.fixture_underpay_sdp_rpc(&phase, &funding);
                let pointer = incoming.plain().unwrap().as_ptr();
                let physical = route.session().fixture_counts();
                assert!(matches!(
                    incoming.sdp_body(intent.clone(), &funding),
                    Err(channel::Refused::Capacity)
                ));
                assert_eq!(
                    incoming.plain().unwrap().as_ptr(),
                    pointer,
                    "same allocation restored on constructor refusal"
                );
                assert_eq!(incoming.plain().unwrap(), encoded);
                assert_eq!(
                    route.session().fixture_counts(),
                    physical,
                    "constructor refusal spends no second nonce"
                );
                drop(incoming);
            }

            // The ACTUAL UI lock wins final admission. Nothing entered Root; reclaim the SAME
            // canonical row/outcome afterwards rather than allocating a replacement purpose.
            let wall = clock.now_ms();
            let mono = clock.monotonic_ms();
            let ticket = crate::voice_handoff::sdp::current(&context, route, |admission| {
                row.resume_sdp_action(parent, admission, &candidate, wall, mono)
            })
            .unwrap()
            .unwrap();
            let mut incoming = opened(state, public, route, parent, &encoded, seal);
            let physical = incoming.hold_position().unwrap();
            let input = payload(&mut incoming, intent, &funding, seal);
            let preparation = Preparation::new(ticket, physical, route.session(), input).unwrap();
            let held = state.ui_session_commit.clone().lock_owned().await;
            assert!(matches!(
                preparation.launch(&context, route),
                Err(factory::Refused::Protocol(VoiceError::Capacity))
            ));
            assert!(matches!(slot.history(), History::NeverEntered));
            drop(held);
            drop(incoming);
            assert_eq!(funding.in_flight(), 0);

            factory::preflight_binding(&context, route).await.unwrap();
            let (actor, _) = factory::native::observation_actor(&context, route).unwrap();
            if !seal {
                let wall = clock.now_ms();
                let mono = clock.monotonic_ms();
                let ticket = crate::voice_handoff::sdp::current(&context, route, |admission| {
                    row.resume_sdp_action(parent, admission, &candidate, wall, mono)
                })
                .unwrap()
                .unwrap();
                let incoming = opened(state, public, route, parent, &encoded, false);
                let preparation = Preparation::new(
                    ticket,
                    incoming.hold_position().unwrap(),
                    route.session(),
                    Payload::Open,
                )
                .unwrap();
                let completion = preparation.launch(&context, route).unwrap().await.unwrap();
                assert!(matches!(
                    completion.result(),
                    Err(factory::Refused::Protocol(VoiceError::Capacity))
                ));
                assert!(matches!(
                    slot.history(),
                    History::Refused(VoiceError::Capacity)
                ));
                assert!(!funding.is_closed());
                drop(completion);
                drop(incoming);
                assert_eq!(funding.in_flight(), 0);
                // The next claim below must resume THIS normal Root Capacity refusal.
            }
            let parked = super::pc::park(&actor).await;
            let wall = clock.now_ms();
            let mono = clock.monotonic_ms();
            let ticket = crate::voice_handoff::sdp::current(&context, route, |admission| {
                row.resume_sdp_action(parent, admission, &candidate, wall, mono)
            })
            .unwrap()
            .unwrap();
            let mut incoming = opened(state, public, route, parent, &encoded, seal);
            let physical = incoming.hold_position().unwrap();
            let input = payload(&mut incoming, intent, &funding, seal);
            // Ordinary pool pressure cannot block the original prepaid Task/Actor handoff.
            // Release it after the positive queue barrier; Root's later factory pressure is
            // covered separately and is not inferred from this native transport scenario.
            let resources = route.selected().resources();
            let group = route.selected().binding().scope.group;
            let held = resources
                .charge(
                    group,
                    catcoms_sync::voice_v2::resources::GROUP_BYTES
                        - resources.group_counts(group).unwrap().0,
                    catcoms_sync::voice_v2::resources::ResourceKind::Retained,
                )
                .unwrap();
            let preparation = Preparation::new(ticket, physical, route.session(), input).unwrap();
            let (queued, accepted) = tokio::sync::oneshot::channel();
            let mut waiter = Some(
                preparation
                    .launch_with_queue_barrier(&context, route, queued)
                    .unwrap(),
            );
            let mut parked = Some(parked);
            accepted.await.unwrap(); // positive actual mailbox admission, not a timing sleep
            drop(held);
            assert!(matches!(slot.history(), History::InFlight));
            drop(incoming);
            let held_output = if retain_output && seal {
                drop(parked.take());
                let completion = waiter.take().unwrap().await.unwrap();
                assert_eq!(completion.result().unwrap().id(), id);
                assert_eq!(
                    funding.in_flight(),
                    1,
                    "only original task payment remains; native request Body is retired"
                );
                let History::Published(action) = slot.history() else {
                    panic!("actual original Seal action");
                };
                let input = action.input().unwrap();
                let catcoms_sync::voice_v2::crypto_actions::CryptoInput::SealSdp(flight) =
                    input.as_input()
                else {
                    panic!("original Root input");
                };
                assert_eq!(flight.plaintext, b"v=0");
                drop(input);
                drop(action);
                Some(completion)
            } else {
                drop(waiter.take()); // waiter loss detaches the actual queued worker
                None
            };
            assert!(state.ui_session_commit.try_lock().is_err());
            assert!(matches!(
                route.session().header(),
                Err(channel::Refused::Capacity)
            ));
            state
                .voice_handoff
                .close_sdp_original(
                    parent,
                    route.selected(),
                    route.session().identity(),
                    &request,
                    purpose,
                )
                .unwrap();
            assert!(!row.try_compact_sdp().unwrap());
            assert!(!funding.is_collected());
            drop(parked.take());
            drop(held_output); // real last output, not a claimed drain bit, releases Task/position
            actor
                .voice_v2_context(route.selected().binding().scope.room)
                .await
                .unwrap();
            route.session().fixture_wait_physical_idle().await; // actual worker/output/body/publication collection
            assert!(state.ui_session_commit.try_lock().is_ok());
            assert!(funding.is_collected());
            if seal {
                let History::Published(action) = slot.history() else {
                    panic!("lost actual LOW publication");
                };
                assert_eq!(
                    row.status().unwrap().action,
                    Some((action.serial(), action.version(), action.input_tag()))
                );
            } else {
                assert!(
                    matches!(slot.history(), History::Refused(VoiceError::Capacity)),
                    "no incoming SDP exists yet"
                );
                assert!(row.status().unwrap().action.is_none());
            }
            assert!(row.try_compact_sdp().unwrap());
            assert_eq!(
                row.status().unwrap().engine.settled,
                seal,
                "only real Root unstarted retirement settles an engine"
            );
            let wall = clock.now_ms();
            let mono = clock.monotonic_ms();
            assert!(matches!(
                crate::voice_handoff::sdp::current(&context, route, |admission| row
                    .resume_sdp_action(parent, admission, &candidate, wall, mono)),
                Err(factory::Refused::Protocol(VoiceError::Unauthorized))
            ));
            assert_eq!(
                state.voice_handoff.sdp_test_rows(parent.id()),
                1,
                "retries never multiply rows"
            );
        },
    )
    .await
    .expect("original finite native SDP submission window");
}
