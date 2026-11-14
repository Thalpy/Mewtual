//! Actual tracked envelopes/constructors, with deterministic destruction cuts.
//! No native authorization, Root vault or physical network acceptance is claimed.
use super::*;
use std::sync::{atomic::AtomicBool, Mutex as StdMutex};

#[derive(Clone)]
struct Notice {
    dropped: Arc<AtomicBool>,
    order: Arc<StdMutex<Vec<&'static str>>>,
}
struct CustodyNotice(Notice);
impl Drop for CustodyNotice {
    fn drop(&mut self) {
        self.0
            .order
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push("custody");
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}
fn preparation() -> (NativeTcpPreparation, Notice) {
    let notice = Notice {
        dropped: Arc::new(AtomicBool::new(false)),
        order: Arc::new(StdMutex::new(Vec::new())),
    };
    (
        NativeTcpPreparation::new(CustodyNotice(notice.clone()), Arc::new(())),
        notice,
    )
}
fn observed_order(notice: &Notice) -> Vec<&'static str> {
    notice
        .order
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}
struct Probe {
    order: Arc<StdMutex<Vec<&'static str>>>,
    ready: bool,
    panic: bool,
    held: Option<(Arc<Notify>, std::sync::mpsc::Receiver<()>)>,
}
impl Future for Probe {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        assert!(!self.panic, "actual key-future poll interrupted");
        if self.ready {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        if let Some((entered, release)) = &self.held {
            entered.notify_one();
            let _ = release.recv();
        }
        self.order
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push("future");
    }
}
fn tracked(preparation: &NativeTcpPreparation, probe: Probe) -> Tracked {
    Tracked::new(
        Box::pin(probe),
        &preparation.executor,
        preparation.executor.state.enter().unwrap(),
    )
}
async fn bounded<T>(work: impl Future<Output = T>) -> T {
    tokio::select! {
        value = work => value,
        _ = SystemClock.sleep(Duration::from_secs(10)) => panic!("native TCP lifetime check exceeded its bound"),
    }
}

#[test]
fn native_tcp_unpolled_preparation_closes_original_and_observer_has_no_custody() {
    let (preparation, notice) = preparation();
    let observation = preparation.observe_drain();
    assert!(!observation.is_drained());
    drop(preparation);
    assert!(observation.is_drained());
    assert!(notice.dropped.load(Ordering::SeqCst));
    assert_eq!(observed_order(&notice), ["custody"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_tcp_unpolled_task_keeps_original_custody_and_count_until_real_destructor_finishes()
{
    let (preparation, notice) = preparation();
    let observation = preparation.observe_drain();
    let entered = Arc::new(Notify::new());
    let (release, released) = std::sync::mpsc::channel();
    struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(release) = self.0.take() {
                let _ = release.send(());
            }
        }
    }
    let release = ReleaseOnDrop(Some(release));
    let task = tracked(
        &preparation,
        Probe {
            order: notice.order.clone(),
            ready: false,
            panic: false,
            held: Some((entered.clone(), released)),
        },
    );
    drop(preparation);
    let worker = std::thread::spawn(move || drop(task));
    // Record the negative observations, then ALWAYS release and join before
    // asserting. A broken destructor cannot strand this worker on test failure.
    let arrived = tokio::select! {
        _ = entered.notified() => true,
        _ = SystemClock.sleep(Duration::from_secs(10)) => false,
    };
    let drained_early = observation.is_drained();
    let custody_dropped_early = notice.dropped.load(Ordering::SeqCst);
    drop(release);
    let joined = worker.join();
    assert!(
        arrived,
        "actual destructor did not reach the controlled boundary"
    );
    assert!(joined.is_ok());
    assert!(!drained_early);
    assert!(!custody_dropped_early);
    assert!(observation.is_drained());
    assert!(notice.dropped.load(Ordering::SeqCst));
    assert_eq!(observed_order(&notice), ["future", "custody"]);
}

#[test]
fn native_tcp_ready_retires_actual_future_before_retained_task_allocation() {
    let (preparation, notice) = preparation();
    let observation = preparation.observe_drain();
    let mut task = tracked(
        &preparation,
        Probe {
            order: notice.order.clone(),
            ready: true,
            panic: false,
            held: None,
        },
    );
    let waker = futures::task::noop_waker();
    assert!(Pin::new(&mut task)
        .poll(&mut Context::from_waker(&waker))
        .is_ready());
    assert_eq!(observed_order(&notice), ["future"]);
    assert!(
        !observation.is_drained(),
        "the open Actor original still exists"
    );
    drop(preparation);
    assert!(observation.is_drained());
    assert!(
        notice.dropped.load(Ordering::SeqCst),
        "completed task allocation must not retain physical custody"
    );
    drop(task);
    assert_eq!(observed_order(&notice), ["future", "custody"]);
}

#[test]
fn native_tcp_poll_unwind_drops_future_before_custody_and_original_ack() {
    let (preparation, notice) = preparation();
    let observation = preparation.observe_drain();
    let task = tracked(
        &preparation,
        Probe {
            order: notice.order.clone(),
            ready: false,
            panic: true,
            held: None,
        },
    );
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let mut task = task;
        let waker = futures::task::noop_waker();
        let _ = Pin::new(&mut task).poll(&mut Context::from_waker(&waker));
    }));
    assert!(caught.is_err());
    assert_eq!(observed_order(&notice), ["future"]);
    drop(preparation);
    assert!(observation.is_drained());
    assert_eq!(observed_order(&notice), ["future", "custody"]);
}

#[test]
fn native_tcp_registration_is_bounded_and_closed_zero_never_reopens() {
    let (preparation, _) = preparation();
    let state = preparation.executor.state.clone();
    let mut ends = Vec::new();
    for _ in 1..MAX_TASKS {
        ends.push(state.enter().unwrap());
    }
    assert!(state.enter().is_none());
    state.close();
    assert!(state.enter().is_none());
    assert!(!state.drained());
    drop(ends);
    assert!(
        !state.drained(),
        "the Actor/construction token is still real"
    );
    drop(preparation);
    assert!(state.drained());
    assert!(state.enter().is_none());
}

#[tokio::test]
async fn native_tcp_constructor_refuses_quic_and_retains_original_failed_observation() {
    let (mut preparation, notice) = preparation();
    let observation = preparation.observe_drain();
    let address = "/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap();
    assert!(preparation
        .build(keypair_from_seed([51; 32]).unwrap(), &[address], &[])
        .is_err());
    bounded(observation.wait()).await;
    assert!(preparation
        .build(keypair_from_seed([51; 32]).unwrap(), &[], &[])
        .is_err());
    assert!(
        !notice.dropped.load(Ordering::SeqCst),
        "failed preparation still owns its passive original"
    );
    drop(preparation);
    assert!(notice.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn native_tcp_owner_close_bypasses_full_command_queue_and_surviving_handles() {
    let (mut preparation, _) = preparation();
    let observation = preparation.observe_drain();
    let (mesh, _, _) = preparation
        .build(keypair_from_seed([52; 32]).unwrap(), &[], &[])
        .unwrap();
    let handle = mesh.handle();
    let mut queued = 0;
    while handle
        .cmd_tx
        .try_send(Command::Dial {
            addr: "/memory/1".parse().unwrap(),
            permit: None,
            reply: None,
        })
        .is_ok()
    {
        queued += 1;
    }
    assert_eq!(queued, 256, "actual queue full before this task yields");
    drop(mesh);
    drop(preparation);
    bounded(observation.wait()).await;
    assert!(handle.cmd_tx.is_closed());
}

#[tokio::test]
async fn native_tcp_actual_executor_exhaustion_closes_actor_instead_of_stranding_pending_dial() {
    let (mut preparation, _) = preparation();
    let observation = preparation.observe_drain();
    let (mesh, _, _) = preparation
        .build(keypair_from_seed([56; 32]).unwrap(), &[], &[])
        .unwrap();
    let handle = mesh.handle();
    let mut occupied = Vec::new();
    for _ in 1..MAX_TASKS {
        occupied.push(preparation.executor.state.enter().unwrap());
    }
    // Actual Actor -> Swarm::dial -> Pool -> supplied Executor. A dropped
    // pending future alone leaves the shared Pool event channel open; the
    // original Actor must terminate instead. No actual socket connect runs.
    handle
        .dial(
            "/ip4/127.0.0.1/tcp/9"
                .parse::<Multiaddr>()
                .unwrap()
                .with(Protocol::P2p(
                    keypair_from_seed([57; 32]).unwrap().public().to_peer_id(),
                )),
        )
        .await
        .unwrap();
    assert!(bounded(mesh.next_listen_addr()).await.is_none());
    assert!(observation.is_closed());
    assert!(handle.cmd_tx.is_closed());
    assert!(
        !observation.is_drained(),
        "the held task registrations are still real"
    );
    drop(occupied);
    bounded(observation.wait()).await;
    drop(mesh);
    drop(preparation);
}

#[tokio::test]
async fn native_tcp_actual_connection_drains_with_original_owner_and_kept_command_handles() {
    let (mut first, first_notice) = preparation();
    let (mut second, second_notice) = preparation();
    let first_drain = first.observe_drain();
    let second_drain = second.observe_drain();
    let (left, left_id, _) = first
        .build(
            keypair_from_seed([53; 32]).unwrap(),
            &["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            &[],
        )
        .unwrap();
    let address = bounded(left.next_listen_addr())
        .await
        .unwrap()
        .with(Protocol::P2p(left_id));
    let (right, _, _) = second
        .build(keypair_from_seed([54; 32]).unwrap(), &[], &[address])
        .unwrap();
    bounded(left.wait_for_peer_connected(right.local_peer()))
        .await
        .unwrap();
    bounded(right.wait_for_peer_connected(left.local_peer()))
        .await
        .unwrap();
    let left_lease =
        connection_leases::current(&left.connection_leases, right.local_peer()).unwrap();
    let right_lease =
        connection_leases::current(&right.connection_leases, left.local_peer()).unwrap();
    let mut left_snapshot = left.connection_snapshot_rx.clone();
    let mut right_snapshot = right.connection_snapshot_rx.clone();
    let left_handle = left.handle();
    let right_handle = right.handle();
    drop(left);
    drop(right);
    drop(first);
    drop(second);
    bounded(async {
        tokio::join!(first_drain.wait(), second_drain.wait());
    })
    .await;
    assert!(left_handle.cmd_tx.is_closed() && right_handle.cmd_tx.is_closed());
    assert!(!left_lease.is_live() && !right_lease.is_live());
    assert!(current_connection_snapshot(&mut left_snapshot).is_err());
    assert!(current_connection_snapshot(&mut right_snapshot).is_err());
    assert!(left_handle.authenticated_dial_routes().is_empty());
    assert!(right_handle.authenticated_dial_routes().is_empty());
    assert!(first_notice.dropped.load(Ordering::SeqCst));
    assert!(second_notice.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn native_tcp_does_not_change_legacy_handle_ownership() {
    let (legacy, _, _) =
        MeshService::new_tcp_with_key(keypair_from_seed([55; 32]).unwrap(), &[], &[]).unwrap();
    assert!(legacy._native_tcp_owner.is_none());
    let handle = legacy.handle();
    drop(legacy);
    assert!(
        !handle.cmd_tx.is_closed(),
        "legacy command handle still keeps its existing Actor alive"
    );
    drop(handle);
}
