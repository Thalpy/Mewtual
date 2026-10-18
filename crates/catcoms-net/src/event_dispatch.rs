//! Bounded application delivery, owned by the networking actor. A full application sink must
//! never suspend swarm responses, command acknowledgements or connection retirement. Payloads
//! retain affine credits through their actual Bytes/responder/lower owners; lifecycle metadata
//! reconciles actual generations instead of accumulating an unbounded edge log.
use super::*;
use catcoms_rt::ConnectionLease;
use catcoms_rt::RequestAuthority;
use std::collections::BTreeMap;
use std::sync::{Mutex as StdMutex, OnceLock};

const QUEUED: usize = 64;
const LIFECYCLE: usize = 2 * catcoms_rt::MAX_CONNECTED_PEER_SNAPSHOT;
const DRIVER_BYTES: usize = 64 * 1024 * 1024;
const PROCESS_BYTES: usize = 128 * 1024 * 1024;
const DRIVER_RECORDS: usize = 320;
const PROCESS_RECORDS: usize = 1024;
const DRIVER_REQUESTS: usize = 16;
const PROCESS_REQUESTS: usize = 64;
const PEER_REQUESTS: usize = 2;
const OVERHEAD: usize = 1024;

#[derive(Default)]
struct Counts {
    bytes: usize,
    records: usize,
    requests: usize,
    peers: HashMap<PeerId, usize>,
}
#[derive(Default)]
struct Pool(StdMutex<Counts>);

/// One process pool across every server driver. It bounds declared delivery/response buffers,
/// not inherited libp2p codec/socket allocations or independently owned application copies.
pub(super) struct Budget {
    pool: Arc<Pool>,
    counts: StdMutex<Counts>,
}
pub(super) struct Credit {
    budget: Arc<Budget>,
    bytes: usize,
    peer: Option<PeerId>,
}
impl std::fmt::Debug for Budget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliveryBudget").finish_non_exhaustive()
    }
}
impl std::fmt::Debug for Credit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliveryCredit").finish_non_exhaustive()
    }
}
impl Budget {
    pub(super) fn shared() -> Arc<Self> {
        static POOL: OnceLock<Arc<Pool>> = OnceLock::new();
        Arc::new(Self {
            pool: POOL.get_or_init(|| Arc::new(Pool::default())).clone(),
            counts: StdMutex::new(Counts::default()),
        })
    }
    /// Count actual retained bytes and record/stream owners. Admission is atomic and callback-
    /// free; lock order is process then driver, including release. A legal 16 MiB request and
    /// two simultaneous response copies fit one driver; overflow refuses before delivery.
    pub(super) fn charge(
        self: &Arc<Self>,
        bytes: usize,
        peer: Option<PeerId>,
    ) -> Option<Arc<Credit>> {
        let bytes = bytes.checked_add(OVERHEAD)?;
        let mut process = self.pool.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut driver = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if bytes > DRIVER_BYTES
            || process.bytes > PROCESS_BYTES - bytes
            || driver.bytes > DRIVER_BYTES - bytes
            || driver.records >= DRIVER_RECORDS
            || process.records >= PROCESS_RECORDS
            || peer.is_some_and(|peer| {
                driver.requests >= DRIVER_REQUESTS
                    || process.requests >= PROCESS_REQUESTS
                    || driver.peers.get(&peer).copied().unwrap_or(0) >= PEER_REQUESTS
            })
        {
            return None;
        }
        process.bytes += bytes;
        driver.bytes += bytes;
        process.records += 1;
        driver.records += 1;
        if let Some(peer) = peer {
            process.requests += 1;
            driver.requests += 1;
            *driver.peers.entry(peer).or_default() += 1;
        }
        Some(Arc::new(Credit {
            budget: self.clone(),
            bytes,
            peer,
        }))
    }
}
impl catcoms_rt::ResponseBudget for Budget {
    fn retain(self: Arc<Self>, bytes: usize) -> Result<SharedRequestKeepalive, TransportError> {
        if bytes > MAX_FRAME {
            return Err(TransportError::NoResponse);
        }
        let peak = bytes.checked_mul(2).ok_or(TransportError::NoResponse)?;
        self.charge(peak, None)
            .map(|credit| credit as SharedRequestKeepalive)
            .ok_or(TransportError::NoResponse)
    }
}
impl Drop for Credit {
    fn drop(&mut self) {
        let mut process = self.budget.pool.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut driver = self.budget.counts.lock().unwrap_or_else(|e| e.into_inner());
        process.bytes -= self.bytes;
        driver.bytes -= self.bytes;
        process.records -= 1;
        driver.records -= 1;
        if let Some(peer) = self.peer {
            process.requests -= 1;
            driver.requests -= 1;
            let count = driver.peers.get_mut(&peer).expect("charged inbound peer");
            *count -= 1;
            if *count == 0 {
                driver.peers.remove(&peer);
            }
        }
    }
}
struct OwnedBytes {
    bytes: Bytes,
    _credit: Arc<Credit>,
}
impl AsRef<[u8]> for OwnedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
pub(super) fn charged_bytes(bytes: Bytes, credit: Arc<Credit>) -> Bytes {
    // No payload copy. Every clone/slice of the delivered Bytes preserves its charge, even if
    // the queue/token/response task disappears before the application releases its last copy.
    Bytes::from_owner(OwnedBytes {
        bytes,
        _credit: credit,
    })
}

/// Original ingress generation plus the actual lower response lifetime. A retired response
/// cannot be revived merely because the same Noise peer reconnects while its event is queued.
#[derive(Debug)]
pub(super) struct IngressAuthority {
    lease: ConnectionLease,
    terminal: watch::Receiver<bool>,
}
impl IngressAuthority {
    pub(super) fn new(lease: ConnectionLease) -> (Arc<Self>, watch::Sender<bool>) {
        let (owner, terminal) = watch::channel(false);
        (Arc::new(Self { lease, terminal }), owner)
    }
}
#[async_trait]
impl RequestAuthority for IngressAuthority {
    fn authorized(&self) -> bool {
        self.lease.is_live() && !*self.terminal.borrow() && self.terminal.has_changed().is_ok()
    }
    async fn revoked(&self) {
        let mut terminal = self.terminal.clone();
        if !self.authorized() {
            return;
        }
        tokio::select! {
            _ = self.lease.revoked() => {},
            _ = async { while !*terminal.borrow_and_update() { if terminal.changed().await.is_err() { return; } } } => {},
        }
    }
}

struct Actual {
    lease: ConnectionLease,
    active: Vec<ConnectionPath>,
}
#[derive(Default)]
struct Lifecycle {
    actual: Option<Actual>,
    announced: Option<ConnectionLease>,
    paths: Option<Vec<ConnectionPath>>,
    newly: Option<ConnectionPath>,
    clear_paths: bool,
}
impl Lifecycle {
    fn dirty(&self) -> bool {
        if self.clear_paths {
            return true;
        }
        match (&self.announced, &self.actual) {
            (Some(old), Some(new)) if old.is_live() && old.is_same(&new.lease) => {
                self.paths.as_ref() != Some(&new.active) || self.newly.is_some()
            }
            (Some(_), _) => true,
            (None, Some(new)) => new.lease.is_live(),
            (None, None) => false,
        }
    }
    fn ready(&self, lease: &ConnectionLease) -> bool {
        lease.is_live()
            && !self.clear_paths
            && self.paths.is_some()
            && self
                .announced
                .as_ref()
                .is_some_and(|announced| announced.is_same(lease))
    }
}
struct Pending {
    event: TransportEvent,
    carrier: PeerId,
    ingress: ConnectionLease,
}
#[derive(Default)]
pub(super) struct Dispatcher {
    lifecycle: BTreeMap<PeerId, Lifecycle>,
    cursor: Option<PeerId>,
    payloads: VecDeque<Pending>,
    lost_gossip: bool,
    // Rotate all three sources, including a coalesced recovery obligation. Connection churn
    // must not keep a lost control/data packet from ever triggering anti-entropy.
    turn: u8,
}
impl Dispatcher {
    pub(super) fn connected(
        &mut self,
        peer: PeerId,
        lease: ConnectionLease,
        active: Vec<ConnectionPath>,
        newly: Option<ConnectionPath>,
    ) -> bool {
        if !self.lifecycle.contains_key(&peer) && self.lifecycle.len() >= LIFECYCLE {
            return false;
        }
        let state = self.lifecycle.entry(peer).or_default();
        if state
            .actual
            .as_ref()
            .is_none_or(|old| !old.lease.is_same(&lease))
        {
            state.newly = None;
        }
        state.actual = Some(Actual { lease, active });
        if newly.is_some() {
            state.newly = newly;
        }
        true
    }
    pub(super) fn disconnected(&mut self, peer: PeerId) {
        if let Some(state) = self.lifecycle.get_mut(&peer) {
            state.actual = None;
            state.newly = None;
            if state.announced.is_none() && !state.clear_paths {
                self.lifecycle.remove(&peer);
            }
        }
    }
    pub(super) fn queue(
        &mut self,
        event: TransportEvent,
        carrier: PeerId,
        ingress: ConnectionLease,
    ) -> bool {
        if self.payloads.len() >= QUEUED
            || !ingress.is_live()
            || self.lifecycle.get(&carrier).is_none_or(|state| {
                state
                    .actual
                    .as_ref()
                    .is_none_or(|actual| !actual.lease.is_same(&ingress))
            })
        {
            if matches!(event, TransportEvent::Gossip { .. }) {
                self.lost_gossip = true;
            }
            // Dropping a request is an honest NoResponse; never fabricate signed/empty success.
            return false;
        }
        self.payloads.push_back(Pending {
            event,
            carrier,
            ingress,
        });
        true
    }
    pub(super) fn note_gossip_loss(&mut self) {
        self.lost_gossip = true;
    }
    pub(super) fn has_pending(&self) -> bool {
        !self.payloads.is_empty()
            || self.lost_gossip
            || self.lifecycle.values().any(Lifecycle::dirty)
    }
    /// Called only after obtaining a real sink permit. Thus "announced" means placed in the
    /// ordered stream, not guessed from a successful application wait. Each turn emits one item.
    pub(super) fn next(&mut self) -> Option<TransportEvent> {
        // Work is bounded by QUEUED. Never rebind old requests to a replacement same-peer lease.
        while self
            .payloads
            .front()
            .is_some_and(|pending| !pending.ingress.is_live())
        {
            if self
                .payloads
                .pop_front()
                .is_some_and(|pending| matches!(pending.event, TransportEvent::Gossip { .. }))
            {
                self.lost_gossip = true;
            }
        }
        let ready = self.payloads.front().is_some_and(|pending| {
            self.lifecycle
                .get(&pending.carrier)
                .is_some_and(|state| state.ready(&pending.ingress))
        });
        for offset in 0..3 {
            let source = (self.turn + offset) % 3;
            let event = match source {
                0 if ready => self.payloads.pop_front().map(|pending| pending.event),
                1 => self.next_lifecycle(),
                2 if self.lost_gossip => {
                    self.lost_gossip = false;
                    Some(TransportEvent::RecoveryNeeded)
                }
                _ => None,
            };
            if event.is_some() {
                self.turn = (source + 1) % 3;
                return event;
            }
        }
        None
    }
    fn next_lifecycle(&mut self) -> Option<TransportEvent> {
        let peer = self
            .lifecycle
            .iter()
            .filter(|(_, state)| state.dirty())
            .min_by_key(|(peer, _)| (self.cursor.is_some_and(|cursor| **peer <= cursor), **peer))
            .map(|(peer, _)| *peer)?;
        self.cursor = Some(peer);
        let state = self.lifecycle.get_mut(&peer).expect("selected lifecycle");
        if state.clear_paths {
            state.clear_paths = false;
            if state.actual.is_none() && state.announced.is_none() {
                self.lifecycle.remove(&peer);
            }
            return Some(TransportEvent::PeerPathsChanged {
                peer,
                active: Vec::new(),
                newly_established: None,
            });
        }
        if state.announced.as_ref().is_some_and(|old| {
            !old.is_live()
                || state
                    .actual
                    .as_ref()
                    .is_none_or(|new| !old.is_same(&new.lease))
        }) {
            state.announced = None;
            state.paths = None;
            state.clear_paths = true;
            return Some(TransportEvent::PeerDisconnected(peer));
        }
        let actual = state.actual.as_ref().expect("dirty live lifecycle");
        if state.announced.is_none() {
            state.announced = Some(actual.lease.clone());
            return Some(TransportEvent::PeerConnected(peer));
        }
        state.paths = Some(actual.active.clone());
        Some(TransportEvent::PeerPathsChanged {
            peer,
            active: actual.active.clone(),
            newly_established: state.newly.take(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::ResponseBudget;

    fn budget(pool: &Arc<Pool>) -> Arc<Budget> {
        Arc::new(Budget {
            pool: pool.clone(),
            counts: StdMutex::new(Counts::default()),
        })
    }
    fn counts(budget: &Budget) -> (usize, usize, usize) {
        let counts = budget.counts.lock().unwrap();
        (counts.bytes, counts.records, counts.requests)
    }
    fn lease(owners: &connection_leases::Owners, peer: PeerId) -> ConnectionLease {
        owners.connected(peer);
        connection_leases::current(&owners.table, peer).unwrap()
    }
    fn path() -> ConnectionPath {
        ConnectionPath {
            family: ConnectionFamily::Ipv4,
            transport: ConnectionTransport::Tcp,
            direction: ConnectionDirection::Listener,
        }
    }

    #[test]
    fn delivery_bytes_are_process_shared_and_driver_pressure_cannot_take_another_driver_share() {
        let pool = Arc::new(Pool::default());
        let a = budget(&pool);
        let b = budget(&pool);
        let c = budget(&pool);
        let first = a.charge(DRIVER_BYTES - OVERHEAD, None).unwrap();
        assert!(a.charge(0, None).is_none());
        let second = b.charge(DRIVER_BYTES - OVERHEAD, None).unwrap();
        assert!(c.charge(0, None).is_none());
        assert_eq!(pool.0.lock().unwrap().bytes, PROCESS_BYTES);
        drop(first);
        let requested = c.charge(MAX_FRAME, Some(PeerId::from_u64(1))).unwrap();
        let response = c.clone().retain(MAX_FRAME).unwrap();
        assert_eq!(counts(&c), (3 * MAX_FRAME + 2 * OVERHEAD, 2, 1));
        assert!(c.clone().retain(MAX_FRAME + 1).is_err());
        assert!(c.charge(usize::MAX, None).is_none());
        drop((second, requested, response));
        assert_eq!(pool.0.lock().unwrap().bytes, 0);
        assert_eq!(pool.0.lock().unwrap().records, 0);
    }

    #[test]
    fn inbound_request_slots_remain_shared_per_peer_driver_and_process_until_actual_release() {
        let pool = Arc::new(Pool::default());
        let mut held = Vec::new();
        let drivers: Vec<_> = (0..5).map(|_| budget(&pool)).collect();
        for driver in drivers.iter().take(4) {
            for peer in 0..8 {
                for _ in 0..2 {
                    held.push(driver.charge(0, Some(PeerId::from_u64(peer))).unwrap());
                }
                assert!(driver.charge(0, Some(PeerId::from_u64(peer))).is_none());
            }
            assert_eq!(counts(driver).2, 16);
            assert!(driver.charge(0, Some(PeerId::from_u64(99))).is_none());
        }
        assert!(drivers[4].charge(0, Some(PeerId::from_u64(1))).is_none());
        let actual = held.pop().unwrap();
        let waiter = actual.clone();
        drop(actual);
        assert!(drivers[4].charge(0, Some(PeerId::from_u64(1))).is_none());
        drop(waiter);
        assert!(drivers[4].charge(0, Some(PeerId::from_u64(1))).is_some());
        drop(held);
        assert_eq!(pool.0.lock().unwrap().requests, 0);
    }

    #[tokio::test]
    async fn response_and_payload_copies_retain_credits_after_outer_owners_disappear() {
        let pool = Arc::new(Pool::default());
        let budget = budget(&pool);
        let owners = connection_leases::Owners::default();
        let peer = PeerId::from_u64(1);
        let ingress = lease(&owners, peer);
        let initial = budget.charge(7, Some(peer)).unwrap();
        let data = charged_bytes(Bytes::from_static(b"request"), initial.clone());
        let (authority, terminal) = IngressAuthority::new(ingress);
        let (responder, receiver) =
            Responder::channel_owned(initial.clone(), authority, budget.clone());
        responder.try_respond(Bytes::from_static(b"reply")).unwrap();
        let response = receiver.recv_owned().await.unwrap();
        let actual_lower = response.keepalive().unwrap();
        let response_copy = response.data().clone();
        drop(response);
        drop(initial);
        drop(terminal);
        assert_eq!(counts(&budget), (2 * OVERHEAD + 7 + 10, 2, 1));
        drop(data);
        assert_eq!(counts(&budget), (OVERHEAD + 10, 1, 0));
        drop(response_copy);
        assert_eq!(
            counts(&budget),
            (OVERHEAD + 10, 1, 0),
            "lower ownership outlives the response waiter and Bytes"
        );
        drop(actual_lower);
        assert_eq!(counts(&budget), (0, 0, 0));
    }

    #[tokio::test]
    async fn retired_ingress_and_full_response_budget_refuse_without_fake_success_or_refund() {
        for retire in [false, true] {
            let pool = Arc::new(Pool::default());
            let budget = budget(&pool);
            let owners = connection_leases::Owners::default();
            let peer = PeerId::from_u64(1);
            let initial = budget.charge(0, Some(peer)).unwrap();
            let (authority, terminal) = IngressAuthority::new(lease(&owners, peer));
            let (responder, receiver) =
                Responder::channel_owned(initial.clone(), authority, budget.clone());
            let pressure = if retire {
                owners.disconnected(peer);
                None
            } else {
                Some(
                    budget
                        .charge(DRIVER_BYTES - OVERHEAD - counts(&budget).0, None)
                        .unwrap(),
                )
            };
            assert!(responder.try_respond(Bytes::from_static(b"reply")).is_err());
            assert!(receiver.recv_owned().await.is_none());
            assert_eq!(
                counts(&budget).2,
                1,
                "actual native request owner still holds its stream slot"
            );
            drop((initial, terminal, pressure));
            assert_eq!(counts(&budget), (0, 0, 0));
        }
    }

    #[tokio::test]
    async fn reconnect_keeps_disconnect_before_new_connect_and_drops_old_generation_request() {
        let owners = connection_leases::Owners::default();
        let peer = PeerId::from_u64(1);
        let first = lease(&owners, peer);
        let mut dispatch = Dispatcher::default();
        assert!(dispatch.connected(peer, first.clone(), vec![path()], Some(path())));
        assert!(matches!(dispatch.next(), Some(TransportEvent::PeerConnected(p)) if p == peer));
        assert!(matches!(
            dispatch.next(),
            Some(TransportEvent::PeerPathsChanged {
                newly_established: Some(_),
                ..
            })
        ));
        let (old, old_response) = Responder::channel();
        assert!(dispatch.queue(
            TransportEvent::Request {
                from: peer,
                proto: ProtocolId(RR_PROTOCOL),
                data: Bytes::from_static(b"old"),
                responder: old
            },
            peer,
            first
        ));
        owners.disconnected(peer);
        dispatch.disconnected(peer);
        let current = lease(&owners, peer);
        assert!(dispatch.connected(peer, current.clone(), vec![path()], None));
        let (new, _new_response) = Responder::channel();
        assert!(dispatch.queue(
            TransportEvent::Request {
                from: peer,
                proto: ProtocolId(RR_PROTOCOL),
                data: Bytes::from_static(b"new"),
                responder: new
            },
            peer,
            current
        ));
        assert!(matches!(dispatch.next(), Some(TransportEvent::PeerDisconnected(p)) if p == peer));
        assert!(
            matches!(dispatch.next(), Some(TransportEvent::PeerPathsChanged { active, .. }) if active.is_empty())
        );
        assert!(matches!(dispatch.next(), Some(TransportEvent::PeerConnected(p)) if p == peer));
        assert!(matches!(
            dispatch.next(),
            Some(TransportEvent::PeerPathsChanged {
                newly_established: None,
                ..
            })
        ));
        assert!(
            matches!(dispatch.next(), Some(TransportEvent::Request { data, .. }) if &data[..] == b"new")
        );
        assert!(old_response.recv().await.is_none());
    }

    #[test]
    fn stale_gossip_and_continuous_lifecycle_churn_cannot_lose_or_starve_recovery() {
        let owners = connection_leases::Owners::default();
        let peer = PeerId::from_u64(1);
        let first = lease(&owners, peer);
        let mut dispatch = Dispatcher::default();
        assert!(dispatch.connected(peer, first.clone(), vec![path()], None));
        dispatch.next();
        dispatch.next();
        for _ in 0..3 {
            assert!(dispatch.queue(
                TransportEvent::Gossip {
                    from: peer,
                    topic: Topic::new(Bytes::from_static(b"control")),
                    data: Bytes::new()
                },
                peer,
                first.clone()
            ));
        }
        owners.disconnected(peer);
        dispatch.disconnected(peer);
        let current = lease(&owners, peer);
        assert!(dispatch.connected(peer, current.clone(), vec![path()], None));
        let mut recovery = 0;
        // Every turn creates another lifecycle obligation. A coalesced loss still gets its
        // own fair source turn within three emissions, independently of path churn.
        for _ in 0..3 {
            assert!(dispatch.connected(peer, current.clone(), vec![path()], Some(path())));
            match dispatch.next() {
                Some(TransportEvent::RecoveryNeeded) => recovery += 1,
                Some(TransportEvent::Gossip { .. }) => panic!("stale gossip escaped"),
                Some(_) => {}
                None => panic!("pending lifecycle or recovery disappeared"),
            }
        }
        assert_eq!(recovery, 1);
        while let Some(event) = dispatch.next() {
            assert!(!matches!(
                event,
                TransportEvent::RecoveryNeeded | TransportEvent::Gossip { .. }
            ));
        }
    }

    #[test]
    fn never_announced_connections_coalesce_and_gossip_overflow_has_one_recovery_wake() {
        let owners = connection_leases::Owners::default();
        let peer = PeerId::from_u64(1);
        let first = lease(&owners, peer);
        let mut dispatch = Dispatcher::default();
        assert!(dispatch.connected(peer, first, vec![path()], Some(path())));
        owners.disconnected(peer);
        dispatch.disconnected(peer);
        assert!(dispatch.next().is_none());
        let current = lease(&owners, peer);
        assert!(dispatch.connected(peer, current.clone(), vec![path()], None));
        dispatch.next();
        dispatch.next();
        for _ in 0..QUEUED {
            assert!(dispatch.queue(
                TransportEvent::Gossip {
                    from: peer,
                    topic: Topic::new(Bytes::from_static(b"topic")),
                    data: Bytes::new()
                },
                peer,
                current.clone()
            ));
        }
        for _ in 0..10 {
            assert!(!dispatch.queue(
                TransportEvent::Gossip {
                    from: peer,
                    topic: Topic::new(Bytes::from_static(b"topic")),
                    data: Bytes::new()
                },
                peer,
                current.clone()
            ));
        }
        let mut recovery = 0;
        let mut gossip = 0;
        while let Some(event) = dispatch.next() {
            match event {
                TransportEvent::Gossip { .. } => gossip += 1,
                TransportEvent::RecoveryNeeded => recovery += 1,
                _ => panic!("unexpected lifecycle"),
            }
        }
        assert_eq!((gossip, recovery), (64, 1));
        assert!(!dispatch.has_pending());
    }
}
