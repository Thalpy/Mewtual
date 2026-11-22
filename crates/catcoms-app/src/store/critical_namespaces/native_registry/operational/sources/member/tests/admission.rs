//! Real OwnedMutexGuard and real runtime Notify; this tests custody ordering,
//! not native authorization (the authenticating App factory is still absent).
use super::*;
use crate::store::{MemberSaveJobReport, MemberSaveNativeAdmission};
use catcoms_rt::Clock;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

struct GuardReleased(Arc<AtomicBool>);
impl Drop for GuardReleased {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
struct Custody {
    guard: Option<tokio::sync::OwnedMutexGuard<()>>,
    _after: GuardReleased,
}
struct RuntimeWake {
    commit: Arc<tokio::sync::Mutex<()>>,
    observed: Arc<AtomicBool>,
    under_commit: Arc<AtomicBool>,
}
impl std::task::Wake for RuntimeWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if self.commit.try_lock().is_err() {
            self.under_commit.store(true, Ordering::SeqCst);
        }
        self.observed.store(true, Ordering::SeqCst);
    }
}
pub(super) fn final_preparation(
    owner: &Owner,
    row: &Row,
    vault: &NativeTextVault,
    budget: &NativeUiInvocationBudget,
) -> NativeTextMemberFinalPreparation<MeshService, ChaCha20Rng> {
    owner
        .capture_member(
            row,
            command(vault, budget),
            owner.policy.prepare_member().unwrap(),
        )
        .finish_admission([0xe7; 32])
        .unwrap()
}
pub(super) async fn custody(
    commit: &Arc<tokio::sync::Mutex<()>>,
    session: &RequestCancellation,
) -> (MemberSaveNativeAdmission, Arc<AtomicBool>) {
    let released = Arc::new(AtomicBool::new(false));
    // Actual allocation precedes guard acquisition. Filling and erasing this
    // same original box performs no new allocation at final admission.
    let mut slot = Box::new(Custody {
        guard: None,
        _after: GuardReleased(released.clone()),
    });
    slot.guard = Some(commit.clone().lock_owned().await);
    (
        MemberSaveNativeAdmission::from_boxed_custody(slot, session.clone()),
        released,
    )
}

#[tokio::test]
async fn native_runtime_member_actual_commit_releases_before_runtime_notify_on_save_or_unrun_drop()
{
    for run in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (vault, budget) = selected(directory.path());
        let (owner, row) = active(&vault, &budget, 0xed10 + u64::from(run));
        let (_sender, receiver) = tokio::sync::watch::channel(false);
        let session = RequestCancellation::new(receiver, None);
        let text = body("actual native wrapper save");
        assert_eq!(
            prepare(&owner, &row, &vault, &budget)
                .prepare_member_save(sign_request(&text), &session),
            Ok(V2MemberSaveOutcome::Ready)
        );
        let final_prepared = final_preparation(&owner, &row, &vault, &budget);
        let cell = owner.entries.lock().unwrap()[0].runtime.clone();
        let commit = Arc::new(tokio::sync::Mutex::new(()));
        let observed = Arc::new(AtomicBool::new(false));
        let under_commit = Arc::new(AtomicBool::new(false));
        let waker = std::task::Waker::from(Arc::new(RuntimeWake {
            commit: commit.clone(),
            observed: observed.clone(),
            under_commit: under_commit.clone(),
        }));
        let wait = cell.returned.notified();
        tokio::pin!(wait);
        assert!(wait
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&waker))
            .is_pending());
        let (native, released) = custody(&commit, &session).await;
        let job = final_prepared.try_admit(native).unwrap();
        assert!(!released.load(Ordering::SeqCst));
        assert!(commit.try_lock().is_err());
        assert!(cell.state.lock().unwrap().running);
        if run {
            assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
        } else {
            drop(job);
        }
        assert!(released.load(Ordering::SeqCst));
        assert!(observed.load(Ordering::SeqCst));
        assert!(
            !under_commit.load(Ordering::SeqCst),
            "actual runtime notification ran while commit remained held"
        );
        assert!(!cell.state.lock().unwrap().running);
        assert!(commit.try_lock().is_ok());
        let next = prepare(&owner, &row, &vault, &budget)
            .prepare_member_save(sign_request(&text), &session)
            .unwrap();
        if run {
            assert!(matches!(next, V2MemberSaveOutcome::Saved { .. }));
        } else {
            assert_eq!(next, V2MemberSaveOutcome::Ready);
        }
    }
}

#[tokio::test]
async fn native_runtime_member_post_borrow_refusal_keeps_guard_before_whole_runtime_tail() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xed12);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let text = body("prepared but final session cancelled");
    prepare(&owner, &row, &vault, &budget)
        .prepare_member_save(sign_request(&text), &session)
        .unwrap();
    let final_prepared = final_preparation(&owner, &row, &vault, &budget);
    let (cancel_sender, cancel_receiver) = tokio::sync::watch::channel(false);
    cancel_sender.send_replace(true);
    let cancelled = RequestCancellation::new(cancel_receiver, None);
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    let commit = Arc::new(tokio::sync::Mutex::new(()));
    let observed = Arc::new(AtomicBool::new(false));
    let under_commit = Arc::new(AtomicBool::new(false));
    let waker = std::task::Waker::from(Arc::new(RuntimeWake {
        commit: commit.clone(),
        observed: observed.clone(),
        under_commit: under_commit.clone(),
    }));
    let wait = cell.returned.notified();
    tokio::pin!(wait);
    assert!(wait
        .as_mut()
        .poll(&mut std::task::Context::from_waker(&waker))
        .is_pending());
    let (native, released) = custody(&commit, &cancelled).await;
    let refused = final_prepared.try_admit(native).unwrap_err();
    assert_eq!(refused.refusal(), V2MemberSaveRefusal::Cancelled);
    assert!(
        cell.state.lock().unwrap().running,
        "refusal retains the original whole loan"
    );
    assert!(!observed.load(Ordering::SeqCst));
    assert!(!released.load(Ordering::SeqCst));
    drop(refused);
    assert!(released.load(Ordering::SeqCst));
    assert!(observed.load(Ordering::SeqCst));
    assert!(!under_commit.load(Ordering::SeqCst));
    assert!(!cell.state.lock().unwrap().running);
    assert_eq!(
        prepare(&owner, &row, &vault, &budget)
            .prepare_member_save(sign_request(&text), &session)
            .unwrap(),
        V2MemberSaveOutcome::Ready
    );
}

#[tokio::test]
async fn native_runtime_member_completed_file_custody_is_not_whole_runtime_retirement() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xed15);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let text = body("file complete, runtime not yet restored");
    prepare(&owner, &row, &vault, &budget)
        .prepare_member_save(sign_request(&text), &session)
        .unwrap();
    let final_prepared = final_preparation(&owner, &row, &vault, &budget);
    let commit = Arc::new(tokio::sync::Mutex::new(()));
    let (native, released) = custody(&commit, &session).await;
    let job = final_prepared.try_admit(native).unwrap();
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    let at_cell = cell.clone();
    // Test-only canonical coordinator. The production file wrapper itself
    // contains no Sources, native controller or issuer backreference.
    let coordinator = Arc::new(Mutex::new(owner));
    let at_commit = coordinator.clone();
    let checked = Arc::new(AtomicBool::new(false));
    let mark = checked.clone();
    AFTER_COMMIT.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(move || {
            assert!(released.load(Ordering::SeqCst));
            {
                let state = at_cell.state.lock().unwrap();
                assert!(
                    state.running,
                    "the actual original loan is still outstanding"
                );
                assert!(
                    state.body.is_none(),
                    "the whole body is still owned by that loan"
                );
            }
            assert!(
                commit.try_lock().is_ok(),
                "actual native custody has already released"
            );
            let owner = at_commit.lock().unwrap();
            let mut retirement = owner.prepare_retirement().unwrap();
            assert_eq!(
                retirement.try_retire().unwrap(),
                NativeTextSourceRetirementStatus::Pending,
                "actual running loan/preparation still prevents whole source completion"
            );
            mark.store(true, Ordering::SeqCst);
        }));
    });
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    assert!(checked.load(Ordering::SeqCst));
    {
        let state = cell.state.lock().unwrap();
        assert!(
            !state.running,
            "actual loan returned after file-custody release"
        );
        assert!(state.body.is_some());
    }
    let mut retirement = coordinator.lock().unwrap().prepare_retirement().unwrap();
    let mut drained = false;
    for _ in 0..1000 {
        if retirement.try_retire().unwrap() == NativeTextSourceRetirementStatus::Drained {
            drained = true;
            break;
        }
        catcoms_rt::SystemClock
            .sleep(std::time::Duration::from_millis(10))
            .await;
    }
    assert!(
        drained,
        "same actual runtime completes after restoration and TCP teardown"
    );
}

#[tokio::test]
async fn native_runtime_member_busy_original_cell_keeps_upgraded_reference_behind_native_custody() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xed16);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let text = body("actual original writer is busy");
    prepare(&owner, &row, &vault, &budget)
        .prepare_member_save(sign_request(&text), &session)
        .unwrap();
    let final_prepared = final_preparation(&owner, &row, &vault, &budget);
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    let original_references = Arc::strong_count(&cell);
    let commit = Arc::new(tokio::sync::Mutex::new(()));
    let (native, released) = custody(&commit, &session).await;
    let state = cell.state.lock().unwrap();
    let refused = final_prepared.try_admit(native).unwrap_err();
    assert_eq!(refused.refusal(), V2MemberSaveRefusal::Busy);
    // Observe the actual original allocation's reference count rather than
    // exposing the failure frame's private strong-cell field to sibling code.
    assert_eq!(Arc::strong_count(&cell), original_references + 1);
    assert!(!state.running, "busy refusal never borrowed the whole body");
    assert!(state.body.is_some());
    drop(state);
    assert!(commit.try_lock().is_err());
    assert!(!released.load(Ordering::SeqCst));
    drop(refused);
    assert!(released.load(Ordering::SeqCst));
    assert!(commit.try_lock().is_ok());
    assert_eq!(Arc::strong_count(&cell), original_references);
    // Canonical Entry retention remains intact here. This case proves the
    // successful-upgrade/refusal branch, not last-cell or whole-source erasure.
}

#[tokio::test]
async fn native_runtime_member_missing_original_cell_returns_whole_native_custody() {
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xed14);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let text = body("missing original evidence");
    prepare(&owner, &row, &vault, &budget)
        .prepare_member_save(sign_request(&text), &session)
        .unwrap();
    let final_prepared = final_preparation(&owner, &row, &vault, &budget);
    // Deliberately sever canonical retention to exercise UNKNOWN evidence.
    // This is not a permitted row-retirement procedure or a shutdown proof.
    let removed = owner.entries.lock().unwrap().pop().unwrap();
    drop(removed); // actual SDK/cell destruction BEFORE taking commit
    assert!(row.original.runtime.upgrade().is_none());
    let commit = Arc::new(tokio::sync::Mutex::new(()));
    let (native, released) = custody(&commit, &session).await;
    let refused = final_prepared.try_admit(native).unwrap_err();
    assert_eq!(refused.refusal(), V2MemberSaveRefusal::Unavailable);
    assert!(commit.try_lock().is_err());
    assert!(!released.load(Ordering::SeqCst));
    drop(refused);
    assert!(released.load(Ordering::SeqCst));
    assert!(commit.try_lock().is_ok());
}

#[tokio::test]
async fn native_runtime_member_commit_unwind_releases_native_before_actual_loan_restoration() {
    use crate::store::persistence::v2_member_outbox::{
        member_outbox_call_at_for_test, member_outbox_disarm_for_test, MemberOutboxTestCut,
    };
    let directory = tempfile::tempdir().unwrap();
    let (vault, budget) = selected(directory.path());
    let (owner, row) = active(&vault, &budget, 0xed13);
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let session = RequestCancellation::new(receiver, None);
    let text = body("unwind after actual commit point");
    prepare(&owner, &row, &vault, &budget)
        .prepare_member_save(sign_request(&text), &session)
        .unwrap();
    let final_prepared = final_preparation(&owner, &row, &vault, &budget);
    let cell = owner.entries.lock().unwrap()[0].runtime.clone();
    let commit = Arc::new(tokio::sync::Mutex::new(()));
    let observed = Arc::new(AtomicBool::new(false));
    let under_commit = Arc::new(AtomicBool::new(false));
    let waker = std::task::Waker::from(Arc::new(RuntimeWake {
        commit: commit.clone(),
        observed: observed.clone(),
        under_commit: under_commit.clone(),
    }));
    let wait = cell.returned.notified();
    tokio::pin!(wait);
    assert!(wait
        .as_mut()
        .poll(&mut std::task::Context::from_waker(&waker))
        .is_pending());
    let (native, released) = custody(&commit, &session).await;
    let job = final_prepared.try_admit(native).unwrap();
    member_outbox_call_at_for_test(
        MemberOutboxTestCut::FirstOutput,
        Box::new(|| panic!("actual member commit unwind")),
    );
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run()));
    member_outbox_disarm_for_test();
    assert!(result.is_err());
    assert!(released.load(Ordering::SeqCst));
    assert!(observed.load(Ordering::SeqCst));
    assert!(!under_commit.load(Ordering::SeqCst));
    assert!(!cell.state.lock().unwrap().running);
    assert!(matches!(
        prepare(&owner, &row, &vault, &budget).prepare_member_save(sign_request(&text), &session),
        Ok(V2MemberSaveOutcome::Saved { .. })
    ));
}
