//! Metadata-only recovery uses the same real terminal action and its actual observer.
//! No browser plaintext, replacement receipt or new delivery allocation is supplied.
use super::*;

#[test]
fn native_open_history_recovers_both_terminal_outcomes_after_actual_input_and_output_drop() {
    for accepted in [false, true] {
        let (mut p, phase, intent) = fixture();
        let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
        assert!(action.recover_sdp_open_history().unwrap().is_none());
        action.admit_engine(action.engine_bytes()).unwrap();
        let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
        let result = submit(
            &mut p,
            &action,
            if accepted {
                output(&action, &bytes)
            } else {
                SdpOpenOutput::AuthenticationFailed
            },
            |_, _| Ok(()),
        )
        .unwrap();
        let original = result.receipt.clone();
        let delivery = result.delivery.as_ref().map(Arc::downgrade);
        action.acknowledge_result(&original).unwrap();
        action.settle_engine();
        drop((result, bytes));
        assert!(action.input().is_err());
        phase.close();
        let recovered = action.recover_sdp_open_history().unwrap().unwrap();
        assert!(recovered.receipt.same_transition(&original));
        assert!(recovered.receipt.retired);
        match (accepted, delivery, recovered.delivery) {
            (true, Some(original), Some(same)) => {
                assert!(Arc::ptr_eq(&original.upgrade().unwrap(), &same));
                assert!(
                    !same.was_admitted(),
                    "history does not grant first delivery"
                );
                assert_eq!(
                    deliver(&p, &action, &same, |_, _| Ok(())),
                    Err(VoiceError::Unauthorized)
                );
            }
            (false, None, None) => {}
            _ => panic!("terminal kind must preserve its actual delivery presence"),
        }
    }
}

#[test]
fn native_open_history_observes_original_clock_before_engine_lock() {
    let (mut p, phase, intent) = fixture();
    let clock = Arc::new(BarrierClock {
        original: p._runtimes[1].clock(),
        reads: AtomicUsize::new(0),
        trigger: AtomicUsize::new(usize::MAX),
        hook: Mutex::new(None),
    });
    let action = with_clock(&mut p, &intent, clock.clone());
    action.admit_engine(action.engine_bytes()).unwrap();
    let bytes = vec![b'a'; MAX_SDP_PLAINTEXT];
    let result = submit(&mut p, &action, output(&action, &bytes), |_, _| Ok(())).unwrap();
    let observed = result.delivery.unwrap();
    let same = action.clone();
    let delivery = observed.clone();
    *clock.hook.lock().unwrap() = Some(Box::new(move || {
        assert!(
            same.owns_sdp_delivery(&delivery).unwrap(),
            "Clock must not run beneath Engine"
        );
    }));
    clock
        .trigger
        .store(clock.reads.load(Ordering::Acquire) + 1, Ordering::Release);
    let history = action.recover_sdp_open_history().unwrap().unwrap();
    assert!(Arc::ptr_eq(&observed, &history.delivery.unwrap()));
    assert!(
        clock.hook.lock().unwrap().is_none(),
        "actual original Clock callback reached"
    );
    action.acknowledge_result(&result.receipt).unwrap();
    action.settle_engine();
    phase.close();
}

#[test]
fn native_open_history_busy_engine_is_capacity_not_absence() {
    let (mut p, phase, intent) = fixture();
    let action = open(&mut p, &intent, |_, _| Ok(())).unwrap();
    action.admit_engine(action.engine_bytes()).unwrap();
    // with_pending holds the ACTUAL Engine fence, without publishing a fake terminal.
    action
        .with_pending(|| {
            assert_eq!(
                action.recover_sdp_open_history().err(),
                Some(VoiceError::Capacity)
            );
            Ok(())
        })
        .unwrap();
    assert!(action.recover_sdp_open_history().unwrap().is_none());
    action.settle_engine();
    phase.close();
}
