//! Real issuer/UI/Actor/native activation, then focused key bookkeeping. Root
//! terminal refusals below are explicit bookkeeping inputs, not successful key
//! publication/SDK/RTC evidence. The primary publication suite owns those paths.
use super::*;
use crate::voice_activation_tests::Fixture;
use crate::voice_call_activation::tests::active_handle_fixture;
use crate::voice_call_activation::Handle;
use catcoms_rt::Clock;
use catcoms_rt::ManualClock;
use catcoms_sync::voice_v2::resources::{GROUP_BYTES, PROCESS_BYTES};
use catcoms_sync::voice_v2::VoiceError;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex as StdMutex, Weak};
use std::task::{Context, Poll, Wake, Waker};

type Runner = Pin<Box<dyn Future<Output = ()> + Send>>;

fn point() -> [u8; 65] {
    hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5")
        .unwrap()
        .try_into()
        .unwrap()
}

async fn active() -> (Fixture, Handle, Runner) {
    let fixture = Fixture::with_voice_clock(Arc::new(ManualClock::new(1000))).await;
    let (handle, runner) = active_handle_fixture(&fixture).await;
    (fixture, handle, runner)
}

async fn close(fixture: Fixture, handle: Handle, runner: Runner) {
    handle.close();
    drop(runner); // joins its original execution allocation; no task is detached
    let (settled, retired) = {
        let mut authority = fixture.state.voice_authority.lock().unwrap();
        authority.end(handle.id());
        let settled = authority.settle_admitted_join_key(handle.id(), [9; 32]);
        let retired = authority.take_retired_admitted_join_key(handle.id());
        (settled, retired)
    };
    if let Ok(settled) = settled {
        // A deliberately unfinished fixture Work may still own publication. Do
        // not invent its completion just to make fixture teardown look drained.
        drop(settled.finish_outside_guard());
    }
    drop(retired);
    drop(handle);
    fixture.close().await;
}

fn prepare(fixture: &Fixture, handle: &Handle) -> (JoinKeyAdmission, AdmittedPublicationWork) {
    let mut authority = fixture.state.voice_authority.lock().unwrap();
    let admission = authority.admit_admitted_join_key(handle, [9; 32]).unwrap();
    let AdmittedPublicationPlan::Start(work) = authority
        .prepare_admitted_join_key_publication(handle, [9; 32], &admission.permit, point())
        .unwrap()
    else {
        panic!("first admitted publication");
    };
    (admission, work)
}

#[test]
fn admitted_row_watch_aliases_and_shared_index_have_preallocation_guards() {
    let row = admitted_generation_layout_bytes();
    let index = admitted_join_index_layout_bytes();
    eprintln!(
        "admitted generation {row}/{ADMITTED_GENERATION_BYTES}; shared key index {index}/{JOIN_INDEX_BYTES}"
    );
    assert!(row <= ADMITTED_GENERATION_BYTES);
    assert!(index <= JOIN_INDEX_BYTES);
}

#[tokio::test]
async fn admitted_lost_admission_recovers_original_under_full_group_pressure() {
    let (fixture, handle, runner) = active().await;
    let resources = crate::voice_runtime(&fixture.state).resources();
    let group = handle.binding().scope.group;
    let before = resources.counts();
    let before_group = resources.group_counts(group).unwrap();
    let first = fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admit_admitted_join_key(&handle, [9; 32])
        .unwrap();
    assert_eq!(first.kind, "admitted");
    let charged = resources.counts();
    assert_eq!(
        charged.0,
        before.0 + JOIN_INDEX_BYTES + ADMITTED_GENERATION_BYTES + GENERATE_BYTES
    );
    let charged_group = resources.group_counts(group).unwrap();
    assert_eq!(
        charged_group.0,
        before_group.0 + ADMITTED_GENERATION_BYTES + GENERATE_BYTES
    );
    assert_eq!(charged_group.1, before_group.1 + 1);
    let serial = fixture.state.voice_authority.lock().unwrap().serial;
    let pressure = resources
        .charge(group, GROUP_BYTES - charged_group.0, ResourceKind::Retained)
        .unwrap();
    let full = resources.counts();
    for _ in 0..8 {
        let mut authority = fixture.state.voice_authority.lock().unwrap();
        let retry = authority.admit_admitted_join_key(&handle, [9; 32]).unwrap();
        assert_eq!(
            (retry.permit, retry.join, retry.kind),
            (first.permit.clone(), first.join.clone(), "admitted")
        );
        assert_eq!(authority.serial, serial);
        let refused = authority.admit_admitted_join_key(&handle, [8; 32]).unwrap();
        assert_eq!(refused.kind, "refused");
        assert!(refused.permit.is_empty() && refused.join.is_empty());
    }
    assert_eq!(resources.counts(), full);
    drop(pressure);
    close(fixture, handle, runner).await;
}

#[tokio::test]
async fn old_generation_and_pilot_apis_cannot_adopt_genuine_admitted_activation() {
    let (fixture, handle, runner) = active().await;
    let resources = crate::voice_runtime(&fixture.state).resources();
    let before = resources.counts();
    {
        let mut authority = fixture.state.voice_authority.lock().unwrap();
        assert!(authority.admit_join_key(handle.id(), [9; 32]).is_err());
        assert!(authority
            .prepare_join_key_publication(handle.id(), [9; 32], "absent", point())
            .is_err());
        assert!(authority
            .selected_handoff_owner(handle.id(), [9; 32])
            .is_err());
        assert!(authority
            .settle_join_key(handle.id(), [9; 32])
            .unwrap()
            .is_none());
        assert!(authority.join_generations.is_empty());
        assert!(authority.join_index_funding.is_none());
    }
    assert_eq!(resources.counts(), before);
    let first = fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admit_admitted_join_key(&handle, [9; 32])
        .unwrap();
    let before = resources.counts();
    {
        let mut authority = fixture.state.voice_authority.lock().unwrap();
        assert!(authority.admit_join_key(handle.id(), [9; 32]).is_err());
        assert!(authority
            .prepare_join_key_publication(handle.id(), [9; 32], &first.permit, point())
            .is_err());
        assert!(authority.settle_join_key(handle.id(), [9; 32]).is_err());
        assert!(authority
            .selected_handoff_owner(handle.id(), [9; 32])
            .is_err());
        assert_eq!(
            authority.join_generations[handle.id()].profile,
            GenerationProfile::Admitted
        );
        assert!(!authority.join_generations[handle.id()].settlement_requested);
    }
    assert_eq!(resources.counts(), before);
    close(fixture, handle, runner).await;
}

#[tokio::test]
async fn admitted_settlement_before_admission_never_rearms_original_key_generation() {
    let (fixture, handle, runner) = active().await;
    let ticket = fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .settle_admitted_join_key(handle.id(), [9; 32])
        .unwrap();
    assert!(ticket.finish_outside_guard().is_none());
    let baseline = crate::voice_runtime(&fixture.state).resources().counts();
    let serial = fixture.state.voice_authority.lock().unwrap().serial;
    for _ in 0..4 {
        let mut authority = fixture.state.voice_authority.lock().unwrap();
        assert_eq!(
            authority
                .admit_admitted_join_key(&handle, [9; 32])
                .unwrap()
                .kind,
            "closed"
        );
        assert_eq!(
            authority
                .admit_admitted_join_key(&handle, [8; 32])
                .unwrap()
                .kind,
            "refused"
        );
        assert!(authority
            .settle_admitted_join_key(handle.id(), [8; 32])
            .is_err());
        assert_eq!(authority.serial, serial);
    }
    assert_eq!(
        crate::voice_runtime(&fixture.state).resources().counts(),
        baseline
    );
    close(fixture, handle, runner).await;
}

struct Probe {
    authority: Weak<StdMutex<Coordinator>>,
    wakes: AtomicUsize,
}
impl Wake for Probe {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if let Some(authority) = self.authority.upgrade() {
            assert!(
                !matches!(
                    authority.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ),
                "admitted key watch woke while Coordinator was held"
            );
        }
        self.wakes.fetch_add(1, Ordering::AcqRel);
    }
}

#[tokio::test]
async fn admitted_known_completion_defers_watch_and_retains_original_fee_through_drain() {
    for drop_completion in [false, true] {
        let (fixture, handle, runner) = active().await;
        let (admission, mut first) = prepare(&fixture, &handle);
        let refused = Err(VoiceError::Capacity);
        let completion = fixture
            .state
            .voice_authority
            .lock()
            .unwrap()
            .finish_admitted_join_key_publication(&mut first, &refused)
            .unwrap();
        assert!(completion.finish_outside_guard().is_err());
        let mut next = {
            let mut authority = fixture.state.voice_authority.lock().unwrap();
            let AdmittedPublicationPlan::Start(next) = authority
                .prepare_admitted_join_key_publication(&handle, [9; 32], &admission.permit, point())
                .unwrap()
            else {
                panic!("retry same frozen key");
            };
            assert!(authority
                .finish_admitted_join_key_publication(&mut first, &refused)
                .is_err());
            assert!(authority.join_generations[handle.id()].publishing);
            next
        };
        let ticket = fixture
            .state
            .voice_authority
            .lock()
            .unwrap()
            .settle_admitted_join_key(handle.id(), [9; 32])
            .unwrap();
        let drain = ticket.finish_outside_guard().unwrap();
        let probe = Arc::new(Probe {
            authority: Arc::downgrade(&fixture.state.voice_authority),
            wakes: AtomicUsize::new(0),
        });
        let waker = Waker::from(probe.clone());
        let mut waiter = Box::pin(drain.wait());
        assert!(waiter
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending());
        let resources = crate::voice_runtime(&fixture.state).resources();
        let charged = resources.counts();
        let completion = {
            let mut authority = fixture.state.voice_authority.lock().unwrap();
            let completion = authority
                .finish_admitted_join_key_publication(&mut next, &refused)
                .unwrap();
            assert_eq!(probe.wakes.load(Ordering::Acquire), 0);
            assert_eq!(
                resources.counts(),
                charged,
                "actual owned engine credit lives in deferred ticket"
            );
            completion
        };
        if drop_completion {
            drop(completion);
        } else {
            assert!(completion.finish_outside_guard().is_err());
        }
        assert!(probe.wakes.load(Ordering::Acquire) > 0);
        assert!(matches!(
            waiter.as_mut().poll(&mut Context::from_waker(&waker)),
            Poll::Ready(Ok(()))
        ));
        drop(waiter);
        assert_eq!(resources.counts().0, charged.0 - GENERATE_BYTES);
        drop(first);
        drop(next);
        drop(probe);
        close(fixture, handle, runner).await;
    }
}

#[tokio::test]
async fn full_process_refusal_does_not_create_admitted_row_or_key_permit() {
    let (fixture, handle, runner) = active().await;
    let resources = crate::voice_runtime(&fixture.state).resources();
    let before = resources.counts();
    let pressure = resources
        .charge_process_retained(PROCESS_BYTES - before.0)
        .unwrap();
    let serial = fixture.state.voice_authority.lock().unwrap().serial;
    {
        let mut authority = fixture.state.voice_authority.lock().unwrap();
        assert!(authority.admit_admitted_join_key(&handle, [9; 32]).is_err());
        assert!(authority.join_generations.is_empty());
        assert!(authority.join_index_funding.is_none());
        assert_eq!(authority.serial, serial);
    }
    drop(pressure);
    close(fixture, handle, runner).await;
}

#[tokio::test]
async fn two_sanctioned_tokio_settlement_tasks_share_original_notification() {
    let (fixture, handle, runner) = active().await;
    let (_, mut work) = prepare(&fixture, &handle);
    let (first, second) = {
        let mut authority = fixture.state.voice_authority.lock().unwrap();
        (
            authority
                .settle_admitted_join_key(handle.id(), [9; 32])
                .unwrap(),
            authority
                .settle_admitted_join_key(handle.id(), [9; 32])
                .unwrap(),
        )
    };
    let first = first.finish_outside_guard().unwrap();
    let second = second.finish_outside_guard().unwrap();
    // Only the native/Tauri Tokio executor is the production waiting contract.
    // An arbitrary Rust RawWaker panic is not a promised watch fanout boundary.
    async fn subscribed(
        drain: AdmittedKeyDrain,
        entered: tokio::sync::oneshot::Sender<()>,
    ) -> Result<(), String> {
        let waiting = drain.wait();
        tokio::pin!(waiting);
        let mut entered = Some(entered);
        std::future::poll_fn(|cx| {
            // Forward the ACTUAL Tokio task context unchanged. The barrier is
            // after watch has genuinely registered Pending, not a scheduling hint.
            let polled = waiting.as_mut().poll(cx);
            if polled.is_pending() {
                if let Some(entered) = entered.take() {
                    let _ = entered.send(());
                }
            }
            polled
        })
        .await
    }
    let (entered_first, pending_first) = tokio::sync::oneshot::channel();
    let (entered_second, pending_second) = tokio::sync::oneshot::channel();
    let mut first = tokio::spawn(subscribed(first, entered_first));
    let mut second = tokio::spawn(subscribed(second, entered_second));
    bounded(pending_first).await.unwrap();
    bounded(pending_second).await.unwrap();
    assert!(!first.is_finished() && !second.is_finished());
    let completion = fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .finish_admitted_join_key_publication(&mut work, &Err(VoiceError::Capacity))
        .unwrap();
    assert!(completion.finish_outside_guard().is_err());
    async fn bounded<F: Future>(future: F) -> F::Output {
        tokio::select! {
            output = future => output,
            _ = catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(30)) => {
                panic!("native key settlement task did not return");
            }
        }
    }
    bounded(&mut first).await.unwrap().unwrap();
    bounded(&mut second).await.unwrap().unwrap();
    drop(work);
    close(fixture, handle, runner).await;
}

#[tokio::test]
async fn genuine_foreign_root_pool_refuses_before_index_generation_or_crypto_charge() {
    let (fixture, handle, runner) = active().await;
    let (owner, controller) = {
        let authority = fixture.state.voice_authority.lock().unwrap();
        let current = authority.controller.as_ref().unwrap();
        (
            authority.owner.as_ref().unwrap().clone(),
            Controller {
                id: current.id.clone(),
                generation: current.generation,
                lease: current.lease.clone(),
            },
        )
    };
    let mut foreign = Coordinator::with_runtime(Runtime::new(Arc::new(ManualClock::new(1000))));
    // Reproduce an accidental native integration wrapping GENUINE original
    // identities in another Coordinator pool. No issuer phase/Ready is forged;
    // every other current-owner check is deliberately satisfied by real aliases.
    foreign.owner = Some(owner);
    foreign.controller = Some(controller);
    foreign.retain_admitted_activation(&handle).unwrap();
    let own = crate::voice_runtime(&fixture.state).resources();
    let before = (own.counts(), foreign.resources.counts(), foreign.serial);
    assert!(foreign.admit_admitted_join_key(&handle, [9; 32]).is_err());
    assert!(foreign.join_generations.is_empty());
    assert!(foreign.join_index_funding.is_none());
    assert_eq!(
        (own.counts(), foreign.resources.counts(), foreign.serial),
        before
    );
    // The fixture must not run foreign revoke hooks against those borrowed real
    // controller/call leases. Detach only the test aliases before foreign Drop.
    drop(foreign.owner.take());
    drop(foreign.controller.take());
    foreign.activations.clear();
    drop(foreign);
    assert!(
        handle.active(),
        "foreign refusal did not close the original call"
    );
    close(fixture, handle, runner).await;
}

#[tokio::test]
async fn closed_owner_task_cleanup_lookup_cannot_adopt_replacement_or_other_executor() {
    use crate::voice_admitted_key::{PendingPublication, PrepaidPublication};

    let clock = Arc::new(ManualClock::new(1000));
    let fixture = Fixture::with_voice_clock(clock.clone()).await;
    let (handle, mut runner) = active_handle_fixture(&fixture).await;
    let prepaid = PrepaidPublication::reserve(&handle).unwrap();
    let (admission, work) = prepare(&fixture, &handle);
    let pending = PendingPublication::new(handle.clone(), work, prepaid, clock.clone());
    pending.install(&fixture.state.voice_authority).unwrap();
    let resources = crate::voice_runtime(&fixture.state).resources();
    let serial = fixture.state.voice_authority.lock().unwrap().serial;
    let charged = resources.counts();
    {
        let authority = fixture.state.voice_authority.lock().unwrap();
        assert!(authority
            .admitted_publication_task_for_cleanup(handle.id(), [9; 32])
            .unwrap()
            .is_same(&pending));
        assert!(authority
            .admitted_publication_task_for_cleanup(handle.id(), [8; 32])
            .is_none());
        assert!(authority
            .admitted_publication_task_for_cleanup("unknown-owner", [9; 32])
            .is_none());
    }
    assert_eq!(resources.counts(), charged);

    // End and join the real activation's Root residence before selecting a
    // successor. The publication task is deliberately still unstarted: this
    // test exercises negative lookup, not a fabricated Root terminal receipt.
    handle.close();
    clock.advance_ms(250);
    assert!(runner
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    fixture.actor.member_count().await;
    assert!(runner
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_ready());
    drop(runner);
    let returned = resources.counts();
    {
        let mut authority = fixture.state.voice_authority.lock().unwrap();
        authority.end(handle.id());
        assert!(authority
            .owner
            .as_ref()
            .is_none_or(|owner| owner.id != handle.id()));
        assert!(authority
            .admitted_publication_task_for_cleanup(handle.id(), [9; 32])
            .unwrap()
            .is_same(&pending));
        assert!(authority
            .admitted_publication_task(&handle, [9; 32], &admission.permit, point())
            .unwrap()
            .is_same(&pending));
        assert_eq!(authority.serial, serial);
    }
    // Actual activation owner return was joined above. Exact negative lookup
    // itself allocates no successor row, permit, RPC, task or credit.
    assert_eq!(resources.counts(), returned);

    let (replacement, replacement_runner) = active_handle_fixture(&fixture).await;
    assert_ne!(replacement.id(), handle.id());
    let replacement_prepaid = PrepaidPublication::reserve(&replacement).unwrap();
    let replacement_work = {
        let mut authority = fixture.state.voice_authority.lock().unwrap();
        let admission = authority
            .admit_admitted_join_key(&replacement, [8; 32])
            .unwrap();
        let AdmittedPublicationPlan::Start(work) = authority
            .prepare_admitted_join_key_publication(
                &replacement,
                [8; 32],
                &admission.permit,
                point(),
            )
            .unwrap()
        else {
            panic!("fresh replacement publication");
        };
        work
    };
    let replacement_pending = PendingPublication::new(
        replacement.clone(),
        replacement_work,
        replacement_prepaid,
        clock,
    );
    replacement_pending
        .install(&fixture.state.voice_authority)
        .unwrap();
    let before = resources.counts();
    {
        let authority = fixture.state.voice_authority.lock().unwrap();
        let old = authority
            .admitted_publication_task_for_cleanup(handle.id(), [9; 32])
            .unwrap();
        let new = authority
            .admitted_publication_task_for_cleanup(replacement.id(), [8; 32])
            .unwrap();
        assert!(old.is_same(&pending) && new.is_same(&replacement_pending));
        assert!(!old.is_same(&new));
        assert!(authority
            .admitted_publication_task_for_cleanup(handle.id(), [8; 32])
            .is_none());
        assert!(authority
            .admitted_publication_task_for_cleanup(replacement.id(), [9; 32])
            .is_none());
        assert!(authority
            .admitted_publication_task(&replacement, [9; 32], &admission.permit, point())
            .is_none());
    }
    assert_eq!(resources.counts(), before);
    // Both tasks retain their actual original Work and fees until normal owner
    // teardown; no fake SDK settlement or Root result is introduced for cleanup.
    drop(pending);
    drop(replacement_pending);
    drop(handle);
    close(fixture, replacement, replacement_runner).await;
}

#[tokio::test]
async fn actual_root_completed_publication_history_recovers_under_full_group_pressure() {
    use crate::voice_admitted_key::{PendingPublication, PrepaidPublication};

    let clock = Arc::new(ManualClock::new(1000));
    let fixture = Fixture::with_voice_clock(clock.clone()).await;
    let (handle, mut runner) = active_handle_fixture(&fixture).await;
    let resources = crate::voice_runtime(&fixture.state).resources();
    let paid = PrepaidPublication::reserve(&handle).unwrap();
    let commit = fixture.state.ui_session_commit.clone().lock_owned().await;
    let (admission, work) = {
        let mut authority = fixture.state.voice_authority.lock().unwrap();
        let admission = authority.admit_admitted_join_key(&handle, [9; 32]).unwrap();
        let before = resources.counts();
        assert!(authority
            .completed_admitted_join_key_publication(&handle, [9; 32], &admission.permit, point())
            .unwrap()
            .is_none());
        assert_eq!(resources.counts(), before);
        let AdmittedPublicationPlan::Start(work) = authority
            .prepare_admitted_join_key_publication(&handle, [9; 32], &admission.permit, point())
            .unwrap()
        else {
            panic!("original Root publication");
        };
        (admission, work)
    };
    let pending = PendingPublication::new(
        handle.clone(),
        work,
        paid,
        crate::voice_runtime(&fixture.state).clock(),
    );
    pending.install(&fixture.state.voice_authority).unwrap();
    let mut publication = Box::pin(pending.initial_task(&fixture.state, commit));
    assert!(publication
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    fixture.actor.member_count().await; // actual Root receipt + original Actor argument return
    assert!(publication
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_ready());
    drop(publication);
    let accepted = pending.reply(&fixture.state).await.unwrap();
    pending.acknowledge_result().unwrap();
    let settled = fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .settle_admitted_join_key(handle.id(), [9; 32])
        .unwrap();
    assert!(settled.finish_outside_guard().is_none());
    let collected = fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .collect_completed_admitted_publication(handle.id())
        .unwrap();
    drop(collected);
    drop(pending); // the completed getter cannot depend on the publication task's separate payer
    assert!(handle.active());

    let group = handle.binding().scope.group;
    let used = resources.group_counts(group).unwrap().0;
    let pressure = resources
        .charge(group, GROUP_BYTES - used, ResourceKind::Retained)
        .unwrap();
    let full = resources.counts();
    let serial = fixture.state.voice_authority.lock().unwrap().serial;
    for _ in 0..8 {
        let authority = fixture.state.voice_authority.lock().unwrap();
        let retry = authority
            .completed_admitted_join_key_publication(&handle, [9; 32], &admission.permit, point())
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                retry.owner,
                retry.executor,
                retry.permit,
                retry.join,
                retry.key_ref
            ),
            (
                accepted.owner.clone(),
                accepted.executor.clone(),
                accepted.permit.clone(),
                accepted.join.clone(),
                accepted.key_ref.clone(),
            )
        );
        assert!(authority
            .completed_admitted_join_key_publication(&handle, [8; 32], &admission.permit, point())
            .is_err());
        assert!(authority
            .completed_admitted_join_key_publication(&handle, [9; 32], "other-permit", point())
            .is_err());
        let mut other_point = point();
        other_point[1] ^= 1;
        assert!(authority
            .completed_admitted_join_key_publication(
                &handle,
                [9; 32],
                &admission.permit,
                other_point
            )
            .is_err());
        assert_eq!(authority.serial, serial);
    }
    assert_eq!(resources.counts(), full);
    drop(pressure);

    // Settling the completed SDK operation does not erase still-current public
    // history; End does remove its positive authority, even with an exact tuple.
    assert!(fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .completed_admitted_join_key_publication(&handle, [9; 32], &admission.permit, point())
        .unwrap()
        .is_some());
    handle.close();
    assert!(fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .completed_admitted_join_key_publication(&handle, [9; 32], &admission.permit, point())
        .is_err());
    clock.advance_ms(250);
    assert!(runner
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    fixture.actor.member_count().await;
    assert!(runner
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_ready());
    drop(runner);
    handle.reply().await.acknowledge();
    {
        let authority = fixture.state.voice_authority.lock().unwrap();
        let row = authority.join_generations.get(handle.id()).unwrap();
        assert!(row.admitted_task.is_none() && row.engine.is_none() && !row.publishing);
    }
    // The same production collector must revisit a taskless ended row. No second
    // SDK settle or test-only manual row extraction may be needed after End.
    crate::voice_admitted_call::collect_retired(&fixture.state);
    {
        let authority = fixture.state.voice_authority.lock().unwrap();
        assert!(!authority.join_generations.contains_key(handle.id()));
        assert!(authority.admitted_activation(handle.id()).is_none());
    }
    drop(handle);
    fixture.close().await;
}

#[tokio::test]
async fn canonical_ok_and_refusal_require_ack_return_and_sdk_settlement_for_collection() {
    use crate::voice_admitted_key::{PendingPublication, PrepaidPublication};

    for root_refuses in [false, true] {
        let clock = Arc::new(ManualClock::new(1000));
        let fixture = Fixture::with_voice_clock(clock.clone()).await;
        let (handle, mut call) = active_handle_fixture(&fixture).await;
        let paid = PrepaidPublication::reserve(&handle).unwrap();
        let commit = fixture.state.ui_session_commit.clone().lock_owned().await;
        let (_, work) = prepare(&fixture, &handle);
        let pending = PendingPublication::new(handle.clone(), work, paid, clock.clone());
        pending.install(&fixture.state.voice_authority).unwrap();
        assert!(pending.acknowledge_result().is_err());
        assert!(!pending.is_collectible());
        assert!(fixture
            .state
            .voice_authority
            .lock()
            .unwrap()
            .collect_one_completed_admitted_publication()
            .is_none());
        let mut publication = Box::pin(pending.initial_task(&fixture.state, commit));
        assert!(publication
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending());
        if root_refuses {
            // This synchronous close occurs before yielding to the actual Actor,
            // so its original queued Root command must return a genuine refusal.
            handle.close();
        }
        fixture.actor.member_count().await;
        assert!(publication
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready());
        drop(publication); // actual execution Box return, not a supplied bool
        let first = pending.reply(&fixture.state).await;
        assert_eq!(first.is_err(), root_refuses);
        assert!(!pending.is_collectible(), "lost reply is not an ACK");

        handle.close();
        clock.advance_ms(250);
        assert!(call
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending());
        fixture.actor.member_count().await;
        assert!(call
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready());
        drop(call);
        handle.reply().await.acknowledge();
        {
            let mut authority = fixture.state.voice_authority.lock().unwrap();
            let original = authority
                .admitted_publication_task_for_cleanup(handle.id(), [9; 32])
                .unwrap();
            assert!(original.is_same(&pending));
            assert!(authority
                .collect_completed_admitted_publication(handle.id())
                .is_none());
            assert!(authority
                .collect_one_acknowledged_admitted_activation()
                .is_none());
        }
        let retry = pending.reply(&fixture.state).await;
        match (&first, retry) {
            (Ok(first), Ok(retry)) => assert_eq!(first.key_ref, retry.key_ref),
            (Err(first), Err(retry)) => assert_eq!(first, &retry),
            _ => panic!("End changed the original known publication result"),
        }
        pending.acknowledge_result().unwrap();
        assert!(pending.is_collectible());
        assert!(
            fixture
                .state
                .voice_authority
                .lock()
                .unwrap()
                .collect_completed_admitted_publication(handle.id())
                .is_none(),
            "SDK operation has not settled"
        );
        let settled = fixture
            .state
            .voice_authority
            .lock()
            .unwrap()
            .settle_admitted_join_key(handle.id(), [9; 32])
            .unwrap();
        assert!(settled.finish_outside_guard().is_none());
        crate::voice_admitted_call::collect_retired(&fixture.state);
        {
            let authority = fixture.state.voice_authority.lock().unwrap();
            assert!(authority
                .admitted_publication_task_for_cleanup(handle.id(), [9; 32])
                .is_none());
            assert!(!authority.join_generations.contains_key(handle.id()));
            assert!(authority.admitted_activation(handle.id()).is_none());
        }
        drop(pending);
        drop(handle);
        fixture.close().await;
    }
}

#[tokio::test]
async fn acknowledged_active_parent_collects_after_actual_close_and_return() {
    let clock = Arc::new(ManualClock::new(1000));
    let fixture = Fixture::with_voice_clock(clock.clone()).await;
    let (handle, mut call) = active_handle_fixture(&fixture).await;
    handle.reply().await.acknowledge();
    assert!(
        fixture
            .state
            .voice_authority
            .lock()
            .unwrap()
            .collect_one_acknowledged_admitted_activation()
            .is_none(),
        "an active call cannot be collected after ACK"
    );
    handle.close();
    clock.advance_ms(250);
    assert!(call
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    fixture.actor.member_count().await;
    assert!(call
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_ready());
    drop(call);
    assert!(handle.collectible());
    let parent = {
        let mut authority = fixture.state.voice_authority.lock().unwrap();
        let parent = authority
            .collect_one_acknowledged_admitted_activation()
            .unwrap();
        assert!(authority.admitted_activation(handle.id()).is_none());
        assert!(authority
            .collect_one_acknowledged_admitted_activation()
            .is_none());
        parent
    };
    drop(parent);
    drop(handle);
    fixture.close().await;
}

#[tokio::test]
async fn never_started_certificate_requires_exact_closed_existing_generation_and_pays_its_tail() {
    let (fixture, handle, runner) = active().await;
    {
        let authority = fixture.state.voice_authority.lock().unwrap();
        assert!(
            authority
                .never_started_closed_admitted_publication(&handle, [9; 32], "missing")
                .is_none(),
            "missing generation is NoProof"
        );
    }
    let admission = fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .admit_admitted_join_key(&handle, [9; 32])
        .unwrap();
    assert!(
        fixture
            .state
            .voice_authority
            .lock()
            .unwrap()
            .never_started_closed_admitted_publication(&handle, [9; 32], &admission.permit)
            .is_none(),
        "a live generation cannot certify ended cleanup"
    );
    handle.close();
    let resources = crate::voice_runtime(&fixture.state).resources();
    let before = resources.counts();
    let certificate = {
        let authority = fixture.state.voice_authority.lock().unwrap();
        let serial = authority.serial;
        let certificate = authority
            .never_started_closed_admitted_publication(&handle, [9; 32], &admission.permit)
            .unwrap();
        assert!(authority
            .never_started_closed_admitted_publication(&handle, [8; 32], &admission.permit)
            .is_none());
        assert!(authority
            .never_started_closed_admitted_publication(&handle, [9; 32], "other-permit")
            .is_none());
        assert_eq!(authority.serial, serial);
        certificate
    };
    assert_eq!(resources.counts(), before);
    assert_eq!(certificate.owner(), handle.id());
    assert_eq!(certificate.executor(), [9; 32]);
    assert_eq!(certificate.permit(), admission.permit);
    assert_eq!(hex::encode(certificate.join()), admission.join);
    let settled = fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .settle_admitted_join_key(handle.id(), [9; 32])
        .unwrap();
    assert!(settled.finish_outside_guard().is_none());
    assert_eq!(resources.counts().0, before.0 - GENERATE_BYTES);
    assert!(fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .join_generations
        .is_empty());
    let settled_counts = resources.counts();
    {
        let authority = fixture.state.voice_authority.lock().unwrap();
        let serial = authority.serial;
        assert!(
            authority
                .never_started_closed_admitted_publication(&handle, [9; 32], &admission.permit)
                .is_none(),
            "a CLOSED original with no row is still NoProof"
        );
        assert_eq!(authority.serial, serial);
    }
    assert_eq!(resources.counts(), settled_counts);
    drop(certificate); // actual reply owner, AFTER row/watch destruction
    assert_eq!(
        resources.counts().0,
        before.0 - GENERATE_BYTES - ADMITTED_GENERATION_BYTES
    );
    close(fixture, handle, runner).await;
}

#[tokio::test]
async fn frozen_publication_point_never_becomes_unstarted_certificate_after_end() {
    let (fixture, handle, runner) = active().await;
    let (admission, work) = prepare(&fixture, &handle);
    // No Actor publication has been queued by this fixture. Even so, frozen
    // native preparation is real ambiguous ownership, never absence of work.
    handle.close();
    assert!(fixture
        .state
        .voice_authority
        .lock()
        .unwrap()
        .never_started_closed_admitted_publication(&handle, [9; 32], &admission.permit)
        .is_none());
    drop(work); // no invented Root outcome, ACK, drained flag or attempt refund
    close(fixture, handle, runner).await;
}
