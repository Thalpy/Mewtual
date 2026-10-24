//! Bounded maintenance workers have outbound transport access, never mutable Server ownership.
//! Slot and per-peer custody outlive a deadline until the actual lower request retires.
use super::*;
use catcoms_rt::SharedRequestKeepalive;
use catcoms_sync::{
    CompletedMemberMaintenance, MemberMaintenanceError as Failure, MemberMaintenanceKind as Kind,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

const MAX_CHAINS: usize = 16;
const MAX_WAITERS: usize = 4;
// Released commit catch-up may return 16 MiB; PEX is at most 512 KiB and other kinds smaller.
// These fixed permits therefore bound RETAINED ANSWERS at 32 MiB/server and 64 MiB/process,
// including completions and timed-out transport-owned work. Root's existing bounded commit
// decoder additionally uses transcript/bundle/record scratch; this is not a total-memory cap.
const LOCAL_FLIGHTS: usize = 2;
const PROCESS_FLIGHTS: usize = 4;
mod admission;
use admission::Admission;

struct Capacity {
    slots: Arc<Semaphore>,
    changed: watch::Sender<u64>,
}
impl Capacity {
    fn new(slots: usize) -> Arc<Self> {
        Arc::new(Self {
            slots: Arc::new(Semaphore::new(slots)),
            changed: watch::channel(0).0,
        })
    }
    fn signal(&self) {
        self.changed
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}
fn process_capacity() -> Arc<Capacity> {
    static CAPACITY: OnceLock<Arc<Capacity>> = OnceLock::new();
    CAPACITY
        .get_or_init(|| Capacity::new(PROCESS_FLIGHTS))
        .clone()
}
struct PeerCustody {
    peer: PeerId,
    held: Arc<Mutex<HashSet<PeerId>>>,
    capacity: Arc<Capacity>,
    flight: AtomicBool,
}
impl Drop for PeerCustody {
    fn drop(&mut self) {
        self.held.lock().unwrap().remove(&self.peer);
        self.capacity.signal();
    }
}
struct Flight {
    process: Option<OwnedSemaphorePermit>,
    local: Option<OwnedSemaphorePermit>,
    _peer: Arc<PeerCustody>,
    capacity: Arc<Capacity>,
}
impl std::fmt::Debug for Flight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemberMaintenanceFlight { .. }")
    }
}
impl Drop for Flight {
    fn drop(&mut self) {
        // Publish availability after both flight permits return. The peer guard separately
        // wakes admission after its actual last owner disappears, including lower-only custody.
        self.process.take();
        self.local.take();
        self._peer.flight.store(false, Ordering::Release);
        self.capacity.signal();
    }
}
type Waiter = oneshot::Sender<Result<bool, String>>;
struct Chain {
    peer: Arc<PeerCustody>,
    stage: Kind,
    discovery: bool,
    /// Autonomous discovery can poll commits without also bypassing PEX's own backoff.
    exchange_records: bool,
    waiters: Vec<Waiter>,
    pending_discovery: bool,
    pending_membership: bool,
    superseded_retry: bool,
}
pub(super) struct Completion {
    peer: PeerId,
    stage: Kind,
    response: CompletedMemberMaintenance,
    _flight: Arc<Flight>,
    _publisher: watch::Sender<bool>,
}
pub(super) enum Event {
    Wake,
    Completed(Box<Completion>),
    Panicked,
}
/// Actual mutations observed around Root completion. An empty/failed commit answer is not proof
/// of currency, while even a partially applied valid bundle can change membership/MLS epoch.
#[derive(Default)]
pub(super) struct Effects {
    pub records_ready: bool,
    pub epoch_changed: bool,
}
pub(super) struct Maintenance {
    capacity: Arc<Capacity>,
    local: Arc<Semaphore>,
    changed: watch::Receiver<u64>,
    held: Arc<Mutex<HashSet<PeerId>>>,
    chains: HashMap<PeerId, Chain>,
    queue: VecDeque<PeerId>,
    jobs: JoinSet<Completion>,
    // One fixed FIFO position, not a task or another per-peer queue. No local slot or
    // transport authority is captured until this owner receives a real process grant.
    admission: Admission,
}
impl Maintenance {
    pub(super) fn new() -> Self {
        Self::with_capacity(process_capacity())
    }
    fn with_capacity(capacity: Arc<Capacity>) -> Self {
        Self {
            changed: capacity.changed.subscribe(),
            capacity,
            local: Arc::new(Semaphore::new(LOCAL_FLIGHTS)),
            held: Arc::new(Mutex::new(HashSet::new())),
            chains: HashMap::new(),
            queue: VecDeque::new(),
            jobs: JoinSet::new(),
            admission: Admission::default(),
        }
    }
    fn enqueue(&mut self, peer: PeerId, discovery: bool, waiter: Option<Waiter>) {
        if let Some(chain) = self.chains.get_mut(&peer) {
            // A discovery tick overlapping an explicit finalization must not discard its PEX
            // obligation after target selection already charged the discovery interval.
            chain.pending_discovery |= discovery
                && (!chain.discovery || (!chain.exchange_records && chain.stage != Kind::Commits));
            chain.pending_membership |= discovery && chain.stage != Kind::Commits;
            chain.exchange_records |= discovery;
            if let Some(waiter) = waiter {
                if chain.waiters.len() < MAX_WAITERS {
                    chain.waiters.push(waiter);
                } else {
                    let _ = waiter.send(Err("member finalization busy".into()));
                }
            }
            return;
        }
        let mut held = self.held.lock().unwrap();
        if self.chains.len() >= MAX_CHAINS || held.len() >= MAX_CHAINS || !held.insert(peer) {
            if let Some(waiter) = waiter {
                let _ = waiter.send(Err("member finalization busy".into()));
            }
            return;
        }
        let custody = Arc::new(PeerCustody {
            peer,
            held: self.held.clone(),
            capacity: self.capacity.clone(),
            flight: AtomicBool::new(false),
        });
        self.chains.insert(
            peer,
            Chain {
                peer: custody,
                stage: if discovery {
                    Kind::Commits
                } else {
                    Kind::Finalize
                },
                discovery,
                exchange_records: discovery,
                waiters: waiter.into_iter().collect(),
                pending_discovery: false,
                pending_membership: false,
                superseded_retry: false,
            },
        );
        self.queue.push_back(peer);
    }
    pub(super) fn discover(&mut self, peers: Vec<PeerId>) {
        for peer in peers {
            self.enqueue(peer, true, None);
        }
    }
    /// A quiet membership poll is autonomous even without an explicit UI waiter. Its terminal
    /// stage never schedules PEX, so independent polling cannot forgive record-exchange backoff.
    pub(super) fn poll_membership(&mut self, peers: Vec<PeerId>) {
        for peer in peers {
            if let Some(chain) = self.chains.get_mut(&peer) {
                chain.pending_membership |= !chain.discovery || chain.stage != Kind::Commits;
                continue;
            }
            self.enqueue(peer, true, None);
            if let Some(chain) = self.chains.get_mut(&peer) {
                chain.exchange_records = false;
            }
        }
    }
    pub(super) fn finalize(&mut self, peer: PeerId, waiter: Waiter) {
        if !waiter.is_closed() {
            self.enqueue(peer, false, Some(waiter));
        }
    }

    fn reserve(&mut self, peer: Arc<PeerCustody>) -> Option<Arc<Flight>> {
        if peer
            .flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        if self.local.available_permits() == 0 {
            peer.flight.store(false, Ordering::Release);
            self.admission.clear(&self.capacity);
            return None;
        }
        let Some(process) = self.admission.try_take(&self.capacity) else {
            peer.flight.store(false, Ordering::Release);
            return None;
        };
        let Ok(local) = self.local.clone().try_acquire_owned() else {
            // Only this Actor acquires local permits; an actual lower can only return one.
            // Still handle local closure/refusal without retaining an unstarted global grant.
            peer.flight.store(false, Ordering::Release);
            drop(process);
            self.capacity.signal();
            return None;
        };
        Some(Arc::new(Flight {
            process: Some(process),
            local: Some(local),
            _peer: peer,
            capacity: self.capacity.clone(),
        }))
    }

    /// Nonblocking admission: shared retirement wakes queued-only actors, so no command/tick is
    /// needed when another server returns a process slot. Each next stage signs fresh context.
    pub(super) fn pump<T: MeshTransport + 'static, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
    ) {
        if self.queue.is_empty() || self.local.available_permits() == 0 {
            self.admission.clear(&self.capacity);
            return;
        }
        let mut eligible = false;
        for _ in 0..MAX_CHAINS {
            let Some(&peer) = self.queue.front() else {
                break;
            };
            let custody = self.chains.get(&peer).expect("queued chain").peer.clone();
            if custody.flight.load(Ordering::Acquire) {
                // A timed-out prior stage still owns this peer below us. Other ready targets
                // may use free capacity meanwhile; actual retirement wakes this queued stage.
                self.queue.rotate_left(1);
                continue;
            }
            eligible = true;
            let Some(flight) = self.reserve(custody) else {
                break;
            };
            self.queue.pop_front();
            let chain = self.chains.get_mut(&peer).expect("queued chain");
            if !chain.discovery && chain.waiters.iter().all(Waiter::is_closed) {
                if chain.pending_discovery || chain.pending_membership {
                    chain.discovery = true;
                    chain.exchange_records = chain.pending_discovery;
                    chain.pending_discovery = false;
                    chain.pending_membership = false;
                    chain.stage = Kind::Commits;
                    chain.waiters.clear();
                } else {
                    self.chains.remove(&peer);
                    continue;
                }
            }
            let stage = chain.stage;
            match server.sync.prepare_member_maintenance(peer, stage) {
                Ok(Some(pending)) => {
                    let (publisher, receiver) = watch::channel(false);
                    let keepalive: SharedRequestKeepalive = flight.clone();
                    self.jobs.spawn(async move {
                        let response = pending
                            .execute(RequestCancellation::new(receiver, Some(keepalive)))
                            .await;
                        Completion {
                            peer,
                            stage,
                            response,
                            _flight: flight,
                            _publisher: publisher,
                        }
                    });
                }
                result => {
                    drop(flight);
                    let result = result.map(|_| false).map_err(Failure::from);
                    self.advance(peer, stage, result, server);
                }
            }
        }
        // Do not hoard an assigned-but-unobserved process grant behind lower-only peers,
        // a now-empty queue or two actual local flights. Their real retirement wakes us.
        if !eligible || self.queue.is_empty() || self.local.available_permits() == 0 {
            self.admission.clear(&self.capacity);
        }
    }
    pub(super) async fn next(&mut self) -> Event {
        tokio::select! {
            _ = self.admission.wait_ready() => Event::Wake,
            changed = async { if self.queue.is_empty() { std::future::pending().await } else { self.changed.changed().await } } => { let _ = changed; Event::Wake },
            result = async { if self.jobs.is_empty() { std::future::pending().await } else { self.jobs.join_next().await } } => {
                match result { Some(Ok(completion)) => Event::Completed(Box::new(completion)), Some(Err(_)) => Event::Panicked, None => Event::Wake }
            }
        }
    }
    pub(super) fn complete<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        event: Event,
        server: &mut Server<T, R>,
    ) -> Option<Effects> {
        match event {
            Event::Wake => Some(Effects::default()),
            Event::Panicked => None,
            Event::Completed(completed) => {
                let epoch = server.epoch();
                let result = server.sync.complete_member_maintenance(completed.response);
                let records_ready = completed.stage == Kind::Pex && result.is_ok();
                let epoch_changed = server.epoch() != epoch;
                self.advance(completed.peer, completed.stage, result, server);
                Some(Effects {
                    records_ready,
                    epoch_changed,
                })
            }
        }
    }
    fn advance<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        peer: PeerId,
        stage: Kind,
        result: Result<bool, Failure>,
        server: &mut Server<T, R>,
    ) {
        let Some(chain) = self.chains.get_mut(&peer) else {
            return;
        };
        tracing::trace!(local = ?server.sync.local_peer(), ?peer, ?stage, ?result,
            "member maintenance stage completed");
        debug_assert_eq!(chain.stage, stage);
        let superseded = matches!(result, Err(Failure::Superseded));
        let waiter_result = result
            .as_ref()
            .map(|value| *value)
            .map_err(|error| error.to_string());
        if superseded && chain.discovery && !chain.superseded_retry {
            // One bounded refresh survives benign local control/descriptor change. Repeated
            // churn stops this chain without branding the current healthy peer as failed.
            chain.superseded_retry = true;
            chain.stage = Kind::Commits;
            self.queue.push_back(peer);
            return;
        }
        let mut next = match stage {
            // A quiet poll learns a gap without data traffic. Released empty replies are
            // ambiguous, and a failed/unsupported poll must not suppress PEX or mark it failed.
            Kind::Commits if superseded => None,
            Kind::Commits if chain.exchange_records => Some(Kind::Pex),
            Kind::Commits if !chain.waiters.is_empty() => Some(Kind::Finalize),
            Kind::Commits => None,
            Kind::Pex if result.is_ok() => Some(Kind::Finalize),
            Kind::Pex => {
                if !superseded {
                    server.note_pex_failure(peer);
                }
                None
            }
            Kind::Finalize => {
                for waiter in chain.waiters.drain(..) {
                    let _ = waiter.send(waiter_result.clone());
                }
                if chain.pending_discovery {
                    chain.exchange_records = true;
                    chain.pending_discovery = false;
                    chain.pending_membership = false;
                    chain.discovery = true;
                    Some(Kind::Commits)
                } else if chain.discovery && chain.exchange_records {
                    Some(Kind::Offer)
                } else {
                    None
                }
            }
            // An explicit caller arriving after the prior kind26 commit gets a fresh exchange,
            // never a cached success whose descriptor/policy could already have changed.
            Kind::Offer if !chain.waiters.is_empty() => {
                chain.discovery = false;
                Some(Kind::Finalize)
            }
            Kind::Offer => None,
        };
        if next.is_none() && chain.pending_membership && !superseded {
            // A new tick after this chain's commit poll is a fresh obligation. Coalesce it
            // once, then return to the same bounded queue without re-running backed-off PEX.
            chain.pending_membership = false;
            chain.discovery = true;
            chain.exchange_records = false;
            next = Some(Kind::Commits);
        }
        if let Some(next) = next {
            chain.stage = next;
            self.queue.push_back(peer);
        } else if let Some(chain) = self.chains.remove(&peer) {
            for waiter in chain.waiters {
                let _ = waiter.send(Err("member finalization unavailable".into()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod fairness;
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{Hub, ManualClock, MemNetwork};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    fn server() -> Server<MemNetwork, ChaCha20Rng> {
        Server::found(
            Hub::new().join(PeerId::from_u64(100)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(100),
            Box::new(ManualClock::new(1_000)),
            "test",
        )
        .unwrap()
    }
    fn reserve_for(maintenance: &mut Maintenance, peer: u64) -> Arc<Flight> {
        let peer = PeerId::from_u64(peer);
        maintenance.enqueue(peer, true, None);
        maintenance
            .reserve(maintenance.chains[&peer].peer.clone())
            .unwrap()
    }

    #[tokio::test]
    async fn maintenance_retains_per_peer_and_flight_custody_until_actual_lower_retirement() {
        let capacity = Capacity::new(4);
        let mut maintenance = Maintenance::with_capacity(capacity.clone());
        let caller = reserve_for(&mut maintenance, 1);
        let lower = caller.clone();
        maintenance.queue.clear();
        maintenance.chains.clear();
        drop(caller);
        assert_eq!(capacity.slots.available_permits(), 3);
        assert_eq!(maintenance.local.available_permits(), 1);
        maintenance.enqueue(PeerId::from_u64(1), true, None);
        assert!(
            maintenance.queue.is_empty(),
            "timeout did not release per-target dedup"
        );
        drop(lower);
        assert_eq!(capacity.slots.available_permits(), 4);
        maintenance.enqueue(PeerId::from_u64(1), true, None);
        assert_eq!(maintenance.queue.len(), 1);
    }

    #[tokio::test]
    async fn maintenance_queued_only_actor_wakes_on_another_servers_actual_retirement() {
        let capacity = Capacity::new(4);
        let mut a = Maintenance::with_capacity(capacity.clone());
        let mut b = Maintenance::with_capacity(capacity.clone());
        let mut waiting = Maintenance::with_capacity(capacity.clone());
        let first = reserve_for(&mut a, 1);
        let _second = reserve_for(&mut a, 2);
        let _third = reserve_for(&mut b, 3);
        let _fourth = reserve_for(&mut b, 4);
        waiting.enqueue(PeerId::from_u64(5), true, None);
        assert!(waiting.jobs.is_empty());
        assert!(waiting
            .reserve(waiting.chains[&PeerId::from_u64(5)].peer.clone())
            .is_none());
        let lower = first.clone();
        drop(first);
        assert_eq!(capacity.slots.available_permits(), 0);
        let wake = waiting.next();
        tokio::pin!(wake);
        assert!(matches!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(wake.as_mut().poll(cx))).await,
            std::task::Poll::Pending
        ));
        drop(lower);
        assert!(matches!(wake.await, Event::Wake));
    }

    #[tokio::test]
    async fn maintenance_idle_actors_ignore_unrelated_capacity_retirement() {
        let capacity = Capacity::new(4);
        let mut idle_a = Maintenance::with_capacity(capacity.clone());
        let mut idle_b = Maintenance::with_capacity(capacity.clone());
        let mut busy = Maintenance::with_capacity(capacity);
        let flight = reserve_for(&mut busy, 1);
        drop(flight);
        for idle in [&mut idle_a, &mut idle_b] {
            let next = idle.next();
            tokio::pin!(next);
            assert!(matches!(
                std::future::poll_fn(|cx| std::task::Poll::Ready(next.as_mut().poll(cx))).await,
                std::task::Poll::Pending
            ));
        }
    }

    #[test]
    fn maintenance_later_chain_stage_waits_for_the_same_peers_actual_lower_retirement() {
        let mut maintenance = Maintenance::with_capacity(Capacity::new(4));
        let lower = reserve_for(&mut maintenance, 1);
        let peer = PeerId::from_u64(1);
        let custody = maintenance.chains[&peer].peer.clone();
        maintenance.chains.get_mut(&peer).unwrap().stage = Kind::Finalize;
        maintenance.queue.clear();
        maintenance.advance(
            peer,
            Kind::Finalize,
            Err(Failure::Failed(catcoms_sync::SyncError::Transport(
                catcoms_rt::TransportError::Timeout(peer),
            ))),
            &mut server(),
        );
        assert_eq!(maintenance.chains[&peer].stage, Kind::Offer);
        assert!(
            maintenance.reserve(custody.clone()).is_none(),
            "old lower kind26 still owns the peer flight"
        );
        drop(lower);
        assert!(maintenance.reserve(custody).is_some());
    }

    #[test]
    fn maintenance_coalesces_discovery_and_bounds_superseded_retries() {
        let mut maintenance = Maintenance::with_capacity(Capacity::new(4));
        let peer = PeerId::from_u64(1);
        let (reply, receiver) = oneshot::channel();
        maintenance.finalize(peer, reply);
        maintenance.discover(vec![peer]);
        assert!(maintenance.chains[&peer].pending_discovery);
        maintenance.queue.clear();
        let mut server = server();
        maintenance.advance(peer, Kind::Finalize, Ok(true), &mut server);
        assert_eq!(maintenance.chains[&peer].stage, Kind::Commits);
        assert!(receiver.blocking_recv().unwrap().unwrap());
        maintenance.queue.clear();
        maintenance.advance(peer, Kind::Commits, Err(Failure::Superseded), &mut server);
        assert_eq!(maintenance.chains[&peer].stage, Kind::Commits);
        assert!(maintenance.chains[&peer].superseded_retry);
        maintenance.queue.clear();
        maintenance.advance(peer, Kind::Commits, Err(Failure::Superseded), &mut server);
        assert!(
            !maintenance.chains.contains_key(&peer),
            "repeated state churn does not retry forever"
        );
    }

    #[tokio::test]
    async fn maintenance_skips_lower_busy_peer_and_keeps_discovery_after_waiter_cancellation() {
        let hub = Hub::new();
        let _remote = hub.join(PeerId::from_u64(2));
        let mut server = Server::found(
            hub.join(PeerId::from_u64(100)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(100),
            Box::new(ManualClock::new(1_000)),
            "test",
        )
        .unwrap();
        let mut maintenance = Maintenance::with_capacity(Capacity::new(4));
        let lower = reserve_for(&mut maintenance, 1);
        maintenance
            .chains
            .get_mut(&PeerId::from_u64(1))
            .unwrap()
            .stage = Kind::Offer;
        let (reply, receiver) = oneshot::channel();
        maintenance.finalize(PeerId::from_u64(2), reply);
        maintenance.discover(vec![PeerId::from_u64(2)]);
        drop(receiver);
        maintenance.pump(&mut server);
        assert_eq!(
            maintenance.jobs.len(),
            1,
            "the second peer used the remaining server flight"
        );
        assert_eq!(
            maintenance.chains[&PeerId::from_u64(2)].stage,
            Kind::Commits,
            "closed explicit waiter did not discard discovery"
        );
        assert_eq!(maintenance.queue.front(), Some(&PeerId::from_u64(1)));
        drop(lower);
    }

    #[test]
    fn maintenance_empty_or_failed_commit_poll_still_advances_ordinary_discovery() {
        for result in [
            Ok(false),
            Err(Failure::Failed(catcoms_sync::SyncError::Malformed)),
        ] {
            let mut maintenance = Maintenance::with_capacity(Capacity::new(4));
            let peer = PeerId::from_u64(1);
            maintenance.discover(vec![peer]);
            assert_eq!(maintenance.chains[&peer].stage, Kind::Commits);
            maintenance.queue.clear();
            let mut server = server();
            maintenance.advance(peer, Kind::Commits, result, &mut server);
            assert_eq!(maintenance.chains[&peer].stage, Kind::Pex);
            assert_eq!(maintenance.queue.front(), Some(&peer));
        }
    }

    #[tokio::test]
    async fn maintenance_poll_only_is_autonomous_and_does_not_start_pex() {
        let hub = Hub::new();
        let peer = PeerId::from_u64(2);
        let _remote = hub.join(peer);
        let mut server = Server::found(
            hub.join(PeerId::from_u64(100)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(100),
            Box::new(ManualClock::new(1_000)),
            "test",
        )
        .unwrap();
        let mut maintenance = Maintenance::with_capacity(Capacity::new(4));
        maintenance.poll_membership(vec![peer]);
        assert!(!maintenance.chains[&peer].exchange_records);
        maintenance.pump(&mut server);
        assert_eq!(maintenance.jobs.len(), 1, "no explicit waiter is needed");
        // Finish through the same stage-selection function as the detached completion. The
        // lower remains retained by its spawned job; removing a chain does not refund it.
        maintenance.advance(peer, Kind::Commits, Ok(false), &mut server);
        assert!(!maintenance.chains.contains_key(&peer));
        assert!(maintenance.queue.is_empty());
    }

    #[test]
    fn maintenance_poll_coalescing_preserves_fresh_membership_and_explicit_finalization() {
        let peer = PeerId::from_u64(2);
        let mut server = server();
        let mut maintenance = Maintenance::with_capacity(Capacity::new(4));
        maintenance.poll_membership(vec![peer]);
        maintenance.discover(vec![peer]);
        assert!(
            maintenance.chains[&peer].exchange_records,
            "in-flight commit can upgrade to PEX"
        );
        maintenance.queue.clear();
        maintenance.advance(peer, Kind::Commits, Ok(false), &mut server);
        maintenance.poll_membership(vec![peer]);
        assert!(maintenance.chains[&peer].pending_membership);
        for stage in [Kind::Pex, Kind::Finalize, Kind::Offer, Kind::Commits] {
            maintenance.queue.clear();
            maintenance.advance(peer, stage, Ok(false), &mut server);
            if stage == Kind::Finalize {
                assert_eq!(
                    maintenance.chains[&peer].stage,
                    Kind::Offer,
                    "the full record chain completes its owed helper offer before the new poll"
                );
            }
            if stage == Kind::Offer {
                assert_eq!(maintenance.chains[&peer].stage, Kind::Commits);
                assert!(!maintenance.chains[&peer].exchange_records);
            }
        }
        assert!(!maintenance.chains.contains_key(&peer));

        maintenance.poll_membership(vec![peer]);
        let (reply, mut receiver) = oneshot::channel();
        maintenance.finalize(peer, reply);
        maintenance.queue.clear();
        maintenance.advance(peer, Kind::Commits, Ok(true), &mut server);
        assert_eq!(maintenance.chains[&peer].stage, Kind::Finalize);
        assert!(
            receiver.try_recv().is_err(),
            "kind2 success is not a kind26 result"
        );
        maintenance.queue.clear();
        maintenance.advance(peer, Kind::Finalize, Ok(false), &mut server);
        assert!(!receiver.blocking_recv().unwrap().unwrap());
        assert!(!maintenance.chains.contains_key(&peer));
    }

    #[test]
    fn maintenance_full_discovery_after_poll_commit_and_during_finalize_is_not_lost() {
        let peer = PeerId::from_u64(2);
        let mut server = server();
        let mut maintenance = Maintenance::with_capacity(Capacity::new(4));
        maintenance.poll_membership(vec![peer]);
        let (reply, receiver) = oneshot::channel();
        maintenance.finalize(peer, reply);
        maintenance.queue.clear();
        maintenance.advance(peer, Kind::Commits, Ok(false), &mut server);
        assert_eq!(maintenance.chains[&peer].stage, Kind::Finalize);
        maintenance.discover(vec![peer]);
        assert!(maintenance.chains[&peer].pending_discovery);
        maintenance.queue.clear();
        maintenance.advance(peer, Kind::Finalize, Ok(true), &mut server);
        assert!(receiver.blocking_recv().unwrap().unwrap());
        assert_eq!(maintenance.chains[&peer].stage, Kind::Commits);
        assert!(maintenance.chains[&peer].exchange_records);
        for stage in [Kind::Commits, Kind::Pex, Kind::Finalize, Kind::Offer] {
            maintenance.queue.clear();
            maintenance.advance(peer, stage, Ok(false), &mut server);
        }
        assert!(!maintenance.chains.contains_key(&peer));
    }

    #[test]
    fn maintenance_bounds_chains_and_waiters_without_replenishing_existing_work() {
        let mut maintenance = Maintenance::with_capacity(Capacity::new(4));
        for peer in 1..=17 {
            maintenance.enqueue(PeerId::from_u64(peer), true, None);
        }
        assert_eq!(maintenance.chains.len(), MAX_CHAINS);
        assert_eq!(maintenance.queue.len(), MAX_CHAINS);
        let mut refused = None;
        for _ in 0..=MAX_WAITERS {
            let (reply, receiver) = oneshot::channel();
            maintenance.finalize(PeerId::from_u64(1), reply);
            refused = Some(receiver);
        }
        assert!(refused.unwrap().try_recv().unwrap().is_err());
        assert_eq!(
            maintenance.chains[&PeerId::from_u64(1)].waiters.len(),
            MAX_WAITERS
        );
    }
}
