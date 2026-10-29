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
    // Intentionally retain NO Action or completion: only the row's original outcome
    // can recover the publication after parent/SDK closure.
    unreconciled: Option<ledger::ResolvedRow>,
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
        if let Some(row) = self.unreconciled {
            // Actual worker/body/completion and every caller Action are gone. Only this
            // row's SAME original outcome retains publication after parent/SDK close.
            assert!(!state.voice_handoff.channel_may_close(route.selected()));
            assert!(!state.voice_handoff.sdp_test_children_closed(parent));
            assert!(!row.status().unwrap().drained);
            use catcoms_sync::voice_v2::sdp::publication::History;
            let slot = row.sdp_test_outcome();
            let History::Published(action) = slot.history() else {
                panic!("original publication missing after actual parent close");
            };
            let identity = (action.serial(), action.version(), action.input_tag());
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
            drop(action);
            drop(slot); // Native must recover it itself; no real Arc is supplied below.
            let changed = row.sdp_test_notification();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(std::future::Future::poll(changed.as_mut(), &mut cx).is_pending());
            assert!(row.try_compact_sdp().unwrap());
            assert!(
                std::future::Future::poll(changed.as_mut(), &mut cx).is_ready(),
                "actual outcome recovery wakes a registered observer"
            );
            assert_eq!(row.status().unwrap().action, Some(identity));
            assert!(row.status().unwrap().engine.settled);
            let History::Published(action) = row.sdp_test_outcome().history() else {
                panic!("compaction discarded original publication");
            };
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
/// Maintenance shares the production nonblocking admission/Index fences. Only an exact
/// local Capacity means this synchronous call did not admit a producer. Retry the SAME
/// candidate, with fresh sanctioned clock samples, within the existing fixture bound;
/// never retry another refusal or discard an already successful Factory/Existing row.
pub(super) async fn current<T>(
    state: &AppState,
    context: &context::Context,
    route: &bootstrap::Route,
    intent: &InitialSdpIntent,
    apply: impl FnMut(
        &crate::voice_handoff::transition::Admission<'_>,
        u64,
        u64,
    ) -> Result<T, VoiceError>,
) -> Result<T, factory::Refused> {
    current_after_refusal(state, context, route, intent, apply, || {}).await
}

/// Deterministic fixture seam: a refusal hook may release the actual competing
/// guard only after the entire failed admission returned and released its own
/// native/Index/State guards. It cannot turn another refusal or an Ok(None) into
/// success. The default helper above has no hook or additional retry allowance.
pub(super) async fn current_after_refusal<T>(
    state: &AppState,
    context: &context::Context,
    route: &bootstrap::Route,
    intent: &InitialSdpIntent,
    mut apply: impl FnMut(
        &crate::voice_handoff::transition::Admission<'_>,
        u64,
        u64,
    ) -> Result<T, VoiceError>,
    mut after_refusal: impl FnMut(),
) -> Result<T, factory::Refused> {
    let clock = crate::voice_runtime(state).clock();
    let wall = clock.now_ms();
    let mono = clock.monotonic_ms();
    // Historical refusals still get one observation after Close. Only retries consume
    // this original remaining Ready interval; no new deadline is granted by the helper.
    let deadline = mono.saturating_add(intent.remaining_at(wall, mono).unwrap_or(0));
    for attempt in 0..32 {
        let wall = clock.now_ms();
        let mono = clock.monotonic_ms();
        let result = crate::voice_handoff::sdp::current(context, route, |admission| {
            apply(admission, wall, mono)
        });
        if !matches!(
            result,
            Err(factory::Refused::Protocol(VoiceError::Capacity))
        ) || attempt == 31
            || clock.monotonic_ms() >= deadline
        {
            return result;
        }
        after_refusal();
        // The failed admission holds no guard across this wait. Original Ready/intent deadlines and all
        // protocol/security refusals remain enforced by the actual admission path.
        clock
            .sleep(std::time::Duration::from_millis(
                10.min(deadline - clock.monotonic_ms().min(deadline)),
            ))
            .await;
    }
    unreachable!("last bounded attempt returns its exact result")
}

/// Negative historical closure has no positive-current/intent deadline gate.
/// Maintenance may hold any real Index/parent/row try-lock, including after the
/// idempotent purpose latch changed. Retry ONLY its actual local Capacity using
/// the SAME original parent/Session/request/purpose and the fixture's existing
///32-observation/10ms scheduling bound; never ask Root for a new action or grant.
///
/// Every call forces its first real Index refusal. The barrier is released
/// before any Clock wait, proving the fixture can recover from actual contention
/// rather than merely hoping the unforced full test schedules that ordering.
async fn close_original(
    state: &AppState,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    request: &requests::sdp_action::Begin<'_>,
    purpose: NativeRowPurpose,
) -> Result<Option<ledger::ResolvedRow>, VoiceError> {
    let clock = crate::voice_runtime(state).clock();
    // This synchronous frame cannot carry the actual mutex guard into a Clock
    // wait, including the acquisition loop's contended branch.
    let refuse_once = || {
        let blocked = state.voice_handoff.fixture_try_hold_index()?;
        let refused = state.voice_handoff.close_sdp_original(
            parent,
            route.selected(),
            route.session().identity(),
            request,
            purpose,
        );
        drop(blocked);
        assert!(matches!(refused, Err(VoiceError::Capacity)));
        Ok::<(), VoiceError>(())
    };
    for attempt in 0..32 {
        match refuse_once() {
            Ok(()) => break,
            Err(VoiceError::Capacity) if attempt < 31 => {
                clock.sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(error) => return Err(error),
        }
    }
    for attempt in 0..32 {
        let result = state.voice_handoff.close_sdp_original(
            parent,
            route.selected(),
            route.session().identity(),
            request,
            purpose,
        );
        if !matches!(result, Err(VoiceError::Capacity)) || attempt == 31 {
            return result;
        }
        clock.sleep(std::time::Duration::from_millis(10)).await;
    }
    unreachable!("the final bounded observation returns its exact result")
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
    let mismatch = mismatch_mode == 1;
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
    // The actual independent SDP allocator shares the parent-claim namespace.
    // A regression to rows.position(None) would steal precisely this first
    // vacant slot despite successful real PC/intent/current-Root authorization.
    let parent_claim = state
        .voice_handoff
        .reserve_parent(
            route.selected().clone(),
            route.session().identity().clone(),
            1,
        )
        .unwrap();
    let reserved_parent_id = parent_claim.fixture_id();
    let before = state.voice_handoff.sdp_test_rows(parent.id());
    // An outer Capacity occurs BEFORE this closure. Keep the complete successful batch,
    // including its affine Factory, then retry only individual LOCAL refusals below.
    let begins = current(state, &context, route, intent, |admission, wall, mono| {
        let barrier = std::sync::Barrier::new(8);
        Ok(std::thread::scope(|scope| {
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
            tasks
                .into_iter()
                .map(|task| task.join().unwrap())
                .collect::<Vec<_>>()
        }))
    })
    .await
    .unwrap();
    let mut admitted = Vec::with_capacity(begins.len());
    for result in begins {
        admitted.push(match result {
            Err(VoiceError::Capacity) => {
                current(state, &context, route, intent, |admission, wall, mono| {
                    state.voice_handoff.begin_sdp_action(
                        parent,
                        route.selected(),
                        admission,
                        &candidate,
                        wall,
                        mono,
                    )
                })
                .await
                .unwrap()
            }
            result => result.unwrap(),
        });
    }
    let mut factory = None;
    let mut ids = Vec::new();
    let mut outcomes = Vec::new();
    for begin in admitted {
        match begin {
            ledger::SdpBegin::Factory(ticket) => {
                outcomes.push(ticket.resolved().sdp_test_outcome());
                ids.push(ticket.id());
                assert!(
                    factory.replace(ticket).is_none(),
                    "one original Row allocation"
                );
            }
            ledger::SdpBegin::Existing(row) => {
                outcomes.push(row.sdp_test_outcome());
                ids.push(row.id());
            }
        }
    }
    let ticket = factory.expect("one Factory plus seven exact existing rows");
    assert_eq!(ids.len(), 8);
    assert!(ids.iter().all(|id| *id == ticket.id()));
    assert!(ids
        .iter()
        .all(|id| id.logical()[..8] != reserved_parent_id.logical()[..8]));
    assert!(state
        .voice_handoff
        .fixture_parent_reserved(reserved_parent_id));
    drop(parent_claim);
    assert!(!state
        .voice_handoff
        .fixture_parent_reserved(reserved_parent_id));
    assert!(
        outcomes.iter().all(|slot| Arc::ptr_eq(slot, &outcomes[0])),
        "every competing candidate observes the ONE actually installed outcome"
    );
    drop(outcomes);
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
    // Deterministically reproduce supported admission contention, rather than relying on
    // the periodic collector happening to win this turn. Capacity is BEFORE the callback;
    // it must leave the one original row/outcome and its unstarted publication untouched.
    let canonical = row.sdp_test_outcome();
    let commit = state.ui_session_commit.clone().lock_owned().await;
    let mut entered = false;
    let blocked = crate::voice_handoff::sdp::current(&context, route, |_| {
        entered = true;
        Ok(())
    });
    assert_eq!(
        blocked,
        Err(factory::Refused::Protocol(VoiceError::Capacity))
    );
    assert!(
        !entered,
        "local admission refusal must not enter the operation"
    );
    drop(commit);
    assert_eq!(state.voice_handoff.sdp_test_rows(parent.id()), before + 1);
    assert!(Arc::ptr_eq(&canonical, &row.sdp_test_outcome()));
    assert!(row.status().unwrap().action.is_none());
    drop(canonical);
    assert_eq!(
        current(state, &context, route, intent, |admission, wall, mono| {
            state.voice_handoff.begin_sdp_action(
                parent,
                route.selected(),
                admission,
                &changed_candidate,
                wall,
                mono,
            )
        })
        .await
        .map(|_| ()),
        Err(factory::Refused::Protocol(VoiceError::RetryMismatch)),
        "changed original SDP input must be rejected before publication"
    );
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
        let publication = sdp_publication::Publication::preparing(
            incoming.hold_position().unwrap(),
            route.session(),
            intent,
            task,
        )
        .unwrap();
        let writer = ticket.publication_writer().unwrap();
        // A real separately funded Root outcome for this same PC/intent is not this
        // native row's canonical slot. No native Open row is installed here and this
        // unused test-only outcome retires before the later Open-row exercise.
        let foreign_outcome = if mismatch_mode == 2 {
            Some(
                catcoms_sync::voice_v2::sdp::publication::Prepared::new(
                    intent.clone(),
                    phase.native_history(NativeRowPurpose::Open).unwrap(),
                    None,
                )
                .unwrap()
                .install(),
            )
        } else {
            None
        };
        let foreign_writer = foreign_outcome
            .as_ref()
            .map(|slot| slot.try_writer().unwrap());
        let paused = super::pc::park(actor).await;
        let mut buffer = phase
            .copy(3 + VOICE_SDP_RPC_METADATA_BYTES)
            .unwrap()
            .adopt(b"v=0".to_vec())
            .unwrap();
        buffer.bind_native_history(funding.clone()).unwrap();
        // The exact admitted UI guard moves into publication; no second lock/generation
        // snapshot or literal unlocked boolean substitutes after the native fence.
        let mut admitted_commit = None;
        for attempt in 0..32 {
            // Clock callbacks run before every native guard. A parked Actor command
            // must never turn contention on its UI fence into an unbounded await.
            let wall = clock.now_ms();
            let mono = clock.monotonic_ms();
            let result = (|| {
                let live = context.upgrade().ok_or(VoiceError::Unauthorized)?;
                let commit = live
                    .commit
                    .clone()
                    .try_lock_owned()
                    .map_err(|_| VoiceError::Capacity)?;
                {
                    let authority = live
                        .authority
                        .try_lock()
                        .map_err(|_| VoiceError::Capacity)?;
                    let admission = crate::voice_handoff::transition::Admission::new(
                        &live, &authority, route, &commit,
                    );
                    if let Some(foreign) = &foreign_writer {
                        match route
                            .try_with_open(|| ticket.mark_handoff(&admission, foreign, wall, mono))
                        {
                            Err(VoiceError::Unauthorized) => {}
                            Err(VoiceError::Capacity) => return Err(VoiceError::Capacity),
                            result => {
                                panic!("same-intent foreign slot must refuse admission: {result:?}")
                            }
                        }
                    }
                    route.try_with_open(|| ticket.mark_handoff(&admission, &writer, wall, mono))?;
                }
                Ok(commit)
            })();
            match result {
                Ok(commit) => {
                    admitted_commit = Some(commit);
                    break;
                }
                Err(VoiceError::Capacity) if attempt < 31 => {
                    // Every guard has left the failed synchronous attempt. This SAME
                    // original Writer may retry only within its original Ready interval.
                    let remaining = intent
                        .remaining_at(clock.now_ms(), clock.monotonic_ms())
                        .unwrap();
                    clock
                        .sleep(std::time::Duration::from_millis(10.min(remaining)))
                        .await;
                }
                Err(error) => panic!("exact original handoff refused: {error:?}"),
            }
        }
        let commit = admitted_commit.expect("one exact original publication admission");
        drop(foreign_writer);
        drop(foreign_outcome);
        publication.admit(commit);
        let publication = Arc::new(publication);
        buffer.attach_keepalive(publication.clone()).unwrap();
        let body = VoiceSdpBody::from_buffer(buffer, 0..3, intent.clone()).unwrap();
        drop(publication);
        let pending = actor
            .queue_voice_sdp_offer_recorded(
                grant.clone(),
                request.pc.base.handshake,
                intent.clone(),
                &body,
                writer,
            )
            .await
            .unwrap();
        drop(incoming);
        assert_eq!(funding.in_flight(), 2);
        // Close wins with a genuine command already queued. It cannot erase that command
        // or assume no publication; the late exact Action is captured even while closed.
        if mismatch_mode != 3 {
            let closed = close_original(state, route, parent, &request, NativeRowPurpose::Seal)
                .await
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
        if mismatch {
            // The canonical Writer binds native's original v=1 body. A v=0 producer
            // mixup is now refused BEFORE Root publication, rather than cleaned up after
            // publishing a different Action. No synthetic engine/ACK may appear.
            assert_eq!(completion.result().err(), Some(VoiceError::Unauthorized));
            assert!(row.status().unwrap().action.is_none());
            assert!(!row.try_compact_sdp().unwrap());
            drop(ticket);
            drop(body);
            assert!(!row.try_compact_sdp().unwrap());
            drop(completion);
            // Receiving completion can overtake the sender's Work destructor. A real
            // same-Actor FIFO command proves that sender has returned; it never releases
            // any still-held body/completion/native owner on the fixture's behalf.
            actor
                .delivery_snapshot(catcoms_app::channel_id("general"))
                .await
                .unwrap();
            assert!(funding.is_collected());
            assert!(row.try_compact_sdp().unwrap());
            let status = row.status().unwrap();
            assert!(status.action.is_none() && !status.engine.settled && !status.acknowledged);
        } else {
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
                let unreconciled_row = ticket.resolved();
                drop(ticket);
                drop(body);
                drop(completion);
                actor
                    .delivery_snapshot(catcoms_app::channel_id("general"))
                    .await
                    .unwrap();
                assert!(!funding.is_closed());
                assert_eq!(
                    row.capture_closed_sdp_publication(route.session().identity(), &action),
                    Err(VoiceError::Unauthorized),
                    "open IO forbids historical adoption"
                );
                assert!(row.status().unwrap().action.is_none());
                close_original(state, route, parent, &request, NativeRowPurpose::Seal)
                    .await
                    .unwrap();
                assert!(funding.is_collected());
                assert!(row.status().unwrap().action.is_none());
                // Do not reconcile yet: after parent close there must be no caller-owned
                // Action/SDK/Actor reply left to supply the real publication.
                assert!(!state.voice_handoff.channel_may_close(route.selected()));
                assert!(!state.voice_handoff.sdp_test_children_closed(parent));
                drop(action);
                retained.unreconciled = Some(unreconciled_row);
            } else {
                if mismatch_mode == 2 {
                    // A real Root permit must survive ordinary native row closure. No SDK result
                    // is fabricated here: the fixture reports settlement only after checking
                    // that negative-only native retirement cannot remove this actual permit.
                    action.admit_engine(action.engine_bytes()).unwrap();
                }
                ticket.capture(&action).unwrap();
                ticket.attach_published().unwrap();
                assert_eq!(
                    row.status().unwrap().action,
                    Some((action.serial(), action.version(), action.input_tag()))
                );
                if mismatch_mode != 2 {
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
                actor
                    .delivery_snapshot(catcoms_app::channel_id("general"))
                    .await
                    .unwrap();
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
        }
    } else {
        // HIGH has no applied incoming Offer in this fixture. Close an actually unstarted
        // native purpose, never ask Root to fabricate an Answer or an input token.
        close_original(state, route, parent, &request, NativeRowPurpose::Seal)
            .await
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
        retained.unreconciled.is_none()
    );

    // Close-before-install persists even when there is no child RowId. A retained already
    // paid candidate cannot revive that purpose; lookup absence is merely intent history.
    let encoded_open = bytes(parent, pc_frame, None);
    let open = decode(parent, &encoded_open, false);
    let open_candidate = parent
        .sdp_action_candidate(route.session().identity(), &open)
        .unwrap();
    if matches!(mismatch_mode, 1 | 2) {
        // Retain a genuinely unstarted Open row through the parent PC/SDK close. Its
        // worker is gone, but open/zero IO is not terminal and cannot compact yet.
        let ledger::SdpBegin::Factory(open_ticket) =
            current(state, &context, route, intent, |admission, wall, mono| {
                state.voice_handoff.begin_sdp_action(
                    parent,
                    route.selected(),
                    admission,
                    &open_candidate,
                    wall,
                    mono,
                )
            })
            .await
            .unwrap()
        else {
            panic!("one original Open row");
        };
        let open_row = open_ticket.resolved();
        if mismatch_mode == 1 {
            let writer = open_ticket.publication_writer().unwrap();
            open_row
                .close_sdp_action(route.session().identity())
                .unwrap();
            drop(open_ticket);
            assert!(
                !open_row.try_compact_sdp().unwrap(),
                "CLOSED+zero and worker retirement cannot overtake a real reserved Writer"
            );
            drop(writer);
            assert!(open_row.try_compact_sdp().unwrap());
            let status = open_row.status().unwrap();
            assert!(status.action.is_none() && !status.engine.settled && !status.acknowledged);
            return retained;
        }
        drop(open_ticket);
        assert!(!open_row.try_compact_sdp().unwrap());
        retained.unstarted = Some(open_row);
        return retained;
    }
    assert!(
        close_original(state, route, parent, &open, NativeRowPurpose::Open)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        current(state, &context, route, intent, |admission, wall, mono| {
            state.voice_handoff.begin_sdp_action(
                parent,
                route.selected(),
                admission,
                &open_candidate,
                wall,
                mono,
            )
        })
        .await
        .map(|_| ()),
        Err(factory::Refused::Protocol(VoiceError::Unauthorized)),
        "a closed original purpose never revives after local contention"
    );
    assert!(
        phase.effect().is_live(),
        "neither purpose Return retires the shared storage/PC"
    );
    retained
}
