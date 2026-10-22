//! Actual authenticated native frame/actor rejection and cancellation barriers. The proven
//! pair below uses the synthetic native discovery fixture, not this ServerActor's MLS pair;
//! Root must reject its first result. These tests do not establish successful live ingress.
use super::*;
use crate::voice_activation_tests::Fixture;
use crate::voice_authority::SelectedHandoffOwner;
use crate::voice_handoff::bootstrap::{ensure, SetupOutput};
use crate::voice_handoff::custody::Phase;
use crate::voice_handoff::factory::tests::{channel_route, park};
use catcoms_sync::voice_v2::crypto_actions::{CryptoAction, CryptoOutcome};
use catcoms_sync::voice_v2::presence::{LocalJoin, PairReceipt, PairSelector};
use rand_core::SeedableRng;
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::task::Poll;

// Shared ONLY inside native handoff tests: the same real action/Phase fixture also pins
// history recovery. This exposes no production constructor or Root fabrication seam.
pub(in crate::voice_handoff) struct Case {
    pub(in crate::voice_handoff) native: Fixture,
    pub(in crate::voice_handoff) context: Context,
    pub(in crate::voice_handoff) route: Route,
    pub(in crate::voice_handoff) public: Arc<SetupOutput>,
    pub(in crate::voice_handoff) selected: Arc<SelectedHandoffOwner>,
    pub(in crate::voice_handoff) selector: PairSelector,
    pub(in crate::voice_handoff) join: LocalJoin,
    pub(in crate::voice_handoff) action: Arc<CryptoAction>,
    pub(in crate::voice_handoff) id: RowId,
    phase: Arc<Phase>,
}
impl Case {
    pub(in crate::voice_handoff) async fn new() -> Self {
        let native = Fixture::new().await;
        let owner = native.candidate().await;
        let runtime = crate::voice_runtime(&native.state);
        let now = runtime.clock().monotonic_ms();
        let selected = {
            let mut authority = native.state.voice_authority.lock().unwrap();
            let crate::voice_authority::CandidateKind::Call { binding, .. } =
                authority.candidate(&owner, now).unwrap()
            else {
                panic!("native call candidate");
            };
            // Synthetic native lifecycle grant, intentionally NOT installed in ServerActor.
            // A real installed grant already binds its LocalJoin to the actual MLS device;
            // reusing it with the deterministic foreign-pair fixture correctly refuses.
            // Keep that check intact: only this test-native grant owns the synthetic pair,
            // so the real Actor MUST later reject first publication with this grant/action.
            let grant =
                CallGrant::from_lease(catcoms_sync::voice_v2::CallLease::default(), binding.scope);
            authority
                .activate(&owner, [1; 32], now, &mut Some(grant))
                .unwrap();
            let admission = authority.admit_join_key(&owner, [9; 32]).unwrap();
            let point: [u8; 65] = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap();
            let crate::voice_authority::PublicationPlan::Start(mut publication) = authority
                .prepare_join_key_publication(&owner, [9; 32], &admission.permit, point)
                .unwrap()
            else {
                panic!("synthetic key publication");
            };
            authority
                .finish_join_key_publication(&mut publication, true)
                .unwrap();
            authority.settle_join_key(&owner, [9; 32]).unwrap();
            authority.selected_handoff_owner(&owner, [9; 32]).unwrap()
        };
        let route = channel_route(&native, &selected).await;
        let public = match ensure(
            &native.state,
            selected.id(),
            selected.executor(),
            &mut rand_chacha::ChaCha20Rng::seed_from_u64(83),
        )
        .await
        .unwrap()
        {
            crate::voice_handoff::bootstrap::Status::Ready(output) => output,
            _ => panic!("ready original channel"),
        };
        let (join, action) = {
            let authority = native.state.voice_authority.lock().unwrap();
            crate::voice_authority::tests::proven_action_with_protection(
                &authority,
                selected.id(),
                selected.resources(),
                true,
            )
        };
        let binding = action.protected_binding().unwrap();
        let selector = PairSelector {
            peer: binding.peer,
            join: binding.peer_join,
            key_ref: binding.peer_key,
            epoch: binding.epoch,
        };
        let crate::voice_handoff::ledger::Begin::Factory(ticket) = native
            .state
            .voice_handoff
            .begin(selected.clone(), Purpose::Derive(selector))
            .unwrap()
        else {
            panic!("new original purpose");
        };
        let id = ticket
            .attach(
                action.clone(),
                PairReceipt {
                    handshake: binding.handshake,
                    peer: binding.peer,
                    local_is_low: binding.local_is_low,
                },
                16 * 1024,
            )
            .unwrap();
        let row = native
            .state
            .voice_handoff
            .resolve(&selected, Purpose::Derive(selector), id)
            .unwrap();
        let work = row.work().unwrap();
        let phase = work.phase().clone();
        drop(work);
        action.admit_engine(action.engine_bytes()).unwrap();
        let context = Context::new(&native.state);
        Self {
            native,
            context,
            route,
            public,
            selected,
            selector,
            join,
            action,
            id,
            phase,
        }
    }
    pub(in crate::voice_handoff) fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![1];
        bytes.extend_from_slice(self.selector.peer.as_bytes());
        bytes.extend_from_slice(&self.selector.join);
        bytes.extend_from_slice(&self.selector.key_ref);
        bytes.extend_from_slice(&self.selector.epoch.to_be_bytes());
        bytes.extend_from_slice(&self.action.serial().to_be_bytes());
        bytes.extend_from_slice(&self.action.version().to_be_bytes());
        bytes.extend_from_slice(&self.action.input_tag());
        bytes.extend_from_slice(&self.action.output_tag(&[]).unwrap());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes
    }
    pub(in crate::voice_handoff) fn frame(&self, sequence: u64, bytes: &[u8]) -> Incoming {
        crate::voice_handoff::service::tests::opened_frame(
            &self.public,
            &self.route,
            (sequence, 3, 1, self.id.logical()),
            bytes,
            Some(&self.phase),
        )
    }
    pub(in crate::voice_handoff) fn row(&self) -> super::super::ledger::ResolvedRow {
        self.native
            .state
            .voice_handoff
            .resolve(&self.selected, Purpose::Derive(self.selector), self.id)
            .unwrap()
    }
    pub(in crate::voice_handoff) fn retire(&self) {
        self.action.settle_engine();
        self.phase.close_confirmation().unwrap();
    }
}
async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
    poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
}

#[tokio::test]
async fn result_identity_purpose_and_shape_refuse_before_work_or_actor_admission() {
    let case = Case::new().await;
    let original = case.bytes();
    for (sequence, offset) in [1usize, 33, 65, 97, 105, 113, 121, 153, 185]
        .into_iter()
        .enumerate()
    {
        let mut changed = original.clone();
        changed[offset] ^= 1;
        let incoming = case.frame(sequence as u64 + 1, &changed);
        let before = case.selected.resources().counts();
        let response = commit(&case.context, &case.route, incoming).await.unwrap();
        assert!(response.result.is_err(), "changed request offset {offset}");
        assert!(response.work.is_none());
        assert!(response.payment.is_none());
        assert_eq!(case.selected.resources().counts(), before);
        assert!(case.row().work().is_ok(), "no native data worker admitted");
        drop(response);
    }
    assert!(case.action.terminal_status().receipt.is_none());
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn an_authenticated_foreign_session_cannot_publish_through_the_original_route() {
    let case = Case::new().await;
    let foreign = Case::new().await;
    assert!(!Arc::ptr_eq(case.route.session(), foreign.route.session()));
    let incoming = crate::voice_handoff::service::tests::opened_frame(
        &foreign.public,
        &foreign.route,
        (1, 3, 1, case.id.logical()),
        &case.bytes(),
        Some(&foreign.phase),
    );
    let local_counts = case.selected.resources().counts();
    let foreign_counts = foreign.selected.resources().counts();
    let response = commit(&case.context, &case.route, incoming).await.unwrap();
    assert!(matches!(
        response.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    assert!(response.work.is_none());
    assert!(response.payment.is_none());
    assert_eq!(case.selected.resources().counts(), local_counts);
    assert_eq!(foreign.selected.resources().counts(), foreign_counts);
    assert!(case.row().work().is_ok());
    drop(response);
    case.retire();
    foreign.retire();
    case.native.close().await;
    foreign.native.close().await;
}

#[tokio::test]
async fn authenticated_changed_header_rowid_refuses_the_valid_original_body() {
    let case = Case::new().await;
    let mut changed = case.id.logical();
    changed[15] = changed[15].checked_add(1).unwrap();
    let incoming = crate::voice_handoff::service::tests::opened_frame(
        &case.public,
        &case.route,
        (1, 3, 1, changed),
        &case.bytes(),
        Some(&case.phase),
    );
    let before = case.selected.resources().counts();
    let response = commit(&case.context, &case.route, incoming).await.unwrap();
    assert!(matches!(
        response.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    assert!(response.work.is_none());
    assert!(response.payment.is_none());
    assert_eq!(case.selected.resources().counts(), before);
    assert!(case.row().work().is_ok());
    assert!(case.action.terminal_status().receipt.is_none());
    drop(response);
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn actual_actor_rejection_retains_work_input_and_task_custody_until_collection() {
    let case = Case::new().await;
    let bytes = case.bytes();
    let incoming = case.frame(1, &bytes);
    let before = case.selected.resources().counts();
    let response = commit(&case.context, &case.route, incoming).await.unwrap();
    assert!(matches!(
        response.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    assert!(response.work.is_some());
    assert_eq!(
        case.selected.resources().counts(),
        before,
        "task borrows the original loan, never ordinary capacity"
    );
    assert!(
        case.phase.buffer(16 * 1024 - TASK_BYTES).is_err(),
        "completed output still owns its original task contribution"
    );
    assert!(matches!(case.row().work(), Err(VoiceError::Capacity)));
    assert!(case.route.session().header().is_err());
    assert!(case.native.state.ui_session_commit.try_lock().is_ok());
    for channel in [false, true] {
        let closure = if channel {
            crate::voice_handoff::bootstrap::collect_channel(
                &case.native.state,
                case.selected.id(),
                case.selected.executor(),
            )
            .await
        } else {
            crate::voice_handoff::bootstrap::settle_setup(
                &case.native.state,
                case.selected.id(),
                case.selected.executor(),
            )
            .await
        };
        assert!(matches!(
            closure,
            Err(crate::voice_handoff::channel::Refused::Capacity)
        ));
        assert!(
            case.route.require_open().is_ok(),
            "an uncollected actual result cannot lose its cleanup route"
        );
        assert!(response.work.is_some());
        assert_eq!(case.selected.resources().counts(), before);
    }
    let (_, incoming) = response.into_parts();
    assert_eq!(case.selected.resources().counts(), before);
    assert!(case.phase.buffer(16 * 1024 - TASK_BYTES).is_ok());
    assert!(case.row().work().is_ok());
    assert_eq!(incoming.plain().unwrap(), bytes);
    assert_eq!(case.route.session().fixture_counts(), (true, 1, 1));
    drop(incoming);
    assert_eq!(case.route.session().fixture_counts(), (false, 1, 1));
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn admitted_result_uses_only_original_task_capacity_under_full_ordinary_group_pressure() {
    use catcoms_sync::voice_v2::resources::ResourceKind;
    let case = Case::new().await;
    let incoming = case.frame(1, &case.bytes());
    let pool = case.selected.resources();
    let group = case.selected.binding().scope.group;
    let mut pressure = Vec::new();
    // Bounded binary fill avoids assuming process-only index bytes belong to this group.
    // Fixture credits are unrelated retained work, not a second task/engine reservation.
    for shift in (0..=18).rev() {
        if let Ok(credit) = pool.charge(group, 1 << shift, ResourceKind::Retained) {
            pressure.push(credit);
        }
    }
    assert!(pool.charge(group, 1, ResourceKind::Retained).is_err());
    let before = pool.counts();
    let response = commit(&case.context, &case.route, incoming).await.unwrap();
    assert!(matches!(
        response.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    assert!(
        response.work.is_some(),
        "actual actor path was admitted despite ordinary pressure"
    );
    assert_eq!(pool.counts(), before);
    drop(response);
    drop(pressure);
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn lock_winning_before_final_result_admission_refuses_without_a_worker_or_new_generation_adoption(
) {
    let case = Case::new().await;
    let incoming = case.frame(1, &case.bytes());
    let boundary = case.native.state.ui_session_commit.lock().await;
    let mut pending = Box::pin(commit(&case.context, &case.route, incoming));
    assert!(poll_once(pending.as_mut()).await.is_pending());
    case.native.actor.member_count().await;
    assert!(poll_once(pending.as_mut()).await.is_pending());
    crate::register_ui_lock_request(&case.native.state);
    drop(boundary);
    let response = pending.await.unwrap();
    assert!(matches!(response.result, Err(Refused::Closed)));
    assert!(response.work.is_none());
    assert!(response.payment.is_none());
    drop(response);
    crate::lock_session_inner(&case.native.state, None)
        .await
        .unwrap();
    let generation = case
        .native
        .state
        .ui_session_generation
        .load(std::sync::atomic::Ordering::Acquire);
    crate::finalize_unlock_session(&case.native.state, generation, false)
        .await
        .unwrap();
    assert!(case.action.terminal_status().receipt.is_none());
    assert!(
        case.row().work().is_err(),
        "unlock cannot revive the original lease"
    );
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn historical_result_survives_end_without_new_work_or_grant_upgrade() {
    let mut case = Case::new().await;
    let incoming = case.frame(1, &case.bytes());
    let receipt = {
        let authority = case.native.state.voice_authority.lock().unwrap();
        crate::voice_authority::tests::derivation_receipt(
            &authority,
            case.selected.id(),
            &mut case.join,
            &case.action,
        )
    };
    assert_eq!(receipt.outcome, CryptoOutcome::Derived);
    case.native
        .state
        .voice_authority
        .lock()
        .unwrap()
        .end(case.selected.id());
    assert!(case.selected.grant().is_err());
    let before = case.selected.resources().counts();
    let response = commit(&case.context, &case.route, incoming).await.unwrap();
    assert!(response
        .result
        .as_ref()
        .unwrap()
        .engine
        .receipt
        .as_ref()
        .unwrap()
        .same_transition(&receipt));
    assert!(response.work.is_none());
    assert!(response.payment.is_none());
    assert_eq!(case.selected.resources().counts(), before);
    drop(response);
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn socket_caller_loss_cannot_finish_shutdown_ahead_of_admitted_result_publication() {
    use crate::voice_handoff::service::{BoundServer, Origin};
    let case = Case::new().await;
    let runtime = case.native.state.voice_authority.lock().unwrap().runtime();
    let running = BoundServer::bind(
        case.native.state.voice_channels.clone(),
        runtime,
        Origin::PackagedHttp,
    )
    .unwrap()
    .start()
    .unwrap();
    let incoming = case.frame(1, &case.bytes());
    let boundary = case.native.state.ui_session_commit.lock().await;
    let mut pending = Box::pin(commit(&case.context, &case.route, incoming));
    assert!(poll_once(pending.as_mut()).await.is_pending());
    case.native.actor.member_count().await;
    assert!(poll_once(pending.as_mut()).await.is_pending());
    let parked = park(&case.native.actor).await;
    drop(boundary);
    assert!(poll_once(pending.as_mut()).await.is_pending());
    drop(pending);
    assert!(case.native.state.ui_session_commit.try_lock().is_err());
    let mut finish = Box::pin(running.finish());
    assert!(poll_once(finish.as_mut()).await.is_pending());
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while case.route.require_open().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(poll_once(finish.as_mut()).await.is_pending());
    drop(parked);
    assert!(
        !tokio::time::timeout(std::time::Duration::from_secs(15), finish)
            .await
            .unwrap()
            .unwrap()
            .io_failed
    );
    assert!(case.native.state.ui_session_commit.try_lock().is_ok());
    assert!(case.row().work().is_ok());
    assert_eq!(case.route.session().fixture_counts(), (false, 1, 1));
    case.retire();
    case.native.close().await;
}
