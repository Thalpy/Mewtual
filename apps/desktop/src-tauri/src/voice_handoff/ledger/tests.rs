//! Synthetic desktop grant and publication completion; real signed discovery, LocalJoin and
//! protected action/result factories. Not ServerActor/MLS/IPC/WebView acceptance evidence.
use super::*;
use crate::voice_authority::{Binding, CandidateKind, Coordinator, PublicationPlan};
use crate::voice_handoff::channel::tests::FixtureChannel;
use catcoms_crypto::DeviceKeypair;
use catcoms_sync::voice_v2::runtime::Runtime;
use catcoms_sync::voice_v2::{key_reference, Scope};

mod finalization;
mod pending;
mod retirement;

fn noop_waker() -> std::task::Waker {
    struct Idle;
    impl std::task::Wake for Idle {
        fn wake(self: Arc<Self>) {}
    }
    std::task::Waker::from(Arc::new(Idle))
}

pub(in crate::voice_handoff) fn selection() -> (Coordinator, Arc<SelectedHandoffOwner>, PairSelector)
{
    selection_with_runtime(Runtime::new(Arc::new(catcoms_rt::ManualClock::new(1000))))
}
pub(in crate::voice_handoff) fn selection_with_runtime(
    runtime: Arc<Runtime>,
) -> (Coordinator, Arc<SelectedHandoffOwner>, PairSelector) {
    let mut coordinator = Coordinator::with_runtime(runtime);
    let controller = coordinator
        .prepare(CandidateKind::Controller(1), 0)
        .unwrap();
    coordinator
        .activate(&controller, [0; 32], 0, &mut None)
        .unwrap();
    let binding = Binding {
        generation: 1,
        server: 7,
        instance: 1,
        group: vec![1],
        scope: Scope::for_group(&[1], Some([8; 32]), 12),
    };
    let id = coordinator
        .prepare(
            CandidateKind::Call {
                controller,
                binding: binding.clone(),
            },
            0,
        )
        .unwrap();
    let grant = CallGrant::from_lease(CallLease::default(), binding.scope);
    coordinator
        .activate(&id, [1; 32], 0, &mut Some(grant))
        .unwrap();
    let point: [u8; 65] = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap();
    let admission = coordinator.admit_join_key(&id, [9; 32]).unwrap();
    let PublicationPlan::Start(mut work) = coordinator
        .prepare_join_key_publication(&id, [9; 32], &admission.permit, point)
        .unwrap()
    else {
        panic!("publication")
    };
    coordinator
        .finish_join_key_publication(&mut work, true)
        .unwrap();
    coordinator.settle_join_key(&id, [9; 32]).unwrap();
    let selected = coordinator.selected_handoff_owner(&id, [9; 32]).unwrap();
    let selector = PairSelector {
        peer: DeviceKeypair::from_seed(&[4; 32]).device_id(),
        join: [7; 32],
        key_ref: key_reference(&point),
        epoch: 1,
    };
    (coordinator, selected, selector)
}
fn ticket(
    ledger: &Arc<Ledger>,
    selected: Arc<SelectedHandoffOwner>,
    selector: PairSelector,
) -> Registration {
    let Begin::Factory(ticket) = ledger.begin(selected, Purpose::Derive(selector)).unwrap() else {
        panic!("factory ticket")
    };
    ticket
}
pub(in crate::voice_handoff) fn actual_action(
    coordinator: &Coordinator,
    selected: &SelectedHandoffOwner,
) -> (
    catcoms_sync::voice_v2::presence::LocalJoin,
    Arc<CryptoAction>,
    PairReceipt,
) {
    let (join, action) = crate::voice_authority::tests::proven_action_with_protection(
        coordinator,
        selected.id(),
        selected.resources(),
        true,
    );
    let binding = action.protected_binding().unwrap();
    let pair = PairReceipt {
        handshake: binding.handshake,
        peer: binding.peer,
        local_is_low: binding.local_is_low,
    };
    (join, action, pair)
}

#[test]
fn complete_records_index_handles_and_one_worker_fit_existing_fixed_payments() {
    let ledger = Ledger::default();
    let waiter = ledger.wait_compacted(RowId { slot: 0, serial: 0 });
    // Selected and Phase have their own paid records. Include full native Work/Registration
    // wrappers, index slot, temporary Status response and Arc allocation counters here.
    let bytes = std::mem::size_of::<Row>()
        + 2 * std::mem::size_of::<usize>()
        + std::mem::size_of::<CallGrant>()
        + 2 * std::mem::size_of::<usize>()
        + std::mem::size_of::<Option<Arc<Row>>>()
        + std::mem::size_of::<Registration>()
        + std::mem::size_of::<Work>()
        + std::mem::size_of::<ResolvedRow>()
        + std::mem::size_of::<Observation>()
        // Pending preparation and a fully compacted plan can never close concurrently on
        // the SAME row. Cover the larger actual closure ticket in this paid position.
        + std::mem::size_of::<PendingRetirement>().max(plan::RETIREMENT_MEMORY_BYTES)
        + std::mem::size_of::<ClosePending>()
        + std::mem::size_of::<Option<Arc<Row>>>() // fixed collector's one removed-slot owner
        + std::mem::size_of_val(&waiter)
        + std::mem::size_of::<Status>();
    assert!(
        bytes <= ROW_BYTES,
        "native row layout {bytes} exceeds {ROW_BYTES}"
    );
}

#[test]
fn pending_retirement_is_nonretryable_until_actual_original_preparation_drops() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    registration.mark_handoff().unwrap();
    let id = registration.id();
    drop(registration);
    let row = ledger.row(id).unwrap();
    let before = selected.resources().counts();
    let ClosePending::Retire(retirement) = ledger.prepare_pending_close(&row).unwrap() else {
        panic!("actual local retirement")
    };
    assert!(matches!(
        ledger.status(id).unwrap().pending,
        PendingStatus::Closing
    ));
    assert_eq!(
        selected.resources().counts(),
        before,
        "original preparation remains owned at the actual retirement boundary"
    );
    assert!(matches!(
        ledger.begin(selected.clone(), Purpose::Derive(selector)),
        Err(VoiceError::Capacity)
    ));
    assert!(matches!(
        ledger.close_pending(&row),
        Err(VoiceError::Capacity)
    ));
    assert!(!ledger.channel_may_close(&selected));
    assert!(ledger.index.try_lock().is_ok());
    assert!(row.state.try_lock().is_ok());
    drop(retirement);
    let status = ledger.status(id).unwrap();
    assert!(matches!(
        status.pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    assert!(selected.resources().counts().0 < before.0);
    assert!(!status.engine.settled && !status.drained && !status.acknowledged);
    let Begin::Existing(retry) = ledger
        .begin(selected.clone(), Purpose::Derive(selector))
        .unwrap()
    else {
        panic!("closed original")
    };
    assert!(retry == id);
}

#[test]
fn authenticated_header_capacity_is_original_owner_only_and_never_full_purpose_authority() {
    let (coordinator, selected, selector) = selection();
    let (_foreign_coordinator, foreign, _) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration.attach(action, pair, 16 * 1024).unwrap();
    let endpoint = FixtureChannel::new(selected.resources(), selected.binding().scope.group);
    let wire = endpoint.wire(
        id.logical(),
        1,
        Operation::Result,
        BodyKind::Data,
        b"wrong encrypted purpose",
    );
    let counts = selected.resources().counts();
    let authenticated = endpoint.authenticate(&wire).unwrap();
    assert_eq!(
        endpoint.session.fixture_counts(),
        (true, 0, 0),
        "MAC proof alone spends no physical counters"
    );
    assert!(matches!(
        ledger.intake_phase(&foreign, &authenticated),
        Err(VoiceError::Unauthorized)
    ));
    let phase = ledger.intake_phase(&selected, &authenticated).unwrap();
    let status = ledger.status(id).unwrap();
    let mut incoming = authenticated.admit(Some(&phase)).unwrap();
    assert_eq!(incoming.logical(), id.logical());
    assert_eq!(incoming.open().unwrap(), b"wrong encrypted purpose");
    let mut changed = selector;
    changed.join[0] ^= 1;
    assert!(matches!(
        ledger.resolve(&selected, Purpose::Derive(changed), id),
        Err(VoiceError::Unauthorized)
    ));
    assert!(
        ledger.status(id).unwrap() == status,
        "bounded intake grants no Action/work/history mutation"
    );
    drop(incoming);
    assert_eq!(endpoint.session.fixture_counts(), (false, 1, 1));
    assert_eq!(selected.resources().counts(), counts);
}

#[test]
fn wrong_mac_cannot_create_capacity_proof_and_stale_header_serial_cannot_adopt_a_row() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration.attach(action, pair, 16 * 1024).unwrap();
    let endpoint = FixtureChannel::new(selected.resources(), selected.binding().scope.group);
    let counts = selected.resources().counts();
    let mut wire = endpoint.wire(id.logical(), 1, Operation::Result, BodyKind::Data, b"x");
    let marker = b"x-voice-admission: ";
    let mac = wire
        .windows(marker.len())
        .position(|part| part == marker)
        .unwrap()
        + marker.len()
        + 143;
    wire[mac] = if wire[mac] == b'0' { b'1' } else { b'0' };
    assert!(matches!(
        endpoint.authenticate(&wire),
        Err(crate::voice_handoff::channel::Refused::Authentication)
    ));
    assert_eq!(endpoint.session.fixture_counts(), (false, 0, 0));
    let wrong = RowId {
        slot: id.slot,
        serial: id.serial + 1,
    };
    let wire = endpoint.wire(wrong.logical(), 1, Operation::Result, BodyKind::Data, b"x");
    let authenticated = endpoint.authenticate(&wire).unwrap();
    assert!(matches!(
        ledger.intake_phase(&selected, &authenticated),
        Err(VoiceError::Unauthorized)
    ));
    drop(authenticated);
    assert_eq!(endpoint.session.fixture_counts(), (false, 0, 0));
    assert_eq!(selected.resources().counts(), counts);
}

#[test]
fn retired_data_phase_refuses_before_counter_debit_while_exact_control_history_stays_available_after_end(
) {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let endpoint = FixtureChannel::new(selected.resources(), selected.binding().scope.group);
    let phase = ledger.work(id).unwrap().phase().clone();
    action.settle_engine();
    coordinator.end(selected.id());
    phase.close();
    assert!(phase.is_drained());
    let wire = endpoint.wire(
        id.logical(),
        1,
        Operation::Result,
        BodyKind::Data,
        b"old data",
    );
    let authenticated = endpoint.authenticate(&wire).unwrap();
    let original = ledger.intake_phase(&selected, &authenticated).unwrap();
    assert!(matches!(
        authenticated.admit(Some(&original)),
        Err(crate::voice_handoff::channel::Refused::Capacity)
    ));
    assert_eq!(endpoint.session.fixture_counts(), (false, 0, 0));
    let wire = endpoint.wire(
        id.logical(),
        1,
        Operation::Settle,
        BodyKind::Control,
        b"original purpose",
    );
    let authenticated = endpoint.authenticate(&wire).unwrap();
    assert!(matches!(
        ledger.intake_phase(&selected, &authenticated),
        Err(VoiceError::Unauthorized)
    ));
    let mut control = authenticated.admit(None).unwrap();
    assert_eq!(control.open().unwrap(), b"original purpose");
    let historical = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap()
        .reconcile()
        .unwrap();
    assert!(historical.engine.settled);
    assert!(
        !historical.drained,
        "closed data capacity alone is not full row/engine collection"
    );
}

#[test]
fn native_row_logical_metadata_is_canonical_and_never_accepts_zero_serial_or_out_of_range_slot() {
    for slot in 0..ROWS {
        for serial in [1, u64::MAX] {
            let id = RowId { slot, serial };
            assert!(RowId::from_logical(id.logical()).unwrap() == id);
        }
    }
    for id in [
        RowId { slot: 0, serial: 0 },
        RowId {
            slot: ROWS,
            serial: 1,
        },
        RowId {
            slot: usize::MAX,
            serial: u64::MAX,
        },
    ] {
        assert!(matches!(
            RowId::from_logical(id.logical()),
            Err(VoiceError::Unauthorized)
        ));
    }
}

#[test]
fn observer_retirement_wakes_the_sole_cleanup_after_phase_drains_without_a_new_data_loan() {
    use std::future::Future;
    use std::sync::atomic::AtomicUsize;
    use std::task::{Context, Wake, Waker};
    struct Wakes(AtomicUsize);
    impl Wake for Wakes {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let work = ledger.work(id).unwrap();
    action.settle_engine();
    let phase = work.phase().clone();
    phase.close();
    let observation = Ledger::observe(&ledger.row(id).unwrap(), true)
        .unwrap()
        .unwrap();
    let wakes = Arc::new(Wakes(AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut context = Context::from_waker(&waker);
    let mut waiter = Box::pin(ledger.wait_compacted(id));
    assert!(waiter.as_mut().poll(&mut context).is_pending());
    let mut duplicate = Box::pin(ledger.wait_compacted(id));
    assert!(matches!(
        duplicate.as_mut().poll(&mut context),
        std::task::Poll::Ready(Err(VoiceError::Capacity))
    ));
    drop(work);
    assert!(phase.is_drained());
    assert!(waiter.as_mut().poll(&mut context).is_pending());
    let before = wakes.0.load(Ordering::SeqCst);
    assert!(waiter.as_mut().poll(&mut context).is_pending());
    assert_eq!(wakes.0.load(Ordering::SeqCst), before);
    drop(observation);
    assert!(wakes.0.load(Ordering::SeqCst) > before);
    assert!(
        matches!(waiter.as_mut().poll(&mut context), std::task::Poll::Ready(Ok(status)) if status.drained)
    );
}

#[test]
fn cancelled_cleanup_releases_only_its_waiter_position_and_reuses_original_history() {
    use std::future::Future;
    use std::task::Context;
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let work = ledger.work(id).unwrap();
    action.settle_engine();
    let phase = work.phase().clone();
    phase.close();
    let waker = noop_waker();
    let mut context = Context::from_waker(&waker);
    let mut first = Box::pin(ledger.wait_compacted(id));
    assert!(first.as_mut().poll(&mut context).is_pending());
    let counts = selected.resources().counts();
    drop(first);
    assert_eq!(selected.resources().counts(), counts);
    assert!(!ledger.status(id).unwrap().drained);
    let mut replacement = Box::pin(ledger.wait_compacted(id));
    assert!(replacement.as_mut().poll(&mut context).is_pending());
    drop(work);
    assert!(
        matches!(replacement.as_mut().poll(&mut context), std::task::Poll::Ready(Ok(status)) if status.drained)
    );
}

#[test]
fn cleanup_waits_for_late_exact_ack_without_waking_itself_on_unchanged_history() {
    use std::future::Future;
    use std::task::Context;
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (mut join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let work = ledger.work(id).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = crate::voice_authority::tests::derivation_receipt(
        &coordinator,
        selected.id(),
        &mut join,
        &action,
    );
    ledger.settle(id).unwrap();
    let phase = work.phase().clone();
    phase.close();
    drop(work);
    let waker = noop_waker();
    let mut context = Context::from_waker(&waker);
    let mut waiter = Box::pin(ledger.wait_compacted(id));
    assert!(waiter.as_mut().poll(&mut context).is_pending());
    assert!(waiter.as_mut().poll(&mut context).is_pending());
    assert!(!ledger.status(id).unwrap().drained);
    ledger.acknowledge(id, &receipt).unwrap();
    assert!(
        matches!(waiter.as_mut().poll(&mut context), std::task::Poll::Ready(Ok(status)) if status.drained)
    );
}

#[test]
fn uncertain_factory_reply_reuses_paid_purpose_identity_and_does_not_rearm_another_worker() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let first = ticket(&ledger, selected.clone(), selector);
    let id = first.id();
    first.mark_handoff().unwrap();
    let counts = selected.resources().counts();
    assert!(
        matches!(ledger.begin(selected.clone(), Purpose::Derive(selector)).unwrap(), Begin::Existing(existing) if existing == id)
    );
    // A native worker lost after actor handoff leaves the original row, even without input.
    drop(first);
    let retry = ticket(&ledger, selected.clone(), selector);
    assert!(retry.id() == id);
    assert_eq!(selected.resources().counts(), counts);
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    assert!(retry.attach(action, pair, 16 * 1024).unwrap() == id);
    assert!(
        matches!(ledger.begin(selected, Purpose::Derive(selector)).unwrap(), Begin::Existing(existing) if existing == id)
    );
}

#[test]
fn dropping_before_handoff_removes_only_its_original_slot_and_credit() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let baseline = selected.resources().counts();
    let first = ticket(&ledger, selected.clone(), selector);
    let old = first.id();
    drop(first);
    assert!(ledger.status(old).is_err());
    assert_eq!(selected.resources().counts(), baseline);
    let next = ticket(&ledger, selected, selector);
    assert_eq!(next.id().slot, old.slot);
    assert_ne!(next.id().serial, old.serial);
    assert!(ledger.status(old).is_err());
}

#[test]
fn lost_terminal_ack_and_settlement_replies_recover_fixed_history_after_all_live_owners_drop() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    registration.mark_handoff().unwrap();
    let (mut join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let work = ledger.work(id).unwrap();
    assert_eq!(
        ledger.work(id).err().map(|_| VoiceError::Capacity),
        Some(VoiceError::Capacity)
    );
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = crate::voice_authority::tests::derivation_receipt(
        &coordinator,
        selected.id(),
        &mut join,
        &action,
    );
    work.refresh().unwrap();
    work.acknowledge(&receipt).unwrap();
    assert!(ledger.status(id).unwrap().acknowledged);
    action.settle_engine();
    let settled = work.refresh().unwrap();
    assert!(settled.engine.settled && !settled.drained);
    let phase = work.phase().clone();
    let buffer = phase.buffer(23).unwrap();
    let input_owner = phase.copy(0).unwrap();
    // This call models the native gate AFTER selected-browser phase/RPC/engine drainage;
    // these tests do not implement or prove that browser-side acknowledgement protocol.
    phase.close();
    assert!(!phase.is_drained());
    assert!(!ledger.compact(id).unwrap());
    // A completed but uncollected native result still holds its zero-byte phase contribution.
    drop(buffer);
    drop(input_owner);
    assert!(!phase.is_drained());
    drop(work);
    assert!(phase.is_drained());
    assert!(ledger.compact(id).unwrap());
    let tombstone = ledger.status(id).unwrap();
    assert!(tombstone.drained && tombstone.engine.settled && tombstone.acknowledged);
    assert!(tombstone
        .engine
        .receipt
        .as_ref()
        .unwrap()
        .same_transition(&receipt));
    assert!(tombstone.engine.receipt.as_ref().unwrap().retired);
    assert!(ledger.work(id).is_err());
    let action_weak = Arc::downgrade(&action);
    let selected_weak = Arc::downgrade(&selected);
    let phase_weak = Arc::downgrade(&phase);
    coordinator.end(selected.id());
    drop((coordinator, selected, join, action, phase));
    assert!(action_weak.upgrade().is_none());
    assert!(selected_weak.upgrade().is_none());
    assert!(phase_weak.upgrade().is_none());
    for _ in 0..5 {
        assert!(ledger.status(id).unwrap() == tombstone);
    }
    assert!(ledger
        .status(RowId {
            slot: id.slot,
            serial: id.serial + 1
        })
        .is_err());
}

#[test]
fn equal_field_foreign_owner_wrong_pair_or_selector_cannot_attach() {
    let (coordinator, selected, selector) = selection();
    let (foreign_coordinator, foreign, _foreign_selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    registration.mark_handoff().unwrap();
    let (_foreign_join, foreign_action, foreign_pair) =
        actual_action(&foreign_coordinator, &foreign);
    assert_eq!(
        registration
            .attach(foreign_action, foreign_pair, 16 * 1024)
            .err(),
        Some(VoiceError::Unauthorized)
    );
    let retry = ticket(&ledger, selected.clone(), selector);
    let (_join, action, mut pair) = actual_action(&coordinator, &selected);
    pair.handshake[0] ^= 1;
    assert_eq!(
        retry.attach(action, pair, 16 * 1024).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(matches!(
        ledger.begin(selected, Purpose::Derive(selector)).unwrap(),
        Begin::Factory(_)
    ));
}

#[test]
fn settlement_and_no_terminal_result_still_require_real_native_worker_collection() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let work = ledger.work(id).unwrap();
    action.settle_engine();
    work.refresh().unwrap();
    let phase = work.phase().clone();
    phase.close();
    assert!(!ledger.compact(id).unwrap());
    drop(work);
    assert!(ledger.compact(id).unwrap());
    assert!(ledger.status(id).unwrap().engine.receipt.is_none());
    assert!(action.admit_engine(action.engine_bytes()).is_err());
}

#[test]
fn phase_waiter_can_compact_at_the_exact_final_worker_retirement_notification() {
    use std::future::Future;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::task::{Context, Wake, Waker};
    struct Cleanup {
        ledger: Arc<Ledger>,
        id: RowId,
        result: AtomicU8,
    }
    impl Wake for Cleanup {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.result.store(
                if self.ledger.compact(self.id).unwrap() {
                    1
                } else {
                    2
                },
                Ordering::SeqCst,
            );
        }
    }
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let work = ledger.work(id).unwrap();
    action.settle_engine();
    work.refresh().unwrap();
    let phase = work.phase().clone();
    phase.close();
    let cleanup = Arc::new(Cleanup {
        ledger: ledger.clone(),
        id,
        result: AtomicU8::new(0),
    });
    let waker = Waker::from(cleanup.clone());
    let mut context = Context::from_waker(&waker);
    let mut waiter = Box::pin(phase.wait_drained());
    assert!(waiter.as_mut().poll(&mut context).is_pending());
    // Wake runs synchronously inside the final contribution's Drop, before Work::drop returns.
    // It must see a compactable row immediately; there is no later notification to rely on.
    drop(work);
    assert_eq!(cleanup.result.load(Ordering::SeqCst), 1);
    assert!(ledger.status(id).unwrap().drained);
}

#[test]
fn settled_close_and_worker_loss_without_refresh_recovers_without_a_new_data_loan() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let work = ledger.work(id).unwrap();
    action.settle_engine();
    let phase = work.phase().clone();
    phase.close();
    drop(work); // deliberately no refresh after actual native settlement
    assert!(phase.is_drained());
    assert!(!ledger.status(id).unwrap().engine.settled);
    assert!(ledger.work(id).is_err());
    assert!(ledger.compact(id).unwrap());
    assert!(ledger.status(id).unwrap().engine.settled);
    assert!(ledger.settle(id).unwrap().drained);
}

#[test]
fn lost_native_ack_latch_recovers_actual_root_ack_without_a_new_data_loan() {
    let (coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (mut join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let work = ledger.work(id).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = crate::voice_authority::tests::derivation_receipt(
        &coordinator,
        selected.id(),
        &mut join,
        &action,
    );
    assert!(!ledger.status(id).unwrap().acknowledged);
    action.acknowledge_result(&receipt).unwrap(); // crash between actual ACK and native latch
    assert!(action.handoff_status().acknowledged);
    assert!(
        !ledger.status(id).unwrap().acknowledged,
        "native cache is still stale"
    );
    assert!(ledger.reconcile(id).unwrap().acknowledged);
    let phase = work.phase().clone();
    ledger.settle(id).unwrap();
    phase.close();
    drop(work);
    assert!(phase.is_drained());
    // An exact actual Root ACK is sufficient even when the earlier native ACK reply/latch
    // was lost. No renderer assertion, receipt presence or settlement inferred that fact.
    assert!(ledger.compact(id).unwrap());
    let mut wrong = receipt.clone();
    wrong.output_tag[0] ^= 1;
    assert_eq!(
        ledger.acknowledge(id, &wrong),
        Err(VoiceError::Unauthorized)
    );
    ledger.acknowledge(id, &receipt).unwrap();
    assert!(ledger.compact(id).unwrap());
    ledger.acknowledge(id, &receipt).unwrap();
    assert!(ledger.acknowledge(id, &wrong).is_err());
}

#[test]
fn reordered_metadata_observations_cannot_erase_rejected_history_or_settlement() {
    use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;
    let receipt = CryptoResultReceipt {
        action: 3,
        input_tag: [1; 32],
        output_tag: [2; 32],
        outcome: CryptoOutcome::Rejected(VoiceError::Unauthorized),
        retired: true,
    };
    let mut status = Status {
        pending: PendingStatus::NotPending,
        action: Some((3, 1, [1; 32])),
        engine: CryptoResultStatus {
            settled: true,
            receipt: Some(receipt.clone()),
        },
        acknowledged: true,
        drained: false,
    };
    merge_observed(
        &mut status,
        CryptoHandoffStatus {
            engine: CryptoResultStatus {
                settled: false,
                receipt: None,
            },
            acknowledged: false,
        },
    )
    .unwrap();
    let mut older = receipt.clone();
    older.retired = false;
    merge_observed(
        &mut status,
        CryptoHandoffStatus {
            engine: CryptoResultStatus {
                settled: false,
                receipt: Some(older),
            },
            acknowledged: false,
        },
    )
    .unwrap();
    assert!(status.engine.settled);
    assert!(status.acknowledged);
    assert_eq!(status.engine.receipt, Some(receipt.clone()));
    let mut changed = receipt;
    changed.outcome = CryptoOutcome::Derived;
    assert_eq!(
        merge_observed(
            &mut status,
            CryptoHandoffStatus {
                engine: CryptoResultStatus {
                    settled: true,
                    receipt: Some(changed),
                },
                acknowledged: true,
            }
        ),
        Err(VoiceError::RetryMismatch)
    );
}

#[test]
fn pending_history_shares_the_finite_row_bound_without_eviction_or_ttl() {
    let (_coordinator, selected, mut selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let mut ids = Vec::new();
    for n in 0..ROWS {
        selector.join[0] = n as u8;
        let registration = ticket(&ledger, selected.clone(), selector);
        ids.push(registration.id());
        registration.mark_handoff().unwrap();
        drop(registration);
    }
    let before = selected.resources().counts();
    selector.join[0] = 99;
    assert_eq!(
        ledger
            .begin(selected.clone(), Purpose::Derive(selector))
            .err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(selected.resources().counts(), before);
    for id in ids {
        assert!(!ledger.status(id).unwrap().drained);
    }
}

#[test]
fn resolved_row_rejects_equal_field_foreign_native_owner_and_each_wrong_selector_field() {
    let (coordinator, selected, selector) = selection();
    let (_foreign_coordinator, foreign, _) = selection();
    // Equal public projections do not identify the actual affine grant/lease allocation.
    assert_eq!(selected.id(), foreign.id());
    assert_eq!(selected.executor(), foreign.executor());
    assert_eq!(selected.join(), foreign.join());
    assert_eq!(selected.point(), foreign.point());
    assert_eq!(selected.binding().scope, foreign.binding().scope);
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration.attach(action, pair, 16 * 1024).unwrap();
    let before = selected.resources().counts();
    assert!(matches!(
        ledger.resolve(&foreign, Purpose::Derive(selector), id),
        Err(VoiceError::Unauthorized)
    ));
    let mut wrong_peer = selector;
    wrong_peer.peer = DeviceKeypair::from_seed(&[5; 32]).device_id();
    let mut wrong_join = selector;
    wrong_join.join[0] ^= 1;
    let mut wrong_key = selector;
    wrong_key.key_ref[0] ^= 1;
    let mut wrong_epoch = selector;
    wrong_epoch.epoch += 1;
    for wrong in [wrong_peer, wrong_join, wrong_key, wrong_epoch] {
        assert!(matches!(
            ledger.resolve(&selected, Purpose::Derive(wrong), id),
            Err(VoiceError::Unauthorized)
        ));
    }
    for wrong in [
        RowId {
            slot: id.slot,
            serial: id.serial + 1,
        },
        RowId {
            slot: ROWS,
            serial: id.serial,
        },
        RowId {
            slot: usize::MAX,
            serial: id.serial,
        },
    ] {
        assert!(matches!(
            ledger.resolve(&selected, Purpose::Derive(selector), wrong),
            Err(VoiceError::Unauthorized)
        ));
    }
    assert_eq!(
        selected.resources().counts(),
        before,
        "refusal creates no credit/work"
    );
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    assert!(row.status().unwrap().action.is_some());
    assert!(
        row.work().is_ok(),
        "exact native provenance still reaches the original work"
    );
    assert_eq!(selected.resources().counts(), before);
}

#[test]
fn every_original_key_component_is_required_even_when_other_identity_components_match() {
    let (_coordinator, selected, selector) = selection();
    let (_foreign_coordinator, foreign, _) = selection();
    let purpose = Purpose::Derive(selector);
    let mut key = Key {
        grant: selected.grant_identity(),
        executor: selected.executor(),
        lease: selected.lease().clone(),
        purpose,
    };
    assert!(key.matches(&selected, purpose));
    key.grant = foreign.grant_identity();
    assert!(!key.matches(&selected, purpose));
    key.grant = selected.grant_identity();
    key.executor[0] ^= 1;
    assert!(!key.matches(&selected, purpose));
    key.executor = selected.executor();
    key.lease = CallLease::default();
    assert!(!key.matches(&selected, purpose));
    key.lease = selected.lease().clone();
    let mut changed = selector;
    changed.join[0] ^= 1;
    key.purpose = Purpose::Derive(changed);
    assert!(!key.matches(&selected, purpose));
}

#[test]
fn lost_factory_reply_lookup_is_history_only_and_authorized_retry_keeps_the_same_row() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let purpose = Purpose::Derive(selector);
    assert!(ledger.lookup_original(&selected, purpose).is_none());
    let registration = ticket(&ledger, selected.clone(), selector);
    let id = registration.id(); // unavailable to the modeled lost-reply caller
    registration.mark_handoff().unwrap();
    drop(registration); // actual failed native factory has retired; its purpose is uncertain
    let original = ledger.row(id).unwrap();
    let before = selected.resources().counts();
    {
        let state = original.state.lock().unwrap();
        let Body::Pending(pending) = &state.body else {
            panic!("uncertain Pending");
        };
        assert!(!pending.worker && pending.handed_off);
        // Holding State pins the pure lookup boundary: it must not inspect/re-arm a worker,
        // reconcile the Action, or wait for any callback-bearing authority observation.
        let recovered = ledger.lookup_original(&selected, purpose).unwrap();
        assert!(recovered.id() == id && Arc::ptr_eq(&recovered.row, &original));
    }
    assert_eq!(selected.resources().counts(), before);
    assert!(ledger
        .lookup_original(&selected, purpose)
        .unwrap()
        .status()
        .unwrap()
        .action
        .is_none());
    // A caller separately admitted by the factory's current UI/selection/Root fences can
    // still resume this same uncertain purpose; history lookup never spends that worker.
    let retry = ticket(&ledger, selected.clone(), selector);
    assert!(retry.id() == id);
    assert!(matches!(
        ledger.begin(selected.clone(), purpose).unwrap(),
        Begin::Existing(existing) if existing == id
    ));
    assert!(ledger.lookup_original(&selected, purpose).unwrap().id() == id);
    drop(retry);
    coordinator.end(selected.id());
    drop(coordinator);
    assert!(selected.grant_identity().upgrade().is_none());
    assert!(ledger.lookup_original(&selected, purpose).unwrap().id() == id);
    assert!(matches!(
        ledger.begin(selected.clone(), purpose),
        Err(VoiceError::Unauthorized)
    ));
}

#[test]
fn original_purpose_lookup_under_full_pressure_rejects_foreign_identity_and_every_selector_field() {
    let (_coordinator, selected, selector) = selection();
    let (_foreign_coordinator, foreign, _) = selection();
    assert_eq!(selected.id(), foreign.id());
    assert_eq!(selected.executor(), foreign.executor());
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let id = registration.id();
    registration.mark_handoff().unwrap();
    drop(registration);
    let pool = selected.resources();
    let _pressure = pool
        .charge(
            selected.binding().scope.group,
            catcoms_sync::voice_v2::resources::GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    let mut peer = selector;
    peer.peer = DeviceKeypair::from_seed(&[5; 32]).device_id();
    let mut join = selector;
    join.join[0] ^= 1;
    let mut key = selector;
    key.key_ref[0] ^= 1;
    let mut epoch = selector;
    epoch.epoch += 1;
    for _ in 0..4 {
        assert!(ledger
            .lookup_original(&foreign, Purpose::Derive(selector))
            .is_none());
        for wrong in [peer, join, key, epoch] {
            assert!(ledger
                .lookup_original(&selected, Purpose::Derive(wrong))
                .is_none());
        }
        let recovered = ledger
            .lookup_original(&selected, Purpose::Derive(selector))
            .unwrap();
        assert!(recovered.id() == id);
        assert!(!recovered.status().unwrap().drained);
    }
    assert_eq!(
        pool.counts(),
        before,
        "historical lookup takes no fresh credit/work"
    );
}

#[test]
fn missing_original_purpose_is_not_drained_and_a_removed_unstarted_row_cannot_alias_a_new_serial() {
    let (_coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let purpose = Purpose::Derive(selector);
    let registration = ticket(&ledger, selected.clone(), selector);
    let old = registration.id();
    let retained = ledger.lookup_original(&selected, purpose).unwrap();
    drop(registration); // proven no actor handoff: existing removal behavior is unchanged
    assert!(ledger.lookup_original(&selected, purpose).is_none());
    assert!(matches!(retained.status(), Err(VoiceError::Unauthorized)));
    let replacement = ticket(&ledger, selected.clone(), selector);
    let current = ledger.lookup_original(&selected, purpose).unwrap();
    assert!(current.id() == replacement.id());
    assert!(current.id().slot == old.slot && current.id().serial > old.serial);
    assert!(matches!(
        ledger.resolve(&selected, purpose, old),
        Err(VoiceError::Unauthorized)
    ));
    assert!(!current.status().unwrap().drained);
}

#[test]
fn resolved_row_preserves_exact_terminal_cleanup_after_end_and_actual_grant_drop() {
    use std::future::Future;
    use std::task::{Context, Poll};
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (mut join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration
        .attach(action.clone(), pair, 16 * 1024)
        .unwrap();
    let row = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let work = row.work().unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = crate::voice_authority::tests::derivation_receipt(
        &coordinator,
        selected.id(),
        &mut join,
        &action,
    );
    work.refresh().unwrap();
    coordinator.end(selected.id());
    drop(coordinator);
    assert!(!selected.lease().is_live());
    assert!(
        selected.grant_identity().upgrade().is_none(),
        "no live affine grant is needed for exact cleanup"
    );
    let recovered = ledger
        .lookup_original(&selected, Purpose::Derive(selector))
        .unwrap();
    assert!(
        recovered.id() == id,
        "lost Begin reply requires no caller-known RowId"
    );
    assert!(matches!(recovered.work(), Err(VoiceError::Unauthorized)));
    assert!(
        matches!(row.work(), Err(VoiceError::Unauthorized)),
        "a pre-End handle gains no data authority"
    );
    assert!(recovered.reconcile().unwrap().engine.receipt.is_some());
    let mut wrong = receipt.clone();
    wrong.output_tag[0] ^= 1;
    assert!(recovered.acknowledge(&wrong).is_err());
    recovered.acknowledge(&receipt).unwrap();
    recovered.acknowledge(&receipt).unwrap(); // lost ACK reply recovers the same transition
    assert!(recovered.settle().unwrap().engine.settled);
    let phase = work.phase().clone();
    phase.close(); // models the separate, still-unimplemented authenticated browser closure
    assert!(!phase.is_drained());
    let waker = noop_waker();
    let mut context = Context::from_waker(&waker);
    let mut waiter = Box::pin(recovered.wait_compacted());
    assert!(waiter.as_mut().poll(&mut context).is_pending());
    drop(work);
    assert!(
        matches!(waiter.as_mut().poll(&mut context), Poll::Ready(Ok(status)) if status.drained)
    );
    drop(waiter);
    assert!(row.status().unwrap().drained);
    assert!(row.reconcile().unwrap().drained);
    assert!(row.settle().unwrap().drained);
    row.acknowledge(&receipt).unwrap();
    assert!(row.acknowledge(&wrong).is_err());
    assert!(row.work().is_err());
    let compacted = ledger
        .lookup_original(&selected, Purpose::Derive(selector))
        .unwrap();
    assert!(compacted.id() == id);
    assert!(compacted.reconcile().unwrap() == row.reconcile().unwrap());
    compacted.acknowledge(&receipt).unwrap();
}

#[test]
fn end_before_blocked_state_admission_refuses_work_without_copy_or_worker_mutation() {
    let (mut coordinator, selected, selector) = selection();
    let ledger = Arc::new(Ledger::default());
    let registration = ticket(&ledger, selected.clone(), selector);
    let (_join, action, pair) = actual_action(&coordinator, &selected);
    let id = registration.attach(action, pair, 16 * 1024).unwrap();
    let resolved = ledger
        .resolve(&selected, Purpose::Derive(selector), id)
        .unwrap();
    let row = ledger.row(id).unwrap();
    let held = row.state.lock().unwrap();
    let Body::Live(live) = &held.body else {
        panic!("live registered work")
    };
    let phase = live.phase.clone();
    assert!(!live.worker);
    let status = held.status.clone();
    // The actual State guard prevents final admission until AFTER revocation. The wrapper
    // has no separate early lease check, so success cannot be hidden by testing only End
    // before an outer check. Both possible thread schedules reach the same held-state fence.
    let entered = std::sync::Barrier::new(2);
    let baseline = std::thread::scope(|scope| {
        let pending = scope.spawn(|| {
            entered.wait();
            resolved.work()
        });
        entered.wait();
        coordinator.end(selected.id());
        assert!(!selected.lease().is_live());
        // End legitimately prunes its settled generation metadata. Measure only the
        // rejected work after that independent retirement, while admission is still held.
        let baseline = selected.resources().counts();
        drop(held);
        assert!(matches!(
            pending.join().unwrap(),
            Err(VoiceError::Unauthorized)
        ));
        baseline
    });
    let held = row.state.lock().unwrap();
    let Body::Live(live) = &held.body else {
        panic!("same live record")
    };
    assert!(!live.worker);
    assert!(
        held.status == status,
        "refusal does not alter receipt or history"
    );
    assert_eq!(selected.resources().counts(), baseline);
    drop(held);
    phase.close();
    assert!(
        phase.is_drained(),
        "no copy was admitted behind the blocked State guard"
    );
}
