//! Actual original PC/Phase/Actor rows. Scoped threads are fixture-only scheduling for
//! candidate uniqueness, not a claim that production may spawn unfunded parallel workers.
//! The LOW case publishes a real Root Offer action, but performs no SDK/SDP encryption/RTC.
use super::*;
use catcoms_app::{ServerActor, VoiceSdpBody, VOICE_SDP_RPC_METADATA_BYTES};
use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;
use catcoms_sync::voice_v2::sdp::InitialSdpIntent;
use catcoms_sync::voice_v2::{CallGrant, VoiceError};

#[derive(Default)]
pub(super) struct Retained {
    unstarted: Option<ledger::ResolvedRow>,
    unresolved: Option<(
        ledger::ResolvedRow,
        Arc<catcoms_sync::voice_v2::crypto_actions::CryptoAction>,
    )>,
}
impl Retained {
    pub(super) fn after_parent_close(
        self,
        state: &AppState,
        route: &bootstrap::Route,
        parent: &ledger::ResolvedRow,
    ) {
        if let Some(row) = self.unstarted {
            assert!(
                row.try_compact_sdp().unwrap(),
                "parent close retires unstarted Open without Return"
            );
            let status = row.status().unwrap();
            assert!(status.drained && status.action.is_none());
            assert!(
                !status.engine.settled,
                "native absence cannot invent a Root engine"
            );
        }
        if let Some((row, action)) = self.unresolved {
            // Actual worker/body/completion are gone and parent storage is closed. None
            // of those observations is evidence that this handed-off action was absent.
            assert!(!row.try_compact_sdp().unwrap());
            assert!(!state.voice_handoff.channel_may_close(route.selected()));
            assert!(!state.voice_handoff.sdp_test_children_closed(parent));
            assert!(!row.status().unwrap().drained);
            // The retained actual Root Arc is positive publication evidence. This proves
            // the closed capture leaf, not a working Actor recovery/negative-witness RPC.
            let foreign = channel::tests::FixtureChannel::new(
                route.selected().resources(),
                route.selected().binding().scope.group,
            );
            assert_eq!(
                row.capture_closed_sdp_publication(foreign.session.identity(), &action),
                Err(VoiceError::Unauthorized)
            );
            assert!(row.status().unwrap().action.is_none());
            drop(foreign);
            let changed = row.sdp_test_notification();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(std::future::Future::poll(changed.as_mut(), &mut cx).is_pending());
            row.capture_closed_sdp_publication(route.session().identity(), &action)
                .unwrap();
            assert!(
                std::future::Future::poll(changed.as_mut(), &mut cx).is_ready(),
                "actual historical capture wakes a registered observer"
            );
            assert_eq!(
                row.status().unwrap().action,
                Some((action.serial(), action.version(), action.input_tag()))
            );
            assert!(row.try_compact_sdp().unwrap());
            assert!(row.status().unwrap().engine.settled);
            let tag = catcoms_sync::voice_v2::sdp::plaintext_commitment(b"v=0").unwrap();
            assert!(action.matches_sdp_plaintext_commitment(&tag));
        }
    }
}

fn bytes(parent: &ledger::ResolvedRow, pc_frame: &[u8], plaintext: Option<&[u8]>) -> Vec<u8> {
    let mut bytes = pc_frame[..193].to_vec();
    bytes[0] = 8;
    bytes.extend_from_slice(&parent.pc_test_proof());
    if let Some(plain) = plaintext {
        bytes.extend_from_slice(&(plain.len() as u16).to_be_bytes());
        bytes.extend_from_slice(plain);
    }
    bytes
}
fn decode<'a>(
    parent: &ledger::ResolvedRow,
    body: &'a [u8],
    seal: bool,
) -> requests::sdp_action::Begin<'a> {
    let request = requests::sdp_action::Request::decode(
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
        body,
    )
    .unwrap();
    let requests::sdp_action::Request::Begin(request) = request else {
        panic!("actual Begin");
    };
    request
}
// These are independently admitted original native objects, not reconstructed identifiers.
#[allow(clippy::too_many_arguments)]
pub(super) async fn exercise(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    pc_frame: &[u8],
    actor: &ServerActor,
    grant: &Arc<CallGrant>,
    intent: &Arc<InitialSdpIntent>,
    mismatch_mode: u8,
) -> Retained {
    let mismatch = matches!(mismatch_mode, 1 | 2);
    let mut retained = Retained::default();
    let context = context::Context::new(state);
    let phase = parent.sdp_test_phase().unwrap();
    let clock = crate::voice_runtime(state).clock();
    let encoded = bytes(
        parent,
        pc_frame,
        Some(if mismatch { b"v=1" } else { b"v=0" }),
    );
    let request = decode(parent, &encoded, true);
    let candidate = parent
        .sdp_action_candidate(route.session().identity(), &request)
        .unwrap();
    let funding = candidate.funding();
    let before = state.voice_handoff.sdp_test_rows(parent.id());
    let wall = clock.now_ms();
    let mono = clock.monotonic_ms();
    let begins = crate::voice_handoff::sdp::current(&context, route, |admission| {
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            let tasks: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        state.voice_handoff.begin_sdp_action(
                            parent,
                            route.selected(),
                            admission,
                            &candidate,
                            wall,
                            mono,
                        )
                    })
                })
                .collect();
            // Try-lock contention is a proven LOCAL refusal. Once the concurrent
            // callers have returned, retry only Capacity with the SAME original paid
            // candidate. Never reinterpret an unknown publication as unstarted work.
            let results: Vec<_> = tasks.into_iter().map(|task| task.join().unwrap()).collect();
            results
                .into_iter()
                .map(|result| match result {
                    Err(VoiceError::Capacity) => state.voice_handoff.begin_sdp_action(
                        parent,
                        route.selected(),
                        admission,
                        &candidate,
                        wall,
                        mono,
                    ),
                    result => result,
                })
                .collect::<Result<Vec<_>, _>>()
        })
    })
    .unwrap();
    let mut factory = None;
    let mut ids = Vec::new();
    for begin in begins {
        match begin {
            ledger::SdpBegin::Factory(ticket) => {
                ids.push(ticket.id());
                assert!(
                    factory.replace(ticket).is_none(),
                    "one original Row allocation"
                );
            }
            ledger::SdpBegin::Existing(row) => ids.push(row.id()),
        }
    }
    let ticket = factory.expect("one Factory plus seven exact existing rows");
    assert_eq!(ids.len(), 8);
    assert!(ids.iter().all(|id| *id == ticket.id()));
    assert_eq!(
        state.voice_handoff.sdp_test_rows(parent.id()),
        before + 1,
        "one counted SDP row for all original candidates"
    );
    let row = ticket.resolved();
    assert!(!state.voice_handoff.channel_may_close(route.selected()));
    assert!(
        matches!(row.confirmation_status(), Err(VoiceError::Unauthorized)),
        "no confirmation fallback"
    );
    let changed = bytes(
        parent,
        pc_frame,
        Some(if mismatch { b"v=0" } else { b"v=1" }),
    );
    let changed = decode(parent, &changed, true);
    let changed_candidate = parent
        .sdp_action_candidate(route.session().identity(), &changed)
        .unwrap();
    let wall = clock.now_ms();
    let mono = clock.monotonic_ms();
    assert!(matches!(
        crate::voice_handoff::sdp::current(&context, route, |admission| state
            .voice_handoff
            .begin_sdp_action(
                parent,
                route.selected(),
                admission,
                &changed_candidate,
                wall,
                mono
            )),
        Err(factory::Refused::Protocol(VoiceError::RetryMismatch))
    ));
    assert!(!row.try_compact_sdp().unwrap());

    if intent.local_is_low() {
        // Bind the worker and actual body to this SAME row's IO before final queueing.
        let incoming = opened_frame(
            public,
            route,
            (
                route.session().fixture_counts().1 + 1,
                14,
                0,
                parent.id().logical(),
            ),
            pc_frame,
            None,
        );
        let mut task = phase.task_copy(sdp_publication::TASK_BYTES).unwrap();
        task.bind_native_history(funding.clone()).unwrap();
        let mut publication = sdp_publication::Publication::preparing(
            incoming.hold_position().unwrap(),
            route.session(),
            intent,
            task,
        )
        .unwrap();
        let paused = super::pc::park(actor).await;
        let mut buffer = phase
            .copy(3 + VOICE_SDP_RPC_METADATA_BYTES)
            .unwrap()
            .adopt(b"v=0".to_vec())
            .unwrap();
        buffer.bind_native_history(funding.clone()).unwrap();
        // The exact admitted UI guard moves into publication; no second lock/generation
        // snapshot or literal unlocked boolean substitutes after the native fence.
        let wall = clock.now_ms();
        let mono = clock.monotonic_ms();
        let live = context.upgrade().unwrap();
        let commit = live.commit.clone().try_lock_owned().unwrap();
        {
            let authority = live.authority.try_lock().unwrap();
            let admission =
                crate::voice_handoff::transition::Admission::new(&live, &authority, route, &commit);
            route
                .try_with_open(|| ticket.mark_handoff(&admission, wall, mono))
                .unwrap();
        }
        drop(live);
        publication.admit(commit);
        let publication = Arc::new(publication);
        buffer.attach_keepalive(publication.clone()).unwrap();
        let body = VoiceSdpBody::from_buffer(buffer, 0..3, intent.clone()).unwrap();
        drop(publication);
        let pending = actor
            .queue_voice_sdp_offer(
                grant.clone(),
                request.pc.base.handshake,
                intent.clone(),
                &body,
            )
            .await
            .unwrap();
        drop(incoming);
        assert_eq!(funding.in_flight(), 2);
        // Close wins with a genuine command already queued. It cannot erase that command
        // or assume no publication; the late exact Action is captured even while closed.
        if mismatch_mode != 3 {
            let closed = state
                .voice_handoff
                .close_sdp_original(
                    parent,
                    route.selected(),
                    route.session().identity(),
                    &request,
                    NativeRowPurpose::Seal,
                )
                .unwrap()
                .unwrap();
            assert_eq!(closed.id(), row.id());
        }
        assert!(!row.try_compact_sdp().unwrap());
        assert!(
            phase.effect().is_live(),
            "Return must not close sibling PC work"
        );
        drop(paused);
        let completion = pending.complete().await.unwrap();
        let action = Arc::clone(completion.result().unwrap());
        let exact = catcoms_sync::voice_v2::sdp::plaintext_commitment(b"v=0").unwrap();
        let wrong = catcoms_sync::voice_v2::sdp::plaintext_commitment(b"v=1").unwrap();
        assert!(action.matches_sdp_plaintext_commitment(&exact));
        assert!(!action.matches_sdp_plaintext_commitment(&wrong));
        assert_eq!(
            row.capture_closed_sdp_publication(route.session().identity(), &action),
            Err(VoiceError::Unauthorized),
            "retained worker forbids historical adoption"
        );
        assert!(row.status().unwrap().action.is_none());
        if mismatch_mode == 3 {
            let unresolved_row = ticket.resolved();
            drop(ticket);
            drop(body);
            drop(completion);
            assert!(!funding.is_closed());
            assert_eq!(
                row.capture_closed_sdp_publication(route.session().identity(), &action),
                Err(VoiceError::Unauthorized),
                "open IO forbids historical adoption"
            );
            assert!(row.status().unwrap().action.is_none());
            state
                .voice_handoff
                .close_sdp_original(
                    parent,
                    route.selected(),
                    route.session().identity(),
                    &request,
                    NativeRowPurpose::Seal,
                )
                .unwrap();
            assert!(funding.is_collected());
            assert!(row.status().unwrap().action.is_none());
            assert!(
                !row.try_compact_sdp().unwrap(),
                "unknown handed-off is not unstarted"
            );
            assert!(!state.voice_handoff.channel_may_close(route.selected()));
            assert!(!state.voice_handoff.sdp_test_children_closed(parent));
            retained.unresolved = Some((unresolved_row, action));
        } else {
            if mismatch_mode == 2 {
                // A real Root permit must survive native mismatch cleanup. No SDK result
                // is fabricated here: the fixture reports settlement only after checking
                // that negative-only native retirement cannot remove this actual permit.
                action.admit_engine(action.engine_bytes()).unwrap();
            }
            if mismatch {
                assert_eq!(ticket.capture(&action), Err(VoiceError::RetryMismatch));
                assert_eq!(ticket.attach_published(), Err(VoiceError::Unauthorized));
            } else {
                ticket.capture(&action).unwrap();
                ticket.attach_published().unwrap();
            }
            assert_eq!(
                row.status().unwrap().action,
                Some((action.serial(), action.version(), action.input_tag()))
            );
            if !mismatch {
                action.settle_engine(); // This actual factory action never admitted an SDK engine.
            }
            drop(ticket);
            assert!(
                !row.try_compact_sdp().unwrap(),
                "actual body/completion/publication still held"
            );
            drop(body);
            assert!(!row.try_compact_sdp().unwrap());
            drop(completion);
            assert!(funding.is_collected());
            if mismatch_mode == 2 {
                assert!(
                    !row.try_compact_sdp().unwrap(),
                    "admitted engine is not unstarted"
                );
                assert!(!row.status().unwrap().engine.settled);
                action.settle_engine();
            }
            assert!(row.try_compact_sdp().unwrap());
            assert!(row.status().unwrap().engine.settled);
            assert!(
                action.matches_sdp_plaintext_commitment(&exact),
                "survives actual input compaction"
            );
            assert!(!action.matches_sdp_plaintext_commitment(&wrong));
            assert!(phase.effect().is_live());
        }
    } else {
        // HIGH has no applied incoming Offer in this fixture. Close an actually unstarted
        // native purpose, never ask Root to fabricate an Answer or an input token.
        state
            .voice_handoff
            .close_sdp_original(
                parent,
                route.selected(),
                route.session().identity(),
                &request,
                NativeRowPurpose::Seal,
            )
            .unwrap();
        assert!(!row.try_compact_sdp().unwrap());
        drop(ticket);
        assert!(row.try_compact_sdp().unwrap());
        let status = row.status().unwrap();
        assert!(status.action.is_none());
        assert!(!status.engine.settled);
    }
    let recovered = state
        .voice_handoff
        .lookup_sdp_action(
            parent,
            route.selected(),
            route.session().identity(),
            &request,
            NativeRowPurpose::Seal,
        )
        .unwrap()
        .unwrap();
    assert_eq!(recovered.id(), row.id());
    assert_eq!(
        recovered.status().unwrap().drained,
        retained.unresolved.is_none()
    );

    // Close-before-install persists even when there is no child RowId. A retained already
    // paid candidate cannot revive that purpose; lookup absence is merely intent history.
    let encoded_open = bytes(parent, pc_frame, None);
    let open = decode(parent, &encoded_open, false);
    let open_candidate = parent
        .sdp_action_candidate(route.session().identity(), &open)
        .unwrap();
    if mismatch {
        // Retain a genuinely unstarted Open row through the parent PC/SDK close. Its
        // worker is gone, but open/zero IO is not terminal and cannot compact yet.
        let wall = clock.now_ms();
        let mono = clock.monotonic_ms();
        let ledger::SdpBegin::Factory(open_ticket) =
            crate::voice_handoff::sdp::current(&context, route, |admission| {
                state.voice_handoff.begin_sdp_action(
                    parent,
                    route.selected(),
                    admission,
                    &open_candidate,
                    wall,
                    mono,
                )
            })
            .unwrap()
        else {
            panic!("one original Open row");
        };
        let open_row = open_ticket.resolved();
        drop(open_ticket);
        assert!(!open_row.try_compact_sdp().unwrap());
        retained.unstarted = Some(open_row);
        return retained;
    }
    assert!(state
        .voice_handoff
        .close_sdp_original(
            parent,
            route.selected(),
            route.session().identity(),
            &open,
            NativeRowPurpose::Open
        )
        .unwrap()
        .is_none());
    let wall = clock.now_ms();
    let mono = clock.monotonic_ms();
    assert!(matches!(
        crate::voice_handoff::sdp::current(&context, route, |admission| state
            .voice_handoff
            .begin_sdp_action(
                parent,
                route.selected(),
                admission,
                &open_candidate,
                wall,
                mono
            )),
        Err(factory::Refused::Protocol(VoiceError::Unauthorized))
    ));
    assert!(
        phase.effect().is_live(),
        "neither purpose Return retires the shared storage/PC"
    );
    retained
}
