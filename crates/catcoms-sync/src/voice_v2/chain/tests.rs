use super::*;
use crate::voice_v2::crypto_actions::{
    ActionQueue, Allocation, CryptoOutcome, ResultBinding, TerminalResult,
};
use crate::voice_v2::crypto_authority::CryptoAuthority;
use crate::voice_v2::presence::PairSetup;
use catcoms_crypto::DeviceKeypair;
use catcoms_rt::ManualClock;

fn binding(seed: u8) -> Binding {
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
fn publish(
    chain: &Arc<PreparedChain>,
    pool: &Arc<Resources>,
    slots: &Arc<slots::Slots>,
) -> (ActionQueue, Arc<CryptoAction>) {
    let (queue, action) = publish_unregistered(chain, pool, slots);
    chain.register_action(&action);
    (queue, action)
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
    let chain = PreparedChain::new(pool.clone(), bound.clone(), slots.clone()).unwrap();
    assert_eq!(
        pool.counts(),
        (ROWS * ROW_BYTES + VARIABLE_BYTES + METADATA_BYTES, 1)
    );
    let ordinary = slots.charge(20).unwrap();
    assert!(slots.charge(1).is_err(), "all twelve future rows protected");
    assert!(PreparedChain::new(pool.clone(), binding(9), slots.clone()).is_err());
    chain.close();
    chain.close();
    assert_eq!(pool.counts(), (METADATA_BYTES, 0));
    assert!(!chain.matches(&bound, &pool, &slots, 1000));
    assert!(chain.prepare_derive().is_err());
    let remaining = slots.charge(12).unwrap();
    let other =
        PreparedChain::new(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
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
    let chain = PreparedChain::new(pool.clone(), binding(8), slots.clone()).unwrap();
    let (row, budget) = chain.prepare_derive().unwrap();
    assert!(chain.prepare_derive().is_err());
    drop((row, budget));
    let (row, budget) = chain.prepare_derive().unwrap();
    drop(budget);
    chain.close();
    assert_eq!(pool.counts(), (ROW_BYTES + METADATA_BYTES, 0));
    let other =
        PreparedChain::new(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
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
        let chain = PreparedChain::new(pool.clone(), binding(8), slots.clone()).unwrap();
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
            PreparedChain::new(pool.clone(), binding(9), Arc::new(slots::Slots::default()))
                .is_err()
        );
        drop(input);
        if admitted {
            assert!(
                PreparedChain::new(pool.clone(), binding(9), Arc::new(slots::Slots::default()))
                    .is_err(),
                "uncertain engine still owns progress"
            );
            action.settle_engine();
        }
        assert_eq!(pool.counts(), (ROW_BYTES + METADATA_BYTES, 0));
        let other = PreparedChain::new(pool.clone(), binding(9), Arc::new(slots::Slots::default()))
            .unwrap();
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
        let chain = PreparedChain::new(pool.clone(), binding(8), slots.clone()).unwrap();
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
            PreparedChain::new(pool.clone(), binding(9), Arc::new(slots::Slots::default()))
                .is_err()
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
        let other = PreparedChain::new(pool.clone(), binding(9), Arc::new(slots::Slots::default()))
            .unwrap();
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
    let chain = PreparedChain::new(pool.clone(), bound.clone(), slots.clone()).unwrap();
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
            PreparedChain::new(pool.clone(), binding(8), Arc::new(slots::Slots::default()))
        });
        barrier.wait();
        let second =
            PreparedChain::new(pool.clone(), binding(9), Arc::new(slots::Slots::default()));
        (first.join().unwrap(), second)
    });
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(
        pool.counts(),
        (ROWS * ROW_BYTES + VARIABLE_BYTES + METADATA_BYTES, 1)
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
                    256 * 1024 - ROWS * ROW_BYTES,
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
                    256 * 1024 - ROWS * ROW_BYTES - VARIABLE_BYTES - METADATA_BYTES + 1,
                    ResourceKind::Retained,
                )
                .unwrap(),
            );
        }
        let before = pool.counts();
        assert!(PreparedChain::new(pool.clone(), bound, slots.clone()).is_err());
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
        let chain = PreparedChain::new(pool.clone(), bound.clone(), slots.clone()).unwrap();
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
            let chain = PreparedChain::new(pool.clone(), binding(8), slots.clone()).unwrap();
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
    let chain = PreparedChain::new(pool.clone(), binding(8), slots.clone()).unwrap();
    let (_queue, action) = publish_unregistered(&chain, &pool, &slots);
    chain.close();
    assert!(
        action.input().is_ok(),
        "actual unpublished-to-adapter input still has an owner"
    );
    assert!(!action.is_pending());
    assert!(action.admit_engine(action.engine_bytes()).is_err());
    assert!(action.with_pending(|| Ok(())).is_err());
    assert!(
        PreparedChain::new(pool.clone(), binding(9), Arc::new(slots::Slots::default())).is_err()
    );
    chain.register_action(&action);
    assert!(action.is_settled());
    assert!(action.input().is_err());
    assert_eq!(pool.counts(), (ROW_BYTES + METADATA_BYTES, 0));
    let other =
        PreparedChain::new(pool.clone(), binding(9), Arc::new(slots::Slots::default())).unwrap();
    other.close();
    drop((other, chain));
    assert_eq!(pool.counts(), (ROW_BYTES, 0));
}
