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
    let route = route(state, selected).await;
    let id = factory::ensure_confirmation_row(
        &context::Context::new(state),
        &route,
        parent,
        request,
        factory::tests::physical(&route),
    )
    .await
    .unwrap();
    resolve(state, selected, parent, request, id)
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
) -> ledger::ResolvedRow {
    // Pay the actual Root snapshot capacity BEFORE its mailbox call. Original action Arcs
    // and any late output retain their own independent Root record/input payments.
    let mut permit = catcoms_sync::voice_v2::crypto_actions::CryptoActionSnapshotPermit::new(
        &selected.resources(),
        selected.binding().scope,
    )
    .unwrap();
    let actor = actor(state, selected).await;
    let grant = selected.grant().unwrap();
    let start = SystemClock.monotonic_ms();
    let action = loop {
        let snapshot = actor
            .voice_confirmation_actions(grant.clone(), permit)
            .await
            .unwrap();
        let found = snapshot
            .result()
            .unwrap()
            .iter()
            .find(|action| {
                action.operation() == CryptoOperation::DecryptConfirmation
                    && parent.contains_plan_action(action).unwrap()
            })
            .cloned();
        permit = snapshot.return_permit();
        if let Some(action) = found {
            break action;
        }
        assert!(
            SystemClock.monotonic_ms().saturating_sub(start) < 15_000,
            "authenticated native confirmation did not arrive through the permitted TCP bridge"
        );
        SystemClock.sleep(Duration::from_millis(25)).await;
    };
    let role = action.protected_row().unwrap();
    let route = route(state, selected).await;
    let id = factory::ensure_incoming_row(
        &context::Context::new(state),
        &route,
        parent,
        action,
        factory::tests::physical(&route),
    )
    .await
    .unwrap();
    resolve(
        state,
        selected,
        parent,
        ledger::ConfirmationPurpose::Incoming(role),
        id,
    )
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
    let (phase, lose_waiter) = {
        let work = row.work().unwrap();
        (
            work.phase().clone(),
            work.action().protected_row().unwrap() == 1
                && work.action().operation() == CryptoOperation::EncryptConfirmation,
        )
    };
    let header = (
        route.session().fixture_counts().1 + 1,
        3,
        1,
        row.id().logical(),
    );
    let incoming = opened_frame(&public, &route, header, &body, Some(&phase));
    let context = context::Context::new(state);
    if lose_waiter {
        use std::future::{poll_fn, Future};
        use std::task::Poll;
        let actor = actor(state, selected).await;
        // Drive the ACTUAL outer handler up to its UI-commit await. A FIFO actor query
        // completes the preceding binding observation, then the existing durable Ready
        // barrier parks the actual publication turn without sleeps or asserted booleans.
        let boundary = state.ui_session_commit.lock().await;
        let commit_owners = Arc::strong_count(&state.ui_session_commit);
        let store_owners = Arc::strong_count(&state.store);
        let mut pending = Box::pin(confirmation_result::commit(&context, &route, incoming));
        assert!(poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx).is_pending())).await);
        // Work is now reserved BEFORE the detached preparation starts. Its worker flag
        // alone cannot establish binding completion/UI admission. The actual original
        // commit handle survives its UI wait while LiveContext has already been destroyed.
        tokio::time::timeout(Duration::from_secs(15), async {
            while Arc::strong_count(&state.ui_session_commit) <= commit_owners
                || Arc::strong_count(&state.store) != store_owners
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let parked = factory::tests::park(&actor).await;
        drop(boundary);
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                assert!(poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx).is_pending())).await);
                // The test's boundary is gone. Only the actual detached worker can now
                // own this original UI commit through the parked Actor publication.
                if state.ui_session_commit.try_lock().is_err() {
                    assert!(matches!(row.work(), Err(VoiceError::Capacity)));
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(pending); // actual outer JoinHandle loss must detach, never cancel publication
        assert!(state.ui_session_commit.try_lock().is_err());
        let mut ordering = Box::pin(state.ui_session_commit.lock());
        assert!(poll_fn(|cx| Poll::Ready(ordering.as_mut().poll(cx).is_pending())).await);
        drop(parked);
        // The real worker retains the commit through Actor/body publication collection.
        // Its Response's field destructors are NOT atomic: another thread can acquire this
        // guard before Incoming/PhysicalHold finishes dropping. Await physical idle separately.
        let guard = tokio::time::timeout(Duration::from_secs(15), ordering)
            .await
            .unwrap();
        drop(guard);
        tokio::time::timeout(
            Duration::from_secs(15),
            route.session().fixture_wait_physical_idle(),
        )
        .await
        .unwrap();
        assert!(!route.session().fixture_counts().0);
        let status = row.reconcile().unwrap();
        assert_eq!(
            status.engine.receipt.as_ref().unwrap().output_tag,
            output_tag
        );
        recover_child_history(state, selected, parent, purpose, row, selector, &status).await;
        return;
    }
    let response = confirmation_result::commit(&context, &route, incoming)
        .await
        .unwrap();
    let reply = response.seal().unwrap();
    let status = row.reconcile().unwrap();
    assert!(status.action == Some((serial, version, tag)));
    assert_eq!(
        status.engine.receipt.as_ref().unwrap().output_tag,
        output_tag
    );
    assert_eq!(
        opened_reply(&public, header, &body, &reply),
        replies::status(row.id(), &status).unwrap()
    );
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
    let mut admission = prefix;
    admission.extend_from_slice(&serial.to_be_bytes());
    admission.extend_from_slice(&work.action().version().to_be_bytes());
    admission.extend_from_slice(&work.action().input_tag());
    admission.extend_from_slice(&(engine_bytes as u32).to_be_bytes());
    drop((input, work, data));
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
        false,
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
) {
    confirmation_round_trip_inner(
        low_state,
        low,
        low_parent,
        high_state,
        high,
        high_parent,
        true,
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
    let opened = incoming(high_state, high, high_parent).await;
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

async fn confirmation_round_trip_inner(
    low_state: &AppState,
    low: &Arc<SelectedHandoffOwner>,
    low_parent: &ledger::ResolvedRow,
    high_state: &AppState,
    high: &Arc<SelectedHandoffOwner>,
    high_parent: &ledger::ResolvedRow,
    framed_results: bool,
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
            framed_results,
            false
        )
        .await,
        CryptoOutcome::Sealed { .. }
    ));
    let opened_hello = incoming(high_state, high, high_parent).await;
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
    let opened_proof = incoming(low_state, low, low_parent).await;
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
    let opened_finish = incoming(high_state, high, high_parent).await;
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
}
