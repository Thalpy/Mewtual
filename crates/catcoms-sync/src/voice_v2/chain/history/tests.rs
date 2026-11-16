use super::super::tests::{binding, prepare_chain, publish};
use super::*;
use crate::voice_v2::crypto_actions::{ActionQueue, CryptoOutcome, ResultBinding, TerminalResult};

#[test]
fn certified_collection_requires_request_and_actual_action_and_fixed_observer_return() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (mut queue, action) = publish(&chain, &pool, &slots);
    let identity = action.protected_plan().unwrap();
    let control = action.confirmation_control().unwrap();
    let fixed_observer = action.history_funding();
    let retirement = control.retire_all();
    assert!(retirement.is_drained());
    let history = identity.action_history(&retirement).unwrap();
    queue.collect_plan_history(&chain.binding);
    assert!(queue.get(Key::Derive(chain.binding.handshake)).is_some());
    assert!(
        !history.is_collected(),
        "ordinary closure is not native certification"
    );

    history.request_collection();
    history.request_collection(); // Lost reply cannot mint another operation or allowance.
    let serial = action.serial();
    queue.collect_plan_history(&chain.binding);
    assert!(queue.get(Key::Derive(chain.binding.handshake)).is_none());
    assert!(
        !history.is_collected(),
        "the actual Action still owns its slot and fee"
    );
    assert!(slots.charge(32).is_err());
    assert!(action.admit_engine(action.engine_bytes()).is_err());
    assert_eq!(action.serial(), serial);
    assert!(
        chain.prepare_derive().is_err(),
        "issued position never reopens"
    );
    drop(action);
    assert!(
        slots.charge(32).is_ok(),
        "actual Action Drop returns the position"
    );
    assert!(
        !history.is_collected(),
        "a fixed result observer still owns the fee"
    );
    drop(fixed_observer);
    assert!(history.is_collected());
    queue.collect_plan_history(&chain.binding);
    assert!(history.is_collected());
    drop((history, identity, retirement, chain, queue));
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn original_retirement_cannot_certify_equal_labels_from_another_allocation() {
    let pool = Arc::new(Resources::default());
    let bound = binding(8);
    let chain = prepare_chain(
        pool.clone(),
        bound.clone(),
        Arc::new(slots::Slots::default()),
    )
    .unwrap();
    let slots = chain.history.pool.clone();
    let (_queue, action) = publish(&chain, &pool, &slots);
    let identity = action.protected_plan().unwrap();
    let retirement = action.confirmation_control().unwrap().retire_all();
    let foreign = prepare_chain(pool.clone(), bound, Arc::new(slots::Slots::default())).unwrap();
    let (_foreign_queue, foreign_action) = publish(&foreign, &pool, &foreign.history.pool);
    let foreign_retirement = foreign_action.confirmation_control().unwrap().retire_all();
    assert_eq!(
        identity.action_history(&foreign_retirement).err(),
        Some(VoiceError::Unauthorized)
    );
    assert!(identity.action_history(&retirement).is_ok());
}

#[test]
fn held_input_or_admitted_engine_prevents_capture_until_real_return() {
    for admitted in [false, true] {
        let pool = Arc::new(Resources::default());
        let slots = Arc::new(slots::Slots::default());
        let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
        let (_queue, action) = publish(&chain, &pool, &slots);
        let identity = action.protected_plan().unwrap();
        let control = action.confirmation_control().unwrap();
        let input = action.input().unwrap();
        if admitted {
            action.admit_engine(action.engine_bytes()).unwrap();
        }
        let retirement = control.retire_all();
        assert!(!retirement.is_drained());
        assert_eq!(
            identity.action_history(&retirement).err(),
            Some(VoiceError::Unauthorized)
        );
        drop(input);
        if admitted {
            assert!(
                !retirement.is_drained(),
                "admitted original engine still owns progress"
            );
            action.settle_engine();
        }
        assert!(retirement.is_drained());
        assert!(identity.action_history(&retirement).is_ok());
    }
}

#[test]
fn terminal_ack_and_uncontended_original_engine_precede_forgetting() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (mut queue, action) = publish(&chain, &pool, &slots);
    let identity = action.protected_plan().unwrap();
    let control = action.confirmation_control().unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = action
        .with_result(&[], action.output_tag(&[]).unwrap(), || {
            Ok(TerminalResult {
                binding: ResultBinding {
                    pair: chain.binding.pair.clone(),
                    attempt: None,
                    deadline: chain.binding.deadline,
                    expires: u64::MAX,
                },
                outcome: CryptoOutcome::Derived,
            })
        })
        .unwrap();
    action.settle_engine();
    let retirement = control.retire_all();
    assert!(!retirement.is_drained());
    assert!(
        identity.action_history(&retirement).is_err(),
        "lost ACK is not retirement"
    );
    action.acknowledge_result(&receipt).unwrap();
    let history = identity.action_history(&retirement).unwrap();
    history.request_collection();
    action.with_engine_fence_for_test(|| queue.collect_plan_history(&chain.binding));
    assert!(
        queue.get(Key::Derive(chain.binding.handshake)).is_some(),
        "busy fence refuses without waiting"
    );
    queue.collect_plan_history(&chain.binding);
    assert!(queue.get(Key::Derive(chain.binding.handshake)).is_none());
    assert!(action
        .recover_result(&[], receipt.output_tag)
        .unwrap()
        .unwrap()
        .same_transition(&receipt));
    assert!(!history.is_collected());
    drop(action);
    assert!(history.is_collected());
}

#[test]
fn certified_record_return_reopens_headroom_for_same_previously_refused_plan() {
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let call = CallLease::default();
    let mut queue = ActionQueue::with_slots(slots.clone());
    let mut retired = Vec::new();
    // These are deliberately one-action abandoned plans, not a claim about completed
    // peer count. Each new plan needs the original twelve positions even though each
    // old terminal Derive retains only one, demonstrating reservation headroom pressure.
    for seed in 1..=21 {
        let mut bound = binding(seed);
        bound.call = call.clone();
        let chain = prepare_chain(pool.clone(), bound, slots.clone()).unwrap();
        let action = super::super::tests::publish_into(&chain, &pool, &mut queue);
        chain.register_action(&action);
        let identity = action.protected_plan().unwrap();
        let retirement = action.confirmation_control().unwrap().retire_all();
        let history = identity.action_history(&retirement).unwrap();
        retired.push((chain, history));
    }
    let mut pending = binding(22);
    pending.call = call;
    let before = pool.counts();
    assert_eq!(
        prepare_chain(pool.clone(), pending.clone(), slots.clone()).err(),
        Some(VoiceError::Capacity)
    );
    assert_eq!(
        pool.counts(),
        before,
        "local capacity refusal grants no allowance and retains no new graph"
    );
    let (first, history) = &retired[0];
    history.request_collection();
    queue.collect_plan_history(&first.binding);
    assert!(history.is_collected());
    assert!(queue.get(Key::Derive(first.binding.handshake)).is_none());
    assert!(
        queue
            .get(Key::Derive(retired[1].0.binding.handshake))
            .is_some(),
        "other original plan remains"
    );
    let next = prepare_chain(pool.clone(), pending.clone(), slots).unwrap();
    assert!(
        next.binding.value.same(&pending),
        "same deadline, call, pair and nonce allocation"
    );
    let action = super::super::tests::publish_into(&next, &pool, &mut queue);
    assert_eq!(action.serial(), 22, "Root serial never reset by forgetting");
    assert!(
        first.prepare_derive().is_err(),
        "old issued position stays closed"
    );
    next.register_action(&action);
    next.close();
}

#[test]
fn actual_unpublished_action_abandonment_and_unwind_return_history_once() {
    use crate::voice_v2::crypto_actions::Allocation;
    use crate::voice_v2::crypto_authority::CryptoAuthority;
    use crate::voice_v2::presence::PairSetup;
    use catcoms_rt::ManualClock;
    for unwind in [false, true] {
        let pool = Arc::new(Resources::default());
        let slots = Arc::new(slots::Slots::default());
        let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
        let mut queue = ActionQueue::with_slots(slots);
        let (history, budget) = chain.prepare_derive().unwrap();
        let (payload_bytes, engine_bytes) =
            crate::voice_v2::crypto_actions::derive_bytes(64).unwrap();
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
        let (issue, inactive) = ActionQueue::build_chain_derive(
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
        assert_eq!(chain.binding.history_records.load(Ordering::Acquire), 1);
        assert!(queue.get(Key::Derive(chain.binding.handshake)).is_none());
        if unwind {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _actual_unpublished_owners = (issue, inactive);
                panic!("deterministic unpublished construction unwind");
            }));
            assert!(result.is_err());
        } else {
            drop((issue, inactive));
        }
        assert_eq!(chain.binding.history_records.load(Ordering::Acquire), 0);
        assert!(
            chain.prepare_derive().is_ok(),
            "unpublished issue returned to its original owner"
        );
        chain.close();
        drop((chain, queue));
        assert_eq!(pool.counts(), (0, 0));
    }
}

#[test]
fn actual_record_credit_destructor_cannot_publish_history_collection_early() {
    #[derive(Debug)]
    struct DuringCreditDrop {
        history: PlanActionHistory,
        resources: Arc<Resources>,
        observed: Arc<AtomicBool>,
    }
    impl Drop for DuringCreditDrop {
        fn drop(&mut self) {
            assert_eq!(
                self.resources.counts(),
                (METADATA_BYTES, 0),
                "the actual fixed Credit already returned"
            );
            assert!(
                !self.history.is_collected(),
                "Credit destructor has not returned to the LAST custody publisher"
            );
            self.observed.store(true, Ordering::Release);
        }
    }
    let pool = Arc::new(Resources::default());
    let slots = Arc::new(slots::Slots::default());
    let chain = prepare_chain(pool.clone(), binding(8), slots.clone()).unwrap();
    let (mut queue, mut action) = publish(&chain, &pool, &slots);
    let identity = action.protected_plan().unwrap();
    let retirement = action.confirmation_control().unwrap().retire_all();
    let history = identity.action_history(&retirement).unwrap();
    history.request_collection();
    queue.collect_plan_history(&chain.binding);
    let observed = Arc::new(AtomicBool::new(false));
    CryptoAction::observe_record_credit_drop_for_test(
        &mut action,
        Arc::new(DuringCreditDrop {
            history: identity.action_history(&retirement).unwrap(),
            resources: pool,
            observed: observed.clone(),
        }),
    );
    drop(action);
    assert!(observed.load(Ordering::Acquire));
    assert!(history.is_collected());
}
