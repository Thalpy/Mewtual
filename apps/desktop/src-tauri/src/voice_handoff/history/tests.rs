//! Actual authenticated requests and replies. Native actor/selection fixtures are synthetic;
//! no successful browser provider or production opcode is claimed by these history tests.
use super::*;
use crate::voice_activation_tests::Fixture;
use crate::voice_authority::SelectedHandoffOwner;
use crate::voice_handoff::bootstrap::{ensure, SetupOutput, Status as SetupStatus};
use crate::voice_handoff::factory::tests::{channel_route, selection};
use crate::voice_handoff::ledger::{Begin, PendingStatus};
use catcoms_sync::voice_v2::presence::PairSelector;
use catcoms_sync::voice_v2::resources::ResourceKind;
use rand_core::SeedableRng;
use std::sync::atomic::Ordering;
use std::sync::Arc;

struct Case {
    native: Fixture,
    context: Context,
    route: Route,
    public: Arc<SetupOutput>,
    selected: Arc<SelectedHandoffOwner>,
    selector: PairSelector,
}
impl Case {
    async fn new() -> Self {
        let native = Fixture::new().await;
        let (_owner, selected, selector) = selection(&native).await;
        let route = channel_route(&native, &selected).await;
        let SetupStatus::Ready(public) = ensure(
            &native.state,
            selected.id(),
            selected.executor(),
            &mut rand_chacha::ChaCha20Rng::seed_from_u64(83),
        )
        .await
        .unwrap() else {
            panic!("ready original")
        };
        let context = Context::new(&native.state);
        Self {
            native,
            context,
            route,
            public,
            selected,
            selector,
        }
    }
    fn pending(&self) -> RowId {
        let Begin::Factory(registration) = self
            .native
            .state
            .voice_handoff
            .begin(self.selected.clone(), Purpose::Derive(self.selector))
            .unwrap()
        else {
            panic!("original factory")
        };
        let id = registration.id();
        registration.mark_handoff().unwrap();
        drop(registration); // lost factory reply may hide Root publication; retain same purpose
        id
    }
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![1];
        bytes.extend_from_slice(self.selector.peer.as_bytes());
        bytes.extend_from_slice(&self.selector.join);
        bytes.extend_from_slice(&self.selector.key_ref);
        bytes.extend_from_slice(&self.selector.epoch.to_be_bytes());
        bytes.push(0); // historical lookup only, never factory retry
        bytes
    }
    fn frame(&self, sequence: u64, operation: u8, logical: [u8; 16], body: &[u8]) -> Incoming {
        crate::voice_handoff::service::tests::opened_frame(
            &self.public,
            &self.route,
            (sequence, operation, 0, logical),
            body,
            None,
        )
    }
    fn query(&self, sequence: u64) -> Response {
        recover(
            &self.context,
            &self.route,
            self.frame(sequence, 8, [0; 16], &self.bytes()),
        )
    }
    fn open(&self, sequence: u64, reply: &CompletedReply) -> Vec<u8> {
        crate::voice_handoff::service::tests::opened_reply(
            &self.public,
            (sequence, 8, 0, [0; 16]),
            &self.bytes(),
            reply,
        )
    }
}

fn attached_query(case: &super::super::derivation_result::tests::Case, sequence: u64) -> Response {
    let mut body = vec![1];
    body.extend_from_slice(case.selector.peer.as_bytes());
    body.extend_from_slice(&case.selector.join);
    body.extend_from_slice(&case.selector.key_ref);
    body.extend_from_slice(&case.selector.epoch.to_be_bytes());
    body.push(0);
    let incoming = crate::voice_handoff::service::tests::opened_frame(
        &case.public,
        &case.route,
        (sequence, 8, 0, [0; 16]),
        &body,
        None,
    );
    recover(&case.context, &case.route, incoming)
}
fn attached_open(
    case: &super::super::derivation_result::tests::Case,
    sequence: u64,
    reply: &CompletedReply,
) -> Vec<u8> {
    let mut body = vec![1];
    body.extend_from_slice(case.selector.peer.as_bytes());
    body.extend_from_slice(&case.selector.join);
    body.extend_from_slice(&case.selector.key_ref);
    body.extend_from_slice(&case.selector.epoch.to_be_bytes());
    body.push(0);
    crate::voice_handoff::service::tests::opened_reply(
        &case.public,
        (sequence, 8, 0, [0; 16]),
        &body,
        reply,
    )
}

#[tokio::test]
async fn actual_accepted_root_result_and_settlement_recover_after_lost_native_refresh_and_end() {
    use catcoms_crypto::DeviceKeypair;
    use catcoms_sync::voice_v2::packets::Authority;
    let mut case = super::super::derivation_result::tests::Case::new().await;
    let stale = case.row().status().unwrap();
    assert!(stale.engine.receipt.is_none() && !stale.engine.settled);
    let (clock, lease, scope) = {
        let authority = case.native.state.voice_authority.lock().unwrap();
        let owner = authority.owner(case.selected.id()).unwrap();
        (
            authority.runtime().clock(),
            owner.lease.clone(),
            owner.binding.scope,
        )
    }; // release native guards BEFORE invoking the actual Root transition/Clock
    let context = catcoms_sync::voice_v2::Context {
        scope,
        local: DeviceKeypair::from_seed(&[3; 32]).device_id(), // original signed fixture device
        now: clock.now_ms(),
        epoch: case.selector.epoch,
        indirect: true,
    };
    let members = |_| true;
    let receipt = case
        .join
        .commit_derivation_result(
            &case.action,
            case.action.output_tag(&[]).unwrap(),
            &Authority {
                context: &context,
                lease: &lease,
                member: &members,
            },
            clock.as_ref(),
        )
        .unwrap();
    assert_eq!(receipt.outcome, CryptoOutcome::Derived);
    case.action.settle_engine();
    assert!(case.action.terminal_status().settled);
    // No native Work/refresh was called. The adapter must observe the original Root action.
    assert!(case.row().status().unwrap() == stale);
    case.native
        .state
        .voice_authority
        .lock()
        .unwrap()
        .end(case.selected.id());
    assert!(case.selected.grant_identity().upgrade().is_none());
    let pool = case.selected.resources();
    let group = case.selected.binding().scope.group;
    let mut pressure = Vec::new();
    for shift in (0..=18).rev() {
        if let Ok(credit) = pool.charge(group, 1 << shift, ResourceKind::Retained) {
            pressure.push(credit);
        }
    }
    assert!(pool.charge(group, 1, ResourceKind::Retained).is_err());
    let full = pool.counts();
    for sequence in [1, 2] {
        let response = attached_query(&case, sequence);
        let (id, status) = response.result.as_ref().unwrap();
        assert!(*id == case.id);
        assert!(
            status.action
                == Some((
                    case.action.serial(),
                    case.action.version(),
                    case.action.input_tag()
                ))
        );
        assert!(status.engine.settled && !status.drained && !status.acknowledged);
        let found = status.engine.receipt.as_ref().unwrap();
        assert!(found.same_transition(&receipt));
        assert!(
            found.retired,
            "End changes observation, never transition identity"
        );
        let expected = replies::status(*id, status).unwrap();
        let completed = response.seal().unwrap();
        let actual = attached_open(&case, sequence, &completed);
        assert_eq!(actual, expected);
        assert_eq!(
            &actual[210..],
            &super::super::requests::receipt_digest(&receipt)
        );
        assert_eq!(
            pool.counts(),
            full,
            "history takes no new Work/task/ordinary credit"
        );
        drop(completed);
    }
    drop(pressure);
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn actual_actor_refusal_then_stale_settlement_recovers_without_inventing_a_receipt() {
    let case = super::super::derivation_result::tests::Case::new().await;
    // This actual Actor deliberately lacks the synthetic fixture grant/MLS pair. Refusal
    // precedes a Root transition: it MUST NOT be relabelled as a rejected terminal receipt.
    let incoming = case.frame(1, &case.bytes());
    let rejected = super::super::derivation_result::commit(&case.context, &case.route, incoming)
        .await
        .unwrap();
    let (result, incoming) = rejected.into_parts();
    assert!(matches!(
        result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    drop(incoming); // actual result/RPC physical owners have drained before settlement
    case.action.settle_engine();
    let actual_root = case.action.terminal_status();
    assert!(actual_root.settled && actual_root.receipt.is_none());
    let stale = case.row().status().unwrap();
    assert!(!stale.engine.settled && stale.engine.receipt.is_none());
    let counts = case.selected.resources().counts();
    let response = attached_query(&case, 2);
    let (id, status) = response.result.as_ref().unwrap();
    assert!(*id == case.id && status.action == stale.action);
    assert!(status.engine == actual_root);
    assert!(!status.drained && !status.acknowledged);
    let expected = replies::status(*id, status).unwrap();
    let completed = response.seal().unwrap();
    assert_eq!(attached_open(&case, 2, &completed), expected);
    assert_eq!(case.selected.resources().counts(), counts);
    drop(completed);
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn lost_begin_reply_recovers_same_original_pending_under_full_group_pressure() {
    let case = Case::new().await;
    let original = case.pending();
    let pool = case.selected.resources();
    let pressure = pool
        .charge(
            case.selected.binding().scope.group,
            256 * 1024 - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let full = pool.counts();
    for sequence in [1, 2] {
        let response = case.query(sequence);
        let (id, status) = response.result.as_ref().unwrap();
        assert!(*id == original);
        assert!(status.pending == PendingStatus::Open && status.action.is_none());
        assert!(!status.drained && !status.engine.settled);
        let expected = replies::status(*id, status).unwrap();
        let completed = response.seal().unwrap();
        assert_eq!(case.open(sequence, &completed), expected);
        assert!(
            case.route.session().header().is_err(),
            "uncollected reply retains original physical position"
        );
        assert_eq!(pool.counts(), full);
        drop(completed);
    }
    // Querying idle Pending never reacquires its worker or gives it another native serial.
    let Begin::Factory(retry) = case
        .native
        .state
        .voice_handoff
        .begin(case.selected.clone(), Purpose::Derive(case.selector))
        .unwrap()
    else {
        panic!("idle original retry")
    };
    assert!(retry.id() == original);
    drop(retry);
    assert_eq!(pool.counts(), full);
    drop(pressure);
    case.native.close().await;
}

#[tokio::test]
async fn unknown_begin_and_requested_retry_do_not_allocate_or_spend_a_native_factory_serial() {
    let case = Case::new().await;
    let pool = case.selected.resources();
    let before = pool.counts();
    let reply = case.query(1).seal().unwrap();
    assert_eq!(
        case.open(1, &reply),
        replies::refused(VoiceError::Unauthorized.into())
    );
    drop(reply);
    let mut retry = case.bytes();
    *retry.last_mut().unwrap() = 1;
    let response = recover(
        &case.context,
        &case.route,
        case.frame(2, 8, [0; 16], &retry),
    );
    assert!(matches!(
        &response.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    drop(response);
    assert_eq!(pool.counts(), before);
    assert!(case
        .native
        .state
        .voice_handoff
        .lookup_original(&case.selected, Purpose::Derive(case.selector))
        .is_none());
    let id = case.pending();
    assert_eq!(
        &id.logical()[8..],
        &1u64.to_be_bytes(),
        "read-only Begin must not consume native IDs"
    );
    case.native.close().await;
}

#[tokio::test]
async fn history_requires_zero_begin_header_full_purpose_canonical_body_and_opcode() {
    let case = Case::new().await;
    let id = case.pending();
    let before = case.selected.resources().counts();
    let mut bodies = Vec::new();
    for offset in [1, 33, 65, 97] {
        let mut bytes = case.bytes();
        bytes[offset] ^= 1;
        bodies.push(bytes);
    }
    let mut extra = case.bytes();
    extra.push(0);
    bodies.push(extra);
    let mut flag = case.bytes();
    *flag.last_mut().unwrap() = 2;
    bodies.push(flag);
    for (i, bytes) in bodies.into_iter().enumerate() {
        let response = recover(
            &case.context,
            &case.route,
            case.frame(i as u64 + 1, 8, [0; 16], &bytes),
        );
        assert!(response.result.is_err());
    }
    for (sequence, operation, logical) in [(7, 8, id.logical()), (8, 1, [0; 16])] {
        let response = recover(
            &case.context,
            &case.route,
            case.frame(sequence, operation, logical, &case.bytes()),
        );
        assert!(matches!(
            &response.result,
            Err(Refused::Protocol(VoiceError::Unauthorized))
        ));
    }
    assert_eq!(case.selected.resources().counts(), before);
    assert!(case.query(9).result.as_ref().unwrap().0 == id);
    case.native.close().await;
}

#[tokio::test]
async fn authenticated_equal_label_foreign_session_cannot_read_original_history() {
    let case = Case::new().await;
    case.pending();
    let foreign = Case::new().await;
    let frame = foreign.frame(1, 8, [0; 16], &case.bytes());
    let response = recover(&case.context, &case.route, frame);
    assert!(matches!(
        &response.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    drop(response);
    assert!(case.query(1).result.is_ok());
    case.native.close().await;
    foreign.native.close().await;
}

#[tokio::test]
async fn original_history_survives_end_lock_cycle_and_explicit_local_pending_closure() {
    let case = Case::new().await;
    let id = case.pending();
    case.native
        .state
        .voice_authority
        .lock()
        .unwrap()
        .end(case.selected.id());
    assert!(case.selected.grant_identity().upgrade().is_none());
    case.native
        .state
        .session_lock_requested
        .store(true, Ordering::Release);
    case.native
        .state
        .ui_session_generation
        .fetch_add(1, Ordering::AcqRel);
    let response = case.query(1);
    assert!(response.result.as_ref().unwrap().0 == id);
    drop(response);
    case.native
        .state
        .session_lock_requested
        .store(false, Ordering::Release);
    case.native
        .state
        .ui_session_generation
        .fetch_add(1, Ordering::AcqRel);
    // No SDK work was ever exposed in this fixture. Local preparation closure remains
    // distinct from uncertain Root publication and from Phase/Workflow retirement.
    case.native
        .state
        .voice_handoff
        .resolve(&case.selected, Purpose::Derive(case.selector), id)
        .unwrap()
        .close_pending()
        .unwrap();
    let response = case.query(2);
    let (found, status) = response.result.as_ref().unwrap();
    assert!(*found == id);
    assert!(
        status.pending
            == PendingStatus::Closed {
                actor_handoff: true
            }
    );
    assert!(
        !status.engine.settled
            && !status.drained
            && !status.acknowledged
            && status.action.is_none()
    );
    let expected = replies::status(*found, status).unwrap();
    let completed = response.seal().unwrap();
    assert_eq!(case.open(2, &completed), expected);
    drop(completed);
    assert!(case.selected.grant_identity().upgrade().is_none());
    case.native.close().await;
}
