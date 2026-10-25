//! Actual original reservation/Copy histories. No synthetic counters/receipt booleans.
use super::super::*;
use super::fixture;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

#[test]
fn original_plain_native_fees_recover_under_pressure_and_survive_phase_destruction() {
    let (resources, scope, _, phase, alive) = fixture();
    phase.arm().unwrap();
    let seal = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let open = phase.native_history(NativeRowPurpose::Open).unwrap();
    assert!(!Arc::ptr_eq(&seal, &open));
    assert_eq!(seal.purpose(), NativeRowPurpose::Seal);
    assert_eq!(open.purpose(), NativeRowPurpose::Open);
    assert!(seal.uses_phase(&phase));
    let pressure = resources
        .charge(
            scope.group,
            crate::voice_v2::resources::GROUP_BYTES - resources.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    for _ in 0..32 {
        assert!(Arc::ptr_eq(
            &seal,
            &phase.native_history(NativeRowPurpose::Seal).unwrap()
        ));
        assert!(Arc::ptr_eq(
            &open,
            &phase.native_history(NativeRowPurpose::Open).unwrap()
        ));
    }
    phase.close();
    assert_eq!(phase.status(), Status::Drained);
    assert!(seal.is_collected() && open.is_collected());
    assert!(Arc::ptr_eq(
        &seal,
        &phase.native_history(NativeRowPurpose::Seal).unwrap()
    ));
    let weak = Arc::downgrade(&phase);
    drop(phase);
    assert!(
        weak.upgrade().is_none(),
        "fixed history must not retain Phase"
    );
    drop(pressure);
    assert_eq!(
        resources.counts().0,
        alive.baseline + METADATA + 2 * NATIVE_ROW_BYTES
    );
    drop(seal);
    assert_eq!(
        resources.counts().0,
        alive.baseline + METADATA + NATIVE_ROW_BYTES
    );
    drop(open);
    assert_eq!(
        resources.counts(),
        (alive.baseline - super::super::super::METADATA_BYTES, 0)
    );
    drop(alive);
    assert_eq!(resources.counts(), (0, 0));
}

#[test]
fn native_history_refuses_foreign_repeated_and_closed_binding_without_losing_original_copies() {
    let (_, _, _, phase, _alive) = fixture();
    phase.arm().unwrap();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let other = Phase::prepare(phase.effect.original.clone()).unwrap();
    other.arm().unwrap();
    let foreign = other.native_history(NativeRowPurpose::Seal).unwrap();
    let mut copy = phase.copy(32).unwrap();
    assert_eq!(
        copy.bind_native_history(foreign.clone()),
        Err(VoiceError::Unauthorized)
    );
    assert_eq!(fee.in_flight(), 0);
    assert_eq!(foreign.in_flight(), 0);
    copy.bind_native_history(fee.clone()).unwrap();
    assert!(!fee.is_closed());
    assert_eq!(
        copy.bind_native_history(fee.clone()),
        Err(VoiceError::RetryMismatch)
    );
    assert_eq!(fee.in_flight(), 1);
    let mut task = phase.task_copy(1024).unwrap();
    task.bind_native_history(fee.clone()).unwrap();
    let mut late = phase.copy(32).unwrap();
    drop(fee.close_io());
    assert!(fee.is_closed());
    assert!(!fee.is_collected(), "closed still retains actual IO");
    assert_eq!(
        late.bind_native_history(fee.clone()),
        Err(VoiceError::Unauthorized)
    );
    let mut late = late.adopt(vec![5; 32]).unwrap();
    assert_eq!(late.bytes(), &[5; 32]);
    // Closing one native row cannot close the shared phase or its opposite action.
    assert_eq!(phase.status(), Status::Open);
    let open = phase.native_history(NativeRowPurpose::Open).unwrap();
    late.bind_native_history(open.clone()).unwrap();
    assert_eq!(open.in_flight(), 1);
    drop(copy);
    assert!(
        !fee.is_collected(),
        "actual worker TaskCopy is still outstanding"
    );
    drop(task);
    assert!(fee.is_collected());
    assert!(fee.is_closed(), "collection cannot reopen admission");
    assert!(
        !open.is_collected(),
        "zero/open is not stable terminal collection"
    );
    phase.close();
    assert_eq!(phase.status(), Status::Closing);
    drop(late);
    assert_eq!(phase.status(), Status::Drained);
    assert!(open.is_collected());
    other.close();
}

#[test]
fn copy_slot_release_cannot_overfill_native_io_during_the_counter_retirement_gap() {
    let (_, _, _, phase, _alive) = fixture();
    phase.arm().unwrap();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let mut copies = Vec::new();
    for _ in 0..COPIES {
        let mut copy = phase.copy(1).unwrap();
        copy.bind_native_history(fee.clone()).unwrap();
        copies.push(copy);
    }
    let claimed = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    let mut first = copies.pop().unwrap();
    let worker = {
        let claimed = claimed.clone();
        let release = release.clone();
        std::thread::spawn(move || {
            first.retire(|| {
                claimed.wait();
                release.wait();
            })
        })
    };
    claimed.wait();
    assert_eq!(fee.in_flight(), COPIES);
    let mut replacement = phase.copy(1).unwrap().adopt(vec![7]).unwrap();
    assert_eq!(
        replacement.bind_native_history(fee.clone()),
        Err(VoiceError::Capacity)
    );
    assert_eq!(
        replacement.bytes(),
        &[7],
        "same paid buffer remains usable after refusal"
    );
    assert_eq!(fee.in_flight(), COPIES);
    release.wait();
    worker.join().unwrap();
    replacement.bind_native_history(fee.clone()).unwrap();
    assert_eq!(fee.in_flight(), COPIES);
    phase.close();
    drop(copies);
    assert_eq!(fee.in_flight(), 1);
    assert!(!fee.is_collected());
    drop(replacement);
    assert!(fee.is_collected());
    assert_eq!(phase.status(), Status::Drained);
}

#[test]
fn history_recovery_cannot_allocate_a_missing_purpose_after_phase_close() {
    let (resources, _, _, phase, _alive) = fixture();
    phase.arm().unwrap();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    phase.close();
    let before = resources.counts();
    assert_eq!(
        phase.native_history(NativeRowPurpose::Open).err(),
        Some(VoiceError::Unauthorized)
    );
    assert_eq!(resources.counts(), before);
    assert!(Arc::ptr_eq(
        &fee,
        &phase.native_history(NativeRowPurpose::Seal).unwrap()
    ));
}

#[test]
fn publication_unwind_still_retires_actual_credit_phase_and_native_io_in_order() {
    #[derive(Debug)]
    struct Panics;
    impl Drop for Panics {
        fn drop(&mut self) {
            panic!("trusted publication destructor unwind");
        }
    }
    let (resources, _, _, phase, alive) = fixture();
    phase.arm().unwrap();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let mut copy = phase.copy(1024).unwrap();
    copy.bind_native_history(fee.clone()).unwrap();
    copy.attach_keepalive(Arc::new(Panics)).unwrap();
    phase.close();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(copy))).is_err());
    assert_eq!(phase.status(), Status::Drained);
    assert!(fee.is_collected());
    assert_eq!(
        resources.counts().0,
        alive.baseline + METADATA + NATIVE_ROW_BYTES
    );
}

#[test]
fn first_native_fee_capacity_refusal_preserves_reservation_and_can_retry_once() {
    let (resources, _, _, phase, alive) = fixture();
    phase.arm().unwrap();
    let held = phase
        .copy(BYTES - METADATA - COPY_METADATA - (NATIVE_ROW_BYTES - 1))
        .unwrap();
    let before = resources.counts();
    assert_eq!(
        phase.native_history(NativeRowPurpose::Seal).err(),
        Some(VoiceError::Capacity)
    );
    assert!(phase.state.lock().unwrap().history[0].is_none());
    assert_eq!(resources.counts(), before);
    drop(held);
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    assert!(Arc::ptr_eq(
        &fee,
        &phase.native_history(NativeRowPurpose::Seal).unwrap()
    ));
    phase.close();
    assert_eq!(
        resources.counts().0,
        alive.baseline + METADATA + NATIVE_ROW_BYTES
    );
    assert!(phase.try_is_collected().unwrap());
}

#[test]
fn native_whole_collection_cannot_overtake_the_final_counter_after_storage_drains() {
    let (_, _, _, phase, _alive) = fixture();
    phase.arm().unwrap();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let mut copy = phase.copy(32).unwrap();
    copy.bind_native_history(fee.clone()).unwrap();
    let claimed = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    let worker = {
        let claimed = claimed.clone();
        let release = release.clone();
        std::thread::spawn(move || {
            copy.retire(|| {
                claimed.wait();
                release.wait();
            })
        })
    };
    claimed.wait();
    phase.close();
    assert_eq!(phase.status(), Status::Drained);
    assert!(!fee.is_collected());
    assert!(
        !phase.try_is_collected().unwrap(),
        "whole native Final must still wait"
    );
    release.wait();
    worker.join().unwrap();
    assert!(phase.try_is_collected().unwrap());
}

struct CountWake {
    wakes: Arc<AtomicUsize>,
    phase: std::sync::Weak<Phase>,
}
impl CountWake {
    fn observe(&self) {
        let phase = self.phase.upgrade().unwrap();
        assert!(
            phase.state.try_lock().is_ok(),
            "actual notify callback beneath Phase guard"
        );
        self.wakes.fetch_add(1, Ordering::AcqRel);
    }
}
impl Wake for CountWake {
    fn wake(self: Arc<Self>) {
        self.observe();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.observe();
    }
}
#[test]
fn native_zero_and_last_copy_notifications_register_before_inspection_and_leave_phase_unlocked() {
    let (_, _, _, phase, _alive) = fixture();
    phase.arm().unwrap();
    let fee = phase.native_history(NativeRowPurpose::Seal).unwrap();
    let wakes = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(CountWake {
        wakes: wakes.clone(),
        phase: Arc::downgrade(&phase),
    }));
    let mut cx = Context::from_waker(&waker);
    let mut waiting = Box::pin(fee.wait_collected());
    assert!(waiting.as_mut().poll(&mut cx).is_pending());
    let closure = fee.close_io();
    assert_eq!(
        wakes.load(Ordering::Acquire),
        0,
        "negative fence itself runs no callback"
    );
    drop(closure);
    assert!(wakes.load(Ordering::Acquire) > 0);
    assert_eq!(waiting.as_mut().poll(&mut cx), Poll::Ready(()));
    drop(waiting);

    let open = phase.native_history(NativeRowPurpose::Open).unwrap();
    let mut copy = phase.copy(32).unwrap();
    copy.bind_native_history(open.clone()).unwrap();
    phase.close();
    let mut waiting = Box::pin(open.wait_collected());
    assert!(waiting.as_mut().poll(&mut cx).is_pending());
    let before = wakes.load(Ordering::Acquire);
    drop(copy);
    assert!(phase.state.try_lock().is_ok());
    assert!(wakes.load(Ordering::Acquire) > before);
    assert_eq!(waiting.as_mut().poll(&mut cx), Poll::Ready(()));
    assert_eq!(phase.status(), Status::Drained);
}
