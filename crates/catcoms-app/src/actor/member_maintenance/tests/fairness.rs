//! Deterministic process fairness/cancellation tests. Original Flight keepalive clones
//! stand for real lower custody; no timer or synthetic slot refund advances the queue.
use super::*;

fn poll_once<F: Future>(future: F) -> std::task::Poll<F::Output> {
    let mut future = std::pin::pin!(future);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    future.as_mut().poll(&mut cx)
}

struct WakeCount(std::sync::atomic::AtomicUsize);
impl std::task::Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

fn queued(capacity: &Arc<Capacity>, peer: u64) -> Maintenance {
    let mut owner = Maintenance::with_capacity(capacity.clone());
    owner.enqueue(PeerId::from_u64(peer), true, None);
    assert!(owner
        .reserve(owner.chains[&PeerId::from_u64(peer)].peer.clone())
        .is_none());
    assert!(matches!(owner.admission, Admission::Waiting(_)));
    assert_eq!(
        owner.local.available_permits(),
        LOCAL_FLIGHTS,
        "waiting globally must retain no local position"
    );
    owner
}

#[tokio::test]
async fn eight_registered_actors_keep_fifo_turns_across_cancelled_polls_and_hot_reacquisition() {
    let capacity = Capacity::new(PROCESS_FLIGHTS);
    let mut hot = Maintenance::with_capacity(capacity.clone());
    let mut other = Maintenance::with_capacity(capacity.clone());
    let first = reserve_for(&mut hot, 1);
    let held = [
        reserve_for(&mut hot, 2),
        reserve_for(&mut other, 3),
        reserve_for(&mut other, 4),
    ];
    let lower = first.clone();
    let mut waiting: Vec<_> = (10..18)
        .map(|peer| (peer, queued(&capacity, peer)))
        .collect();
    for (_, owner) in &mut waiting {
        for _ in 0..32 {
            let _ = poll_once(owner.next());
        }
        assert!(matches!(owner.admission, Admission::Waiting(_)));
    }
    drop(first);
    assert_eq!(capacity.slots.available_permits(), 0);
    assert!(
        hot.reserve(hot.chains[&PeerId::from_u64(1)].peer.clone())
            .is_none(),
        "logical caller Drop cannot return actual lower custody"
    );
    drop(lower); // only now can the oldest stored acquisition receive this position
    for (index, (peer, owner)) in waiting.iter_mut().enumerate() {
        // Hot pumps FIRST on every turn. A cancelled/acquired-and-recreated future would
        // lose FIFO priority and let it steal the released position in this exact schedule.
        assert!(hot
            .reserve(hot.chains[&PeerId::from_u64(1)].peer.clone())
            .is_none());
        let flight = owner
            .reserve(owner.chains[&PeerId::from_u64(*peer)].peer.clone())
            .unwrap_or_else(|| panic!("registered actor{index} lost its original FIFO place"));
        assert_eq!(capacity.slots.available_permits(), 0);
        let actual_lower = flight.clone();
        drop(flight);
        assert!(hot
            .reserve(hot.chains[&PeerId::from_u64(1)].peer.clone())
            .is_none());
        drop(actual_lower);
    }
    assert!(
        hot.reserve(hot.chains[&PeerId::from_u64(1)].peer.clone())
            .is_some(),
        "hot actor remains eligible after all earlier registered owners got their turn"
    );
    drop((waiting, held, hot, other));
    assert_eq!(capacity.slots.available_permits(), PROCESS_FLIGHTS);
}

#[tokio::test]
async fn observed_ready_grant_survives_another_cancelled_outer_select_turn() {
    let capacity = Capacity::new(PROCESS_FLIGHTS);
    let mut hot = Maintenance::with_capacity(capacity.clone());
    let mut other = Maintenance::with_capacity(capacity.clone());
    let first = reserve_for(&mut hot, 1);
    let held = [
        reserve_for(&mut hot, 2),
        reserve_for(&mut other, 3),
        reserve_for(&mut other, 4),
    ];
    let mut waiting = queued(&capacity, 5);
    drop(first);
    assert!(poll_once(waiting.admission.wait_ready()).is_ready());
    assert!(matches!(waiting.admission, Admission::Ready(_)));
    let _ = poll_once(waiting.next());
    assert!(matches!(waiting.admission, Admission::Ready(_)));
    assert!(hot
        .reserve(hot.chains[&PeerId::from_u64(1)].peer.clone())
        .is_none());
    let actual = waiting
        .reserve(waiting.chains[&PeerId::from_u64(5)].peer.clone())
        .unwrap();
    drop((actual, waiting, held, hot, other));
    assert_eq!(capacity.slots.available_permits(), PROCESS_FLIGHTS);
}

#[tokio::test]
async fn dropping_assigned_but_unobserved_future_returns_only_its_unused_process_position() {
    let capacity = Capacity::new(PROCESS_FLIGHTS);
    let mut a = Maintenance::with_capacity(capacity.clone());
    let mut b = Maintenance::with_capacity(capacity.clone());
    let first = reserve_for(&mut a, 1);
    let held = [
        reserve_for(&mut a, 2),
        reserve_for(&mut b, 3),
        reserve_for(&mut b, 4),
    ];
    let abandoned = queued(&capacity, 5);
    let mut following = queued(&capacity, 6);
    drop(first);
    assert!(
        matches!(abandoned.admission, Admission::Waiting(_)),
        "semaphore assignment precedes the owner's next poll"
    );
    assert_eq!(capacity.slots.available_permits(), 0);
    drop(abandoned);
    let actual = following
        .reserve(following.chains[&PeerId::from_u64(6)].peer.clone())
        .unwrap();
    assert_eq!(
        capacity.slots.available_permits(),
        0,
        "three real flights and the next owner still hold all four"
    );
    drop((actual, following, held, a, b));
    assert_eq!(capacity.slots.available_permits(), PROCESS_FLIGHTS);
}

#[tokio::test]
async fn assigned_future_drop_wakes_following_actor_without_a_flight_or_broadcast_signal() {
    let capacity = Capacity::new(PROCESS_FLIGHTS);
    let mut a = Maintenance::with_capacity(capacity.clone());
    let mut b = Maintenance::with_capacity(capacity.clone());
    let first = reserve_for(&mut a, 1);
    let held = [
        reserve_for(&mut a, 2),
        reserve_for(&mut b, 3),
        reserve_for(&mut b, 4),
    ];
    let mut abandoned = queued(&capacity, 5);
    let mut following = queued(&capacity, 6);
    drop(first); // assign abandoned, without polling its stored acquisition
    following.changed.borrow_and_update(); // consume ONLY this earlier flight's wake
    let wake = Arc::new(WakeCount(std::sync::atomic::AtomicUsize::new(0)));
    let waker = std::task::Waker::from(wake.clone());
    let mut cx = std::task::Context::from_waker(&waker);
    {
        let mut next = std::pin::pin!(following.next());
        assert!(next.as_mut().poll(&mut cx).is_pending());
        let before = wake.0.load(Ordering::Acquire);
        // Drop ONLY the stored acquisition, retaining the rest of its owner: dropping the
        // whole Maintenance would also drop PeerCustody, whose broadcast would mask a lost
        // semaphore wake. This drop has NO Flight and calls NO Capacity::signal.
        drop(std::mem::take(&mut abandoned.admission));
        assert!(wake.0.load(Ordering::Acquire) > before);
        assert!(matches!(
            next.as_mut().poll(&mut cx),
            std::task::Poll::Ready(Event::Wake)
        ));
    }
    assert!(
        !following.changed.has_changed().unwrap(),
        "no unrelated broadcast supplied this wake"
    );
    assert!(matches!(following.admission, Admission::Ready(_)));
    let actual = following
        .reserve(following.chains[&PeerId::from_u64(6)].peer.clone())
        .unwrap();
    drop((actual, following, abandoned, held, a, b));
    assert_eq!(capacity.slots.available_permits(), PROCESS_FLIGHTS);
}

#[tokio::test]
async fn local_saturation_returns_assigned_unobserved_grant_without_touching_other_flights() {
    let capacity = Capacity::new(PROCESS_FLIGHTS);
    let mut a = Maintenance::with_capacity(capacity.clone());
    let mut b = Maintenance::with_capacity(capacity.clone());
    let first = reserve_for(&mut a, 1);
    let held = [
        reserve_for(&mut a, 2),
        reserve_for(&mut b, 3),
        reserve_for(&mut b, 4),
    ];
    let mut waiting = queued(&capacity, 5);
    let mut following = queued(&capacity, 6);
    drop(first);
    // Deterministically occupy the real local semaphore before the assigned process grant
    // is observed. This isolated capacity fixture supplies no request/transport authority.
    let local = waiting
        .local
        .clone()
        .try_acquire_many_owned(LOCAL_FLIGHTS as u32)
        .unwrap();
    waiting.pump(&mut server());
    assert!(matches!(waiting.admission, Admission::Idle));
    let actual = following
        .reserve(following.chains[&PeerId::from_u64(6)].peer.clone())
        .unwrap();
    assert_eq!(capacity.slots.available_permits(), 0);
    drop((actual, local, waiting, following, held, a, b));
    assert_eq!(capacity.slots.available_permits(), PROCESS_FLIGHTS);
}

#[tokio::test]
async fn queue_removal_and_lower_only_peers_do_not_hoard_a_fifo_grant() {
    for empty in [false, true] {
        let capacity = Capacity::new(PROCESS_FLIGHTS);
        let mut waiting = Maintenance::with_capacity(capacity.clone());
        let mut other = Maintenance::with_capacity(capacity.clone());
        let real_lower = reserve_for(&mut waiting, 1);
        let first = reserve_for(&mut other, 2);
        let third = reserve_for(&mut other, 3);
        let mut fourth_owner = Maintenance::with_capacity(capacity.clone());
        let fourth = reserve_for(&mut fourth_owner, 4);
        waiting.enqueue(PeerId::from_u64(5), true, None);
        assert!(waiting
            .reserve(waiting.chains[&PeerId::from_u64(5)].peer.clone())
            .is_none());
        let mut following = queued(&capacity, 6);
        // Remove the unstarted eligible work while the SAME peer1 Lower still owns its
        // original flight. These private queue mutations isolate pump's no-ready-work arm.
        waiting.chains.remove(&PeerId::from_u64(5));
        waiting
            .queue
            .retain(|peer| !empty && *peer == PeerId::from_u64(1));
        drop(first);
        waiting.pump(&mut server());
        assert!(matches!(waiting.admission, Admission::Idle));
        assert!(waiting.chains[&PeerId::from_u64(1)]
            .peer
            .flight
            .load(Ordering::Acquire));
        let actual = following
            .reserve(following.chains[&PeerId::from_u64(6)].peer.clone())
            .unwrap();
        assert_eq!(capacity.slots.available_permits(), 0);
        drop((
            actual,
            following,
            real_lower,
            third,
            fourth,
            waiting,
            other,
            fourth_owner,
        ));
        assert_eq!(capacity.slots.available_permits(), PROCESS_FLIGHTS);
    }
}
