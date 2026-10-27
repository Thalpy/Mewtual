//! Actual native owners and charged row-zero publication. These tests supply the empty trusted
//! browser completion marker; real WebCrypto derivation is tested separately in the frontend.
use super::*;
use crate::voice_v2::crypto_actions::{CryptoInput, CryptoOutcome, CryptoResultReceipt};
use crate::voice_v2::resources::ResourceKind;
use std::sync::atomic::Ordering;

fn prepare_checked(
    end: &mut End,
    actor: &ActorOwner,
    id: Nonce,
    clock: &ManualClock,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<Arc<CryptoAction>, VoiceError> {
    let room = room(end);
    end.join.prepare_protected_derivation(
        ProtectedDerivation {
            actor,
            room: &room,
            handshake: id,
        },
        &ca(&end.context, &end.lease),
        Arc::new(clock.clone()),
        final_owner,
    )
}

fn commit_checked(
    end: &mut End,
    actor: &ActorOwner,
    action: &Arc<CryptoAction>,
    clock: &ManualClock,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<CryptoResultReceipt, VoiceError> {
    let room = room(end);
    end.join.commit_protected_derivation_result(
        ProtectedDerivationResult {
            actor,
            room: &room,
            action,
            claimed_tag: action.output_tag(&[]).unwrap(),
        },
        &ca(&end.context, &end.lease),
        clock,
        final_owner,
    )
}

#[test]
fn protected_child_identity_uses_actual_plan_and_original_published_role() {
    let mut p = fixture();
    let mut other = fixture();
    let derive = prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap();
    let foreign = prepare_checked(
        &mut other.f.low,
        &other.low,
        other.f.id,
        &other.f.clock,
        |_, _| Ok(()),
    )
    .unwrap();
    // Independently created fixtures deliberately project exactly equal copied metadata.
    // Neither that projection nor the equal role number may join their native custody.
    assert_eq!(derive.protected_binding(), foreign.protected_binding());
    assert_eq!(derive.protected_row(), Ok(0));
    assert_eq!(foreign.protected_row(), Ok(0));
    assert!(!derive.same_protected_plan(&foreign));
    assert!(!foreign.same_protected_plan(&derive));
    assert!(derive.same_protected_plan(&derive));
    let original_plan = derive.protected_plan().unwrap();
    assert!(original_plan.contains(&derive));
    assert!(!original_plan.contains(&foreign));

    derive.admit_engine(derive.engine_bytes()).unwrap();
    let receipt = commit_checked(&mut p.f.low, &p.low, &derive, &p.f.clock, |_, _| Ok(())).unwrap();
    derive.settle_engine();
    derive.acknowledge_result(&receipt).unwrap();
    let hello = start(&mut p, None);
    assert_eq!(hello.protected_row(), Ok(1));
    assert!(hello.same_protected_plan(&derive));
    assert!(original_plan.contains(&hello));
    assert!(derive.same_protected_plan(&hello));
    assert!(!hello.same_protected_plan(&foreign));
    let _ = seal(&mut p.f.low, &hello, &p.f.clock);
    assert!(derive.input().is_err());
    assert!(hello.input().is_err());
    p.f.low.join.confirmations.pairs[&p.f.id]
        .chain
        .as_ref()
        .unwrap()
        .close();
    assert_eq!(derive.protected_row(), Ok(0));
    assert_eq!(hello.protected_row(), Ok(1));
    assert!(hello.same_protected_plan(&derive));
    assert!(!hello.same_protected_plan(&foreign));
    assert!(!hello.is_pending(), "identity is never renewed authority");
    let pool = p.f.pool.clone();
    drop((derive, hello, p));
    // The custodian retains ONLY Root's already-paid fixed plan identity. It cannot keep
    // Workflow, input, engine or dropped CryptoAction allocations alive after their credits.
    assert_eq!(pool.counts(), (2048, 0));
    assert!(!original_plan.contains(&foreign));
    drop(original_plan);
    assert_eq!(pool.counts(), (0, 0));
}

#[test]
fn protected_derivation_recovers_the_same_row_before_and_after_ack_for_both_roles() {
    let mut p = fixture();
    let low_id = p.f.low.context.local;
    for (end, actor) in [(&mut p.f.low, &p.low), (&mut p.f.high, &p.high)] {
        let action = prepare_checked(end, actor, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap();
        let input = action.input().unwrap();
        let CryptoInput::Derive(setup) = input.as_input() else {
            panic!("derive input")
        };
        assert_eq!(setup.handshake, p.f.id);
        assert_eq!(setup.local_is_low, end.context.local == low_id);
        let tag = action.output_tag(&[]).unwrap();
        let counts = end.join.resources.counts();
        assert!(Arc::ptr_eq(
            &action,
            &prepare_checked(end, actor, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap()
        ));
        assert_eq!(end.join.resources.counts(), counts);
        assert_eq!(
            commit_checked(end, actor, &action, &p.f.clock, |_, _| Ok(())).err(),
            Some(VoiceError::Unauthorized),
            "no native engine admitted yet"
        );
        action.admit_engine(action.engine_bytes()).unwrap();
        assert_eq!(end.join.resources.counts().1, 1);
        let receipt = commit_checked(end, actor, &action, &p.f.clock, |_, _| Ok(())).unwrap();
        assert_eq!(receipt.outcome, CryptoOutcome::Derived);
        assert!(!action.is_settled(), "commit is not actual settlement");
        action.settle_engine();
        action.acknowledge_result(&receipt).unwrap();
        drop(input);
        assert!(action.input().is_err());
        let compacted = end.join.resources.counts();
        for _ in 0..4 {
            assert!(Arc::ptr_eq(
                &action,
                &prepare_checked(end, actor, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap()
            ));
            let recovered = commit_checked(end, actor, &action, &p.f.clock, |_, _| {
                panic!("metadata lookup cannot grant fresh authority")
            })
            .unwrap();
            assert!(receipt.same_transition(&recovered));
            assert_eq!(recovered.output_tag, tag);
        }
        assert_eq!(end.join.resources.counts(), compacted);
        assert_eq!(
            compacted.1, counts.1,
            "open Workflow retains its prepaid execution lane"
        );
        end.join.confirmations.pairs[&p.f.id]
            .chain
            .as_ref()
            .unwrap()
            .close();
        assert_eq!(end.join.resources.counts().1, 0);
    }
}

#[test]
fn protected_derivation_capacity_refusal_publishes_no_row_and_exact_pair_can_retry() {
    let mut p = fixture();
    let before = p.f.pool.counts();
    assert_eq!(
        prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Err(
            VoiceError::Capacity
        ))
        .err(),
        Some(VoiceError::Capacity)
    );
    assert!(p.f.low.join.actions.get(Key::Derive(p.f.id)).is_none());
    assert_eq!(p.f.pool.counts(), before);
    // Deliberately hold the real Group mutex. Preparation must return Capacity at its final
    // try-lock, without publishing or waiting under a stale time observation.
    assert_eq!(
        p.low
            .with_limits(|_| {
                prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(()))
            })
            .err(),
        Some(VoiceError::Capacity)
    );
    assert!(p.f.low.join.actions.get(Key::Derive(p.f.id)).is_none());
    assert_eq!(p.f.pool.counts(), before);
    let action = prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap();
    assert!(action.is_pending());
    assert!(p.f.low.join.actions.key_of(&action) == Some(Key::Derive(p.f.id)));
}

#[test]
fn protected_derivation_and_result_progress_with_full_ordinary_bytes() {
    let mut p = fixture();
    let pressure =
        p.f.pool
            .charge(
                p.f.low.context.scope.group,
                crate::voice_v2::resources::GROUP_BYTES - p.f.pool.counts().0,
                ResourceKind::Retained,
            )
            .unwrap();
    let action = prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = commit_checked(&mut p.f.low, &p.low, &action, &p.f.clock, |_, _| Ok(())).unwrap();
    assert_eq!(receipt.outcome, CryptoOutcome::Derived);
    action.settle_engine();
    action.acknowledge_result(&receipt).unwrap();
    p.f.low.join.confirmations.pairs[&p.f.id]
        .chain
        .as_ref()
        .unwrap()
        .close();
    assert_eq!(p.f.pool.counts().1, 0);
    drop(pressure);
}

#[test]
fn protected_derivation_result_local_refusal_can_retry_without_another_engine() {
    let mut p = fixture();
    let action = prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let counts = p.f.pool.counts();
    assert_eq!(
        commit_checked(&mut p.f.low, &p.low, &action, &p.f.clock, |_, _| Err(
            VoiceError::Capacity
        ))
        .err(),
        Some(VoiceError::Capacity)
    );
    assert!(action.terminal_status().receipt.is_none());
    assert!(action.input().is_ok());
    assert_eq!(p.f.pool.counts(), counts);
    let receipt = commit_checked(&mut p.f.low, &p.low, &action, &p.f.clock, |_, _| Ok(())).unwrap();
    assert_eq!(receipt.outcome, CryptoOutcome::Derived);
    assert_eq!(p.f.pool.counts(), counts);
    action.settle_engine();
}

#[test]
fn protected_derivation_factory_and_result_refuse_busy_native_policy_without_spending_work() {
    for result in [false, true] {
        let mut p = fixture();
        let action = result.then(|| {
            prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap()
        });
        if let Some(action) = &action {
            action.admit_engine(action.engine_bytes()).unwrap();
        }
        let counts = p.f.pool.counts();
        std::thread::scope(|threads| {
            let (held, ready) = std::sync::mpsc::sync_channel(0);
            let (release, released) = std::sync::mpsc::sync_channel(0);
            let actor = &p.low;
            let holder = threads.spawn(move || {
                actor.with_held_policy_for_test(|| {
                    held.send(()).unwrap();
                    released.recv().unwrap();
                })
            });
            ready.recv().unwrap();
            let error = if let Some(action) = &action {
                commit_checked(&mut p.f.low, &p.low, action, &p.f.clock, |_, _| {
                    panic!("busy Policy")
                })
                .err()
            } else {
                prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| {
                    panic!("busy Policy")
                })
                .err()
            };
            // Release before assertions so a failed assertion cannot strand a scoped thread.
            release.send(()).unwrap();
            holder.join().unwrap();
            assert_eq!(error, Some(VoiceError::Capacity));
        });
        assert_eq!(p.f.pool.counts(), counts);
        if let Some(action) = action {
            assert!(action.terminal_status().receipt.is_none());
            assert_eq!(
                commit_checked(&mut p.f.low, &p.low, &action, &p.f.clock, |_, _| Ok(()))
                    .unwrap()
                    .outcome,
                CryptoOutcome::Derived
            );
            action.settle_engine();
        } else {
            assert!(p.f.low.join.actions.get(Key::Derive(p.f.id)).is_none());
            assert!(
                prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).is_ok()
            );
        }
    }
}

#[test]
fn protected_derivation_first_publication_and_result_recheck_last_clock_owner_closure() {
    for result in [false, true] {
        for owner in ["call", "pair", "chain", "room", "actor"] {
            let mut p = fixture();
            let action = result.then(|| {
                prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap()
            });
            if let Some(action) = &action {
                action.admit_engine(action.engine_bytes()).unwrap();
            }
            let room = RoomAuthority {
                lease: CallLease::default(),
                ..room(&p.f.low)
            };
            let pair = &p.f.low.join.confirmations.pairs[&p.f.id];
            let close = match owner {
                "call" => p.f.low.lease.clone(),
                "pair" => pair.pin.clone(),
                "chain" => pair.chain.as_ref().unwrap().lease(),
                "room" => room.lease.clone(),
                _ => p.low.revocation_observer_for_test(),
            };
            // Runtime's effective now samples source mono, then with_local_origin samples mono
            // itself. Close on the latter, last callback immediately before pure native gates.
            let last = p.origin_clock.reads.load(Ordering::Acquire) + 2;
            *p.origin_clock.revoke.lock().unwrap() = Some((last, close));
            let error = if let Some(action) = &action {
                p.f.low
                    .join
                    .commit_protected_derivation_result(
                        ProtectedDerivationResult {
                            actor: &p.low,
                            room: &room,
                            action,
                            claimed_tag: action.output_tag(&[]).unwrap(),
                        },
                        &ca(&p.f.low.context, &p.f.low.lease),
                        &p.f.clock,
                        |_, _| Ok(()),
                    )
                    .err()
            } else {
                p.f.low
                    .join
                    .prepare_protected_derivation(
                        ProtectedDerivation {
                            actor: &p.low,
                            room: &room,
                            handshake: p.f.id,
                        },
                        &ca(&p.f.low.context, &p.f.low.lease),
                        Arc::new(p.f.clock.clone()),
                        |_, _| Ok(()),
                    )
                    .err()
            };
            assert_eq!(
                error,
                Some(VoiceError::Unauthorized),
                "result={result}, {owner}"
            );
            if let Some(action) = action {
                assert!(action.terminal_status().receipt.is_none());
                action.settle_engine();
            } else {
                assert!(p.f.low.join.actions.get(Key::Derive(p.f.id)).is_none());
            }
        }
    }
}

#[test]
fn protected_derivation_atomic_session_close_wins_before_factory_or_result_with_live_child() {
    for result in [false, true] {
        let mut p = fixture();
        let action = result.then(|| {
            prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap()
        });
        if let Some(action) = &action {
            action.admit_engine(action.engine_bytes()).unwrap();
        }
        let child = p.low.child_lease().unwrap();
        let closed = p.low.revocation_observer_for_test();
        let runtime = p._runtimes[0].clone();
        let mut revoker = None;
        let final_owner = |_, _| {
            revoker = Some(std::thread::spawn(move || runtime.revoke_session()));
            futures::executor::block_on(closed.wait_revoked());
            assert!(
                child.is_live(),
                "Policy is still held, original native child remains live"
            );
            Ok(())
        };
        let error = if let Some(action) = &action {
            commit_checked(&mut p.f.low, &p.low, action, &p.f.clock, final_owner).err()
        } else {
            prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, final_owner).err()
        };
        revoker.unwrap().join().unwrap();
        assert_eq!(error, Some(VoiceError::Unauthorized));
        assert!(!child.is_live());
        if let Some(action) = action {
            assert!(action.terminal_status().receipt.is_none());
            action.settle_engine();
        } else {
            assert!(p.f.low.join.actions.get(Key::Derive(p.f.id)).is_none());
        }
    }
}

#[test]
fn protected_derivation_settlement_or_unwind_during_preparation_cannot_publish_first_result() {
    for unwind in [false, true] {
        let mut p = fixture();
        let action =
            prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        // pair_current samples this clock only after acquiring the affine preparation loan.
        // A real competing settlement must fence publication but retain the running custody.
        #[derive(Debug)]
        struct SettleClock<'a> {
            action: &'a CryptoAction,
            clock: &'a ManualClock,
            chain: Arc<crate::voice_v2::chain::PreparedChain>,
            unwind: bool,
        }
        impl Clock for SettleClock<'_> {
            fn now_ms(&self) -> u64 {
                self.clock.now_ms()
            }
            fn monotonic_ms(&self) -> u64 {
                self.action.settle_engine();
                self.chain.close();
                assert!(!self.action.is_settled());
                assert!(self.action.input().is_ok());
                assert!(self.action.engine_bytes() > 0);
                assert!(self.action.result_fence_available_for_test());
                assert!(!self.unwind, "injected preparation unwind");
                self.clock.monotonic_ms()
            }
            fn sleep(
                &self,
                duration: std::time::Duration,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
                self.clock.sleep(duration)
            }
        }
        let room = room(&p.f.low);
        let clock = SettleClock {
            action: &action,
            clock: &p.f.clock,
            chain: p.f.low.join.confirmations.pairs[&p.f.id]
                .chain
                .as_ref()
                .unwrap()
                .clone(),
            unwind,
        };
        let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            p.f.low.join.commit_protected_derivation_result(
                ProtectedDerivationResult {
                    actor: &p.low,
                    room: &room,
                    action: &action,
                    claimed_tag: action.output_tag(&[]).unwrap(),
                },
                &ca(&p.f.low.context, &p.f.low.lease),
                &clock,
                |_, _| Ok(()),
            )
        }));
        if unwind {
            assert!(attempt.is_err());
        } else {
            assert_eq!(attempt.unwrap().err(), Some(VoiceError::Unauthorized));
        }
        assert!(action.is_settled());
        assert_eq!(p.f.pool.counts().1, 0);
        assert!(action.terminal_status().receipt.is_none());
    }
}

#[test]
fn protected_derivation_history_survives_retirement_but_never_reissues_key_work() {
    let mut p = fixture();
    let action = prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let receipt = commit_checked(&mut p.f.low, &p.low, &action, &p.f.clock, |_, _| Ok(())).unwrap();
    action.settle_engine();
    action.acknowledge_result(&receipt).unwrap();
    p._runtimes[0].revoke_session();
    p.f.low.lease.revoke(); // Unit End has independent call custody; Root tests use Directory.
    let recovered = commit_checked(&mut p.f.low, &p.low, &action, &p.f.clock, |_, _| {
        panic!("retired metadata has no first-effect authority")
    })
    .unwrap();
    assert!(receipt.same_transition(&recovered));
    assert!(recovered.retired);
    assert!(prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).is_err());
    assert!(action.input().is_err());
    assert!(action.admit_engine(action.engine_bytes()).is_err());
}

#[test]
fn protected_derivation_original_deadline_cannot_be_renewed_by_wall_rollback_or_epoch_change() {
    for epoch in [false, true] {
        let mut p = fixture();
        let action =
            prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        if epoch {
            p.f.low.context.epoch += 1;
        } else {
            p.f.clock.advance_ms(MAX_PACKET_LIFETIME_MS);
            p.f.clock.set_wall_ms(0);
        }
        assert!(commit_checked(&mut p.f.low, &p.low, &action, &p.f.clock, |_, _| Ok(())).is_err());
        assert!(prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).is_err());
        assert!(action.terminal_status().receipt.is_none());
        action.settle_engine();
        p.f.low.join.confirmations.pairs[&p.f.id]
            .chain
            .as_ref()
            .unwrap()
            .close();
        assert_eq!(p.f.pool.counts().1, 0);
    }
}

#[test]
fn protected_derivation_factory_cached_lookup_and_result_refuse_expiry_on_the_last_native_clock() {
    for boundary in ["factory", "cached", "result"] {
        let mut p = fixture();
        let action = (boundary != "factory").then(|| {
            prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).unwrap()
        });
        if boundary == "result" {
            action
                .as_ref()
                .unwrap()
                .admit_engine(action.as_ref().unwrap().engine_bytes())
                .unwrap();
        }
        let last = p.origin_clock.reads.load(Ordering::Acquire) + 2;
        *p.origin_clock.advance.lock().unwrap() = Some((last, MAX_PACKET_LIFETIME_MS));
        let error = if boundary == "result" {
            commit_checked(
                &mut p.f.low,
                &p.low,
                action.as_ref().unwrap(),
                &p.f.clock,
                |_, _| Ok(()),
            )
            .err()
        } else {
            prepare_checked(&mut p.f.low, &p.low, p.f.id, &p.f.clock, |_, _| Ok(())).err()
        };
        assert_eq!(error, Some(VoiceError::Unauthorized), "{boundary}");
        if let Some(action) = action {
            assert!(action.terminal_status().receipt.is_none());
            action.settle_engine();
        } else {
            assert!(p.f.low.join.actions.get(Key::Derive(p.f.id)).is_none());
        }
    }
}
