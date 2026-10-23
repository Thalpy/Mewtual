//! Test-only access to REAL native adapters for the TCP fixture. No synthetic action/grant,
//! watcher injection or bypassed authentication is used. This is not WebView acceptance.
use super::*;
use crate::{voice_authority::SelectedHandoffOwner, AppState};
use catcoms_sync::voice_v2::presence::PairSelector;
use service::tests::{opened_frame, opened_reply, opened_reply_kind};
use sha2::{Digest, Sha256};
use std::sync::Arc;
mod confirmation;
pub(crate) use confirmation::{
    confirmation_alias_claims, confirmation_framed_round_trip, confirmation_known_incoming_retry,
    confirmation_rejected_result, confirmation_round_trip,
};

pub(crate) async fn derivation(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    selector: PairSelector,
) {
    let _ = derivation_cycle(state, selected, selector, false).await;
}

/// Test-only native continuation. The same real Input/admit/Actor-result/ACK/settlement
/// path returns its original loan instead of Ending. This does NOT run browser WebCrypto
/// or prove browser key transfer; it supplies the actual paid parent to factory tests.
pub(crate) async fn derivation_returned(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    selector: PairSelector,
) -> ledger::ResolvedRow {
    derivation_cycle(state, selected, selector, true)
        .await
        .unwrap()
}

async fn derivation_cycle(
    state: &AppState,
    selected: &Arc<SelectedHandoffOwner>,
    selector: PairSelector,
    return_for_confirmation: bool,
) -> Option<ledger::ResolvedRow> {
    let bootstrap::Status::Ready(public) = bootstrap::ensure(
        state,
        selected.id(),
        selected.executor(),
        &mut catcoms_rt::OsCryptoRng,
    )
    .await
    .unwrap() else {
        panic!("original native channel ready");
    };
    let route = state.voice_channels.route(public.public().slot).unwrap();
    let context = context::Context::new(state);
    // Client's real P256/HKDF construction completes before its authenticated Status9 ACK.
    // Use the SHIPPING handler so native composition excludes no setup loan by assumption.
    let mut setup = vec![1];
    setup.extend_from_slice(&route.session_id());
    setup.extend_from_slice(&route.hash());
    let header = (1, 9, 0, route.session_id());
    let reply =
        service::status(&route, opened_frame(&public, &route, header, &setup, None)).unwrap();
    let status = opened_reply(&public, header, &setup, &reply);
    assert_eq!(status, vec![1, 1, 1]);
    drop(reply);
    let mut purpose = vec![1];
    purpose.extend_from_slice(selector.peer.as_bytes());
    purpose.extend_from_slice(&selector.join);
    purpose.extend_from_slice(&selector.key_ref);
    purpose.extend_from_slice(&selector.epoch.to_be_bytes());
    let id = if return_for_confirmation {
        // Native-origin work has no browser request to borrow. Exercise its OWN paid
        // producer/Session drain position on these genuine blocked-TCP pairs. The child
        // provider below is still a test provider, not browser P256/HKDF acceptance.
        let output = factory::native::produce_derivation(&context, &route, selector)
            .unwrap()
            .await
            .unwrap();
        let id = output.result().unwrap();
        drop(output);
        let row = state
            .voice_handoff
            .resolve(selected, ledger::Purpose::Derive(selector), id)
            .unwrap();
        let original = row.status().unwrap();
        assert!(original.action.is_some() && original.engine.receipt.is_none());
        let retry = factory::native::produce_derivation(&context, &route, selector)
            .unwrap()
            .await
            .unwrap();
        assert_eq!(retry.result().unwrap(), id);
        assert!(
            row.status().unwrap() == original,
            "native producer retry preserves the same uncommitted action"
        );
        drop(retry);
        id
    } else {
        let mut begin = purpose.clone();
        begin.push(1);
        let incoming = opened_frame(&public, &route, (2, 8, 0, [0; 16]), &begin, None);
        let physical = incoming.hold_position().unwrap();
        let id = factory::ensure_derivation_row(&context, &route, selector, physical)
            .await
            .unwrap();
        drop(incoming);
        id
    };
    let row = state
        .voice_handoff
        .resolve(selected, ledger::Purpose::Derive(selector), id)
        .unwrap();
    let (serial, version, tag) = row
        .status()
        .unwrap()
        .action
        .expect("actual registered Root action");
    // This is an ACTUAL extra Root input borrower, independent of the physical Input reply.
    // Final issuance must stay pending through End/compaction until this owner really drops.
    let late_input = row.work().unwrap().action().input().unwrap();
    let header = (3, 1, 0, id.logical());
    let incoming = opened_frame(&public, &route, header, &purpose, None);
    let reply = input::pull(&context, &route, incoming).await.unwrap();
    let mut plain = opened_reply_kind(&public, header, &purpose, &reply, 1);
    assert_eq!(&plain[..2], &[1, 5]);
    assert_eq!(&plain[2..18], &id.logical());
    assert_eq!(
        u64::from_be_bytes(plain[18..26].try_into().unwrap()),
        serial
    );
    assert_eq!(&plain[34..66], &tag);
    assert_eq!(
        &plain[163..195],
        state
            .servers
            .lock()
            .await
            .get(&selected.binding().server)
            .unwrap()
            .device_id
            .as_bytes()
    );
    let engine = u32::from_be_bytes(plain[404..408].try_into().unwrap());
    plain.fill(0);
    drop(plain);
    drop(reply); // collect actual Input before the next physical flight
    let mut action = purpose.clone();
    action.extend_from_slice(&serial.to_be_bytes());
    action.extend_from_slice(&version.to_be_bytes());
    action.extend_from_slice(&tag);
    let mut admit = action.clone();
    admit.extend_from_slice(&engine.to_be_bytes());
    let header = (4, 2, 0, id.logical());
    let reply = admission::admit(
        &context,
        &route,
        opened_frame(&public, &route, header, &admit, None),
    )
    .await
    .unwrap();
    let plain = opened_reply(&public, header, &admit, &reply);
    assert_eq!(&plain[..2], &[1, 6]);
    assert_eq!(
        u64::from_be_bytes(plain[66..74].try_into().unwrap()),
        serial
    );
    drop(reply);
    let work = row.work().unwrap();
    let original_deadline = work.action().protected_binding().unwrap().deadline;
    let phase = work.phase().clone();
    drop(work);
    let mut result = action.clone();
    result.extend_from_slice(&Sha256::digest([]));
    result.extend_from_slice(&0u16.to_be_bytes());
    let clock = crate::voice_runtime(state).clock();
    let mut capacity_refusals = 0u32;
    let status = loop {
        // Native local contention and Root capacity are retryable on the SAME output,
        // action/permit and original deadline. A fresh physical nonce is not new work.
        // Capacity may reflect local registry contention or Root pressure; the refusal
        // alone does not identify its cause. The old one-shot fixture treated it as
        // terminal even though the browser preserves this exact request for retry.
        assert!(
            clock.monotonic_ms() < original_deadline,
            "original Derive deadline exhausted after {capacity_refusals} capacity refusals"
        );
        let header = (route.session().fixture_counts().1 + 1, 3, 1, id.logical());
        let response = derivation_result::commit(
            &context,
            &route,
            opened_frame(&public, &route, header, &result, Some(&phase)),
        )
        .await
        .expect("actual original result worker must fit its unchanged payment");
        let (status, incoming) = response.into_parts();
        drop(incoming); // collect actual physical owner before an exact new transmission
        match status {
            Ok(status) => break status,
            Err(factory::Refused::Protocol(catcoms_sync::voice_v2::VoiceError::Capacity))
                if clock.monotonic_ms() < original_deadline =>
            {
                capacity_refusals += 1;
                eprintln!("native Derive exact result retry {capacity_refusals}; row {:?}; action {serial}; remaining original ms {}", id.logical(), original_deadline.saturating_sub(clock.monotonic_ms()));
                // The same one-second retry cadence as retained browser cleanup. Do not
                // spin on a busy native fence or grant another action/engine budget.
                clock.sleep(std::time::Duration::from_secs(1)).await;
            }
            Err(error) => panic!("actual original Derive result failed after {capacity_refusals} capacity refusals: {error:?}"),
        }
    };
    let receipt = status.engine.receipt.expect("exact native receipt");
    assert!(matches!(
        receipt.outcome,
        catcoms_sync::voice_v2::crypto_actions::CryptoOutcome::Derived
    ));
    let mut ack = action.clone();
    ack.extend_from_slice(&requests::receipt_digest(&receipt));
    for (operation, body) in [(4, ack), (5, action)] {
        let sequence = route.session().fixture_counts().1 + 1;
        let header = (sequence, operation, 0, id.logical());
        let reply = settlement::apply(
            &context,
            &route,
            opened_frame(&public, &route, header, &body, None),
        )
        .seal()
        .unwrap();
        let plain = opened_reply(&public, header, &body, &reply);
        assert_eq!(plain.len(), replies::STATUS_BYTES);
        assert_eq!(&plain[..2], &[1, 0]);
        drop(reply);
    }
    eprintln!(
        "native Derive complete before End; aggregate bytes/crypto {:?}; original group {:?} (setup ACKed; not an SDP peak)",
        selected.resources().counts(), selected.resources().group_counts(selected.binding().scope.group)
    );
    if return_for_confirmation {
        drop(late_input);
        let mut returned = purpose;
        returned.push(0);
        let header = (route.session().fixture_counts().1 + 1, 6, 0, id.logical());
        let reply = settlement::apply(
            &context,
            &route,
            opened_frame(&public, &route, header, &returned, None),
        )
        .seal()
        .unwrap();
        let status = opened_reply(&public, header, &returned, &reply);
        assert_eq!(status.len(), replies::STATUS_BYTES);
        assert_eq!(
            status[209], 1,
            "actual native Return compacted its local loan"
        );
        drop(reply);
        assert!(selected.lease().is_live());
        return Some(row);
    }
    crate::end_voice_owner_inner(state, selected.id()).await;
    assert!(!selected.lease().is_live());
    let mut retire = purpose.clone();
    retire.push(1); // terminal pilot; never Return then relabel
    let header = (route.session().fixture_counts().1 + 1, 6, 0, id.logical());
    let reply = finalization::respond(
        &context,
        &route,
        opened_frame(&public, &route, header, &retire, None),
    )
    .seal()
    .unwrap();
    let pending = opened_reply(&public, header, &retire, &reply);
    assert_eq!(pending.len(), replies::FINAL_BYTES);
    assert_eq!(pending[67], 0);
    drop(reply);
    let status = row.reconcile().unwrap();
    assert!(status.engine.settled && status.acknowledged && status.drained);
    assert!(!state.voice_handoff.channel_may_close(selected));
    drop(late_input);
    let mut proof = None;
    for _ in 0..2 {
        let sequence = route.session().fixture_counts().1 + 1;
        let header = (sequence, 6, 0, id.logical());
        let reply = finalization::respond(
            &context,
            &route,
            opened_frame(&public, &route, header, &retire, None),
        )
        .seal()
        .unwrap();
        let issued = opened_reply(&public, header, &retire, &reply);
        assert_eq!(issued[67], 1);
        let actual: [u8; 32] = issued[68..100].try_into().unwrap();
        if let Some(original) = proof {
            assert_eq!(
                actual, original,
                "lost issuance reply recovers the SAME native proof"
            );
        }
        proof = Some(actual);
        drop(reply);
    }
    let mut final_ack = purpose;
    final_ack.extend_from_slice(&proof.unwrap());
    let header = (route.session().fixture_counts().1 + 1, 7, 0, id.logical());
    let reply = finalization::respond(
        &context,
        &route,
        opened_frame(&public, &route, header, &final_ack, None),
    )
    .seal()
    .unwrap();
    let acknowledged = opened_reply(&public, header, &final_ack, &reply);
    assert_eq!(acknowledged[67], 2);
    assert_eq!(&acknowledged[68..100], &proof.unwrap());
    drop(reply);
    assert!(state.voice_handoff.channel_may_close(selected));
    None
}
