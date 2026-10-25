//! Real typed Open/token/result/delivery transaction, with SDK AEAD output mocked. These
//! tests establish Root lifecycle/ownership, not native/browser IPC or a real RTC engine.
use super::*;
use crate::voice_v2::crypto_actions::{CryptoAction, CryptoOutcome};
use crate::voice_v2::sdp::{SdpDelivery, SdpOpenOutput, SdpOpenResult};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

// The original action Clock is the sanctioned seam for parking the actual final native
// view AFTER preparation. A literal Capacity callback would not exercise those try-locks.
struct BarrierClock {
    original: Arc<dyn Clock>,
    reads: AtomicUsize,
    trigger: AtomicUsize,
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl std::fmt::Debug for BarrierClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BarrierClock")
    }
}
impl Clock for BarrierClock {
    fn now_ms(&self) -> u64 {
        self.original.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        let read = self.reads.fetch_add(1, Ordering::AcqRel) + 1;
        if read == self.trigger.load(Ordering::Acquire) {
            let hook = self.hook.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        self.original.monotonic_ms()
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.original.sleep(duration)
    }
}
fn with_clock(
    p: &mut ProtectedFixture,
    intent: &Arc<InitialSdpIntent>,
    clock: Arc<dyn Clock>,
) -> Arc<CryptoAction> {
    p.f.high
        .join
        .prepare_high_sdp_open(
            &p.high,
            &room(&p.f.high),
            p.f.id,
            intent,
            &ca(&p.f.high.context, &p.f.high.lease),
            clock,
            |_, _| Ok(()),
            None,
        )
        .unwrap()
}

#[test]
fn sdp_open_actual_final_group_and_policy_contention_preserve_token_and_same_action() {
    let (mut baseline, baseline_phase, intent) = fixture();
    let baseline_clock = Arc::new(BarrierClock {
        original: baseline._runtimes[1].clock(),
        reads: AtomicUsize::new(0),
        trigger: AtomicUsize::new(usize::MAX),
        hook: Mutex::new(None),
    });
    let action = with_clock(&mut baseline, &intent, baseline_clock.clone());
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    baseline_clock.reads.store(0, Ordering::Release);
    let first = submit(&mut baseline, &action, output(&action, &bytes), |_, _| {
        Ok(())
    })
    .unwrap();
    // The final recovery after publication samples once more. Stop at the sample BEFORE
    // that historical read, immediately before the actual Engine->Group/Policy apply.
    let final_read = baseline_clock.reads.load(Ordering::Acquire) - 1;
    assert!(final_read > 0);
    action.acknowledge_result(&first.receipt).unwrap();
    action.settle_engine();
    baseline_phase.close();
    for policy in [false, true] {
        let (mut p, phase, intent) = fixture();
        let clock = Arc::new(BarrierClock {
            original: p._runtimes[1].clock(),
            reads: AtomicUsize::new(0),
            trigger: AtomicUsize::new(usize::MAX),
            hook: Mutex::new(None),
        });
        let action = with_clock(&mut p, &intent, clock.clone());
        action.admit_engine(action.engine_bytes()).unwrap();
        let (held_tx, held_rx) = std::sync::mpsc::sync_channel(0);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
        let (start_tx, start_rx) = std::sync::mpsc::sync_channel(0);
        let hold = p
            .high
            .native_view_holder_for_test(policy, held_tx, release_rx);
        let holder = std::thread::spawn(move || {
            if start_rx.recv().is_ok() {
                hold();
            }
        });
        *clock.hook.lock().unwrap() = Some(Box::new(move || {
            start_tx.send(()).unwrap();
            held_rx.recv().unwrap();
        }));
        clock.reads.store(0, Ordering::Release);
        clock.trigger.store(final_read, Ordering::Release);
        let result = submit(&mut p, &action, output(&action, &bytes), |_, _| Ok(()));
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        assert_eq!(result.err(), Some(VoiceError::Capacity));
        assert!(
            clock.hook.lock().unwrap().is_none(),
            "actual final Clock barrier reached"
        );
        assert!(action.terminal_status().receipt.is_none());
        action.with_pending(|| Ok(())).unwrap();
        let first = submit(&mut p, &action, output(&action, &bytes), |_, _| Ok(())).unwrap();
        assert_eq!(first.receipt.outcome, CryptoOutcome::Opened);
        action.acknowledge_result(&first.receipt).unwrap();
        action.settle_engine();
        phase.close();
    }
}

pub(super) fn submit(
    p: &mut ProtectedFixture,
    action: &Arc<CryptoAction>,
    output: SdpOpenOutput<'_>,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<SdpOpenResult, VoiceError> {
    p.f.high.join.submit_sdp_open_result(
        &p.high,
        &room(&p.f.high),
        action,
        output,
        &ca(&p.f.high.context, &p.f.high.lease),
        &p.f.clock,
        final_owner,
    )
}
pub(super) fn deliver(
    p: &ProtectedFixture,
    action: &Arc<CryptoAction>,
    delivery: &Arc<SdpDelivery>,
    final_owner: impl FnOnce(u64, u64) -> Result<(), VoiceError>,
) -> Result<(), VoiceError> {
    p.f.high.join.admit_sdp_delivery(
        &p.high,
        &room(&p.f.high),
        action,
        delivery,
        &ca(&p.f.high.context, &p.f.high.lease),
        final_owner,
    )
}
pub(super) fn output<'a>(action: &CryptoAction, bytes: &'a [u8]) -> SdpOpenOutput<'a> {
    SdpOpenOutput::Plaintext {
        bytes,
        claimed_tag: action.output_tag(bytes).unwrap(),
    }
}

#[test]
fn sdp_open_maximum_capacity_retry_preserves_token_then_exact_lost_result_keeps_same_delivery() {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let output = output(&action, &bytes);
    for _ in 0..3 {
        assert_eq!(
            submit(&mut p, &action, output, |_, _| Err(VoiceError::Capacity)).err(),
            Some(VoiceError::Capacity)
        );
        assert!(action.terminal_status().receipt.is_none());
        action.with_pending(|| Ok(())).unwrap();
    }
    let first = submit(&mut p, &action, output, |_, _| Ok(())).unwrap();
    assert_eq!(first.receipt.outcome, CryptoOutcome::Opened);
    let delivery = first.delivery.unwrap();
    assert!(!delivery.was_admitted(), "Open is not RTC permission");
    assert!(
        action.with_pending(|| Ok(())).is_err(),
        "original token was consumed"
    );
    assert_eq!(
        deliver(&p, &action, &delivery, |_, _| Err(VoiceError::Capacity)),
        Err(VoiceError::Capacity)
    );
    assert!(!delivery.was_admitted());
    deliver(&p, &action, &delivery, |_, _| Ok(())).unwrap();
    deliver(&p, &action, &delivery, |_, _| Ok(())).unwrap();
    assert!(delivery.was_admitted());
    action.acknowledge_result(&first.receipt).unwrap();
    action.settle_engine();
    assert!(action.input().is_err());
    phase.close();
    let retry = submit(&mut p, &action, output, |_, _| {
        panic!("historical result cannot consume again")
    })
    .unwrap();
    assert!(retry.receipt.same_transition(&first.receipt));
    assert!(retry.receipt.retired);
    assert!(Arc::ptr_eq(&delivery, &retry.delivery.unwrap()));
    assert_eq!(
        deliver(&p, &action, &delivery, |_, _| Ok(())),
        Err(VoiceError::Unauthorized),
        "old admitted receipt cannot restore current permission"
    );
    let mut changed = bytes;
    changed[0] ^= 1;
    assert_eq!(
        submit(&mut p, &action, self::output(&action, &changed), |_, _| Ok(
            ()
        ))
        .err(),
        Some(VoiceError::RetryMismatch)
    );
}

#[test]
fn sdp_open_aead_failure_and_invalid_utf8_retain_rejection_without_fake_plaintext_or_delivery() {
    for aead_failure in [false, true] {
        let (mut p, phase, intent) = fixture();
        let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        let bytes = vec![0xff; MAX_SDP_PLAINTEXT];
        let output = if aead_failure {
            SdpOpenOutput::AuthenticationFailed
        } else {
            output(&action, &bytes)
        };
        let first = submit(&mut p, &action, output, |_, _| Ok(())).unwrap();
        assert_eq!(
            first.receipt.outcome,
            CryptoOutcome::Rejected(VoiceError::Malformed)
        );
        assert!(first.delivery.is_none());
        action.acknowledge_result(&first.receipt).unwrap();
        action.settle_engine();
        phase.close();
        let retry = action.recover_sdp_open_result(output).unwrap().unwrap();
        assert!(retry.receipt.same_transition(&first.receipt));
        assert!(retry.receipt.retired);
        assert!(retry.delivery.is_none());
        let other = if aead_failure {
            self::output(&action, &bytes)
        } else {
            SdpOpenOutput::AuthenticationFailed
        };
        assert_eq!(
            action.recover_sdp_open_result(other).err(),
            Some(VoiceError::RetryMismatch)
        );
    }
}

#[test]
fn sdp_open_and_delivery_final_pc_close_refuse_first_effect_and_never_refund_running_engine() {
    for close_during_open in [false, true] {
        let (mut p, phase, intent) = fixture();
        let owner = pending(&p);
        let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
        let output = output(&action, &bytes);
        if close_during_open {
            assert_eq!(
                submit(&mut p, &action, output, |_, _| {
                    phase.request_close();
                    Ok(())
                })
                .err(),
                Some(VoiceError::Unauthorized)
            );
            assert!(action.terminal_status().receipt.is_none());
        } else {
            let first = submit(&mut p, &action, output, |_, _| Ok(())).unwrap();
            let delivery = first.delivery.unwrap();
            assert_eq!(
                deliver(&p, &action, &delivery, |_, _| {
                    phase.request_close();
                    Ok(())
                }),
                Err(VoiceError::Unauthorized)
            );
            assert!(!delivery.was_admitted());
            action.acknowledge_result(&first.receipt).unwrap();
        }
        assert!(
            matches!(
                owner.charge(crate::voice_v2::negotiation::Stage::Crypto, 1),
                Err(VoiceError::Capacity)
            ),
            "PC Close is not actual engine settlement"
        );
        action.settle_engine();
        phase.close();
    }
}

#[test]
fn sdp_delivery_outlives_action_and_join_with_only_plain_fixed_funding() {
    let (mut p, phase, intent) = fixture();
    let phase_weak = Arc::downgrade(&phase);
    let exchange_weak = Arc::downgrade(&pending(&p));
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let result = submit(&mut p, &action, output(&action, &bytes), |_, _| Ok(())).unwrap();
    action.acknowledge_result(&result.receipt).unwrap();
    action.settle_engine();
    let delivery = result.delivery.unwrap();
    let funding = Arc::downgrade(&action.history_funding());
    let action_weak = Arc::downgrade(&action);
    phase.close();
    drop((action, p, intent, phase));
    assert!(
        action_weak.upgrade().is_none(),
        "delivery cannot own an Action backedge"
    );
    assert!(
        phase_weak.upgrade().is_none(),
        "delivery cannot retain actual native Phase custody"
    );
    assert!(
        exchange_weak.upgrade().is_none(),
        "delivery cannot retain actual Exchange progress custody"
    );
    assert!(
        funding.upgrade().is_some(),
        "retained fixed observation stays paid"
    );
    drop(delivery);
    assert!(
        funding.upgrade().is_none(),
        "fixed fee returns at actual last observation Drop"
    );
}

#[test]
fn accepted_sdp_delivery_cannot_cross_room_incarnation_epoch_policy_or_session_replacement() {
    use crate::voice_v2::helper::RoomAuthority;
    for replace in ["room", "epoch", "policy", "session"] {
        let (mut p, phase, intent) = before_offer();
        let original_room = RoomAuthority {
            scope: p.f.high.context.scope,
            lease: CallLease::default(),
            custody: None,
        };
        let object = super::super::super::admission::offer(&p, 219, 4);
        super::super::super::admission::admit(&mut p, &object, &original_room, |_, _| Ok(()))
            .unwrap();
        let action =
            p.f.high
                .join
                .prepare_high_sdp_open(
                    &p.high,
                    &original_room,
                    p.f.id,
                    &intent,
                    &ca(&p.f.high.context, &p.f.high.lease),
                    p._runtimes[1].clock(),
                    |_, _| Ok(()),
                    None,
                )
                .unwrap();
        action.admit_engine(action.engine_bytes()).unwrap();
        let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
        let output = output(&action, &bytes);
        let result =
            p.f.high
                .join
                .submit_sdp_open_result(
                    &p.high,
                    &original_room,
                    &action,
                    output,
                    &ca(&p.f.high.context, &p.f.high.lease),
                    &p.f.clock,
                    |_, _| Ok(()),
                )
                .unwrap();
        let delivery = result.delivery.unwrap();
        action.acknowledge_result(&result.receipt).unwrap();
        action.settle_engine();
        let mut current_room = original_room.clone();
        if replace == "room" {
            original_room.lease.revoke();
            current_room.lease = CallLease::default();
        } else if replace == "session" {
            p._runtimes[1].revoke_session();
        } else {
            let mut view = View {
                epoch: p.f.high.context.epoch,
                members: [p.f.low.context.local, p.f.high.context.local].into(),
                policy: p.f.high.context.scope.policy,
                eligible: true,
                indirect: true,
            };
            if replace == "epoch" {
                view.epoch += 1;
            } else {
                let mut replacement = view.policy.unwrap_or([0; 32]);
                replacement[0] ^= 1;
                view.policy = Some(replacement);
            }
            p.high.sync_view(view).unwrap();
        }
        assert_eq!(
            p.f.high.join.admit_sdp_delivery(
                &p.high,
                &current_room,
                &action,
                &delivery,
                &ca(&p.f.high.context, &p.f.high.lease),
                |_, _| Ok(()),
            ),
            Err(VoiceError::Unauthorized),
            "{replace}"
        );
        assert!(!delivery.was_admitted());
        let recovered = action.recover_sdp_open_result(output).unwrap().unwrap();
        assert!(recovered.receipt.same_transition(&result.receipt));
        assert!(Arc::ptr_eq(&delivery, &recovered.delivery.unwrap()));
        phase.close();
    }
}
