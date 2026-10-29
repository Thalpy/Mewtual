//! Actual admitted Actor/Root/physical owners, not a published-capacity stub.
use super::*;
use crate::voice_handoff::bootstrap::AdmittedRoute;
use crate::voice_handoff::context::Context;
use crate::voice_handoff::requests::admitted_admission::{Kind, Request, State as View};
use catcoms_rt::Clock;
use catcoms_sync::voice_v2::resources::{ResourceKind, GROUP_BYTES};

async fn setup() -> (Fixture, Arc<ManualClock>, Handle, AdmittedRoute, Context) {
    let clock = Arc::new(ManualClock::new(1000));
    let f = Fixture::with_voice_clock(clock.clone()).await;
    let handle = activate(&f, &controller(&f).await).await;
    let output = ready(
        ensure_admitted(&f.state, handle.id(), [9; 32], &mut Rng::new())
            .await
            .unwrap(),
    );
    let route = f
        .state
        .voice_channels
        .admitted_route(output.public().slot)
        .unwrap();
    let context = Context::new(&f.state);
    drop(output);
    (f, clock, handle, route, context)
}
fn command(
    route: &AdmittedRoute,
    context: &Context,
    kind: Kind,
    g: u64,
) -> super::super::super::super::requests::admitted_admission::Snapshot {
    route
        .admitted_control(
            context,
            Request {
                kind,
                generation: g,
            },
        )
        .unwrap()
}
async fn observed(
    route: &AdmittedRoute,
    g: u64,
    wanted: View,
) -> super::super::super::super::requests::admitted_admission::Snapshot {
    bounded(async {
        loop {
            route.nudge_admitted();
            let snapshot = route.admitted_snapshot(g).unwrap();
            if snapshot.state == wanted {
                return snapshot;
            }
            assert!(
                !matches!(snapshot.state, View::Closed | View::Uncertain),
                "unexpected terminal {:?}",
                snapshot
            );
            catcoms_rt::SystemClock
                .sleep(std::time::Duration::from_millis(1))
                .await;
        }
    })
    .await
}
async fn cleanup(f: Fixture, clock: Arc<ManualClock>, handle: Handle, route: AdmittedRoute) {
    handle.close();
    route.nudge_admitted();
    end(&f, &clock, &handle).await;
    bounded(async {
        while !route.admitted_drained_for_test() {
            route.nudge_admitted();
            catcoms_rt::SystemClock
                .sleep(std::time::Duration::from_millis(1))
                .await;
        }
    })
    .await;
    drop(route);
    settle_admitted_setup(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    collect_admitted_channel(&f.state, handle.id(), [9; 32])
        .await
        .unwrap();
    bounded(f.close()).await;
}

#[test]
fn admitted_slot_complete_task_and_concrete_publisher_invoices_fit_before_allocation() {
    let (task, rpc) = AdmittedRoute::admitted_graphs_for_test();
    eprintln!("admitted native Slot/task {task}/8192; concrete RPC/publisher {rpc}/4096");
    assert!(
        task <= 8192,
        "never cut task/return/reclamation terms to fit"
    );
    assert!(
        rpc <= 4096,
        "never hide concrete native publisher behind a trait size"
    );
}
#[tokio::test]
async fn admitted_setup_ack_and_observation_never_create_a_slot_or_root_registration() {
    let (f, clock, handle, route, context) = bounded(setup()).await;
    let resources = route.selected().resources();
    let before = resources.counts();
    assert_eq!(
        command(&route, &context, Kind::Observe, 1).state,
        View::Unstarted
    );
    assert!(
        route
            .admitted_control(
                &context,
                Request {
                    kind: Kind::Register,
                    generation: 1
                }
            )
            .is_err(),
        "SDK setup not acknowledged"
    );
    assert_eq!(resources.counts(), before);
    let ack = command(&route, &context, Kind::SetupAck, 0);
    assert_eq!(ack.canonical, 0);
    assert!(ack.incarnation.is_none());
    let after = resources.counts();
    assert_eq!(command(&route, &context, Kind::SetupAck, 0), ack);
    assert_eq!(resources.counts(), after);
    assert_eq!(
        command(&route, &context, Kind::Cancel, 1).state,
        View::Returned
    );
    assert!(route
        .admitted_control(
            &context,
            Request {
                kind: Kind::Cancel,
                generation: u64::MAX
            }
        )
        .is_err());
    assert_eq!(command(&route, &context, Kind::Observe, 1).cancelled, 1);
    command(&route, &context, Kind::Register, 2);
    let reserved = observed(&route, 2, View::Reserved).await;
    assert_eq!(reserved.canonical, 2);
    assert!(reserved.incarnation.is_some());
    cleanup(f, clock, handle, route).await;
}
#[tokio::test]
async fn admitted_slot_reuses_whole_root_candidate_after_ledger_busy_and_canceled_generation() {
    let (f, clock, handle, route, context) = bounded(setup()).await;
    command(&route, &context, Kind::SetupAck, 0);
    let ledger = f.state.voice_handoff.clone();
    let mut admitted = Vec::new();
    let mut existing = Vec::new();
    for g in 10..26 {
        admitted.push(
            ledger
                .reserve_admitted_parent(
                    route.selected().clone(),
                    route.session().identity().clone(),
                    g,
                )
                .unwrap(),
        );
        existing.push(crate::voice_handoff::ledger::fixture_existing_claim(
            &ledger,
        ));
    }
    command(&route, &context, Kind::Register, 1);
    let incarnation = bounded(async {
        loop {
            route.nudge_admitted();
            let snapshot = route.admitted_snapshot(1).unwrap();
            if let Some(incarnation) = snapshot.incarnation {
                assert_ne!(snapshot.state, View::Reserved);
                break incarnation;
            }
            catcoms_rt::SystemClock
                .sleep(std::time::Duration::from_millis(1))
                .await;
        }
    })
    .await;
    let before = route.selected().resources().counts();
    for _ in 0..8 {
        command(&route, &context, Kind::Register, 1);
        route.nudge_admitted();
        catcoms_rt::SystemClock
            .sleep(std::time::Duration::from_millis(1))
            .await;
        assert_eq!(
            route.admitted_snapshot(1).unwrap().incarnation,
            Some(incarnation)
        );
    }
    assert_eq!(
        route.selected().resources().counts(),
        before,
        "local Busy cannot remint or replace the original candidate"
    );
    drop(existing.pop());
    assert_eq!(
        observed(&route, 1, View::Reserved).await.incarnation,
        Some(incarnation)
    );
    command(&route, &context, Kind::Cancel, 1);
    observed(&route, 1, View::Returned).await;
    drop(admitted);
    drop(existing);
    command(&route, &context, Kind::Register, 2);
    assert_eq!(
        observed(&route, 2, View::Reserved).await.incarnation,
        Some(incarnation)
    );
    assert_eq!(
        command(&route, &context, Kind::Observe, 1).state,
        View::Superseded
    );
    cleanup(f, clock, handle, route).await;
}
#[tokio::test]
async fn actual_admitted_task_abort_recovers_same_native_hold_under_full_ordinary_pool() {
    let (f, clock, handle, route, context) = bounded(setup()).await;
    command(&route, &context, Kind::SetupAck, 0);
    let task = super::super::super::admitted_slot::register_observing_task(&context, &route, 1)
        .unwrap()
        .unwrap();
    observed(&route, 1, View::Reserved).await;
    let resources = route.selected().resources();
    let group = route.selected().binding().scope.group;
    let pressure = resources
        .charge(
            group,
            GROUP_BYTES - resources.group_counts(group).unwrap().0,
            ResourceKind::Retained,
        )
        .unwrap();
    assert_eq!(resources.group_counts(group).unwrap().0, GROUP_BYTES);
    let ordinary_sender = f.actor.clone();
    task.abort();
    bounded(async {
        while !route.admitted_task_returned_for_test() {
            catcoms_rt::SystemClock
                .sleep(std::time::Duration::from_millis(1))
                .await;
        }
    })
    .await;
    route.nudge_admitted();
    assert!(route.admitted_drained_for_test());
    assert!(
        route.selected().lease().is_live(),
        "Slot cancellation cannot end the underlying call"
    );
    assert!(
        resources.group_counts(group).unwrap().0 < GROUP_BYTES,
        "only actual retained owners return capacity"
    );
    assert!(
        route
            .admitted_control(
                &context,
                Request {
                    kind: Kind::Register,
                    generation: 1
                }
            )
            .is_err(),
        "no replacement task after unexpected return"
    );
    drop(pressure);
    drop(ordinary_sender);
    cleanup(f, clock, handle, route).await;
}
#[tokio::test]
async fn admitted_actual_reclamation_unwind_returns_owned_candidate_without_a_replacement_task() {
    let (f, clock, handle, route, context) = bounded(setup()).await;
    command(&route, &context, Kind::SetupAck, 0);
    command(&route, &context, Kind::Register, 1);
    observed(&route, 1, View::Reserved).await;
    route.unwind_admitted_reclamation_for_test();
    command(&route, &context, Kind::Cancel, 1);
    bounded(async {
        while !route.admitted_drained_for_test() {
            route.nudge_admitted();
            catcoms_rt::SystemClock
                .sleep(std::time::Duration::from_millis(1))
                .await;
        }
    })
    .await;
    assert_eq!(route.admitted_snapshot(1).unwrap().state, View::Uncertain);
    assert!(
        route
            .admitted_control(
                &context,
                Request {
                    kind: Kind::Register,
                    generation: 2
                }
            )
            .is_err(),
        "unwind cannot replace the original task/registration"
    );
    assert!(
        handle.active(),
        "admission cleanup cannot terminate the whole call"
    );
    cleanup(f, clock, handle, route).await;
}

#[tokio::test]
async fn aborted_native_task_keeps_actual_queued_work_until_the_actor_returns_its_whole_result() {
    let (f, clock, handle, route, context) = bounded(setup()).await;
    command(&route, &context, Kind::SetupAck, 0);
    // A genuine durable-send Ready parks the ordinary Actor before this voice
    // command. Its Drop cancels the unsent message on every unwind path too;
    // this does not replace the voice handler or manufacture a returned Work.
    let send_context = bounded(f.actor.durable_send_context()).await.unwrap();
    let parked = bounded(f.actor.prepare_durable_send(
        catcoms_app::durable_chat::DurableSendRequest {
            token: [32; 16],
            expected_context: send_context,
            channel: catcoms_app::channel_id("general"),
            text: "never authored".into(),
            reply_to: String::new(),
        },
    ))
    .await
    .unwrap();
    let task = super::super::super::admitted_slot::register_observing_task(&context, &route, 1)
        .unwrap()
        .unwrap();
    let work = bounded(async {
        loop {
            if let Some(work) = route.admitted_work() {
                break work;
            }
            catcoms_rt::SystemClock
                .sleep(std::time::Duration::from_millis(1))
                .await;
        }
    })
    .await;
    assert!(
        !work.is_retired(),
        "the actual command is queued behind Ready"
    );
    let ordinary_sender = f.actor.clone();
    task.abort();
    bounded(async {
        while !route.admitted_task_returned_for_test() {
            catcoms_rt::SystemClock
                .sleep(std::time::Duration::from_millis(1))
                .await;
        }
    })
    .await;
    route.nudge_admitted();
    assert!(!work.is_retired());
    assert!(
        !route.admitted_drained_for_test(),
        "owed original Work blocks physical collection"
    );
    // Root may still mint after the native observer disappears: the accepted
    // original command was never canceled. Preserve/destroy its WHOLE eventual
    // result; native task return must not be interpreted as NoMint.
    drop(parked);
    bounded(work.wait_retired()).await;
    route.nudge_admitted();
    assert!(route.admitted_drained_for_test());
    assert_eq!(route.admitted_snapshot(1).unwrap().state, View::Closed);
    assert!(route
        .admitted_control(
            &context,
            Request {
                kind: Kind::Register,
                generation: 1
            }
        )
        .is_err());
    assert!(
        handle.active(),
        "negative recovery cannot end the underlying call"
    );
    bounded(ordinary_sender.member_count()).await;
    drop(work);
    drop(ordinary_sender);
    cleanup(f, clock, handle, route).await;
}

#[tokio::test]
async fn next_generation_cancel_before_native_entry_is_noncreating_and_late_register_cannot_revive()
{
    let (f, clock, handle, route, context) = bounded(setup()).await;
    command(&route, &context, Kind::SetupAck, 0);
    let before = route.selected().resources().counts();
    for g in 1..=3 {
        let canceled = command(&route, &context, Kind::Cancel, g);
        assert_eq!(
            (canceled.state, canceled.canonical, canceled.cancelled),
            (View::Returned, g, g)
        );
        assert!(canceled.incarnation.is_none());
        assert_eq!(
            command(&route, &context, Kind::Register, g).state,
            View::Returned
        );
        assert_eq!(
            route.selected().resources().counts(),
            before,
            "negative advance allocates no task/Box/RPC"
        );
    }
    assert!(route
        .admitted_control(
            &context,
            Request {
                kind: Kind::Cancel,
                generation: 5
            }
        )
        .is_err());
    assert!(route
        .admitted_control(
            &context,
            Request {
                kind: Kind::Cancel,
                generation: u64::MAX
            }
        )
        .is_err());
    assert_eq!(command(&route, &context, Kind::Observe, 3).cancelled, 3);
    assert!(
        handle.active(),
        "negative per-generation cleanup preserves siblings/whole call"
    );
    command(&route, &context, Kind::Register, 4);
    let first = observed(&route, 4, View::Reserved).await;
    let incarnation = first.incarnation;
    command(&route, &context, Kind::Cancel, 4);
    observed(&route, 4, View::Returned).await;
    // A browser's g5 Register was lost BEFORE entry: exact-next Cancel is a
    // genuine negative fact, not a reason to mint g5 just to clean it up.
    let canceled = command(&route, &context, Kind::Cancel, 5);
    assert_eq!(
        (canceled.state, canceled.canonical, canceled.cancelled),
        (View::Returned, 5, 5)
    );
    assert_eq!(
        command(&route, &context, Kind::Register, 5).state,
        View::Returned
    );
    command(&route, &context, Kind::Register, 6);
    assert_eq!(
        observed(&route, 6, View::Reserved).await.incarnation,
        incarnation
    );
    cleanup(f, clock, handle, route).await;
}

#[tokio::test]
async fn canceled_waiting_snapshot_is_closing_until_actual_return_and_matches_v12_decoder_contract()
{
    let (f, clock, handle, route, context) = bounded(setup()).await;
    command(&route, &context, Kind::SetupAck, 0);
    // The actual physical Session already owns one Crypto lane. Fill its three
    // remaining real lanes so Root returns Some(registration), Capacity.
    let resources = route.selected().resources();
    let group = route.selected().binding().scope.group;
    let pressure: Vec<_> = (0..3)
        .map(|_| resources.charge(group, 1, ResourceKind::Crypto).unwrap())
        .collect();
    command(&route, &context, Kind::Register, 1);
    let waiting = bounded(async {
        loop {
            route.nudge_admitted();
            let snapshot = route.admitted_snapshot(1).unwrap();
            if snapshot.state == View::Waiting && snapshot.incarnation.is_some() {
                break snapshot;
            }
            catcoms_rt::SystemClock
                .sleep(std::time::Duration::from_millis(1))
                .await;
        }
    })
    .await;
    // No await: this is the transient after the negative floor but before worker
    // reclamation. Waiting+floor==g would contradict the closed browser grammar.
    let canceled = command(&route, &context, Kind::Cancel, 1);
    assert_eq!((canceled.state, canceled.cancelled), (View::Closing, 1));
    assert_eq!(canceled.incarnation, waiting.incarnation);
    assert_eq!(canceled.encode()[1], 4);
    observed(&route, 1, View::Returned).await;
    drop(pressure);
    cleanup(f, clock, handle, route).await;
}

#[tokio::test]
async fn actual_cancel_during_accepted_or_final_attachment_preserves_registration_for_successor() {
    for after_accept in [true, false] {
        let (f, clock, handle, route, context) = bounded(setup()).await;
        command(&route, &context, Kind::SetupAck, 0);
        command(&route, &context, Kind::Register, 1);
        let original = observed(&route, 1, View::Reserved).await.incarnation;
        command(&route, &context, Kind::Cancel, 1);
        observed(&route, 1, View::Returned).await;
        route.cancel_admitted_race_for_test(after_accept);
        command(&route, &context, Kind::Register, 2);
        let canceled = observed(&route, 2, View::Returned).await;
        assert_eq!(canceled.incarnation, original);
        assert!(handle.active());
        command(&route, &context, Kind::Register, 3);
        assert_eq!(
            observed(&route, 3, View::Reserved).await.incarnation,
            original
        );
        cleanup(f, clock, handle, route).await;
    }
}
