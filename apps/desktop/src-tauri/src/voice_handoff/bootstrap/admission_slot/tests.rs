//! Genuine native/Actor/Root custody. These are not browser pairing or Ready/RTC evidence.
use super::*;
use crate::voice_activation_tests::Fixture;
use crate::voice_handoff::context::Context;
use crate::voice_handoff::factory::tests::{park, selection, unacknowledged_channel_route};
use crate::voice_handoff::requests::negotiation_admission::{Kind, Request};
use catcoms_sync::voice_v2::admission::ADMISSION_RPC_BYTES;
use catcoms_sync::voice_v2::resources::{ResourceKind, GROUP_BYTES};
use std::future::Future;
mod parent_claim;
mod reserved_cancel;

fn capacity(
    route: &Route,
    context: &Context,
    kind: CapacityKind,
    generation: u64,
) -> Result<Snapshot, Refused> {
    route.admission_capacity(context, CapacityRequest { kind, generation })
}

async fn finalize_original(
    route: &Route,
    context: &Context,
    native: &crate::voice_handoff::channel::NativeHold,
) {
    until(|| {
        route
            .admission_work()
            .as_ref()
            .is_some_and(VoiceAdmissionWork::is_retired)
    })
    .await;
    route
        .take_admission_finalization(native)
        .unwrap()
        .expect("actual staged outcome")
        .finish(context);
}

#[tokio::test]
async fn capacity_cancel_between_pending_and_actor_acceptance_returns_without_closing_registration()
{
    for known_registration in [false, true] {
        let (f, context, route, _cleanup) = setup().await;
        // A synchronous native caller owns the SAME real position and original
        //8KiB payment as the production worker. This deterministic hook lies at
        //its actual Pending->accepted boundary, not a simulated actor outcome.
        let _payment = route
            .selected()
            .resources()
            .charge(
                route.selected().binding().scope.group,
                NATIVE_TASK_BYTES,
                ResourceKind::Retained,
            )
            .unwrap();
        let native = route.session().native_hold().unwrap();
        let mut original = None;
        let canceled_generation = if known_registration {
            queue_requested(&context, &route, &native, 1).unwrap();
            finalize_original(&route, &context, &native).await;
            assert_eq!(view(&route).phase, Phase::Reserved);
            original = view(&route).incarnation;
            cancel(&route, 1).unwrap();
            route
                .take_admission_finalization(&native)
                .unwrap()
                .unwrap()
                .finish(&context);
            assert_eq!(view(&route).phase, Phase::Returned);
            2
        } else {
            1
        };
        queue_requested_with_hooks(
            &context,
            &route,
            &native,
            canceled_generation,
            || {},
            || {
                assert_eq!(view(&route).phase, Phase::Pending);
                assert!(!view(&route).work);
                cancel(&route, canceled_generation).unwrap();
            },
        )
        .unwrap();
        finalize_original(&route, &context, &native).await;
        assert_eq!(view(&route).phase, Phase::Returned);
        assert!(!route.row.state.lock().unwrap().admission_slot.closed);
        assert_eq!(
            view(&route).incarnation,
            original,
            "never-entered cancellation cannot close or mint registration"
        );
        queue_requested(&context, &route, &native, canceled_generation + 1).unwrap();
        finalize_original(&route, &context, &native).await;
        assert_eq!(view(&route).phase, Phase::Reserved);
        if let Some(original) = original {
            assert_eq!(view(&route).incarnation, Some(original));
        }
        route.close_admission_slot();
        route
            .take_admission_finalization(&native)
            .unwrap()
            .unwrap()
            .finish(&context);
        drop(native);
        f.close().await;
    }
}

#[tokio::test]
async fn capacity_control_observes_actual_parked_work_and_never_projects_successor_capacity_onto_old_request(
) {
    let (f, context, route, _cleanup) = setup().await;
    let before = route.selected().resources().counts();
    for _ in 0..3 {
        let initial = capacity(&route, &context, CapacityKind::Observe, 1).unwrap();
        assert_eq!(
            (
                initial.state,
                initial.canonical_generation,
                initial.incarnation
            ),
            (CapacityState::Unstarted, 0, None)
        );
    }
    assert_eq!(route.selected().resources().counts(), before);
    let parked = park(&f.actor).await;
    capacity(&route, &context, CapacityKind::Register, 1).unwrap();
    until(|| view(&route).work && !view(&route).retired).await;
    let preparing = capacity(&route, &context, CapacityKind::Observe, 1).unwrap();
    assert_eq!(
        (
            preparing.state,
            preparing.canonical_generation,
            preparing.incarnation
        ),
        (CapacityState::Preparing, 1, None)
    );
    let canceled = capacity(&route, &context, CapacityKind::Cancel, 1).unwrap();
    drop(parked); // release the real actor even if an assertion below fails
    assert_eq!(
        (canceled.state, canceled.cancelled_through),
        (CapacityState::Closing, 1)
    );
    until(|| view(&route).phase == Phase::Returned).await;
    let returned = capacity(&route, &context, CapacityKind::Observe, 1).unwrap();
    assert_eq!(returned.state, CapacityState::Returned);
    let original = returned.incarnation.unwrap();
    capacity(&route, &context, CapacityKind::Register, 2).unwrap();
    until(|| view(&route).phase == Phase::Reserved && view(&route).generation == 2).await;
    let held = route.selected().resources().counts();
    let reserved = capacity(&route, &context, CapacityKind::Observe, 2).unwrap();
    assert_eq!(
        (reserved.state, reserved.incarnation),
        (CapacityState::Reserved, Some(original))
    );
    for kind in [
        CapacityKind::Register,
        CapacityKind::Observe,
        CapacityKind::Cancel,
    ] {
        let old = capacity(&route, &context, kind, 1).unwrap();
        assert_eq!(
            (
                old.state,
                old.requested_generation,
                old.canonical_generation,
                old.cancelled_through
            ),
            (CapacityState::Superseded, 1, 2, 1)
        );
        assert_eq!(old.incarnation, Some(original));
    }
    for future in [3, 4, u64::MAX] {
        assert_eq!(
            capacity(&route, &context, CapacityKind::Observe, future),
            Err(Refused::Bounds)
        );
    }
    assert_eq!(route.selected().resources().counts(), held);
    assert!(view(&route).capacity);
    finish(f, &route).await;
}

#[tokio::test]
async fn capacity_control_cancel_before_ack_fences_late_first_register_without_task_or_root_mint() {
    let f = Fixture::new().await;
    let (_, selected, _) = selection(&f).await;
    let route = unacknowledged_channel_route(&f, &selected).await;
    let _cleanup = CloseOnDrop(route.clone());
    let context = Context::new(&f.state);
    assert_eq!(
        capacity(&route, &context, CapacityKind::Observe, 1),
        Err(Refused::Binding)
    );
    route
        .admission_mode(Request {
            kind: Kind::Register,
            generation: 1,
        })
        .unwrap();
    assert_eq!(
        capacity(&route, &context, CapacityKind::Register, 1),
        Err(Refused::Binding)
    );
    let canceled = capacity(&route, &context, CapacityKind::Cancel, 1).unwrap();
    assert_eq!(
        (
            canceled.state,
            canceled.canonical_generation,
            canceled.cancelled_through,
            canceled.incarnation
        ),
        (CapacityState::Returned, 1, 1, None)
    );
    route.acknowledge_setup().unwrap();
    let before = route.selected().resources().counts();
    // Both the public capacity adapter and its underlying native API must fence
    // the late retry; it cannot allocate a forever-idle permanent8KiB task.
    assert_eq!(
        capacity(&route, &context, CapacityKind::Register, 1).unwrap(),
        canceled
    );
    register(&context, &route, 1).unwrap();
    assert_eq!(route.selected().resources().counts(), before);
    assert!(!route.row.state.lock().unwrap().admission_slot.task_started);
    assert!(view(&route).incarnation.is_none());
    capacity(&route, &context, CapacityKind::Register, 2).unwrap();
    until(|| view(&route).phase == Phase::Reserved && view(&route).generation == 2).await;
    finish(f, &route).await;
}

#[tokio::test]
async fn capacity_control_whole_mode_cancel_and_end_keep_closing_until_original_actor_work_returns()
{
    for whole_mode in [true, false] {
        let (f, context, route, _cleanup) = setup().await;
        let parked = park(&f.actor).await;
        capacity(&route, &context, CapacityKind::Register, 1).unwrap();
        until(|| view(&route).work && !view(&route).retired).await;
        let owner = route.selected().id().to_string();
        let mut ending = Box::pin(crate::end_voice_owner_inner(&f.state, &owner));
        if whole_mode {
            route
                .admission_mode(Request {
                    kind: Kind::Cancel,
                    generation: 1,
                })
                .unwrap();
        } else {
            // Logical End precedes its awaited actor Stop. Poll that ACTUAL
            // future once while the original actor remains deterministically parked.
            std::future::poll_fn(|cx| {
                let _ = ending.as_mut().poll(cx);
                std::task::Poll::Ready(())
            })
            .await;
            assert!(!route.selected().lease().is_live());
        }
        let closing = capacity(&route, &context, CapacityKind::Observe, 1).unwrap();
        let pending = view(&route).work && !view(&route).retired;
        drop(parked);
        assert_eq!(closing.state, CapacityState::Closing);
        assert!(pending);
        if !whole_mode {
            ending.as_mut().await;
        }
        until(|| view(&route).returned).await;
        let closed = capacity(&route, &context, CapacityKind::Observe, 1).unwrap();
        assert_eq!(
            (closed.state, closed.incarnation),
            (CapacityState::Closed, None)
        );
        drop(ending);
        f.close().await;
    }
}

#[tokio::test]
async fn capacity_control_full_pressure_keeps_observation_and_exact_never_started_cancel_available()
{
    let (f, context, route, _cleanup) = setup().await;
    let resources = route.selected().resources();
    let filler = resources
        .charge(
            route.selected().binding().scope.group,
            GROUP_BYTES
                - resources
                    .group_counts(route.selected().binding().scope.group)
                    .unwrap()
                    .0,
            ResourceKind::Retained,
        )
        .unwrap();
    let full = resources.counts();
    let refused = capacity(&route, &context, CapacityKind::Register, 1).unwrap();
    assert_eq!(
        (refused.state, refused.canonical_generation),
        (CapacityState::Unstarted, 0)
    );
    assert_eq!(
        capacity(&route, &context, CapacityKind::Observe, 1).unwrap(),
        refused
    );
    let canceled = capacity(&route, &context, CapacityKind::Cancel, 1).unwrap();
    assert_eq!(canceled.state, CapacityState::Returned);
    assert_eq!(resources.counts(), full);
    assert!(!route.row.state.lock().unwrap().admission_slot.task_started);
    drop(filler);
    f.close().await;
}

#[tokio::test]
async fn native_admission_slot_both_close_paths_keep_control_until_actual_actor_retirement() {
    for closure in [Closure::Setup, Closure::Channel] {
        let (f, context, route, _cleanup) = setup().await;
        let parked = park(&f.actor).await;
        register(&context, &route, 1).unwrap();
        until(|| view(&route).work && !view(&route).retired).await;
        let close = super::super::finish(
            &f.state,
            route.selected().id(),
            route.selected().executor(),
            closure,
        )
        .await;
        // Do not strand the real actor on an assertion failure. Releasing this
        // barrier still leaves its exact Work/outcome, never a replacement task.
        drop(parked);
        assert!(matches!(close, Err(Refused::Capacity)));
        {
            let state = route.row.state.lock().unwrap();
            assert!(state.effects_closed && !state.closed);
        }
        assert!(f.state.voice_channels.route(route.row.slot).is_ok());
        assert_eq!(
            route
                .admission_mode(Request {
                    kind: Kind::Observe,
                    generation: 1
                })
                .unwrap()[1],
            3,
            "authenticated negative/history routing survives the close-first fence"
        );
        route
            .admission_mode(Request {
                kind: Kind::Cancel,
                generation: 1,
            })
            .unwrap();
        until(|| view(&route).returned).await;
        assert!(matches!(
            collect_channel(&f.state, route.selected().id(), route.selected().executor()).await,
            Ok(Status::Closed {
                native_setup_drained: true
            })
        ));
        assert!(route.row.state.lock().unwrap().closed);
        assert!(f.state.voice_channels.route(route.row.slot).is_err());
        f.close().await;
    }
}

#[tokio::test]
async fn native_admission_slot_rechecks_mounted_state_at_both_actual_ui_admission_fences() {
    for queue in [false, true] {
        for remove_store in [false, true] {
            let (f, context, route, _cleanup) = setup().await;
            let generation = f.state.ui_session_generation.load(Ordering::Acquire);
            let before = route.selected().resources().counts();
            let mounted = Arc::new(Mutex::new(None));
            let saved = mounted.clone();
            let ui = f.state.ui_session_commit.clone();
            let store = f.state.store.clone();
            let resumable = f.state.session_resumable.clone();
            // Actual Store/resumable removal wins AFTER the earlier observation
            // and BEFORE the final UI admission. No literal bool substitutes for
            // the same production mutex/generation checks at either boundary.
            let hook = move || {
                let _commit = ui.try_lock_owned().unwrap();
                if remove_store {
                    *saved.lock().unwrap() = Some(store.try_lock().unwrap().take());
                } else {
                    *resumable.try_lock().unwrap() = false;
                }
            };
            let result = if queue {
                let native = route.session().native_hold().unwrap();
                queue_requested_at(&context, &route, &native, 1, hook)
            } else {
                register_at(&context, &route, 1, hook)
            };
            if remove_store {
                let original = mounted.lock().unwrap().take().unwrap();
                *f.state.store.lock().await = original;
            } else {
                *f.state.session_resumable.lock().await = true;
            }
            assert!(matches!(
                result,
                Err(crate::voice_handoff::factory::Refused::Closed)
            ));
            assert_eq!(
                generation,
                f.state.ui_session_generation.load(Ordering::Acquire)
            );
            assert_eq!(route.selected().resources().counts(), before);
            {
                let state = route.row.state.lock().unwrap();
                let slot = &state.admission_slot;
                assert!(!slot.task_started && slot.invocation.is_none());
                assert!(slot.registration.is_none() && slot.capacity.is_none());
                assert_eq!(slot.generation, 0, "no Pending or Root queue admission");
            }
            register(&context, &route, 1).unwrap();
            until(|| view(&route).phase == Phase::Reserved).await;
            finish(f, &route).await;
        }
    }
}

#[tokio::test]
async fn native_admission_slot_next_generation_survives_pre_pending_pressure_with_only_host_nudges()
{
    let (f, context, route, _cleanup) = setup().await;
    register(&context, &route, 1).unwrap();
    until(|| view(&route).phase == Phase::Reserved).await;
    let original = view(&route).incarnation.unwrap();
    cancel(&route, 1).unwrap();
    until(|| view(&route).phase == Phase::Returned).await;
    let ui = f.state.ui_session_commit.clone().lock_owned().await;
    register(&context, &route, 2).unwrap();
    until(|| {
        let state = route.row.state.lock().unwrap();
        state.admission_slot.desired == 2 && state.admission_slot.requested.is_none()
    })
    .await;
    assert_eq!(
        view(&route).generation,
        1,
        "local refusal has not installed the next Pending"
    );
    drop(ui);
    route.nudge_admission();
    until(|| view(&route).phase == Phase::Reserved && view(&route).generation == 2).await;
    assert_eq!(view(&route).incarnation, Some(original));
    finish(f, &route).await;
}

fn abortable(context: &Context, route: &Route) -> tokio::task::AbortHandle {
    let mut abort = None;
    register_with_hooks(context, route, 1, || {}, |task| abort = Some(task)).unwrap();
    abort.unwrap()
}

#[tokio::test]
async fn native_admission_slot_abort_before_first_poll_cannot_start_or_revive_root_work() {
    let (f, context, route, _cleanup) = setup().await;
    let before = route.selected().resources().counts();
    // This current-thread runtime cannot poll the spawned task until we yield.
    let abort = abortable(&context, &route);
    abort.abort();
    assert!(!view(&route).work && view(&route).generation == 0);
    until(|| view(&route).returned).await;
    assert!(route.row.state.lock().unwrap().admission_slot.collected());
    assert!(route.session().native_hold().is_ok());
    assert_eq!(
        route.selected().resources().counts().0,
        before.0 + NATIVE_TASK_BYTES
    );
    assert!(register(&context, &route, 1).is_err());
    assert!(!view(&route).work && view(&route).incarnation.is_none());
    drop(abort);
    f.close().await;
}

#[tokio::test]
async fn native_admission_slot_emergency_reclamation_unwind_keeps_same_position_for_final_recovery()
{
    let (f, context, route, _cleanup) = setup().await;
    let abort = abortable(&context, &route);
    until(|| view(&route).phase == Phase::Reserved).await;
    abort.abort();
    until(|| view(&route).returned).await;
    let funding = {
        let mut state = route.row.state.lock().unwrap();
        state.admission_slot.unwind_reclamation = true;
        state.admission_slot.task_funding.as_ref().unwrap().clone()
    };
    assert!(route.recover_terminal_admission());
    {
        let state = route.row.state.lock().unwrap();
        let slot = &state.admission_slot;
        assert!(!slot.reclaiming && !slot.recovery_active);
        assert!(slot.emergency_native.is_some() && !slot.collected());
        assert!(Arc::ptr_eq(slot.task_funding.as_ref().unwrap(), &funding));
    }
    assert!(route.recover_terminal_admission());
    assert!(route.row.state.lock().unwrap().admission_slot.collected());
    assert_eq!(view(&route).generation, 1);
    assert!(register(&context, &route, 2).is_err());
    drop((funding, abort));
    f.close().await;
}

#[tokio::test]
async fn native_admission_slot_emergency_queued_cleanup_preserves_control_and_one_original_position(
) {
    let (f, context, route, _cleanup) = setup().await;
    let parked = park(&f.actor).await;
    let abort = abortable(&context, &route);
    until(|| view(&route).work && !view(&route).retired).await;
    abort.abort();
    until(|| view(&route).returned).await;
    assert!(matches!(
        collect_channel(&f.state, route.selected().id(), route.selected().executor()).await,
        Err(Refused::Capacity)
    ));
    assert!(f.state.voice_channels.route(route.row.slot).is_ok());
    let retained = route.take_emergency_custody().unwrap();
    assert!(retained
        .native
        .as_ref()
        .unwrap()
        .belongs_to(route.session()));
    assert!(
        route.take_emergency_custody().is_none(),
        "exclusive original position"
    );
    assert!(!route.row.state.lock().unwrap().admission_slot.collected());
    drop(parked);
    until(|| view(&route).retired).await;
    drop(retained);
    // Keep the aborted task observer alive: its backing overlaps cleanup under
    // the original8KiB. Neither End nor recovery buys another task or Root mint.
    assert!(route.recover_terminal_admission());
    assert!(route.row.state.lock().unwrap().admission_slot.collected());
    assert!(matches!(
        collect_channel(&f.state, route.selected().id(), route.selected().executor()).await,
        Ok(Status::Closed {
            native_setup_drained: true
        })
    ));
    assert!(register(&context, &route, 1).is_err());
    drop(abort);
    f.close().await;
}

#[tokio::test]
async fn native_admission_slot_emergency_staged_cleanup_needs_no_capacity_or_replacement_task() {
    let (f, context, route, _cleanup) = setup().await;
    let parked = park(&f.actor).await;
    let abort = abortable(&context, &route);
    until(|| view(&route).work && !view(&route).retired).await;
    let ui = f.state.ui_session_commit.clone().lock_owned().await;
    drop(parked);
    until(|| view(&route).phase == Phase::Staged && view(&route).retired).await;
    abort.abort();
    until(|| view(&route).returned).await;
    let resources = route.selected().resources();
    let filler = resources
        .charge(
            route.selected().binding().scope.group,
            GROUP_BYTES - resources.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    drop(ui);
    std::thread::scope(|threads| {
        for _ in 0..4 {
            threads.spawn(|| {
                route.recover_terminal_admission();
            });
        }
    });
    assert!(route.row.state.lock().unwrap().admission_slot.collected());
    assert!(view(&route).incarnation.is_none() && !view(&route).capacity);
    assert!(
        route.session().native_hold().is_ok(),
        "original position actually returned"
    );
    drop(filler);
    drop(abort);
    f.close().await;
}

#[tokio::test]
async fn native_admission_slot_shutdown_recovers_aborted_actor_work_without_maintenance() {
    let (f, context, route, _cleanup) = setup().await;
    let parked = park(&f.actor).await;
    let abort = abortable(&context, &route);
    until(|| view(&route).work && !view(&route).retired).await;
    abort.abort();
    until(|| view(&route).returned).await;
    let channels = f.state.voice_channels.clone();
    let stopping = tokio::spawn(async move { channels.stop_and_wait_for_physical().await });
    until(|| f.state.voice_channels.stopping.load(Ordering::Acquire)).await;
    let was_pending = !stopping.is_finished();
    drop(parked); // never strand the actual actor if an assertion below fails
    until(|| stopping.is_finished()).await;
    stopping.await.unwrap();
    assert!(
        was_pending,
        "shutdown cannot outrun the original admitted actor work"
    );
    assert!(route.row.state.lock().unwrap().admission_slot.collected());
    drop(abort);
    f.close().await;
}

struct CloseOnDrop(Route);
impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.close_admission_slot();
    }
}
async fn setup() -> (Fixture, Context, Route, CloseOnDrop) {
    let f = Fixture::new().await;
    let (_, selected, _) = selection(&f).await;
    let route = unacknowledged_channel_route(&f, &selected).await;
    route
        .admission_mode(Request {
            kind: Kind::Register,
            generation: 1,
        })
        .unwrap();
    route.acknowledge_setup().unwrap();
    let context = Context::new(&f.state);
    let cleanup = CloseOnDrop(route.clone());
    (f, context, route, cleanup)
}
#[derive(Debug)]
struct View {
    phase: Phase,
    generation: u64,
    floor: u64,
    incarnation: Option<[u8; 32]>,
    capacity: bool,
    work: bool,
    retired: bool,
    returned: bool,
    reclaiming: bool,
}
fn view(route: &Route) -> View {
    let state = route
        .row
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let slot = &state.admission_slot;
    let work = slot
        .invocation
        .as_ref()
        .and_then(|original| original.work.as_ref());
    View {
        phase: slot.phase,
        generation: slot.generation,
        floor: slot.cancelled_through,
        incarnation: slot
            .registration
            .as_ref()
            .map(RootAdmissionSession::incarnation),
        capacity: slot
            .capacity
            .as_ref()
            .is_some_and(RootHighPreclaim::is_current),
        work: work.is_some(),
        retired: work.is_none_or(VoiceAdmissionWork::is_retired),
        returned: slot.task_returned,
        reclaiming: slot.reclaiming,
    }
}
async fn until(mut predicate: impl FnMut() -> bool) {
    use catcoms_rt::Clock;
    let clock = catcoms_rt::SystemClock;
    tokio::select! {
        _ = async { while !predicate() { tokio::task::yield_now().await; } } => {},
        _ = clock.sleep(std::time::Duration::from_secs(15)) => panic!("native admission harness stalled"),
    }
}
fn cancel(route: &Route, generation: u64) -> Result<(), Refused> {
    let result = route
        .row
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .admission_slot
        .cancel(generation);
    route.row.admission_changed.notify_waiters();
    result
}
async fn finish(f: Fixture, route: &Route) {
    route.close_admission_slot();
    until(|| view(route).returned).await;
    f.close().await;
}

#[test]
fn native_admission_slot_task_and_concrete_callback_fit_original_fixed_fees() {
    let rpc = rpc_layout_bytes();
    let task = worker_layout(run_worker);
    eprintln!("native admission RPC {rpc}/{ADMISSION_RPC_BYTES}; persistent task {task}/{NATIVE_TASK_BYTES}");
    assert!(
        rpc <= ADMISSION_RPC_BYTES,
        "actual admission RPC {rpc}/{ADMISSION_RPC_BYTES}"
    );
    assert!(
        task <= NATIVE_TASK_BYTES,
        "actual persistent admission task {task}/{NATIVE_TASK_BYTES}"
    );
}

#[tokio::test]
async fn native_admission_slot_recovers_reply_and_reuses_one_incarnation_across_cancelled_generations(
) {
    let (f, context, route, _cleanup) = setup().await;
    register(&context, &route, 1).unwrap();
    until(|| view(&route).phase == Phase::Reserved).await;
    let original = view(&route).incarnation.unwrap();
    let held = route.selected().resources().counts();
    for _ in 0..3 {
        register(&context, &route, 1).unwrap();
    }
    assert_eq!(view(&route).incarnation, Some(original));
    assert_eq!(
        route.selected().resources().counts(),
        held,
        "lost reply starts no new task/factory/payment"
    );
    cancel(&route, 1).unwrap();
    until(|| view(&route).phase == Phase::Returned).await;
    assert_eq!(view(&route).incarnation, Some(original));
    assert!(!view(&route).capacity);
    register(&context, &route, 2).unwrap();
    until(|| view(&route).phase == Phase::Reserved && view(&route).generation == 2).await;
    cancel(&route, 1).unwrap();
    let current = view(&route);
    assert_eq!(current.incarnation, Some(original));
    assert!(current.capacity && current.floor == 1);
    assert_eq!(cancel(&route, 4), Err(Refused::Bounds));
    assert_eq!(cancel(&route, u64::MAX), Err(Refused::Bounds));
    finish(f, &route).await;
}

#[tokio::test]
async fn native_admission_slot_cancel_while_actual_actor_is_parked_preserves_original_root_outcome()
{
    let (f, context, route, _cleanup) = setup().await;
    let parked = park(&f.actor).await;
    register(&context, &route, 1).unwrap();
    until(|| view(&route).work && !view(&route).retired).await;
    cancel(&route, 1).unwrap();
    assert!(!view(&route).returned && !view(&route).retired);
    drop(parked);
    until(|| view(&route).phase == Phase::Returned).await;
    let original = view(&route).incarnation.unwrap();
    assert!(!view(&route).capacity);
    register(&context, &route, 2).unwrap();
    until(|| view(&route).phase == Phase::Reserved).await;
    assert_eq!(view(&route).incarnation, Some(original));
    finish(f, &route).await;
}

#[tokio::test]
async fn native_admission_slot_busy_ui_keeps_same_candidate_and_cleanup_needs_no_fresh_capacity() {
    let (f, context, route, _cleanup) = setup().await;
    let parked = park(&f.actor).await;
    register(&context, &route, 1).unwrap();
    until(|| view(&route).work && !view(&route).retired).await;
    let ui = f.state.ui_session_commit.clone().lock_owned().await;
    drop(parked);
    until(|| {
        let state = view(&route);
        state.phase == Phase::Staged && state.incarnation.is_some() && state.retired
    })
    .await;
    let original = view(&route).incarnation.unwrap();
    let resources = route.selected().resources();
    let filler = resources
        .charge(
            route.selected().binding().scope.group,
            GROUP_BYTES - resources.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    drop(ui);
    route.nudge_admission();
    until(|| view(&route).phase == Phase::Reserved).await;
    assert_eq!(view(&route).incarnation, Some(original));
    cancel(&route, 1).unwrap();
    until(|| view(&route).phase == Phase::Returned).await;
    assert_eq!(view(&route).incarnation, Some(original));
    route.close_admission_slot();
    until(|| view(&route).returned).await;
    drop(filler);
    f.close().await;
}

#[tokio::test]
async fn native_admission_slot_post_detach_unwind_releases_reclaiming_and_completes_terminal_cleanup(
) {
    let (f, context, route, _cleanup) = setup().await;
    register(&context, &route, 1).unwrap();
    until(|| view(&route).phase == Phase::Reserved).await;
    route
        .row
        .state
        .lock()
        .unwrap()
        .admission_slot
        .unwind_reclamation = true;
    cancel(&route, 1).unwrap();
    until(|| view(&route).returned).await;
    let terminal = view(&route);
    assert_eq!(terminal.phase, Phase::Closed);
    assert!(
        !terminal.reclaiming
            && !terminal.work
            && terminal.incarnation.is_none()
            && !terminal.capacity
    );
    assert!(
        register(&context, &route, 2).is_err(),
        "uncertainty never remints inside this physical Session"
    );
    f.close().await;
}
