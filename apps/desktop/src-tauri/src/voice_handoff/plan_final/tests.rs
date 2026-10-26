//! Actual native registry, plan and Root input custody with the existing synthetic desktop
//! owner fixture. These metadata/closure tests do not run a provider, IPC, Actor or WebView.
use super::*;
use crate::voice_authority::Coordinator;
use crate::voice_handoff::ledger::{Begin, ConfirmationPurpose, FinalKind, Purpose};
use crate::voice_handoff::{bootstrap, channel, custody::CloseMode, ledger};
use crate::AppState;
use catcoms_sync::voice_v2::crypto_actions::CryptoAction;
use catcoms_sync::voice_v2::presence::{LocalJoin, PairSelector};
use catcoms_sync::voice_v2::VoiceError;
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::task::Poll;

struct Case {
    state: AppState,
    coordinator: Coordinator,
    selected: Arc<SelectedHandoffOwner>,
    selector: PairSelector,
    row: ResolvedRow,
    route: Route,
    _output: Arc<bootstrap::SetupOutput>,
    action: Arc<CryptoAction>,
    _join: LocalJoin,
}
impl Case {
    fn new() -> Self {
        let state = AppState::default();
        let (coordinator, selected, selector) = ledger::tests::selection();
        let (output, route) = bootstrap::fixture_installed(&state.voice_channels, selected.clone());
        route.acknowledge_setup().unwrap();
        let Begin::Factory(registration) = state
            .voice_handoff
            .begin(selected.clone(), Purpose::Derive(selector))
            .unwrap()
        else {
            panic!("actual original parent")
        };
        let (join, action, pair) = ledger::tests::actual_action(&coordinator, &selected);
        let id = registration
            .attach(action.clone(), pair, 16 * 1024)
            .unwrap();
        let row = state
            .voice_handoff
            .resolve(&selected, Purpose::Derive(selector), id)
            .unwrap();
        Self {
            state,
            coordinator,
            selected,
            selector,
            row,
            route,
            _output: output,
            action,
            _join: join,
        }
    }
    fn returned(&self) {
        self.action.settle_engine(); // No provider ran; cancelled Derive still needs cleanup.
        self.row
            .close_phase(self.route.session().identity(), CloseMode::Return)
            .unwrap();
        assert!(self.row.try_compact().unwrap());
    }
    async fn finish(&self, setup: bool) -> Result<bootstrap::Status, channel::Refused> {
        if setup {
            bootstrap::settle_setup(&self.state, self.selected.id(), self.selected.executor()).await
        } else {
            bootstrap::collect_channel(&self.state, self.selected.id(), self.selected.executor())
                .await
        }
    }
    fn predicted_plan_proof(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"Mewtual native voice final retirement v1\0");
        hash.update(self.route.session().identity().0);
        hash.update(self.row.id().logical());
        hash.update([0]);
        hash.update(self.selector.peer.as_bytes());
        hash.update(self.selector.join);
        hash.update(self.selector.key_ref);
        hash.update(self.selector.epoch.to_be_bytes());
        hash.update(self.action.serial().to_be_bytes());
        hash.update(self.action.version().to_be_bytes());
        hash.update(self.action.input_tag());
        hash.update([2]);
        hash.finalize().into()
    }
    fn body(&self, mode: u8) -> Vec<u8> {
        let mut body = vec![1];
        body.extend_from_slice(self.selector.peer.as_bytes());
        body.extend_from_slice(&self.selector.join);
        body.extend_from_slice(&self.selector.key_ref);
        body.extend_from_slice(&self.selector.epoch.to_be_bytes());
        body.push(mode);
        body
    }
    fn frame(&self, sequence: u64, operation: u8, logical: [u8; 16], body: &[u8]) -> Incoming {
        crate::voice_handoff::service::tests::opened_frame(
            &self._output,
            &self.route,
            (sequence, operation, 0, logical),
            body,
            None,
        )
    }
    fn open_reply(
        &self,
        sequence: u64,
        operation: u8,
        body: &[u8],
        reply: &CompletedReply,
    ) -> Vec<u8> {
        crate::voice_handoff::service::tests::opened_reply(
            &self._output,
            (sequence, operation, 0, self.row.id().logical()),
            body,
            reply,
        )
    }
}
async fn once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
    poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
}

pub(super) fn future_layout_bytes() -> usize {
    fn handler<F: Future>(
        _: impl FnOnce(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    let case = Case::new();
    let context = Context::new(&case.state);
    // Unpolled actual production futures, not substitute structs with similar fields.
    // Conservatively count both, even if the compiler stores admit inside request.
    let bytes = std::mem::size_of_val(&admit(&context, &case.route))
        + std::mem::size_of_val(&request(&context, &case.route, &case.row))
        + handler(apply);
    bytes // Drop all actual unpolled future temporaries before their fixture owners.
}

#[tokio::test]
async fn returned_and_root_retired_parent_cannot_close_control_before_its_actual_final_intent_and_ack(
) {
    for setup in [false, true] {
        let mut case = Case::new();
        case.returned();
        case.coordinator.end(case.selected.id());
        assert!(
            case.row.retire_plan().unwrap(),
            "actual original Root control/witness retired"
        );
        assert!(case.row.reconcile().unwrap().drained);
        // Native End/Root drainage is independent from delivery of final proof to the
        // original browser. The first close-first call must preserve that Control path.
        assert!(matches!(
            case.finish(setup).await,
            Err(channel::Refused::Capacity)
        ));
        assert!(case.route.require_open().is_err());
        assert!(case
            .state
            .voice_channels
            .require_cleanup_route(&case.route)
            .is_ok());
        let context = Context::new(&case.state);
        let final_receipt = request(&context, &case.route, &case.row).await.unwrap();
        assert_eq!(final_receipt.kind, FinalKind::Plan);
        assert!(final_receipt.proof.is_some() && !final_receipt.acknowledged);
        assert!(matches!(
            case.finish(setup).await,
            Err(channel::Refused::Capacity)
        ));
        case.row
            .acknowledge_final(
                case.route.session().identity(),
                final_receipt.proof.unwrap(),
            )
            .unwrap();
        assert!(matches!(
            case.finish(setup).await,
            Ok(bootstrap::Status::Closed { .. })
        ));
    }
}

#[tokio::test]
async fn original_parent_final_waits_for_actual_successor_preparation_after_old_workflow_drains() {
    let mut case = Case::new();
    case.returned();
    let prepared = case.row.successor_before_retirement_for_test();
    case.coordinator.end(case.selected.id());
    let context = Context::new(&case.state);
    let pending = request(&context, &case.route, &case.row).await.unwrap();
    assert_eq!(pending.kind, FinalKind::Plan);
    assert!(pending.proof.is_none());
    assert!(matches!(
        case.finish(false).await,
        Err(channel::Refused::Capacity)
    ));
    drop(prepared);
    let final_receipt = request(&context, &case.route, &case.row).await.unwrap();
    assert_eq!(final_receipt.kind, FinalKind::Plan);
    case.row
        .acknowledge_final(
            case.route.session().identity(),
            final_receipt.proof.unwrap(),
        )
        .unwrap();
    assert!(matches!(
        case.finish(false).await,
        Ok(bootstrap::Status::Closed { .. })
    ));
}

#[tokio::test]
async fn encrypted_plan_intent_lost_results_and_exact_ack_use_the_original_session_after_end_and_lock(
) {
    let mut case = Case::new();
    let late = case.action.input().unwrap();
    case.returned();
    case.coordinator.end(case.selected.id());
    crate::register_ui_lock_request(&case.state);
    let context = Context::new(&case.state);
    let body = case.body(2);
    let incoming = case.frame(1, 6, case.row.id().logical(), &body);
    let reply = apply(&context, &case.route, incoming).await.unwrap();
    assert_eq!(case.open_reply(1, 6, &body, &reply)[66..68], [2, 0]);
    assert!(
        case.route.session().header().is_err(),
        "actual completed output retains physical position"
    );
    drop(reply);
    assert!(!case.state.voice_handoff.channel_may_close(&case.selected));
    drop(late);

    let incoming = case.frame(2, 6, case.row.id().logical(), &body);
    drop(apply(&context, &case.route, incoming).await.unwrap()); // Issued reply is lost.
    let incoming = case.frame(3, 6, case.row.id().logical(), &body);
    let reply = apply(&context, &case.route, incoming).await.unwrap();
    let issued = case.open_reply(3, 6, &body, &reply);
    assert_eq!(issued[66..68], [2, 1]);
    assert_eq!(&issued[68..], &case.predicted_plan_proof());
    drop(reply);
    let mut ack = body[..105].to_vec();
    ack.extend_from_slice(&issued[68..]);
    for sequence in [4, 5] {
        let incoming = case.frame(sequence, 7, case.row.id().logical(), &ack);
        let reply = crate::voice_handoff::finalization::respond(&context, &case.route, incoming)
            .seal()
            .unwrap();
        let acknowledged = case.open_reply(sequence, 7, &ack, &reply);
        assert_eq!(acknowledged[66..68], [2, 2]);
        assert_eq!(&acknowledged[68..], &issued[68..]);
        drop(reply); // Includes a committed ACK whose response is lost.
    }
    assert!(case.state.voice_handoff.channel_may_close(&case.selected));
}

#[tokio::test]
async fn authenticated_wrong_purpose_row_session_or_mode_cannot_install_plan_intent() {
    let case = Case::new();
    case.returned();
    let context = Context::new(&case.state);
    for change in 0..10 {
        let mut body = case.body(2);
        let mut logical = case.row.id().logical();
        let mut operation = 6;
        match change {
            0..=3 => body[[1, 33, 65, 104][change]] ^= 1,
            4 => logical = [0; 16],
            5 => operation = 1,
            6 => body[105] = 0,
            7 => body[105] = 1,
            8 => {
                body.pop();
            }
            9 => body.push(0),
            _ => unreachable!(),
        }
        let incoming = case.frame(change as u64 + 1, operation, logical, &body);
        let reply = apply(&context, &case.route, incoming).await.unwrap();
        // The reply is a fixed refusal, never a final receipt; native intent remains absent.
        assert_eq!(reply.body().len(), 3 + 16);
        drop(reply);
        assert!(case
            .row
            .issue_final(case.route.session().identity())
            .is_err());
    }
    let foreign = Case::new();
    let incoming = case.frame(11, 6, case.row.id().logical(), &case.body(2));
    let reply = apply(&context, &foreign.route, incoming).await.unwrap();
    assert_eq!(case.open_reply(11, 6, &case.body(2), &reply), [1, 3, 5]);
    drop(reply);
    assert!(case
        .row
        .issue_final(case.route.session().identity())
        .is_err());
}

#[tokio::test]
async fn cancelling_encrypted_intent_while_waiting_for_actual_commit_installs_nothing() {
    let case = Case::new();
    case.returned();
    let context = Context::new(&case.state);
    let boundary = case.state.ui_session_commit.clone().lock_owned().await;
    let store_owners = Arc::strong_count(&case.state.store);
    let charged = case.selected.resources().counts().0;
    let incoming = case.frame(1, 6, case.row.id().logical(), &case.body(2));
    let mut future = Box::pin(apply(&context, &case.route, incoming));
    assert!(once(future.as_mut()).await.is_pending());
    assert_eq!(
        Arc::strong_count(&case.state.store),
        store_owners,
        "parked Plan-final retains only its original commit handle, not mounted context"
    );
    assert_eq!(case.selected.resources().counts().0, charged);
    assert!(case.route.session().header().is_err());
    assert!(case
        .row
        .issue_final(case.route.session().identity())
        .is_err());
    drop(future);
    assert!(case.route.session().header().is_ok());
    assert_eq!(case.selected.resources().counts().0, charged);
    drop(boundary);
    assert!(case
        .row
        .issue_final(case.route.session().identity())
        .is_err());
    for sequence in [2, 3] {
        let incoming = case.frame(sequence, 6, case.row.id().logical(), &case.body(2));
        let reply = if sequence == 2 {
            crate::voice_handoff::finalization::respond(&context, &case.route, incoming).seal()
        } else {
            crate::voice_handoff::settlement::apply(&context, &case.route, incoming).seal()
        }
        .unwrap();
        assert_eq!(
            case.open_reply(sequence, 6, &case.body(2), &reply),
            [1, 3, if sequence == 2 { 5 } else { 0 }]
        );
        drop(reply);
        assert!(case
            .row
            .issue_final(case.route.session().identity())
            .is_err());
    }
}

#[tokio::test]
async fn parked_plan_final_does_not_keep_app_state_alive_or_admit_after_teardown() {
    let case = Case::new();
    case.returned();
    let context = Context::new(&case.state);
    let route = case.route.clone();
    let store = Arc::downgrade(&case.state.store);
    let resources = case.selected.resources();
    let boundary = case.state.ui_session_commit.clone().lock_owned().await;
    let incoming = case.frame(1, 6, case.row.id().logical(), &case.body(2));
    let mut future = Box::pin(apply(&context, &route, incoming));
    assert!(once(future.as_mut()).await.is_pending());
    let charged = resources.counts().0;
    assert!(
        route.session().header().is_err(),
        "actual original request remains busy during wait"
    );
    drop(case);
    assert!(
        store.upgrade().is_none(),
        "a parked cleanup must not outlive the app by retaining its store"
    );
    drop(boundary);
    let reply = future.await.unwrap(); // fresh weak-context refusal, still framed by original Session
    assert!(context.upgrade().is_none());
    assert!(
        route.session().fixture_counts().0,
        "actual output owns the same physical position"
    );
    assert!(
        resources.counts().0 <= charged,
        "teardown/refusal adds no fresh payment"
    );
    drop(reply);
    assert!(
        !route.session().fixture_counts().0,
        "only actual output collection releases the original position"
    );
}

#[tokio::test]
async fn admitted_original_session_and_row_identity_remain_paid_across_forced_shutdown_and_gc() {
    for original_proof_first in [false, true] {
        let case = Case::new();
        let late_input = case.action.input().unwrap();
        case.returned(); // The original variable Phase is actually gone before admission.
        let context = Context::new(&case.state);
        let admission = admit(&context, &case.route).await.unwrap();
        let session = Arc::downgrade(case.route.session());
        let identity = Arc::downgrade(admission.session());
        let Case {
            state,
            mut coordinator,
            selected,
            selector,
            row,
            route,
            _output: output,
            action,
            _join: join,
        } = case;
        let id = row.id();
        let purpose = Purpose::Derive(selector);

        // A real foreign physical witness may arrive BEFORE intent. Its identical public
        // transcript must not cover the original Session subsequently retained by delivery.
        let foreign = bootstrap::Registry::default();
        let (foreign_output, foreign_route) =
            bootstrap::fixture_installed(&foreign, selected.clone());
        foreign_route.acknowledge_setup().unwrap();
        assert_eq!(foreign_route.hash(), route.hash());
        coordinator.end(selected.id());
        bootstrap::fixture_retire(&foreign, &selected);
        drop((foreign_output, foreign_route));
        foreign.collect_retired(&state.voice_handoff);

        // Actual process-shutdown path, intentionally independent of normal UI-commit
        // closure. Detach canonical owners while Admission is between await and latch.
        state.voice_channels.stop_and_wait_for_physical().await;
        bootstrap::fixture_retire(&state.voice_channels, &selected);
        drop((route, output));
        assert_eq!(
            session.strong_count(),
            1,
            "only Admission owns the paid actual Session"
        );
        state.voice_channels.collect_retired(&state.voice_handoff);
        assert!(state
            .voice_handoff
            .lookup_original(&selected, purpose)
            .is_some());
        row.request_plan_final(&admission).unwrap();
        assert!(row
            .issue_final(admission.session())
            .unwrap()
            .proof
            .is_none());
        drop(admission);
        assert_eq!(session.strong_count(), 0);

        if original_proof_first {
            state.voice_channels.collect_retired(&state.voice_handoff);
            assert_eq!(
                identity.strong_count(),
                1,
                "only the fixed paid Row owns identity after bootstrap collection"
            );
            drop((row, action, join));
            state.voice_channels.collect_retired(&state.voice_handoff);
            assert!(
                state
                    .voice_handoff
                    .lookup_original(&selected, purpose)
                    .is_some(),
                "actual late Root input still prevents history collection"
            );
            drop(late_input);
            state.voice_channels.collect_retired(&state.voice_handoff);
        } else {
            drop((row, action, join, late_input));
            foreign.collect_retired(&state.voice_handoff);
            assert!(
                state
                    .voice_handoff
                    .lookup_original(&selected, purpose)
                    .is_some(),
                "pre-intent foreign witness cannot replace original physical collection"
            );
            state.voice_channels.collect_retired(&state.voice_handoff);
        }
        assert!(state
            .voice_handoff
            .lookup_original(&selected, purpose)
            .is_none());
        assert!(state.voice_handoff.resolve(&selected, purpose, id).is_err());
        assert_eq!(identity.strong_count(), 0);
    }
}

#[tokio::test]
async fn original_plan_final_after_return_waits_for_actual_late_root_input_and_exact_issued_ack() {
    let mut case = Case::new();
    let late = case.action.input().unwrap();
    case.returned();
    case.coordinator.end(case.selected.id());
    assert!(
        case.route.require_open().is_err(),
        "ordinary first-effect gate remains closed after End"
    );
    let context = Context::new(&case.state);
    let admission = admit(&context, &case.route).await.unwrap();
    case.row.request_plan_final(&admission).unwrap();
    let pending = case.row.issue_final(admission.session()).unwrap();
    assert_eq!(pending.kind, FinalKind::Plan);
    assert!(pending.proof.is_none() && !pending.acknowledged);
    assert_eq!(
        &crate::voice_handoff::replies::final_receipt(pending).unwrap()[66..68],
        &[2, 0]
    );
    let predicted = case.predicted_plan_proof();
    assert!(case
        .row
        .acknowledge_final(admission.session(), predicted)
        .is_err());
    assert!(!case.state.voice_handoff.channel_may_close(&case.selected));
    drop(late); // The actual last Root input, never a timeout or renderer drainage flag.
    assert!(
        case.row
            .acknowledge_final(admission.session(), predicted)
            .is_err(),
        "ready is not issued"
    );
    let issued = case.row.issue_final(admission.session()).unwrap();
    assert_eq!(issued.proof, Some(predicted));
    assert!(!issued.acknowledged);
    assert_eq!(
        issued,
        case.row.issue_final(admission.session()).unwrap(),
        "lost issuance recovers exact history"
    );
    assert!(case
        .row
        .acknowledge_final(admission.session(), [99; 32])
        .is_err());
    let acked = case
        .row
        .acknowledge_final(admission.session(), predicted)
        .unwrap();
    assert!(acked.acknowledged);
    assert_eq!(
        &crate::voice_handoff::replies::final_receipt(acked).unwrap()[66..68],
        &[2, 2]
    );
    assert_eq!(
        acked,
        case.row
            .acknowledge_final(admission.session(), predicted)
            .unwrap()
    );
    assert!(case.state.voice_handoff.channel_may_close(&case.selected));
    drop(admission);
    assert!(matches!(
        case.finish(false).await,
        Ok(bootstrap::Status::Closed { .. })
    ));
}

#[tokio::test]
async fn phase_and_plan_final_meanings_cannot_replace_each_other() {
    for plan_first in [false, true] {
        let case = Case::new();
        if plan_first {
            case.returned();
        } else {
            case.action.settle_engine();
            case.row
                .close_phase(case.route.session().identity(), CloseMode::Retire)
                .unwrap();
            assert!(case.row.try_compact().unwrap());
        }
        let context = Context::new(&case.state);
        let admission = admit(&context, &case.route).await.unwrap();
        if plan_first {
            case.row.request_plan_final(&admission).unwrap();
            case.row.request_plan_final(&admission).unwrap();
            assert!(case
                .row
                .close_phase(admission.session(), CloseMode::Retire)
                .is_err());
            assert_eq!(
                case.row.issue_final(admission.session()).unwrap().kind,
                FinalKind::Plan
            );
        } else {
            assert!(case.row.request_plan_final(&admission).is_err());
            assert_eq!(
                case.row.issue_final(admission.session()).unwrap().kind,
                FinalKind::Phase
            );
        }
    }
}

#[tokio::test]
async fn automatic_work_hides_only_issued_acked_and_actually_retired_original_plan() {
    let case = Case::new();
    let ledger = &case.state.voice_handoff;
    let visible = || {
        ledger
            .project_work(&case.selected, 0)
            .unwrap()
            .metadata
            .is_some()
    };
    assert!(!ledger.automatic_parent_available(&case.selected).unwrap());
    assert!(visible());
    let late = case.action.input().unwrap();
    case.returned();
    let context = Context::new(&case.state);
    let admission = admit(&context, &case.route).await.unwrap();
    case.row.request_plan_final(&admission).unwrap();
    assert!(case
        .row
        .issue_final(admission.session())
        .unwrap()
        .proof
        .is_none());
    assert!(!ledger.automatic_parent_available(&case.selected).unwrap());
    assert!(
        visible(),
        "an actual held Root input must keep this plan outstanding"
    );
    drop(late);
    assert!(!ledger.automatic_parent_available(&case.selected).unwrap());
    assert!(
        visible(),
        "retirement without issuance/ACK is not a certificate"
    );
    let issued = case.row.issue_final(admission.session()).unwrap();
    let proof = issued.proof.expect("actual original Plan retirement");
    assert!(!ledger.automatic_parent_available(&case.selected).unwrap());
    assert!(
        visible(),
        "lost issuance is still outstanding until original ACK"
    );
    let acked = case
        .row
        .acknowledge_final(admission.session(), proof)
        .unwrap();
    assert!(ledger.automatic_parent_available(&case.selected).unwrap());
    assert!(!visible());
    // Omission is never forgetting or authority renewal. Native keeps this same row,
    // Plan-final meaning/proof, and all paid history for exact direct recovery.
    assert_eq!(case.row.issue_final(admission.session()).unwrap(), acked);
    assert_eq!(
        case.row
            .acknowledge_final(admission.session(), proof)
            .unwrap(),
        acked
    );
    assert_eq!(
        ledger
            .lookup_original(&case.selected, Purpose::Derive(case.selector))
            .unwrap()
            .id(),
        case.row.id()
    );
    assert!(
        case.route.require_open().is_ok(),
        "the original live Session was not ended"
    );
}

#[tokio::test]
async fn phase_final_ack_never_hides_a_parent_or_admits_the_next_automatic_plan() {
    let case = Case::new();
    case.action.settle_engine();
    case.row
        .close_phase(case.route.session().identity(), CloseMode::Retire)
        .unwrap();
    assert!(case.row.try_compact().unwrap());
    drop(case.action);
    let issued = case
        .row
        .issue_final(case.route.session().identity())
        .unwrap();
    assert_eq!(issued.kind, FinalKind::Phase);
    case.row
        .acknowledge_final(case.route.session().identity(), issued.proof.unwrap())
        .unwrap();
    assert!(!case
        .state
        .voice_handoff
        .automatic_parent_available(&case.selected)
        .unwrap());
    assert!(case
        .state
        .voice_handoff
        .project_work(&case.selected, 0)
        .unwrap()
        .metadata
        .is_some());
}

#[tokio::test]
async fn certified_projection_hides_exact_children_but_not_an_equal_label_foreign_parent() {
    let case = Case::new();
    case.returned();
    let ledger = &case.state.voice_handoff;
    let request = ConfirmationPurpose::Start(None);
    let (Begin::Factory(registration), _) = ledger
        .begin_child(case.selected.clone(), &case.row, request)
        .unwrap()
    else {
        panic!("actual original child reservation")
    };
    let child_id = registration.id();
    registration.mark_handoff().unwrap();
    drop(registration);
    let context = Context::new(&case.state);
    let admission = admit(&context, &case.route).await.unwrap();
    assert_eq!(
        case.row.request_plan_final(&admission),
        Err(VoiceError::Capacity)
    );
    let parent_snapshot = ledger.project_work(&case.selected, 0).unwrap();
    let child_snapshot = ledger
        .project_work(&case.selected, parent_snapshot.next_slot)
        .unwrap();
    assert_eq!(child_snapshot.metadata.as_ref().unwrap().row, child_id);
    let child_purpose = Purpose::Confirmation {
        parent: case.row.id(),
        request,
    };
    let child = ledger
        .resolve(&case.selected, child_purpose, child_id)
        .unwrap();
    child.close_pending().unwrap();
    assert!(!ledger.automatic_parent_available(&case.selected).unwrap());
    case.row.request_plan_final(&admission).unwrap();
    let issued = case.row.issue_final(admission.session()).unwrap();
    case.row
        .acknowledge_final(admission.session(), issued.proof.unwrap())
        .unwrap();
    assert!(ledger.automatic_parent_available(&case.selected).unwrap());
    assert!(ledger
        .project_work(&case.selected, 0)
        .unwrap()
        .metadata
        .is_none());
    assert!(ledger
        .project_work(&case.selected, parent_snapshot.next_slot)
        .unwrap()
        .metadata
        .is_none());
    // Previously returned metadata and direct authenticated child recovery remain valid
    // history. Neither the parent nor child slot was forgotten or reused by omission.
    assert_eq!(child_snapshot.metadata.unwrap().row, child.id());
    assert_eq!(
        ledger
            .resolve(&case.selected, child_purpose, child_id)
            .unwrap()
            .id(),
        child_id
    );
    let (_coordinator, foreign, foreign_selector) = ledger::tests::selection();
    let Begin::Factory(foreign_row) = ledger
        .begin(foreign.clone(), Purpose::Derive(foreign_selector))
        .unwrap()
    else {
        panic!("foreign actual allocation remains independent")
    };
    let foreign_id = foreign_row.id();
    assert!(!ledger.automatic_parent_available(&foreign).unwrap());
    assert_eq!(
        ledger
            .project_work(&foreign, 0)
            .unwrap()
            .metadata
            .unwrap()
            .row,
        foreign_id
    );
    assert!(ledger.automatic_parent_available(&case.selected).unwrap());
}

#[tokio::test]
async fn foreign_equal_hash_session_or_context_cannot_install_plan_final_intent() {
    let case = Case::new();
    case.returned();
    let other = AppState::default();
    let (_output, foreign) =
        bootstrap::fixture_installed(&other.voice_channels, case.selected.clone());
    foreign.acknowledge_setup().unwrap();
    assert_eq!(foreign.hash(), case.route.hash());
    assert!(!Arc::ptr_eq(
        foreign.session().identity(),
        case.route.session().identity()
    ));
    let context = Context::new(&case.state);
    assert!(
        admit(&context, &foreign).await.is_err(),
        "canonical original registry allocation is required"
    );
    assert!(case
        .row
        .issue_final(case.route.session().identity())
        .is_err());
    let foreign_context = Context::new(&other);
    let foreign_admission = admit(&foreign_context, &foreign).await.unwrap();
    assert!(
        case.row.request_plan_final(&foreign_admission).is_err(),
        "equal owner labels cannot substitute another ledger"
    );
    drop(foreign_admission);
    let admission = admit(&context, &case.route).await.unwrap();
    case.row.request_plan_final(&admission).unwrap();
    assert!(case.row.issue_final(foreign.session().identity()).is_err());
}

#[tokio::test]
async fn child_reservation_and_plan_intent_are_ordered_without_reopening_a_closed_plan() {
    let case = Case::new();
    case.returned();
    let request = ConfirmationPurpose::Start(None);
    let (Begin::Factory(child), _) = case
        .state
        .voice_handoff
        .begin_child(case.selected.clone(), &case.row, request)
        .unwrap()
    else {
        panic!("original reserved child")
    };
    let child_id = child.id();
    child.mark_handoff().unwrap();
    drop(child);
    let context = Context::new(&case.state);
    let admission = admit(&context, &case.route).await.unwrap();
    assert_eq!(
        case.row.request_plan_final(&admission).err(),
        Some(VoiceError::Capacity)
    );
    assert!(
        case.row.issue_final(admission.session()).is_err(),
        "refusal installed no intent"
    );
    let purpose = Purpose::Confirmation {
        parent: case.row.id(),
        request,
    };
    let child = case
        .state
        .voice_handoff
        .resolve(&case.selected, purpose, child_id)
        .unwrap();
    child.close_pending().unwrap();
    assert!(
        child.request_plan_final(&admission).is_err(),
        "a child cannot own parent final delivery"
    );
    case.row.request_plan_final(&admission).unwrap();
    assert!(case
        .state
        .voice_handoff
        .begin_child(
            case.selected.clone(),
            &case.row,
            ConfirmationPurpose::Start(Some([66; 32]))
        )
        .is_err());
    assert!(
        matches!(case.state.voice_handoff.begin_child(case.selected.clone(), &case.row, request).unwrap().0,
        Begin::Existing(id) if id == child_id)
    );
}

#[tokio::test]
async fn an_in_flight_original_retirement_ticket_is_not_a_missing_control_success() {
    let case = Case::new();
    case.returned();
    let ticket = case.row.park_retirement_for_test().unwrap();
    let context = Context::new(&case.state);
    let admission = admit(&context, &case.route).await.unwrap();
    assert_eq!(
        case.row.request_plan_final(&admission).err(),
        Some(VoiceError::Capacity)
    );
    assert!(case.row.issue_final(admission.session()).is_err());
    drop(ticket); // Actual production Retirement publishes its SAME original Root witness.
    case.row.request_plan_final(&admission).unwrap();
    assert!(case
        .row
        .issue_final(admission.session())
        .unwrap()
        .proof
        .is_some());
}

#[tokio::test]
async fn plan_intent_and_exact_final_delivery_need_no_new_ordinary_group_or_process_payment() {
    use catcoms_sync::voice_v2::resources::{ResourceKind, GROUP_BYTES, PROCESS_BYTES};
    let case = Case::new();
    let late = case.action.input().unwrap();
    case.returned();
    let resources = case.selected.resources();
    let group = case.selected.binding().scope.group;
    let _group_pressure = resources
        .charge(
            group,
            GROUP_BYTES - resources.group_counts(group).unwrap().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let _process_pressure = resources
        .charge_process_retained(PROCESS_BYTES - resources.counts().0)
        .unwrap();
    assert_eq!(resources.counts().0, PROCESS_BYTES);
    let context = Context::new(&case.state);
    let admission = admit(&context, &case.route).await.unwrap();
    case.row.request_plan_final(&admission).unwrap();
    assert!(case
        .row
        .issue_final(admission.session())
        .unwrap()
        .proof
        .is_none());
    drop(late);
    let issued = case.row.issue_final(admission.session()).unwrap();
    let proof = issued.proof.unwrap();
    case.row
        .acknowledge_final(admission.session(), proof)
        .unwrap();
    assert!(case.state.voice_handoff.channel_may_close(&case.selected));
}

#[tokio::test]
async fn intent_and_both_normal_closures_share_the_same_actual_ui_commit_in_both_orderings() {
    for setup in [false, true] {
        for intent_first in [false, true] {
            let case = Case::new();
            case.returned();
            assert!(case.row.retire_plan().unwrap()); // Existing trusted retirement happened first.
            let context = Context::new(&case.state);
            if intent_first {
                let admission = admit(&context, &case.route).await.unwrap();
                case.row.request_plan_final(&admission).unwrap();
                let mut close = Box::pin(case.finish(setup));
                assert!(once(close.as_mut()).await.is_pending());
                drop(admission);
                assert!(matches!(close.await, Err(channel::Refused::Capacity)));
                assert!(case
                    .state
                    .voice_channels
                    .require_cleanup_route(&case.route)
                    .is_ok());
            } else {
                let boundary = case.state.ui_session_commit.clone().lock_owned().await;
                let mut close = Box::pin(case.finish(setup));
                assert!(once(close.as_mut()).await.is_pending());
                let mut terminal = Box::pin(request(&context, &case.route, &case.row));
                assert!(once(terminal.as_mut()).await.is_pending());
                drop(boundary);
                assert!(
                    matches!(close.await, Err(channel::Refused::Capacity)),
                    "close-first fences effects but must preserve original final delivery"
                );
                let issued = terminal.await.unwrap();
                assert!(issued.proof.is_some() && !issued.acknowledged);
                assert!(matches!(
                    case.finish(setup).await,
                    Err(channel::Refused::Capacity)
                ));
                case.row
                    .acknowledge_final(case.route.session().identity(), issued.proof.unwrap())
                    .unwrap();
                assert!(matches!(
                    case.finish(setup).await,
                    Ok(bootstrap::Status::Closed { .. })
                ));
            }
        }
    }
}
