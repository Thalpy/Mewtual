//! Pin callback-free Engine/Group/Policy consumption on an actual typed native action.
//! The terminal payload is synthetic here; full Open/token/delivery has separate coverage.
use super::*;
use crate::voice_v2::crypto_actions::{CryptoAction, CryptoOutcome, TerminalResult};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex, Weak,
};

#[derive(Debug)]
struct ReenterClock {
    original: Arc<dyn Clock>,
    action: Mutex<Weak<CryptoAction>>,
    armed: AtomicBool,
    checks: AtomicUsize,
}
impl ReenterClock {
    fn check(&self) {
        if self.armed.swap(false, Ordering::AcqRel) {
            let action = self.action.lock().unwrap().upgrade().unwrap();
            assert!(
                action.result_fence_available_for_test(),
                "Clock callback ran beneath Engine"
            );
            // Reenter the actual historical API too. Its own Clock sees the already
            // disarmed probe, avoiding recursion without inventing a separate time origin.
            action.handoff_status();
            self.checks.fetch_add(1, Ordering::AcqRel);
        }
    }
}
impl Clock for ReenterClock {
    fn now_ms(&self) -> u64 {
        self.check();
        self.original.now_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        self.check();
        self.original.monotonic_ms()
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        self.original.sleep(duration)
    }
}

#[test]
fn typed_sdp_result_publication_and_terminal_recovery_never_sample_clock_under_engine() {
    let (mut p, phase, intent) = fixture();
    let clock = Arc::new(ReenterClock {
        original: p._runtimes[1].clock(),
        action: Mutex::new(Weak::new()),
        armed: AtomicBool::new(false),
        checks: AtomicUsize::new(0),
    });
    let action =
        p.f.high
            .join
            .prepare_high_sdp_open(
                &p.high,
                &room(&p.f.high),
                p.f.id,
                &intent,
                &ca(&p.f.high.context, &p.f.high.lease),
                clock.clone(),
                |_, _| Ok(()),
                None,
            )
            .unwrap();
    *clock.action.lock().unwrap() = Arc::downgrade(&action);
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let tag = action.output_tag(&bytes).unwrap();
    let room = room(&p.f.high);
    let original = p
        .high
        .prepare_local_origin(p.f.low.context.local, &p.f.high.context)
        .unwrap();
    clock.armed.store(true, Ordering::Release);
    let observed = action.observe_sdp_result(&bytes, tag).unwrap();
    assert_eq!(clock.checks.load(Ordering::Acquire), 1);
    assert!(observed.recover().unwrap().is_none());
    let binding = p.f.high.join.result_binding(p.f.id, 16000, 16000);
    clock.armed.store(true, Ordering::Release);
    let receipt = observed
        .commit(|own_mono, own_wall| {
            original.with_origin(|wall, mono, origin| {
                assert!(room.lease.is_live());
                action.check_pending_at(own_mono, own_wall)?;
                assert!(intent.current_at(wall, mono));
                origin.publish(|| ())
            })?;
            Ok(TerminalResult {
                binding,
                outcome: CryptoOutcome::Opened,
            })
        })
        .unwrap();
    assert!(
        clock.armed.load(Ordering::Acquire),
        "both native tail and receipt use captured clocks"
    );
    action.acknowledge_result(&receipt).unwrap();
    action.settle_engine();
    assert!(action.input().is_err());
    phase.close();
    let observed = action.observe_sdp_result(&bytes, tag).unwrap();
    assert_eq!(clock.checks.load(Ordering::Acquire), 2);
    assert!(observed.recover().unwrap().unwrap().retired);
    clock.armed.store(true, Ordering::Release);
    let retry = observed
        .commit(|_, _| panic!("exact terminal never repeats a transition"))
        .unwrap();
    assert!(retry.same_transition(&receipt));
    assert!(retry.retired);
    assert!(
        clock.armed.load(Ordering::Acquire),
        "terminal-hit receipt is also callback-free"
    );
    // The public native recovery API must obey the same rule after Root/input retirement.
    let retry = action.recover_result(&bytes, tag).unwrap().unwrap();
    assert!(retry.same_transition(&receipt));
    assert!(retry.retired);
    assert_eq!(clock.checks.load(Ordering::Acquire), 3);
}

#[test]
fn typed_sdp_observation_refuses_a_held_engine_without_waiting_or_consuming_token() {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let observed = action
        .observe_sdp_result(&bytes, action.output_tag(&bytes).unwrap())
        .unwrap();
    // with_pending holds the actual Engine through this callback; both new operations
    // must refuse immediately, rather than block on themselves or apply a first result.
    action
        .with_pending(|| {
            assert_eq!(observed.recover().err(), Some(VoiceError::Capacity));
            assert_eq!(
                observed
                    .commit(|_, _| panic!("busy result cannot publish"))
                    .err(),
                Some(VoiceError::Capacity)
            );
            Ok(())
        })
        .unwrap();
    assert!(action.terminal_status().receipt.is_none());
    action.settle_engine();
    phase.close();
}
