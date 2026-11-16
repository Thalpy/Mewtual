//! Real original channel MAC/AEAD and protected Root action custody, with the existing
//! synthetic desktop grant fixture. No production opcode or normal Actor success is enabled.
use super::*;
use crate::voice_activation_tests::Fixture;
use crate::voice_handoff::bootstrap::{fixture_installed as installed, SetupOutput};
use crate::voice_handoff::ledger::{
    tests::{actual_action, selection},
    Begin,
};
use catcoms_sync::voice_v2::crypto_actions::CryptoInputLease;
use catcoms_sync::voice_v2::presence::{LocalJoin, PairSelector};
use catcoms_sync::voice_v2::resources::ResourceKind;
use std::sync::Arc;

struct Case {
    native: Fixture,
    coordinator: crate::voice_authority::Coordinator,
    route: Route,
    public: Arc<SetupOutput>,
    selector: PairSelector,
    id: RowId,
    input: Option<CryptoInputLease>,
    _join: LocalJoin,
}
impl Case {
    async fn new() -> Self {
        let native = Fixture::new().await;
        let (coordinator, selected, selector) = selection();
        let (public, route) = installed(&native.state.voice_channels, selected.clone());
        let ledger = &native.state.voice_handoff;
        let Begin::Factory(registration) = ledger
            .begin(selected.clone(), Purpose::Derive(selector))
            .unwrap()
        else {
            panic!("new original")
        };
        let (join, action, pair) = actual_action(&coordinator, &selected);
        let input = action.input().unwrap();
        let id = registration
            .attach(action.clone(), pair, 16 * 1024)
            .unwrap();
        action.settle_engine();
        drop(action);
        Self {
            native,
            coordinator,
            route,
            public,
            selector,
            id,
            input: Some(input),
            _join: join,
        }
    }
    fn bytes(&self, proof: Option<[u8; 32]>) -> Vec<u8> {
        let mut bytes = vec![1];
        bytes.extend_from_slice(self.selector.peer.as_bytes());
        bytes.extend_from_slice(&self.selector.join);
        bytes.extend_from_slice(&self.selector.key_ref);
        bytes.extend_from_slice(&self.selector.epoch.to_be_bytes());
        if let Some(proof) = proof {
            bytes.extend_from_slice(&proof);
        } else {
            bytes.push(1);
        }
        bytes
    }
    fn frame(&self, sequence: u64, proof: Option<[u8; 32]>) -> Incoming {
        crate::voice_handoff::service::tests::opened_frame(
            &self.public,
            &self.route,
            (
                sequence,
                if proof.is_some() { 7 } else { 6 },
                0,
                self.id.logical(),
            ),
            &self.bytes(proof),
            None,
        )
    }
    fn respond(&self, sequence: u64, proof: Option<[u8; 32]>) -> Response {
        respond(
            &Context::new(&self.native.state),
            &self.route,
            self.frame(sequence, proof),
        )
    }
    fn opened(&self, sequence: u64, proof: Option<[u8; 32]>, reply: &CompletedReply) -> Vec<u8> {
        crate::voice_handoff::service::tests::opened_reply(
            &self.public,
            (
                sequence,
                if proof.is_some() { 7 } else { 6 },
                0,
                self.id.logical(),
            ),
            &self.bytes(proof),
            reply,
        )
    }
}

#[tokio::test]
async fn delayed_root_owner_and_lost_issuance_and_ack_replies_recover_on_original_session() {
    let mut case = Case::new().await;
    let response = case.respond(1, None);
    let pending = response.result.as_ref().unwrap();
    assert!(pending.proof.is_none() && !pending.acknowledged);
    assert!(
        case.route.session().header().is_err(),
        "uncollected response owns actual physical position"
    );
    let reply = response.seal().unwrap();
    let bytes = case.opened(1, None, &reply);
    assert_eq!(bytes.len(), 100);
    assert_eq!(bytes[67], 0);
    assert!(bytes[68..].iter().all(|b| *b == 0));
    drop(reply);
    let selected = case.route.selected();
    let ledger = &case.native.state.voice_handoff;
    assert!(!ledger.channel_may_close(selected));
    // Fill the ordinary group pool AFTER compaction, while the real late Root input remains.
    let pool = selected.resources();
    let pressure = pool
        .charge(
            selected.binding().scope.group,
            catcoms_sync::voice_v2::resources::GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let full = pool.counts();
    let response = case.respond(2, None);
    assert!(response.result.as_ref().unwrap().proof.is_none());
    drop(response);
    assert_eq!(pool.counts(), full);
    drop(case.input.take());
    let response = case.respond(3, None);
    let issued = *response.result.as_ref().unwrap();
    let proof = issued.proof.unwrap();
    drop(response); // actual completed-but-lost issued response
    assert!(!ledger.channel_may_close(selected));
    let response = case.respond(4, None);
    assert_eq!(*response.result.as_ref().unwrap(), issued);
    let reply = response.seal().unwrap();
    assert_eq!(
        case.opened(4, None, &reply),
        replies::final_receipt(issued).unwrap()
    );
    assert!(case.route.session().header().is_err());
    drop(reply);
    case.coordinator.end(selected.id()); // original cleanup remains available after End
    let response = case.respond(5, Some(proof));
    assert!(response.result.as_ref().unwrap().acknowledged);
    drop(response); // ACK committed; only its reply is lost
    assert!(ledger.channel_may_close(selected));
    let response = case.respond(6, Some(proof));
    let acked = *response.result.as_ref().unwrap();
    assert!(acked.acknowledged && acked.proof == Some(proof));
    let reply = response.seal().unwrap();
    assert_eq!(
        case.opened(6, Some(proof), &reply),
        replies::final_receipt(acked).unwrap()
    );
    drop((reply, pressure));
    case.native.close().await;
}

#[tokio::test]
async fn exact_opened_purpose_row_operation_and_original_session_precede_terminal_mutation() {
    let case = Case::new().await;
    for (sequence, operation, logical, change) in [
        (1, 6, case.id.logical(), true),
        (2, 6, [0; 16], false),
        (3, 1, case.id.logical(), false),
    ] {
        let mut bytes = case.bytes(None);
        if change {
            bytes[1] ^= 1;
        }
        if operation == 1 {
            bytes.pop();
        }
        let incoming = crate::voice_handoff::service::tests::opened_frame(
            &case.public,
            &case.route,
            (sequence, operation, 0, logical),
            &bytes,
            None,
        );
        let response = respond(&Context::new(&case.native.state), &case.route, incoming);
        assert!(matches!(
            response.result,
            Err(Refused::Protocol(VoiceError::Unauthorized))
        ));
        drop(response);
    }
    let row = case
        .native
        .state
        .voice_handoff
        .resolve(
            case.route.selected(),
            Purpose::Derive(case.selector),
            case.id,
        )
        .unwrap();
    assert_eq!(
        row.confirmation_status().unwrap(),
        crate::voice_handoff::custody::ConfirmationStatus::NotRequested
    );
    let foreign = Case::new().await;
    let incoming = case.frame(4, None);
    let response = respond(&Context::new(&case.native.state), &foreign.route, incoming);
    assert!(matches!(
        response.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    drop(response);
    assert_eq!(
        row.confirmation_status().unwrap(),
        crate::voice_handoff::custody::ConfirmationStatus::NotRequested
    );
    drop(row);
    foreign.native.close().await;
    case.native.close().await;
}
