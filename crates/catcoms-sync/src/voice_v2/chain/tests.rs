use super::*;
use crate::voice_v2::crypto_actions::{
    ActionQueue, Allocation, CryptoOutcome, ResultBinding, TerminalResult,
};
use crate::voice_v2::crypto_authority::CryptoAuthority;
use crate::voice_v2::presence::PairSetup;
use catcoms_crypto::DeviceKeypair;
use catcoms_rt::ManualClock;
mod handoff;
mod progress;

#[test]
fn workflow_registry_clears_exact_last_owner_without_stranding_its_weak_allocation() {
    let resources = Arc::new(Resources::default());
    let first = prepare_chain(
        resources.clone(),
        binding(1),
        Arc::new(slots::Slots::default()),
    )
    .unwrap();
    let group = first.binding.scope.group;
    let independent = resources.charge(group, 17, ResourceKind::Retained).unwrap();
    let workflow = first.workflow().unwrap();
    let original = Arc::as_ptr(&workflow);
    // A test-only address guard prevents allocator reuse while checking delayed cleanup.
    // Production must clear its own weak index before its last metadata payment returns.
    let address_guard = Arc::downgrade(&workflow);
    assert_eq!(
        resources.retained_phase_indices_for_test(group),
        (true, false)
    );
    first.close();
    drop((workflow, first));
    assert_eq!(
        resources.retained_phase_indices_for_test(group),
        (false, false)
    );
    assert_eq!(resources.counts(), (17, 0));
    let next = prepare_chain(
        resources.clone(),
        binding(2),
        Arc::new(slots::Slots::default()),
    )
    .unwrap();
    let current = next.workflow().unwrap();
    assert_ne!(original, Arc::as_ptr(&current));
    resources.unregister_workflow(group, original);
    assert!(Arc::ptr_eq(&resources.workflow(group).unwrap(), &current));
    drop((current, next, independent, address_guard));
    assert_eq!(resources.counts(), (0, 0));
}

pub(super) fn binding(seed: u8) -> Binding {
    Binding {
        scope: Scope::for_group(b"prepared chain", Some([7; 32]), 44),
        epoch: 3,
        local: DeviceKeypair::from_seed(&[1; 32]).device_id(),
        local_join: [1; 32],
        local_key: [2; 32],
        handshake: [seed; 32],
        peer: DeviceKeypair::from_seed(&[2; 32]).device_id(),
        peer_join: [3; 32],
        peer_key: [4; 32],
        generation: u64::from(seed),
        call: CallLease::default(),
        pair: CallLease::default(),
        deadline: 16_000,
        low: true,
    }
}
/// These tests isolate history/action/workflow custody; real ledger sharing is covered by
/// packet and LocalJoin regressions. Each fixture still uses the production fixed-size pools.
pub(super) fn prepare_chain(
    resources: Arc<Resources>,
    bound: Binding,
    slots: Arc<slots::Slots>,
) -> Result<Arc<PreparedChain>, VoiceError> {
    PreparedChain::new(
        resources,
        bound,
        slots,
        &super::super::packets::PacketSlots::new(),
    )
}
pub(super) fn publish(
    chain: &Arc<PreparedChain>,
    pool: &Arc<Resources>,
    slots: &Arc<slots::Slots>,
) -> (ActionQueue, Arc<CryptoAction>) {
    let (queue, action) = publish_unregistered(chain, pool, slots);
    chain.register_action(&action);
    (queue, action)
}

#[test]
fn fixed_metadata_allowance_covers_retained_owner_layouts() {
    use std::mem::size_of;
    // Conservative structural accounting: include the shared ledger slot pools too, all
    // reservation/lane owners and their Arc counters, and the preallocated weak-action list.
    // Actual action/input/result rows have separate credits. This is a growth guard for our
    // retained Rust owners, not an allocator-slack or framework/engine heap measurement.
    // The two outgoing WorkLanes and their Arc headers now have an independent shared
    // group payment. Workflow still counts its actual handle and all enlarged Credits.
    const ARC_OWNERS: usize = 4 + 2 + 1 + 3 + 3;
    let bytes = size_of::<PlanBinding>()
        + size_of::<HistoryPlan>()
        + size_of::<PreparedChain>()
        + size_of::<Workflow>()
        + 2 * size_of::<Reservation>()
        + size_of::<WorkLane>()
        + 3 * size_of::<slots::Slots>()
        + 3 * size_of::<slots::ReservedSlots>()
        + crate::voice_v2::runtime::EpochObservation::retained_graph_bytes()
        + ARC_OWNERS * 2 * size_of::<usize>();
    assert!(
        bytes <= METADATA_BYTES,
        "retained native layouts need {bytes} bytes, allowance is {METADATA_BYTES}"
    );
    // The witness occupies the caller's already-funded state, not another Root heap
    // allocation. Keep its growth guard separate from the UNCHANGED retained2KiB contract.
    // A future native consumer must also measure its surrounding task/output/restore owner.
    let observer = size_of::<ConfirmationRetirement>();
    eprintln!(
        "retained workflow graph {bytes}/{METADATA_BYTES}; caller observer {observer}/{}",
        ConfirmationRetirement::OBSERVER_MEMORY_BYTES
    );
    assert!(observer <= ConfirmationRetirement::OBSERVER_MEMORY_BYTES);
}

#[test]
fn four_closed_workflows_release_idle_process_slots_but_an_actual_late_lower_keeps_custody() {
    let pool = Arc::new(Resources::default());
    let mut phases = Vec::new();
    for group in 1..=4 {
        let mut bound = binding(group);
        bound.scope.group = [group; 32];
        let chain = prepare_chain(pool.clone(), bound, Arc::new(slots::Slots::default())).unwrap();
        let workflow = chain
            .control
            .lock()
            .unwrap()
            .workflow
            .as_ref()
            .unwrap()
            .clone();
        phases.push((chain, workflow));
    }
    assert_eq!(
        pool.charge([5; 32], 1, ResourceKind::Forward).err(),
        Some(VoiceError::Capacity)
    );
    let late = Arc::new(
        phases[0]
            .1
            .charge(&pool, [1; 32], 1, ResourceKind::Forward)
            .unwrap(),
    );
    for (chain, _) in &phases {
        chain.close();
    }
    // Keep every closed phase alive, as a late input/query handle would. Only the real
    // Lower retains the shared two-position owner; closed idle handles release theirs.
    let mut work: Vec<_> = (0..4)
        .map(|_| pool.charge([5; 32], 1, ResourceKind::Forward).unwrap())
        .collect();
    for _ in 0..2 {
        work.push(pool.charge([6; 32], 1, ResourceKind::Forward).unwrap());
    }
    assert_eq!(
        pool.charge([6; 32], 1, ResourceKind::Forward).err(),
        Some(VoiceError::Capacity)
    );
    let last = late.clone();
    drop(late);
    assert!(pool.charge([6; 32], 1, ResourceKind::Forward).is_err());
    drop(last); // actual transport/output custody ends, not a timeout or synthetic refund
    for _ in 0..2 {
        work.push(pool.charge([6; 32], 1, ResourceKind::Forward).unwrap());
    }
    drop((work, phases));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn real_confirmation_and_sdp_share_capacity_under_full_ordinary_pressure_without_cross_close() {
    use crate::voice_v2::negotiation::{Exchange, Stage};
    let pool = Arc::new(Resources::default());
    let bound = binding(1);
    let group = bound.scope.group;
    let chain = prepare_chain(pool.clone(), bound, Arc::new(slots::Slots::default())).unwrap();
    let workflow = chain
        .control
        .lock()
        .unwrap()
        .workflow
        .as_ref()
        .unwrap()
        .clone();
    let exchange =
        Exchange::new_for_test(pool.clone(), group, Arc::new(slots::Slots::default())).unwrap();
    let confirmation_lower = workflow
        .charge(&pool, group, 512, ResourceKind::Forward)
        .unwrap();
    let sdp_lower = exchange.charge(Stage::Lower, 512).unwrap();
    // The actual phase pair reserves only TWO positions. Ordinary public/helper traffic can
    // use the other two without borrowing another phase's protocol or prepaid byte authority.
    let ordinary: Vec<_> = (0..2)
        .map(|_| pool.charge(group, 512, ResourceKind::Forward).unwrap())
        .collect();
    let fill = pool
        .charge(
            group,
            crate::voice_v2::resources::GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    assert_eq!(
        exchange.charge(Stage::Lower, 512).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(pool.counts(), before);
    drop(confirmation_lower); // actual old transport returns its original shared position
    let sdp_next = exchange.charge(Stage::Lower, 512).unwrap();
    chain.close();
    drop(sdp_lower);
    // Closing the original confirmation phase cannot close the surviving SDP owner's lane.
    let sdp_after_close = exchange.charge(Stage::Lower, 512).unwrap();
    assert!(workflow
        .charge(&pool, group, 1, ResourceKind::Forward)
        .is_err());
    drop((
        sdp_next,
        sdp_after_close,
        exchange,
        ordinary,
        fill,
        workflow,
        chain,
    ));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn hello_receipt_and_action_layouts_fit_the_original_single_row() {
    let receipt = super::admission::ADMISSION_RECORD_BYTES;
    let action = crate::voice_v2::crypto_actions::fixed_record_layout_bytes();
    assert!(receipt < ROW_BYTES);
    assert!(
        action <= ROW_BYTES - receipt,
        "fixed action {action} plus receipt {receipt} exceeds original {ROW_BYTES}-byte row"
    );
}

#[test]
fn low_start_receipt_and_all_three_crypto_gates_fit_the_original_single_row() {
    let receipt = super::start::START_RECORD_BYTES;
    let action = crate::voice_v2::crypto_actions::fixed_record_layout_bytes();
    assert!(receipt < ROW_BYTES);
    assert!(
        action <= ROW_BYTES - receipt,
        "fixed action {action} plus low start {receipt} exceeds original {ROW_BYTES}-byte row"
    );
}

#[test]
fn low_final_history_contention_preserves_the_original_lane_for_an_exact_fresh_retry() {
    use crate::voice_v2::runtime::{Runtime, View};
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let bound = binding(8);
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let (mut actor, registration) = runtime
        .prepare_actor(bound.scope.group, bound.local, &ticket)
        .unwrap();
    registration.register().unwrap();
    actor
        .sync_view(View {
            epoch: bound.epoch,
            members: [bound.local, bound.peer].into(),
            policy: bound.scope.policy,
            eligible: true,
            indirect: true,
        })
        .unwrap();
    let context = crate::voice_v2::Context {
        scope: bound.scope,
        local: bound.local,
        epoch: bound.epoch,
        now: 1000,
        indirect: true,
    };
    let pool = runtime.resources();
    let chain = prepare_chain(pool, bound.clone(), Arc::new(slots::Slots::default())).unwrap();
    let consent = || ConfirmationConsent {
        room: bound.call.clone(),
        _room_custody: None,
        effect: actor.effect(bound.local, bound.peer, 16_000).unwrap(),
        clock: runtime.clock(),
    };
    let (first, receipt, workflow) = chain
        .prepare_start(None, [31; 32], 1, 16_000, consent())
        .unwrap();
    let PreparedHistory {
        record,
        slot,
        mut issue,
    } = first;
    let mut published = false;
    let guard = chain.history.state.lock().unwrap();
    assert_eq!(
        actor.with_local_origin(bound.peer, &context, |wall, mono, origin| {
            issue.publish_start(&receipt, origin, wall, mono, || {
                published = true;
            })
        }),
        Err(VoiceError::Capacity)
    );
    assert!(!published);
    assert_eq!(guard.issued, 0);
    assert!(guard.candidates.low().unwrap().iter().all(Option::is_none));
    drop(guard);
    // RowIssue::drop reacquires History. It must happen after the final native guards, never
    // as destruction of a moved closure inside a refused Group/Policy/History transaction.
    drop((issue, record, slot, receipt, workflow));
    assert!(matches!(
        chain.classify_start(None).unwrap(),
        StartCandidate::New(0)
    ));
    let (retry, receipt, _workflow) = chain
        .prepare_start(None, [32; 32], 1, 16_000, consent())
        .unwrap();
    let PreparedHistory {
        record: _record,
        slot: _slot,
        mut issue,
    } = retry;
    actor
        .with_local_origin(bound.peer, &context, |wall, mono, origin| {
            issue.publish_start(&receipt, origin, wall, mono, || {
                published = true;
            })
        })
        .unwrap();
    assert!(published);
    assert!(
        matches!(chain.classify_start(None).unwrap(), StartCandidate::Cached(actual)
        if Arc::ptr_eq(&actual, &receipt))
    );
}

#[test]
fn derivation_final_history_contention_does_not_issue_row_zero_and_refused_owner_retries() {
    use crate::voice_v2::runtime::{Runtime, View};
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let bound = binding(8);
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let (mut actor, registration) = runtime
        .prepare_actor(bound.scope.group, bound.local, &ticket)
        .unwrap();
    registration.register().unwrap();
    actor
        .sync_view(View {
            epoch: bound.epoch,
            members: [bound.local, bound.peer].into(),
            policy: bound.scope.policy,
            eligible: true,
            indirect: true,
        })
        .unwrap();
    let context = crate::voice_v2::Context {
        scope: bound.scope,
        local: bound.local,
        epoch: bound.epoch,
        now: 1000,
        indirect: true,
    };
    let chain = prepare_chain(
        runtime.resources(),
        bound.clone(),
        Arc::new(slots::Slots::default()),
    )
    .unwrap();
    let (
        PreparedHistory {
            record,
            slot,
            mut issue,
        },
        workflow,
    ) = chain.prepare_derive().unwrap();
    let guard = chain.history.state.lock().unwrap();
    let mut published = false;
    assert_eq!(
        actor.with_local_origin(bound.peer, &context, |_, mono, origin| {
            issue.publish_derivation(origin, mono, || {
                published = true;
            })
        }),
        Err(VoiceError::Capacity)
    );
    assert!(!published);
    assert_eq!(guard.issued, 0);
    drop(guard);
    // The unpublished issue destructor must reacquire History AFTER Group/Policy are released.
    drop((issue, record, slot, workflow));
    let (
        PreparedHistory {
            record: _record,
            slot: _slot,
            mut issue,
        },
        _workflow,
    ) = chain.prepare_derive().unwrap();
    actor
        .with_local_origin(bound.peer, &context, |_, mono, origin| {
            issue.publish_derivation(origin, mono, || {
                published = true;
            })
        })
        .unwrap();
    assert!(published);
    assert_eq!(chain.history.state.lock().unwrap().issued, 1);
    assert_eq!(chain.prepare_derive().err(), Some(VoiceError::Capacity));
}

#[test]
fn hello_final_history_contention_spends_no_origin_or_candidate_and_exact_row_retries() {
    use crate::voice_v2::runtime::{Runtime, View};
    use crate::voice_v2::{Body, Context, EncryptedHeader, EncryptedKind, Envelope, SignedObject};
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock.clone());
    let session = runtime.session_ticket();
    runtime.activate_session(&session).unwrap();
    let a = DeviceKeypair::from_seed(&[1; 32]);
    let b = DeviceKeypair::from_seed(&[2; 32]);
    let (low, high) = if a.device_id() < b.device_id() {
        (a, b)
    } else {
        (b, a)
    };
    let mut bound = binding(8);
    bound.local = high.device_id();
    bound.peer = low.device_id();
    bound.low = false;
    let context = Context {
        scope: bound.scope,
        local: bound.local,
        epoch: bound.epoch,
        now: 1000,
        indirect: true,
    };
    let (mut actor, registration) = runtime
        .prepare_actor(bound.scope.group, bound.local, &session)
        .unwrap();
    registration.register().unwrap();
    actor
        .sync_view(View {
            epoch: bound.epoch,
            members: [bound.local, bound.peer].into(),
            policy: bound.scope.policy,
            eligible: true,
            indirect: true,
        })
        .unwrap();
    let pool = runtime.resources();
    let chain = prepare_chain(
        pool.clone(),
        bound.clone(),
        Arc::new(slots::Slots::default()),
    )
    .unwrap();
    let object = SignedObject::issue(
        bound.scope,
        &low,
        [74; 32],
        1000,
        16_000,
        Body::Encrypted {
            header: EncryptedHeader {
                source_join: bound.peer_join,
                target: bound.local,
                target_join: bound.local_join,
                source_key_ref: bound.peer_key,
                target_key_ref: bound.local_key,
                handshake: bound.handshake,
                low_to_high: true,
                kind: EncryptedKind::ConfirmHello,
                sequence: 1,
                offer: None,
            },
            ciphertext: vec![22; 32],
        },
    )
    .unwrap();
    let envelope = Envelope::Direct(object.clone());
    let ticket = actor
        .admit_transport_verification(bound.peer, &envelope.encode())
        .unwrap();
    let room = CallLease::default();
    let consent = || ConfirmationConsent {
        room: room.clone(),
        _room_custody: None,
        effect: actor
            .effect(bound.peer, bound.local, object.expires())
            .unwrap(),
        clock: clock.clone(),
    };
    // Leave exactly one original setup allowance. Refused History contention must not debit it.
    actor
        .with_limits(|limits| {
            for _ in 0..39 {
                limits.origin(bound.peer, crate::voice_v2::limits::Traffic::Setup, 1000)?;
            }
            Ok(())
        })
        .unwrap();
    let (mut row, receipt, workflow) = chain.prepare_confirmation(&object, 1, consent()).unwrap();
    let guard = chain.history.state.lock().unwrap();
    let mut published = false;
    let result = actor.with_endpoint_origin(&ticket, &envelope, &context, |wall, mono, origin| {
        row.issue
            .publish_confirmation(&receipt, origin, wall, mono, || published = true)
    });
    assert_eq!(result, Err(VoiceError::Capacity));
    assert!(!published);
    assert!(guard.candidates.high().unwrap().iter().all(Option::is_none));
    // The outer owner must be destroyed only after the final guards, including our test's
    // deliberately held History guard. Otherwise RowIssue::drop would synchronously deadlock.
    drop(guard);
    drop((row, receipt, workflow));
    let (mut row, receipt, _workflow) = chain.prepare_confirmation(&object, 1, consent()).unwrap();
    assert_eq!(receipt.candidate(), 0);
    actor
        .with_endpoint_origin(&ticket, &envelope, &context, |wall, mono, origin| {
            row.issue
                .publish_confirmation(&receipt, origin, wall, mono, || published = true)
        })
        .unwrap();
    assert!(published);
    assert!(Arc::ptr_eq(
        &chain.find_confirmation(&object).unwrap().unwrap(),
        &receipt
    ));
    // Exercise the distinct response publication fence, not just Hello's fence. Hold History
    // after actual output-row preparation; refusal must leave the very same lane unpublished.
    let output_consent = || ConfirmationConsent {
        room: room.clone(),
        _room_custody: None,
        effect: actor.effect(bound.local, bound.peer, 16_000).unwrap(),
        clock: clock.clone(),
    };
    let (mut output, response, workflow) = chain
        .prepare_response(&receipt, [80; 32], 1, 16_000, output_consent())
        .unwrap();
    let guard = chain.history.state.lock().unwrap();
    let mut output_published = false;
    assert_eq!(
        actor.with_local_origin(bound.peer, &context, |wall, mono, origin| {
            output
                .issue
                .publish_response(&response, &receipt, origin, (wall, mono), || {
                    output_published = true
                })
        }),
        Err(VoiceError::Capacity)
    );
    assert!(!output_published);
    assert_eq!(guard.issued, 1 << 1);
    assert!(receipt.response.get().is_none());
    drop(guard);
    drop((output, response, workflow));
    assert!(chain.cached_response(&receipt).unwrap().is_none());
    let (mut output, response, _workflow) = chain
        .prepare_response(&receipt, [81; 32], 1, 16_000, output_consent())
        .unwrap();
    actor
        .with_local_origin(bound.peer, &context, |wall, mono, origin| {
            output
                .issue
                .publish_response(&response, &receipt, origin, (wall, mono), || {
                    output_published = true
                })
        })
        .unwrap();
    assert!(output_published);
    assert_eq!(
        chain.history.state.lock().unwrap().issued,
        (1 << 1) | (1 << 2)
    );
    assert!(Arc::ptr_eq(
        &chain.cached_response(&receipt).unwrap().unwrap(),
        &response
    ));
}
fn publish_unregistered(
    chain: &Arc<PreparedChain>,
    pool: &Arc<Resources>,
    slots: &Arc<slots::Slots>,
) -> (ActionQueue, Arc<CryptoAction>) {
    let mut queue = ActionQueue::with_slots(slots.clone());
    let (history, budget) = chain.prepare_derive().unwrap();
    let (payload_bytes, engine_bytes) = super::super::crypto_actions::derive_bytes(64).unwrap();
    let prepared = queue
        .prepare(
            Key::Derive(chain.binding.handshake),
            Allocation {
                resources: pool.clone(),
                scope: chain.binding.scope,
                call: chain.binding.call.clone(),
                budget,
                history: Some(history),
                payload_bytes,
                engine_bytes,
            },
        )
        .unwrap();
    let authority = CryptoAuthority::new(
        chain.binding.call.clone(),
        chain.binding.pair.clone(),
        Arc::new(ManualClock::new(1000)),
        chain.binding.deadline,
        u64::MAX,
    )
    .unwrap()
    .with_pending(chain.lease())
    .unwrap();
    let action = queue
        .publish_chain_derive(
            prepared,
            PairSetup {
                handshake: chain.binding.handshake,
                transcript: vec![3; 64],
                peer: chain.binding.peer,
                peer_key: [4; 65],
                local_is_low: true,
            },
            authority,
        )
        .unwrap();
    (queue, action)
}

#[test]
fn idle_close_releases_every_unused_byte_and_slot_despite_retained_handles() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let bound = binding(8);
    let chain = prepare_chain(pool.clone(), bound.clone(), slots.clone()).unwrap();
    assert_eq!(
        pool.counts(),
        (
            ROWS * ROW_BYTES
                + VARIABLE_BYTES
                + METADATA_BYTES
                + crate::voice_v2::resources::GROUP_FORWARD_METADATA_BYTES,
            1
        )
    );
    let ordinary = slots.charge(20).unwrap();
    assert!(slots.charge(1).is_err(), "all twelve future rows protected");
    assert!(prepare_chain(pool.clone(), binding(9), slots.clone()).is_err());
    chain.close();
    chain.close();
    assert_eq!(pool.counts(), (METADATA_BYTES, 0));
    assert!(!chain.matches(&bound, &pool, &slots, 1000));
    assert!(chain.prepare_derive().is_err());
    let remaining = slots.charge(12).unwrap();
    let other = prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
    assert!(
        chain.prepare_derive().is_err(),
        "old handle never adopts replacement capacity"
    );
    other.close();
    drop((other, ordinary, remaining));
    assert!(slots.charge(32).is_ok());
    assert_eq!(pool.counts(), (METADATA_BYTES, 0));
    drop(chain);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn abandoned_unpublished_row_returns_once_but_stale_issue_cannot_publish_or_pin_progress() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (row, budget) = chain.prepare_derive().unwrap();
    assert!(chain.prepare_derive().is_err());
    drop((row, budget));
    let (row, budget) = chain.prepare_derive().unwrap();
    drop(budget);
    chain.close();
    assert_eq!(pool.counts(), (ROW_BYTES + METADATA_BYTES, 0));
    let other = prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
    assert!(row
        .issue
        .publish(|| panic!("closed row must not publish"))
        .is_err());
    drop((row.record, row.slot));
    other.close();
    drop(other);
    assert_eq!(pool.counts(), (METADATA_BYTES, 0));
    drop(chain);
    assert_eq!(pool.counts(), (0, 0));
    assert!(slots.charge(32).is_ok());
}

#[test]
fn actual_input_reader_and_uncertain_engine_keep_old_progress_until_last_retirement() {
    for admitted in [false, true] {
        let pool = Arc::new(Resources::default());
        let slots = Arc::new(slots::Slots::default());
        let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
        let (_queue, action) = publish(&chain, &pool, &slots);
        let input = action.input().unwrap();
        if admitted {
            action.admit_engine(action.engine_bytes()).unwrap();
        }
        chain.close();
        assert!(!action.is_pending());
        assert!(action.admit_engine(action.engine_bytes()).is_err());
        assert!(
            action.with_pending(|| Ok(())).is_err(),
            "End fences already-admitted effects too"
        );
        assert!(
            prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).is_err()
        );
        drop(input);
        if admitted {
            assert!(
                prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).is_err(),
                "uncertain engine still owns progress"
            );
            action.settle_engine();
        }
        assert_eq!(pool.counts(), (ROW_BYTES + METADATA_BYTES, 0));
        let other =
            prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
        other.close();
        drop(other);
        assert_eq!(
            pool.counts(),
            (ROW_BYTES + METADATA_BYTES, 0),
            "history remains independently charged"
        );
    }
}

#[test]
fn lost_ack_history_outlives_flow_without_rearming_any_operation() {
    for rejected in [false, true] {
        let pool = Arc::new(Resources::default());
        let slots = Arc::new(slots::Slots::default());
        let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
        let (_queue, action) = publish(&chain, &pool, &slots);
        action.admit_engine(action.engine_bytes()).unwrap();
        let receipt = action
            .with_result(&[], action.output_tag(&[]).unwrap(), || {
                Ok(TerminalResult {
                    binding: ResultBinding {
                        pair: chain.binding.pair.clone(),
                        attempt: None,
                        deadline: 16_000,
                        expires: u64::MAX,
                    },
                    outcome: if rejected {
                        CryptoOutcome::Rejected(VoiceError::Malformed)
                    } else {
                        CryptoOutcome::Derived
                    },
                })
            })
            .unwrap();
        action.settle_engine();
        chain.close();
        assert!(
            action.input().is_ok(),
            "settled terminal with lost ACK is not unstarted"
        );
        assert!(
            prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).is_err()
        );
        action.acknowledge_result(&receipt).unwrap();
        assert!(action.input().is_err());
        let recovered = action
            .recover_result(&[], receipt.output_tag)
            .unwrap()
            .unwrap();
        assert!(recovered.same_transition(&receipt));
        assert!(recovered.retired);
        assert_eq!(pool.counts(), (ROW_BYTES + METADATA_BYTES, 0));
        let other =
            prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
        assert!(chain.prepare_derive().is_err());
        assert!(action.admit_engine(action.engine_bytes()).is_err());
        other.close();
    }
}

#[test]
fn row_rejects_other_resource_slot_call_key_and_budget_owners_before_charging() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let bound = binding(8);
    let chain = prepare_chain(pool.clone(), bound.clone(), slots.clone()).unwrap();
    let (row, budget) = chain.prepare_derive().unwrap();
    let before = pool.counts();
    let key = Key::Derive(bound.handshake);
    assert!(row
        .issue
        .validate(&pool, &slots, bound.scope, &bound.call, key, &budget)
        .is_ok());
    assert!(row
        .issue
        .validate(
            &Arc::new(Resources::default()),
            &slots,
            bound.scope,
            &bound.call,
            key,
            &budget
        )
        .is_err());
    assert!(row
        .issue
        .validate(
            &pool,
            &Arc::new(slots::Slots::default()),
            bound.scope,
            &bound.call,
            key,
            &budget
        )
        .is_err());
    assert!(row
        .issue
        .validate(
            &pool,
            &slots,
            bound.scope,
            &CallLease::default(),
            key,
            &budget
        )
        .is_err());
    assert!(row
        .issue
        .validate(
            &pool,
            &slots,
            bound.scope,
            &bound.call,
            Key::Derive([9; 32]),
            &budget
        )
        .is_err());
    assert!(row
        .issue
        .validate(
            &pool,
            &slots,
            bound.scope,
            &bound.call,
            key,
            &ByteBudget::Ordinary
        )
        .is_err());
    assert_eq!(pool.counts(), before);
    drop((row, budget));
    chain.close();
    assert_eq!(pool.counts(), (METADATA_BYTES, 0));
    drop(chain);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn concurrent_preparations_install_only_one_group_workflow_and_loser_returns_all_capacity() {
    let pool = Arc::new(Resources::default());
    let barrier = std::sync::Barrier::new(2);
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            prepare_chain(pool.clone(), binding(8), Arc::new(slots::Slots::default()))
        });
        barrier.wait();
        let second = prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default()));
        (first.join().unwrap(), second)
    });
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(
        pool.counts(),
        (
            ROWS * ROW_BYTES
                + VARIABLE_BYTES
                + METADATA_BYTES
                + crate::voice_v2::resources::GROUP_FORWARD_METADATA_BYTES,
            1
        )
    );
    first.or(second).unwrap().close();
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn failed_preparation_returns_partial_bytes_and_slots_at_each_resource_boundary() {
    for failure in 0..5 {
        let pool = Arc::new(Resources::default());
        let slots = Arc::new(slots::Slots::default());
        let bound = binding(8);
        let mut blockers = Vec::new();
        let slot_blocker = if failure == 0 {
            Some(slots.charge(21).unwrap())
        } else {
            None
        };
        if failure == 1 {
            blockers.push(
                pool.charge(
                    bound.scope.group,
                    crate::voice_v2::resources::GROUP_BYTES - ROWS * ROW_BYTES,
                    ResourceKind::Retained,
                )
                .unwrap(),
            );
        }
        if failure == 2 {
            for _ in 0..4 {
                blockers.push(
                    pool.charge(bound.scope.group, 0, ResourceKind::Crypto)
                        .unwrap(),
                );
            }
        }
        if failure == 3 {
            for _ in 0..3 {
                blockers.push(
                    pool.charge(bound.scope.group, 1, ResourceKind::Forward)
                        .unwrap(),
                );
            }
        }
        if failure == 4 {
            blockers.push(
                pool.charge(
                    bound.scope.group,
                    crate::voice_v2::resources::GROUP_BYTES
                        - ROWS * ROW_BYTES
                        - VARIABLE_BYTES
                        - METADATA_BYTES
                        + 1,
                    ResourceKind::Retained,
                )
                .unwrap(),
            );
        }
        let before = pool.counts();
        assert!(prepare_chain(pool.clone(), bound, slots.clone()).is_err());
        assert_eq!(
            pool.counts(),
            before,
            "partial preparation leaked at boundary {failure}"
        );
        drop((blockers, slot_blocker));
        assert_eq!(pool.counts(), (0, 0));
        assert!(slots.charge(32).is_ok());
    }
}

#[test]
fn quiet_expiry_or_parent_revocation_closes_idle_capacity_with_old_control_retained() {
    for end in [false, true] {
        let clock = Arc::new(ManualClock::new(1000));
        let runtime = super::super::runtime::Runtime::new(clock.clone());
        let pool = runtime.resources();
        let slots = Arc::new(slots::Slots::default());
        let bound = binding(8);
        let chain = prepare_chain(pool.clone(), bound.clone(), slots.clone()).unwrap();
        if end {
            bound.call.revoke();
        }
        if !end {
            clock.advance_ms(bound.deadline);
        }
        runtime.collect_expired();
        assert_eq!(pool.counts(), (METADATA_BYTES, 0));
        assert!(chain.prepare_derive().is_err());
        assert!(slots.charge(32).is_ok());
        drop(chain);
        assert_eq!(pool.counts(), (0, 0));
    }
}

#[test]
fn close_racing_row_publication_or_engine_admission_never_restores_work() {
    for engine in [false, true] {
        for _ in 0..8 {
            let pool = Arc::new(Resources::default());
            let slots = Arc::new(slots::Slots::default());
            let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
            let barrier = std::sync::Barrier::new(2);
            if engine {
                let (_queue, action) = publish(&chain, &pool, &slots);
                let result = std::thread::scope(|scope| {
                    let admit = scope.spawn(|| {
                        barrier.wait();
                        action.admit_engine(action.engine_bytes())
                    });
                    barrier.wait();
                    chain.close();
                    admit.join().unwrap()
                });
                // A genuine admission that won the race remains charged; End does not report
                // its actual settlement. Both outcomes permanently refuse effects/re-admission.
                assert_eq!(pool.counts().1, usize::from(result.is_ok()));
                assert!(action.with_pending(|| Ok(())).is_err());
                assert!(action.admit_engine(action.engine_bytes()).is_err());
                action.settle_engine();
                assert_eq!(pool.counts(), (ROW_BYTES + METADATA_BYTES, 0));
            } else {
                let (row, budget) = chain.prepare_derive().unwrap();
                drop(budget);
                let published = std::sync::atomic::AtomicBool::new(false);
                let result = std::thread::scope(|scope| {
                    let issue = scope.spawn(|| {
                        barrier.wait();
                        row.issue
                            .publish(|| published.store(true, std::sync::atomic::Ordering::SeqCst))
                    });
                    barrier.wait();
                    chain.close();
                    issue.join().unwrap()
                });
                assert_eq!(
                    published.load(std::sync::atomic::Ordering::SeqCst),
                    result.is_ok()
                );
                assert!(chain.prepare_derive().is_err());
                drop((row.record, row.slot));
                assert_eq!(pool.counts(), (METADATA_BYTES, 0));
            }
        }
    }
}

#[test]
fn close_in_publication_registration_gap_fences_effects_before_retiring_unstarted_input() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (_queue, action) = publish_unregistered(&chain, &pool, &slots);
    chain.close();
    assert!(
        action.input().is_ok(),
        "actual unpublished-to-adapter input still has an owner"
    );
    assert!(!action.is_pending());
    assert!(action.admit_engine(action.engine_bytes()).is_err());
    assert!(action.with_pending(|| Ok(())).is_err());
    assert!(prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).is_err());
    chain.register_action(&action);
    assert!(action.is_settled());
    assert!(action.input().is_err());
    assert_eq!(pool.counts(), (ROW_BYTES + METADATA_BYTES, 0));
    let other = prepare_chain(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
    other.close();
    drop((other, chain));
    // Fixed historical binding now shares the existing paid PlanBinding. It conveys no live
    // progress ownership, but its metadata cannot become uncharged while the action survives.
    assert_eq!(pool.counts(), (ROW_BYTES + METADATA_BYTES, 0));
    drop((action, _queue));
    assert_eq!(pool.counts(), (0, 0));
}
