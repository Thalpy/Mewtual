//! Actual native three-flight confirmation over the fixture's real permitted TCP graph.
//! Test-only AES-GCM keys stand in for already-derived browser keys; this is NOT WebCrypto,
//! native IPC child input/admission, P256/HKDF, SDP/ICE/RTC or audible-media acceptance.
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
        let closed = child.close_pending().unwrap();
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
) -> CryptoOutcome {
    let work = row.work().unwrap();
    let input = work.phase().input(work.action()).unwrap();
    let _adapter = work.phase().copy(4096).unwrap();
    let low = work.action().protected_binding().unwrap().local_is_low;
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
            // Admit the actual Root action before the test provider runs. The production
            // child frame/UI commit adapter remains a separate unimplemented boundary.
            work.action()
                .admit_engine(work.action().engine_bytes())
                .unwrap();
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
            work.action()
                .admit_engine(work.action().engine_bytes())
                .unwrap();
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
    let tag = work.action().output_tag(&buffer.bytes()[..length]).unwrap();
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
    assert!(!matches!(outcome, CryptoOutcome::Rejected(_)));
    row.acknowledge(&receipt).unwrap();
    row.settle().unwrap();
    drop((work, _adapter));
    let route = route(state, selected).await;
    row.close_phase(route.session().identity(), custody::CloseMode::Return)
        .unwrap();
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
    outcome
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
    let hello = outgoing(
        low_state,
        low,
        low_parent,
        ledger::ConfirmationPurpose::Start(None),
    )
    .await;
    assert!(matches!(
        crypto_and_return(low_state, low, &hello).await,
        CryptoOutcome::Sealed { .. }
    ));
    let opened_hello = incoming(high_state, high, high_parent).await;
    assert_eq!(
        crypto_and_return(high_state, high, &opened_hello).await,
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
        crypto_and_return(high_state, high, &proof).await,
        CryptoOutcome::Sealed { .. }
    ));
    let opened_proof = incoming(low_state, low, low_parent).await;
    assert_eq!(
        crypto_and_return(low_state, low, &opened_proof).await,
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
        crypto_and_return(low_state, low, &finish).await,
        CryptoOutcome::Sealed { .. }
    ));
    let opened_finish = incoming(high_state, high, high_parent).await;
    assert_eq!(
        crypto_and_return(high_state, high, &opened_finish).await,
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
