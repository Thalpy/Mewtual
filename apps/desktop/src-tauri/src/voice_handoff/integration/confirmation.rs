//! Actual native three-flight confirmation over the fixture's real permitted TCP graph.
//! Test-only AES-GCM keys stand in for already-derived browser keys. Actual encrypted frames
//! exercise native child Input/admission/results/cleanup, not WebCrypto/P256/HKDF, production
//! effect dispatch, SDP/ICE/RTC or audible-media acceptance.
use super::*;
use aes_gcm::{aead::AeadInPlace, Aes256Gcm, KeyInit, Nonce, Tag};
use catcoms_rt::{Clock, SystemClock};
use catcoms_sync::voice_v2::crypto_actions::{CryptoInput, CryptoOperation, CryptoOutcome};
use catcoms_sync::voice_v2::VoiceError;
use std::time::Duration;

async fn route(state: &AppState, selected: &SelectedHandoffOwner) -> bootstrap::Route {
    let bootstrap::Status::Ready(public) = bootstrap::ensure(
        state,
        selected.id(),
        selected.executor(),
        &mut catcoms_rt::OsCryptoRng,
    )
    .await
    .unwrap() else {
        panic!("original channel ready")
    };
    state.voice_channels.route(public.public().slot).unwrap()
}
async fn actor(state: &AppState, selected: &SelectedHandoffOwner) -> catcoms_app::ServerActor {
    state
        .servers
        .lock()
        .await
        .get(&selected.binding().server)
        .unwrap()
        .actor
        .clone()
}
fn resolve(
    state: &AppState,
    selected: &SelectedHandoffOwner,
    parent: &ledger::ResolvedRow,
    request: ledger::ConfirmationPurpose,
    id: ledger::RowId,
) -> ledger::ResolvedRow {
    state
        .voice_handoff
        .resolve(
            selected,
            ledger::Purpose::Confirmation {
                parent: parent.id(),
                request,
            },
            id,
        )
        .unwrap()
}
async fn outgoing(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    parent: &ledger::ResolvedRow,
    request: ledger::ConfirmationPurpose,
) -> ledger::ResolvedRow {
    let bytes = begin_frame(state, selected, parent, request).await;
    assert_eq!(bytes.len(), replies::STATUS_BYTES);
    let id = ledger::RowId::from_logical(bytes[2..18].try_into().unwrap()).unwrap();
    resolve(state, selected, parent, request, id)
}

/// Actual v2 Begin(true) request/Control reply, independently authenticated by the browser
/// codec fixture. These allocation-heavy fixture Vecs do not claim application-copy bounds.
async fn begin_frame(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    parent: &ledger::ResolvedRow,
    purpose: ledger::ConfirmationPurpose,
) -> Vec<u8> {
    let bootstrap::Status::Ready(public) = bootstrap::ensure(
        state,
        selected.id(),
        selected.executor(),
        &mut catcoms_rt::OsCryptoRng,
    )
    .await
    .unwrap() else {
        panic!("original channel")
    };
    let route = state.voice_channels.route(public.public().slot).unwrap();
    begin_frame_on_route(state, &public, &route, parent, purpose).await
}

/// Reuse the actual original channel after setup. Contention fixtures must not call
/// bootstrap while holding the server map: bootstrap legitimately waits for that map.
/// Only the exact framed Begin below exercises its deliberately nonblocking preflight.
async fn begin_frame_on_route(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    purpose: ledger::ConfirmationPurpose,
) -> Vec<u8> {
    let selector = parent.confirmation_selector().unwrap();
    let mut body = confirmation_prefix(parent.id(), purpose, selector);
    body.push(1); // exact original factory/attachment retry, never a replacement purpose
    let header = (route.session().fixture_counts().1 + 1, 8, 0, [0; 16]);
    let reply = confirmation_begin::begin(
        &context::Context::new(state),
        route,
        opened_frame(public, route, header, &body, None),
    )
    .await
    .unwrap();
    opened_reply(public, header, &body, &reply)
}

/// Known lost-input publications must reach the exact permanent refusal even when local
/// preflight is temporarily busy. Capacity is an observation to retry, never a passing
/// negative result or evidence that the rotating incoming snapshot was consulted.
async fn known_attachment_refusals(
    state: &AppState,
    selected: &SelectedHandoffOwner,
    parent: &ledger::ResolvedRow,
    purpose: ledger::ConfirmationPurpose,
    child: &ledger::ResolvedRow,
) {
    let status = child.status().unwrap();
    assert!(status.action.is_none() && child.has_pending_publication().unwrap());
    let bootstrap::Status::Ready(public) = bootstrap::ensure(
        state,
        selected.id(),
        selected.executor(),
        &mut catcoms_rt::OsCryptoRng,
    )
    .await
    .unwrap() else {
        panic!("same original channel")
    };
    let route = state.voice_channels.route(public.public().slot).unwrap();
    let clock = crate::voice_runtime(state).clock();
    // One fixed harness bound covers forced contention and both required final refusals.
    // It never changes the original Root pair/action deadline, purpose or Action allocation.
    tokio::time::timeout(Duration::from_secs(15), async {
        let busy = state.servers.lock().await;
        assert_eq!(
            begin_frame_on_route(state, &public, &route, parent, purpose).await,
            replies::refused(factory::Refused::Protocol(VoiceError::Capacity)),
            "actual local preflight contention must not reach attachment"
        );
        assert!(child.status().unwrap() == status);
        assert!(child.has_pending_publication().unwrap());
        drop(busy);
        for _ in 0..2 {
            loop {
                let bytes = begin_frame_on_route(state, &public, &route, parent, purpose).await;
                assert!(child.status().unwrap() == status);
                assert!(child.has_pending_publication().unwrap());
                if bytes == replies::refused(factory::Refused::Protocol(VoiceError::Capacity)) {
                    // The actual framed reply/physical owner is collected before sleeping.
                    // A broken empty-snapshot fallback stays Capacity and exhausts the bound.
                    clock.sleep(Duration::from_millis(25)).await;
                    continue;
                }
                assert_eq!(
                    bytes,
                    replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized)),
                    "known action attachment refusal, not missing snapshot Capacity"
                );
                break;
            }
        }
    })
    .await
    .expect("original known attachment did not recover from local preflight contention");
}

/// Two real native registrations compete for the SAME actual Root Hello. Exercise both
/// explicit claim orderings with a barrier, without sleeps or synthetic action identities.
/// The wrong normalized predecessor must refuse BEFORE storing the known publication.
pub(crate) async fn confirmation_alias_claims(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    parent: &ledger::ResolvedRow,
    wrong_first: bool,
    lose_input: bool,
) {
    let context = context::Context::new(state).upgrade().unwrap();
    let _commit = context.commit.clone().lock_owned().await;
    context.gate().require_unlocked().await.unwrap();
    {
        let authority = context.authority.lock().unwrap();
        factory::require_current_selection(context.gate(), &authority, selected).unwrap();
    }
    let valid_request = ledger::ConfirmationPurpose::Start(None);
    let wrong_request = ledger::ConfirmationPurpose::Start(Some([254; 32]));
    let (ledger::Begin::Factory(valid), _) = context
        .ledger
        .begin_child(selected.clone(), parent, valid_request)
        .unwrap()
    else {
        panic!("original native factory")
    };
    let (ledger::Begin::Factory(wrong), _) = context
        .ledger
        .begin_child(selected.clone(), parent, wrong_request)
        .unwrap()
    else {
        panic!("distinct normalized purpose")
    };
    valid.mark_handoff().unwrap();
    let action = actor(state, selected)
        .await
        .start_voice_confirmation(
            selected.grant().unwrap(),
            parent.confirmation_handshake().unwrap(),
            None,
        )
        .await
        .unwrap();
    let binding = action.protected_binding().unwrap();
    let selector = PairSelector {
        peer: binding.peer,
        join: binding.peer_join,
        key_ref: binding.peer_key,
        epoch: binding.epoch,
    };
    if lose_input {
        action.settle_engine();
        assert!(action.input().is_err());
    }
    let id = valid.id();
    let wrong_id = wrong.id();
    let other_action = action.clone();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let other_barrier = barrier.clone();
    let (sent, received) = std::sync::mpsc::channel();
    let (valid_receive, wrong_receive, valid_send, wrong_send) = if wrong_first {
        (Some(received), None, None, Some(sent))
    } else {
        (None, Some(received), Some(sent), None)
    };
    let first = std::thread::spawn(move || {
        barrier.wait();
        if let Some(received) = valid_receive {
            received.recv().unwrap();
        }
        let result = valid.attach_child(action, 16 * 1024);
        if let Some(sent) = valid_send {
            sent.send(()).unwrap();
        }
        result
    });
    let second = std::thread::spawn(move || {
        other_barrier.wait();
        if let Some(received) = wrong_receive {
            received.recv().unwrap();
        }
        let result = wrong.attach_child(other_action, 16 * 1024);
        if let Some(sent) = wrong_send {
            sent.send(()).unwrap();
        }
        result
    });
    let valid_result = first.join().unwrap();
    if lose_input {
        assert!(matches!(valid_result, Err(VoiceError::Unauthorized)));
    } else {
        assert!(valid_result.unwrap() == id);
    }
    assert!(matches!(
        second.join().unwrap(),
        Err(VoiceError::RetryMismatch)
    ));
    let (ledger::Begin::Factory(retry), _) = context
        .ledger
        .begin_child(selected.clone(), parent, wrong_request)
        .unwrap()
    else {
        panic!("same original refusing purpose")
    };
    assert!(retry.id() == wrong_id && !retry.has_publication().unwrap());
    drop(retry);
    resolve(state, selected, parent, wrong_request, wrong_id)
        .close_pending()
        .unwrap();
    let child = resolve(state, selected, parent, valid_request, id);
    if lose_input {
        let (ledger::Begin::Factory(retry), _) = context
            .ledger
            .begin_child(selected.clone(), parent, valid_request)
            .unwrap()
        else {
            panic!("original known publication retry")
        };
        assert!(retry.id() == id && retry.has_publication().unwrap());
        assert!(matches!(
            retry.attach_published(16 * 1024),
            Err(VoiceError::Unauthorized)
        ));
        drop(_commit); // framed cleanup/bootstrap uses the same original UI boundary
        known_attachment_refusals(state, selected, parent, valid_request, &child).await;
        cleanup_child(
            state,
            selected,
            parent,
            valid_request,
            &child,
            selector,
            Cleanup::PendingReturn,
        )
        .await;
        let closed = child.status().unwrap();
        assert!(!closed.drained && closed.action.is_none());
        return;
    }
    child.settle().unwrap();
    drop(_commit); // bootstrap routing takes this same original UI commit
    let route = route(state, selected).await;
    child
        .close_phase(route.session().identity(), custody::CloseMode::Return)
        .unwrap();
    assert!(child.try_compact().unwrap());
}
async fn incoming(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    parent: &ledger::ResolvedRow,
    role: u8,
) -> ledger::ResolvedRow {
    let start = SystemClock.monotonic_ms();
    let request = ledger::ConfirmationPurpose::Incoming(role);
    let id = loop {
        // The HANDLER does one bounded observation. Only this test harness retries while
        // real network ingress is progressing; no fifteen-second loop enters the listener.
        let bytes = begin_frame(state, selected, parent, request).await;
        if bytes.len() == replies::STATUS_BYTES {
            break ledger::RowId::from_logical(bytes[2..18].try_into().unwrap()).unwrap();
        }
        assert_eq!(
            bytes,
            replies::refused(factory::Refused::Protocol(VoiceError::Capacity))
        );
        assert!(
            SystemClock.monotonic_ms().saturating_sub(start) < 15_000,
            "authenticated native confirmation did not arrive through the permitted TCP bridge"
        );
        SystemClock.sleep(Duration::from_millis(25)).await;
    };
    resolve(state, selected, parent, request, id)
}

async fn crypto_and_return(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    row: &ledger::ResolvedRow,
    parent: &ledger::ResolvedRow,
    purpose: ledger::ConfirmationPurpose,
    framed_result: bool,
    reject_opened: bool,
) -> CryptoOutcome {
    project_and_admit(state, selected, row, parent, purpose).await;
    let work = row.work().unwrap();
    let input = work.phase().input(work.action()).unwrap();
    let _adapter = work.phase().copy(4096).unwrap();
    let low = work.action().protected_binding().unwrap().local_is_low;
    let binding = work.action().protected_binding().unwrap();
    let selector = PairSelector {
        peer: binding.peer,
        join: binding.peer_join,
        key_ref: binding.peer_key,
        epoch: binding.epoch,
    };
    let opening = work.action().operation() == CryptoOperation::DecryptConfirmation;
    let key = if low != opening { [11; 32] } else { [12; 32] };
    let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
    let (buffer, length) = match input.input() {
        CryptoInput::EncryptConfirmation(flight) => {
            let mut buffer = work.phase().buffer(flight.plaintext.len() + 16).unwrap();
            let len = flight.plaintext.len();
            buffer.bytes_mut()[..len].copy_from_slice(&flight.plaintext);
            let mut iv = [0; 12];
            iv[4..].copy_from_slice(&flight.reservation.sequence.to_be_bytes());
            // The real framed child admission above already committed this exact engine.
            let tag = cipher
                .encrypt_in_place_detached(
                    Nonce::from_slice(&iv),
                    &flight.reservation.aad,
                    &mut buffer.bytes_mut()[..len],
                )
                .unwrap();
            buffer.bytes_mut()[len..].copy_from_slice(&tag);
            (buffer, len + 16)
        }
        CryptoInput::DecryptConfirmation(packet) => {
            let len = packet.ciphertext().len().checked_sub(16).unwrap();
            let mut buffer = work.phase().buffer(packet.ciphertext().len()).unwrap();
            buffer.bytes_mut()[..len].copy_from_slice(&packet.ciphertext()[..len]);
            let mut iv = [0; 12];
            iv[4..].copy_from_slice(&packet.header.sequence.to_be_bytes());
            cipher
                .decrypt_in_place_detached(
                    Nonce::from_slice(&iv),
                    packet.aad(),
                    &mut buffer.bytes_mut()[..len],
                    Tag::from_slice(&packet.ciphertext()[len..]),
                )
                .unwrap();
            (buffer, len)
        }
        CryptoInput::Derive(_) => panic!("confirmation child"),
    };
    let mut buffer = buffer;
    if reject_opened {
        assert!(opening && length != 0);
        // Deliberately corrupt a provider's otherwise authenticated opened result. Native
        // Root must record the actual protocol rejection, not accept a renderer success flag.
        buffer.bytes_mut()[0] ^= 1;
    }
    let tag = work.action().output_tag(&buffer.bytes()[..length]).unwrap();
    let (work, _adapter) = if framed_result {
        // The native test provider is finished before native result admission. Its own
        // temporary4KiB adapter cannot overlap the separate8KiB worker +4KiB actor body.
        drop((work, _adapter));
        framed_submission(
            state,
            selected,
            parent,
            purpose,
            row,
            selector,
            &buffer.bytes()[..length],
        )
        .await;
        let work = row.work().unwrap();
        let adapter = work.phase().copy(4096).unwrap();
        (work, adapter)
    } else {
        (work, _adapter)
    };
    // Keep the original test-provider buffer/typed body/completed output after an exact
    // Actor retry as well. Framed publication must not let cleanup forget these late owners.
    let body = work
        .phase()
        .result_body(buffer, 0..length)
        .unwrap_or_else(|(error, _)| panic!("paid result body: {error:?}"));
    let actor = actor(state, selected).await;
    let completion = if opening {
        actor
            .commit_voice_confirmation_result(
                selected.grant().unwrap(),
                work.action().clone(),
                &body,
                tag,
            )
            .await
            .unwrap()
    } else {
        actor
            .submit_voice_confirmation_result(
                selected.grant().unwrap(),
                work.action().clone(),
                &body,
                tag,
            )
            .await
            .unwrap()
    };
    let receipt = completion.result().unwrap();
    let outcome = receipt.outcome;
    assert_eq!(matches!(outcome, CryptoOutcome::Rejected(_)), reject_opened);
    let mut expected = row.status().unwrap();
    if !framed_result {
        assert!(
            expected.engine.receipt.is_none(),
            "native cache has not observed Actor publication"
        );
    } else {
        assert!(
            expected
                .engine
                .receipt
                .as_ref()
                .unwrap()
                .same_transition(&receipt),
            "framed first publication and typed retry keep the exact same result"
        );
    }
    expected.engine = work.action().terminal_status();
    assert!(expected
        .engine
        .receipt
        .as_ref()
        .unwrap()
        .same_transition(&receipt));
    recover_child_history(state, selected, parent, purpose, row, selector, &expected).await;
    assert!(row
        .status()
        .unwrap()
        .engine
        .receipt
        .as_ref()
        .unwrap()
        .same_transition(&receipt));
    cleanup_child(
        state,
        selected,
        parent,
        purpose,
        row,
        selector,
        Cleanup::Receipt(&receipt),
    )
    .await;
    cleanup_child(
        state,
        selected,
        parent,
        purpose,
        row,
        selector,
        Cleanup::Settle,
    )
    .await;
    drop((work, _adapter));
    cleanup_child(
        state,
        selected,
        parent,
        purpose,
        row,
        selector,
        Cleanup::Return,
    )
    .await;
    assert!(
        !row.try_compact().unwrap(),
        "actual Input/body/completed output still retain the loan"
    );
    drop(input);
    assert!(
        !row.try_compact().unwrap(),
        "completed but uncollected body/output are real owners"
    );
    drop((completion, body));
    assert!(row.try_compact().unwrap());
    assert!(row.status().unwrap().drained);
    recover_child_history(
        state,
        selected,
        parent,
        purpose,
        row,
        selector,
        &row.status().unwrap(),
    )
    .await;
    outcome
}

/// Pass actual output through the opened Data frame and native detached result worker.
/// The Root receipt is independently bound to the original action/body, not a renderer flag.
async fn framed_submission(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    parent: &ledger::ResolvedRow,
    purpose: ledger::ConfirmationPurpose,
    row: &ledger::ResolvedRow,
    selector: PairSelector,
    output: &[u8],
) {
    let bootstrap::Status::Ready(public) = bootstrap::ensure(
        state,
        selected.id(),
        selected.executor(),
        &mut catcoms_rt::OsCryptoRng,
    )
    .await
    .unwrap() else {
        panic!("original channel")
    };
    let route = state.voice_channels.route(public.public().slot).unwrap();
    let mut body = confirmation_prefix(parent.id(), purpose, selector);
    let (serial, version, tag) = row.status().unwrap().action.unwrap();
    body.extend_from_slice(&serial.to_be_bytes());
    body.extend_from_slice(&version.to_be_bytes());
    body.extend_from_slice(&tag);
    let output_tag: [u8; 32] = Sha256::digest(output).into();
    body.extend_from_slice(&output_tag);
    body.extend_from_slice(&(output.len() as u16).to_be_bytes());
    body.extend_from_slice(output);
    let (phase, role, operation, original_deadline) = {
        let work = row.work().unwrap();
        (
            work.phase().clone(),
            work.action().protected_row().unwrap(),
            work.action().operation(),
            work.action().protected_binding().unwrap().deadline,
        )
    };
    let clock = crate::voice_runtime(state).clock();
    let context = context::Context::new(state);
    if role == 1 && operation == CryptoOperation::EncryptConfirmation {
        use std::future::{poll_fn, Future};
        use std::task::Poll;
        // Resolve Actor before deliberately holding its map; otherwise the fixture itself
        // would block before exercising actual preflight's nonblocking refusal.
        let actor = actor(state, selected).await;
        let mut force_capacity = true;
        let mut refusals = 0u32;
        loop {
            assert!(
                clock.monotonic_ms() < original_deadline,
                "original Hello result deadline exhausted after {refusals} capacity refusals"
            );
            let header = (
                route.session().fixture_counts().1 + 1,
                3,
                1,
                row.id().logical(),
            );
            let incoming = opened_frame(&public, &route, header, &body, Some(&phase));
            // Hold UI admission while distinguishing actual preparation completion from
            // arrival at that wait. A completed Capacity response must be polled/collected.
            // Separate Arc counts can form a torn snapshot during Context upgrade/drop;
            // only this exact request's actual post-preflight marker proves arrival.
            let boundary = state.ui_session_commit.lock().await;
            let pressure = if force_capacity {
                Some(state.servers.lock().await)
            } else {
                None
            };
            let store_owners = Arc::strong_count(&state.store);
            let mut pending = Box::pin(confirmation_result::commit(&context, &route, incoming));
            let early = tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    if let Poll::Ready(response) =
                        poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx))).await
                    {
                        break Some(response);
                    }
                    if route.session().fixture_result_preflight_sequence() == header.0 {
                        break None;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            if let Some(response) = early {
                let reply = response
                    .expect("actual original result worker must fit its unchanged payment")
                    .seal()
                    .unwrap();
                let plain = opened_reply(&public, header, &body, &reply);
                drop(pending);
                drop(reply);
                assert!(!route.session().fixture_counts().0);
                assert_eq!(
                    plain,
                    replies::refused(factory::Refused::Protocol(VoiceError::Capacity)),
                    "only authenticated Capacity may precede UI admission"
                );
                let status = row.reconcile().unwrap();
                assert_eq!(status.action, Some((serial, version, tag)));
                assert!(status.engine.receipt.is_none());
                assert_ne!(
                    route.session().fixture_result_preflight_sequence(),
                    header.0,
                    "preflight refusal must never mark this request prepared"
                );
                drop(pressure);
                drop(boundary);
                force_capacity = false;
                refusals += 1;
                eprintln!(
                    "native Hello exact preflight retry {refusals}; remaining original ms {}",
                    original_deadline.saturating_sub(clock.monotonic_ms())
                );
                clock.sleep(Duration::from_millis(100)).await;
                continue;
            }
            assert!(
                !force_capacity && refusals > 0,
                "held map must refuse before UI admission"
            );
            assert_eq!(
                Arc::strong_count(&state.store),
                store_owners,
                "actual prepared worker must not retain broad store ownership through its UI wait"
            );
            drop(pressure);
            // A FIFO actor query completes preparation, then the existing durable Ready
            // barrier parks the ACTUAL publication. This successful retry must still prove
            // waiter loss, commit ownership and original receipt before any later replay.
            let parked = factory::tests::park(&actor).await;
            drop(boundary);
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    assert!(
                        poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx).is_pending())).await
                    );
                    if state.ui_session_commit.try_lock().is_err() {
                        assert!(matches!(row.work(), Err(VoiceError::Capacity)));
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            drop(pending); // real waiter loss detaches, never cancels actual publication
            assert!(state.ui_session_commit.try_lock().is_err());
            let mut ordering = Box::pin(state.ui_session_commit.lock());
            assert!(poll_fn(|cx| Poll::Ready(ordering.as_mut().poll(cx).is_pending())).await);
            drop(parked);
            let guard = tokio::time::timeout(Duration::from_secs(15), ordering)
                .await
                .unwrap();
            drop(guard);
            // Commit and physical field destructors are not atomic; collect both witnesses.
            tokio::time::timeout(
                Duration::from_secs(15),
                route.session().fixture_wait_physical_idle(),
            )
            .await
            .unwrap();
            assert!(!route.session().fixture_counts().0);
            let status = row.reconcile().unwrap();
            assert_eq!(
                status
                    .engine
                    .receipt
                    .as_ref()
                    .expect("detached original publication must finish before any replay")
                    .output_tag,
                output_tag
            );
            recover_child_history(state, selected, parent, purpose, row, selector, &status).await;
            return;
        }
    }
    // Force one real, authenticated local refusal on Finish. The actual native worker
    // observes the held registry; it must not manufacture a terminal result. Releasing
    // that fence must allow this SAME genuine child result to reach its original Root
    // receipt, before crypto_and_return's later typed retry can hide a missing handoff.
    let mut inject_capacity = role == 3 && operation == CryptoOperation::EncryptConfirmation;
    let mut capacity_refusals = 0u32;
    loop {
        assert!(
            clock.monotonic_ms() < original_deadline,
            "original confirmation deadline exhausted: role {role}, operation {operation:?}, action {serial}, capacity refusals {capacity_refusals}"
        );
        let pressure = if inject_capacity {
            Some(state.servers.lock().await)
        } else {
            None
        };
        let header = (
            route.session().fixture_counts().1 + 1,
            3,
            1,
            row.id().logical(),
        );
        let response = confirmation_result::commit(
            &context,
            &route,
            opened_frame(&public, &route, header, &body, Some(&phase)),
        )
        .await
        .expect("actual original result worker must fit its unchanged payment");
        let reply = response.seal().unwrap();
        // Authenticate and classify BEFORE assuming that publication produced a receipt.
        // A Capacity reply is neither terminal failure nor evidence that no work exists.
        let plain = opened_reply(&public, header, &body, &reply);
        drop(reply); // collect Work/task/output/physical ownership before retransmission
        assert!(!route.session().fixture_counts().0);
        let status = row.reconcile().unwrap();
        assert!(status.action == Some((serial, version, tag)));
        let capacity = replies::refused(factory::Refused::Protocol(VoiceError::Capacity));
        if inject_capacity {
            assert_eq!(plain, capacity, "held native registry must refuse locally");
            assert!(status.engine.receipt.is_none());
            inject_capacity = false;
        }
        drop(pressure);
        if plain == capacity {
            capacity_refusals += 1;
            eprintln!("native confirmation exact result retry {capacity_refusals}; role {role}; operation {operation:?}; action {serial}; remaining original ms {}", original_deadline.saturating_sub(clock.monotonic_ms()));
            // Keep the immutable body/output/action/permit/Phase and original deadline.
            // Only the collected physical request's sequence changes. No refunds or new
            // factories are permitted, even if an earlier reply's publication was lost.
            clock.sleep(Duration::from_secs(1)).await;
            continue;
        }
        assert!(
            same_result_observation(&plain, &replies::status(row.id(), &status).unwrap()),
            "unexpected authenticated confirmation reply {plain:?}: role {role}, operation {operation:?}, action {serial}, capacity refusals {capacity_refusals}"
        );
        assert_eq!(
            status
                .engine
                .receipt
                .as_ref()
                .expect("canonical native result must contain its actual terminal receipt")
                .output_tag,
            output_tag
        );
        break;
    }
}

/// Compare the authenticated reply with a later canonical native observation. Root's
/// receipt.retired is observational (same_transition excludes it): crossing expiry can
/// change only that bit from0 to1 between sealing and reconciliation. No other field,
/// receipt disappearance, backward transition or noncanonical boolean is acceptable.
fn same_result_observation(reply: &[u8], later: &[u8; replies::STATUS_BYTES]) -> bool {
    reply == later
        || (reply.len() == replies::STATUS_BYTES
            && reply[69] == 1
            && later[69] == 1
            && reply[207] == 0
            && later[207] == 1
            && reply[..207] == later[..207]
            && reply[208..] == later[208..])
}

#[test]
fn result_observation_allows_only_forward_retirement_of_the_same_canonical_receipt() {
    use catcoms_sync::voice_v2::crypto_actions::{CryptoResultReceipt, CryptoResultStatus};
    let mut logical = [0; 16];
    logical[15] = 1;
    let id = ledger::RowId::from_logical(logical).unwrap();
    // These are metadata-only codec inputs, not fabricated action/engine authority. Cover
    // rejected as well as accepted outcomes: retirement never changes the original result.
    for outcome in [
        CryptoOutcome::Opened,
        CryptoOutcome::Rejected(VoiceError::Freshness),
    ] {
        let mut status = ledger::Status {
            pending: ledger::PendingStatus::NotPending,
            action: Some((7, 3, [9; 32])),
            engine: CryptoResultStatus {
                settled: false,
                receipt: Some(CryptoResultReceipt {
                    action: 7,
                    input_tag: [9; 32],
                    output_tag: [8; 32],
                    outcome,
                    retired: false,
                }),
            },
            acknowledged: false,
            drained: false,
        };
        let original = replies::status(id, &status).unwrap();
        status.engine.receipt.as_mut().unwrap().retired = true;
        let retired = replies::status(id, &status).unwrap();
        assert!(same_result_observation(&original, &original));
        assert!(same_result_observation(&original, &retired));
        assert!(!same_result_observation(&retired, &original));
        // A relaxed observational comparison must never hide even one changed binding,
        // digest, outcome, ACK, settlement, drainage, reserved byte or invalid boolean.
        for index in 0..replies::STATUS_BYTES {
            if index == 207 {
                continue;
            }
            let mut changed = retired;
            changed[index] ^= 1;
            assert!(
                !same_result_observation(&original, &changed),
                "byte {index}"
            );
        }
        let mut invalid = retired;
        invalid[207] = 2;
        assert!(!same_result_observation(&original, &invalid));
        assert!(!same_result_observation(&original[..241], &retired));
        assert!(!same_result_observation(&[1, 3, 7], &retired));
    }
}

/// Test-only canonical request prefix; renderer/native shared vectors test this grammar.
fn confirmation_prefix(
    parent: ledger::RowId,
    purpose: ledger::ConfirmationPurpose,
    selector: PairSelector,
) -> Vec<u8> {
    let mut body = vec![2];
    body.extend_from_slice(selector.peer.as_bytes());
    body.extend_from_slice(&selector.join);
    body.extend_from_slice(&selector.key_ref);
    body.extend_from_slice(&selector.epoch.to_be_bytes());
    body.extend_from_slice(&parent.logical());
    match purpose {
        ledger::ConfirmationPurpose::Start(None) => body.push(0),
        ledger::ConfirmationPurpose::Start(Some(previous)) => {
            body.push(1);
            body.extend_from_slice(&previous);
        }
        ledger::ConfirmationPurpose::Respond(trigger) => {
            body.push(2);
            body.extend_from_slice(&trigger.logical());
        }
        ledger::ConfirmationPurpose::Incoming(role) => body.extend_from_slice(&[3, role]),
    }
    body
}

enum Cleanup<'a> {
    Receipt(&'a catcoms_sync::voice_v2::crypto_actions::CryptoResultReceipt),
    Settle,
    Return,
    PendingReturn,
}
/// Exact authenticated action cleanup, including Return while actual Input/body/completion
/// owners still survive. Original receipt digests are verified by native, not trusted here.
async fn cleanup_child(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    parent: &ledger::ResolvedRow,
    purpose: ledger::ConfirmationPurpose,
    row: &ledger::ResolvedRow,
    selector: PairSelector,
    cleanup: Cleanup<'_>,
) {
    let bootstrap::Status::Ready(public) = bootstrap::ensure(
        state,
        selected.id(),
        selected.executor(),
        &mut catcoms_rt::OsCryptoRng,
    )
    .await
    .unwrap() else {
        panic!("original channel")
    };
    let route = state.voice_channels.route(public.public().slot).unwrap();
    let mut body = confirmation_prefix(parent.id(), purpose, selector);
    let operation = match cleanup {
        Cleanup::Receipt(_) => 4,
        Cleanup::Settle => 5,
        Cleanup::Return | Cleanup::PendingReturn => 6,
    };
    if operation == 6 {
        body.push(0);
    } else {
        let (serial, version, tag) = row.status().unwrap().action.unwrap();
        body.extend_from_slice(&serial.to_be_bytes());
        body.extend_from_slice(&version.to_be_bytes());
        body.extend_from_slice(&tag);
        if let Cleanup::Receipt(receipt) = cleanup {
            body.extend_from_slice(&requests::receipt_digest(receipt));
        }
    }
    let header = (
        route.session().fixture_counts().1 + 1,
        operation,
        0,
        row.id().logical(),
    );
    let reply = confirmation_settlement::apply(
        &context::Context::new(state),
        &route,
        opened_frame(&public, &route, header, &body, None),
    )
    .seal()
    .unwrap();
    let bytes = opened_reply(&public, header, &body, &reply);
    assert_eq!(bytes.len(), replies::STATUS_BYTES);
    if matches!(cleanup, Cleanup::PendingReturn) {
        let status = row.status().unwrap();
        assert!(matches!(
            status.pending,
            ledger::PendingStatus::Closed {
                actor_handoff: true
            }
        ));
        assert!(
            status.action.is_none()
                && !status.engine.settled
                && !status.drained
                && !status.acknowledged
        );
        assert_eq!(
            bytes,
            replies::status(row.id(), &status).unwrap(),
            "actual ClosedPending; no invented ACK/settlement/drainage"
        );
        return;
    }
    assert_eq!(bytes[208], 1, "actual original receipt acknowledged");
    if operation != 4 {
        assert_eq!(bytes[68], 1, "actual engine settled");
    }
}

/// Actual authenticated child Input and Clock-first native engine admission. Test provider
/// AES below remains separate from browser/WebView acceptance; compare every lent AAD/body
/// byte to the actual Root input before that provider consumes it.
async fn project_and_admit(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    row: &ledger::ResolvedRow,
    parent: &ledger::ResolvedRow,
    purpose: ledger::ConfirmationPurpose,
) {
    let selector = {
        let work = row.work().unwrap();
        let binding = work.action().protected_binding().unwrap();
        PairSelector {
            peer: binding.peer,
            join: binding.peer_join,
            key_ref: binding.peer_key,
            epoch: binding.epoch,
        }
    };
    let bootstrap::Status::Ready(public) = bootstrap::ensure(
        state,
        selected.id(),
        selected.executor(),
        &mut catcoms_rt::OsCryptoRng,
    )
    .await
    .unwrap() else {
        panic!("original channel")
    };
    let route = state.voice_channels.route(public.public().slot).unwrap();
    let prefix = confirmation_prefix(parent.id(), purpose, selector);
    let context = context::Context::new(state);
    let clock = crate::voice_runtime(state).clock();
    let (header, reply) = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let header = (
                route.session().fixture_counts().1 + 1,
                1,
                0,
                row.id().logical(),
            );
            let reply = confirmation_input::pull(
                &context,
                &route,
                opened_frame(&public, &route, header, &prefix, None),
            )
            .await
            .unwrap();
            let admission = hex::decode(reply.admission()).unwrap();
            if admission[36] == 1 {
                break (header, reply);
            }
            // The live mesh's nonblocking selection fence can return Capacity before
            // Input acquires any Work. Authenticate that exact refusal before retrying
            // this SAME input. A Control reply is not an uncollected Data owner; other
            // refusals remain failures, and the original Root deadline is never renewed.
            assert_eq!(
                opened_reply(&public, header, &prefix, &reply),
                replies::refused(factory::Refused::Protocol(VoiceError::Capacity))
            );
            drop(reply);
            clock.sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("original confirmation Input stayed busy");
    assert!(
        row.work().is_err(),
        "uncollected Data reply owns its actual Work/input"
    );
    let data = opened_reply_kind(&public, header, &prefix, &reply, 1);
    assert_eq!(&data[..2], &[1, 7]);
    assert_eq!(&data[2..18], &row.id().logical());
    assert_eq!(&data[66..82], &parent.id().logical());
    drop(reply);
    let work = row.work().unwrap();
    let input = work.phase().input(work.action()).unwrap();
    let role = work.action().protected_row().unwrap();
    assert_eq!(data[417], role);
    let (operation, sequence, aad, body) = match input.input() {
        CryptoInput::EncryptConfirmation(flight) => {
            if purpose == ledger::ConfirmationPurpose::Start(None) {
                assert_ne!(
                    flight.action,
                    work.action().serial(),
                    "pair-flight and Action serial spaces differ after Derive"
                );
            }
            (
                1,
                flight.reservation.sequence,
                flight.reservation.aad.as_slice(),
                flight.plaintext.as_slice(),
            )
        }
        CryptoInput::DecryptConfirmation(packet) => {
            (2, packet.header.sequence, packet.aad(), packet.ciphertext())
        }
        CryptoInput::Derive(_) => panic!("actual child input"),
    };
    assert_eq!(data[416], operation);
    assert_eq!(&data[418..426], &sequence.to_be_bytes());
    assert_eq!(
        u16::from_be_bytes(data[426..428].try_into().unwrap()) as usize,
        aad.len()
    );
    assert_eq!(
        u16::from_be_bytes(data[428..430].try_into().unwrap()) as usize,
        body.len()
    );
    assert_eq!(&data[430..430 + aad.len()], aad);
    assert_eq!(&data[430 + aad.len()..], body);
    let serial = work.action().serial();
    let engine_bytes = work.action().engine_bytes();
    let original_deadline = work.action().protected_binding().unwrap().deadline;
    let mut admission = prefix;
    admission.extend_from_slice(&serial.to_be_bytes());
    admission.extend_from_slice(&work.action().version().to_be_bytes());
    admission.extend_from_slice(&work.action().input_tag());
    admission.extend_from_slice(&(engine_bytes as u32).to_be_bytes());
    drop((input, work, data));
    // Exercise the actual new synchronous admission constructor with a genuine live
    // child. Read-only identification may run, but dropping its UNPOLLED future must
    // release the original physical request and leave the exact child state untouched.
    // Use row-local evidence: unrelated TCP discovery/Source owners can change global
    // process byte counts while this real-transport scenario runs.
    let status_before = row.reconcile().unwrap();
    let header = (
        route.session().fixture_counts().1 + 1,
        2,
        0,
        row.id().logical(),
    );
    let unpolled = confirmation_admission::admit(
        &context,
        &route,
        opened_frame(&public, &route, header, &admission, None),
    );
    assert!(route.session().fixture_counts().0);
    assert!(row.reconcile().unwrap() == status_before);
    drop(unpolled);
    assert!(!route.session().fixture_counts().0);
    assert!(row.reconcile().unwrap() == status_before);
    let clock = crate::voice_runtime(state).clock();
    let bytes = loop {
        let header = (
            route.session().fixture_counts().1 + 1,
            2,
            0,
            row.id().logical(),
        );
        let reply = confirmation_admission::admit(
            &context,
            &route,
            opened_frame(&public, &route, header, &admission, None),
        )
        .await
        .unwrap();
        let bytes = opened_reply(&public, header, &admission, &reply);
        drop(reply);
        if bytes != replies::refused(factory::Refused::Protocol(VoiceError::Capacity)) {
            break bytes;
        }
        // Final engine admission deliberately refuses a busy native fence. Retry only
        // that authenticated Capacity, with this exact body/action and its first deadline;
        // no new row, engine permit, transcript or execution allowance is manufactured.
        assert!(
            clock.monotonic_ms() < original_deadline,
            "original engine admission stayed busy"
        );
        clock.sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(bytes.len(), replies::PERMIT_BYTES);
    assert_eq!(
        &bytes[66..74],
        &serial.to_be_bytes(),
        "actual engine commit serial"
    );
    assert_eq!(&bytes[74..78], &(engine_bytes as u32).to_be_bytes());
}

/// Recover the actual accepted result before native refresh and again after local
/// Return/compaction, not a synthetic receipt projection. This read borrows no new Phase.
async fn recover_child_history(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    parent: &ledger::ResolvedRow,
    purpose: ledger::ConfirmationPurpose,
    row: &ledger::ResolvedRow,
    selector: PairSelector,
    expected: &ledger::Status,
) {
    let bootstrap::Status::Ready(public) = bootstrap::ensure(
        state,
        selected.id(),
        selected.executor(),
        &mut catcoms_rt::OsCryptoRng,
    )
    .await
    .unwrap() else {
        panic!("original ready channel")
    };
    let route = state.voice_channels.route(public.public().slot).unwrap();
    let mut body = confirmation_prefix(parent.id(), purpose, selector);
    body.push(0); // Lookup only. Never reserve/rearm when recovering a lost reply.
    let expected = replies::status(row.id(), expected).unwrap();
    // Actual Source/Lower/intake work keeps progressing on other native tasks while this
    // query runs. Global pool parity is not a valid live-network assertion. The isolated
    // full-pool history tests pin zero new credit/work; here pin the actual original result.
    for _ in 0..2 {
        let sequence = route.session().fixture_counts().1 + 1;
        let header = (sequence, 8, 0, [0; 16]);
        let incoming = opened_frame(&public, &route, header, &body, None);
        let reply = confirmation_history::recover(&context::Context::new(state), &route, incoming)
            .seal()
            .unwrap();
        assert_eq!(opened_reply(&public, header, &body, &reply), expected);
    }
}

/// Pin FIRST Respond after actual Opened -> Return -> compaction, for BOTH response roles.
/// Actual Root signature/admission/source/helper/intake and original native custody participate;
/// none of these checks substitutes for the still-missing browser/RTC chain.
pub(crate) async fn confirmation_round_trip(
    low_state: &AppState,
    low: &Arc<SelectedHandoffOwner>,
    low_parent: &ledger::ResolvedRow,
    high_state: &AppState,
    high: &Arc<SelectedHandoffOwner>,
    high_parent: &ledger::ResolvedRow,
) {
    confirmation_round_trip_inner(
        low_state,
        low,
        low_parent,
        high_state,
        high,
        high_parent,
        ConfirmationMode::Legacy,
    )
    .await;
}
/// Full native framed Input/admit/result/ACK/settle/Return, with explicit test-only AES keys.
/// This is not browser key derivation, production effect dispatch or RTC acceptance.
pub(crate) async fn confirmation_framed_round_trip(
    low_state: &AppState,
    low: &Arc<SelectedHandoffOwner>,
    low_parent: &ledger::ResolvedRow,
    high_state: &AppState,
    high: &Arc<SelectedHandoffOwner>,
    high_parent: &ledger::ResolvedRow,
    orphan_low: bool,
) {
    confirmation_round_trip_inner(
        low_state,
        low,
        low_parent,
        high_state,
        high,
        high_parent,
        if orphan_low {
            ConfirmationMode::Orphan
        } else {
            ConfirmationMode::Framed
        },
    )
    .await;
}

/// Actual native rejection is durable result history too. The browser/provider is untrusted:
/// a malformed opened body of the expected length must not establish the pair. The same
/// framed result/ACK/settlement/Return/history adapters handle its exact rejected receipt.
pub(crate) async fn confirmation_rejected_result(
    low_state: &AppState,
    low: &Arc<SelectedHandoffOwner>,
    low_parent: &ledger::ResolvedRow,
    high_state: &AppState,
    high: &Arc<SelectedHandoffOwner>,
    high_parent: &ledger::ResolvedRow,
) {
    let hello = outgoing(
        low_state,
        low,
        low_parent,
        ledger::ConfirmationPurpose::Start(None),
    )
    .await;
    assert!(matches!(
        crypto_and_return(
            low_state,
            low,
            &hello,
            low_parent,
            ledger::ConfirmationPurpose::Start(None),
            true,
            false,
        )
        .await,
        CryptoOutcome::Sealed { .. }
    ));
    let opened = incoming(high_state, high, high_parent, 1).await;
    assert!(matches!(
        crypto_and_return(
            high_state,
            high,
            &opened,
            high_parent,
            ledger::ConfirmationPurpose::Incoming(1),
            true,
            true,
        )
        .await,
        CryptoOutcome::Rejected(_)
    ));
}

/// Native known-Incoming retry must not depend on the rotating pending snapshot after a
/// lost attachment/permanent input loss. This deliberately stops before confirmation, and
/// verifies exact local Pending closure rather than inventing a successful decrypt receipt.
pub(crate) async fn confirmation_known_incoming_retry(
    low_state: &AppState,
    low: &Arc<SelectedHandoffOwner>,
    low_parent: &ledger::ResolvedRow,
    high_state: &AppState,
    high: &Arc<SelectedHandoffOwner>,
    high_parent: &ledger::ResolvedRow,
) {
    let hello = outgoing(
        low_state,
        low,
        low_parent,
        ledger::ConfirmationPurpose::Start(None),
    )
    .await;
    assert!(matches!(
        crypto_and_return(
            low_state,
            low,
            &hello,
            low_parent,
            ledger::ConfirmationPurpose::Start(None),
            true,
            false
        )
        .await,
        CryptoOutcome::Sealed { .. }
    ));
    let mut permit = catcoms_sync::voice_v2::crypto_actions::CryptoActionSnapshotPermit::new(
        &high.resources(),
        high.binding().scope,
    )
    .unwrap();
    let actor = actor(high_state, high).await;
    let start = SystemClock.monotonic_ms();
    let action = loop {
        // Test harness only: this needs the ACTUAL original Action before simulating lost
        // input. The production Begin handler performs only one snapshot per invocation.
        let snapshot = actor
            .voice_confirmation_actions(high.grant().unwrap(), permit)
            .await
            .unwrap();
        let action = snapshot
            .result()
            .unwrap()
            .iter()
            .find(|action| {
                action.operation() == CryptoOperation::DecryptConfirmation
                    && action.protected_row().unwrap() == 1
                    && high_parent.contains_plan_action(action).unwrap()
            })
            .cloned();
        permit = snapshot.return_permit();
        if let Some(action) = action {
            break action;
        }
        assert!(SystemClock.monotonic_ms().saturating_sub(start) < 15_000);
        SystemClock.sleep(Duration::from_millis(25)).await;
    };
    drop(permit);
    action.settle_engine(); // permanent loss before any native Phase/Input exposure
    let route = route(high_state, high).await;
    let clock = crate::voice_runtime(high_state).clock();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            // Hold the SAME actual settled Root Action across every proven local refusal.
            // No rotating snapshot, new token, replacement factory action or attempt refund.
            match factory::ensure_incoming_row(
                &context::Context::new(high_state),
                &route,
                high_parent,
                action.clone(),
                factory::tests::physical(&route),
            )
            .await
            {
                Err(factory::Refused::Protocol(VoiceError::Unauthorized)) => break,
                Err(factory::Refused::Protocol(VoiceError::Capacity)) => {
                    assert!(
                        action.input().is_err(),
                        "settled original input must stay lost"
                    );
                    clock.sleep(Duration::from_millis(25)).await;
                }
                other => panic!("expected original settled-input attachment refusal: {other:?}"),
            }
        }
    })
    .await
    .expect("original incoming attachment did not recover from local preflight contention");
    let purpose = ledger::ConfirmationPurpose::Incoming(1);
    let child = high_state
        .voice_handoff
        .lookup_child(high, high_parent, purpose, None)
        .unwrap();
    assert!(child.has_pending_publication().unwrap());
    known_attachment_refusals(high_state, high, high_parent, purpose, &child).await;
    cleanup_child(
        high_state,
        high,
        high_parent,
        purpose,
        &child,
        high_parent.confirmation_selector().unwrap(),
        Cleanup::PendingReturn,
    )
    .await;
}

enum ConfirmationMode {
    Legacy,
    Framed,
    Orphan,
}
async fn confirmation_round_trip_inner(
    low_state: &AppState,
    low: &Arc<SelectedHandoffOwner>,
    low_parent: &ledger::ResolvedRow,
    high_state: &AppState,
    high: &Arc<SelectedHandoffOwner>,
    high_parent: &ledger::ResolvedRow,
    mode: ConfirmationMode,
) {
    let framed_results = !matches!(mode, ConfirmationMode::Legacy);
    let hello = outgoing(
        low_state,
        low,
        low_parent,
        ledger::ConfirmationPurpose::Start(None),
    )
    .await;
    assert!(matches!(
        crypto_and_return(
            low_state,
            low,
            &hello,
            low_parent,
            ledger::ConfirmationPurpose::Start(None),
            framed_results,
            false
        )
        .await,
        CryptoOutcome::Sealed { .. }
    ));
    let opened_hello = incoming(high_state, high, high_parent, 1).await;
    assert_eq!(
        crypto_and_return(
            high_state,
            high,
            &opened_hello,
            high_parent,
            ledger::ConfirmationPurpose::Incoming(1),
            framed_results,
            false
        )
        .await,
        CryptoOutcome::Opened
    );
    assert!(
        opened_hello
            .status()
            .unwrap()
            .engine
            .receipt
            .unwrap()
            .retired,
        "native local history compacted before FIRST response"
    );
    let proof = outgoing(
        high_state,
        high,
        high_parent,
        ledger::ConfirmationPurpose::Respond(opened_hello.id()),
    )
    .await;
    assert!(matches!(
        crypto_and_return(
            high_state,
            high,
            &proof,
            high_parent,
            ledger::ConfirmationPurpose::Respond(opened_hello.id()),
            framed_results,
            false
        )
        .await,
        CryptoOutcome::Sealed { .. }
    ));
    let opened_proof = incoming(low_state, low, low_parent, 2).await;
    assert_eq!(
        crypto_and_return(
            low_state,
            low,
            &opened_proof,
            low_parent,
            ledger::ConfirmationPurpose::Incoming(2),
            framed_results,
            false
        )
        .await,
        CryptoOutcome::Opened
    );
    let finish = outgoing(
        low_state,
        low,
        low_parent,
        ledger::ConfirmationPurpose::Respond(opened_proof.id()),
    )
    .await;
    assert!(matches!(
        crypto_and_return(
            low_state,
            low,
            &finish,
            low_parent,
            ledger::ConfirmationPurpose::Respond(opened_proof.id()),
            framed_results,
            false
        )
        .await,
        CryptoOutcome::Sealed { .. }
    ));
    let opened_finish = incoming(high_state, high, high_parent, 3).await;
    assert_eq!(
        crypto_and_return(
            high_state,
            high,
            &opened_finish,
            high_parent,
            ledger::ConfirmationPurpose::Incoming(3),
            framed_results,
            false
        )
        .await,
        CryptoOutcome::Opened
    );
    let route = route(high_state, high).await;
    let impossible = ledger::Purpose::Confirmation {
        parent: high_parent.id(),
        request: ledger::ConfirmationPurpose::Respond(opened_finish.id()),
    };
    assert!(
        matches!(
            factory::ensure_confirmation_row(
                &context::Context::new(high_state),
                &route,
                high_parent,
                ledger::ConfirmationPurpose::Respond(opened_finish.id()),
                factory::tests::physical(&route),
            )
            .await,
            Err(factory::Refused::Protocol(VoiceError::Unauthorized))
        ),
        "terminal Finish has no response"
    );
    assert!(
        high_state
            .voice_handoff
            .lookup_original(high, impossible)
            .is_none(),
        "terminal Finish must refuse before reserving uncertain native history"
    );
    if framed_results {
        // Only actual application-framed Finish results arm native custody. The legacy
        // test-only result path deliberately cannot manufacture this transition proof.
        super::transition::custody_cycle(
            high_state,
            high,
            high_parent,
            if matches!(mode, ConfirmationMode::Orphan) {
                super::transition::Cleanup::SdkAdmissionRace
            } else {
                super::transition::Cleanup::Live
            },
        )
        .await;
        super::transition::custody_cycle(
            low_state,
            low,
            low_parent,
            if matches!(mode, ConfirmationMode::Orphan) {
                super::transition::Cleanup::Orphan
            } else {
                super::transition::Cleanup::Ended
            },
        )
        .await;
    }
}
