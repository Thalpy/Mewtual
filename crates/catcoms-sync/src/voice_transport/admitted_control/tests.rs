//! Actual normal-dispatch fixture. The transport rejects A-C, including direct
//! requests/dials; merely omitting a bound proof would not establish this oracle.
use super::*;
use catcoms_rt::RngCore;
use catcoms_rt::{ConnectionLease, ManualClock, PeerConnectionSnapshot};
use rand_core::SeedableRng;
use std::sync::Mutex;
use voice_v2::admitted_control::{
    DirectHelperSupportObservation as HelperObservation,
    EndpointSupportObservation as EndpointObservation,
};
use voice_v2::call_activation::{ActivationObserver, ActivationOwner};
use voice_v2::runtime::Runtime;

#[derive(Clone, Debug)]
struct Chain {
    inner: catcoms_rt::MemNetwork,
    neighbours: HashSet<PeerId>,
    requests: Arc<Mutex<Vec<(PeerId, PeerId, u8)>>>,
    control_hops: Arc<Mutex<Vec<(PeerId, PeerId, voice_v2::Nonce)>>>,
}
impl Chain {
    fn edge(&self, peer: PeerId, data: &[u8]) {
        assert!(
            self.neighbours.contains(&peer),
            "forbidden non-neighbour request"
        );
        let mut log = self.requests.lock().unwrap();
        assert!(log.len() < 4096, "bounded harness request inventory");
        log.push((self.local_peer(), peer, data.first().copied().unwrap_or(0)));
        // Passive harness observer only. It proves the SAME signed original
        // crossed an actual allowed driver request; it supplies no production
        // authorization, adapter success, endpoint proof or route selection.
        if let Ok(outer) = Outer::parse(data) {
            if let Ok(envelope) = control::wire::Envelope::decode(outer.inner) {
                let mut hops = self.control_hops.lock().unwrap();
                assert!(hops.len() < 4096);
                hops.push((self.local_peer(), peer, envelope.object().metadata().id()));
            }
        }
    }
}
#[async_trait::async_trait]
impl MeshTransport for Chain {
    fn local_peer(&self) -> PeerId {
        self.inner.local_peer()
    }
    fn connection_snapshot(&self) -> Vec<PeerConnectionSnapshot> {
        self.inner
            .connection_snapshot()
            .into_iter()
            .filter(|row| self.neighbours.contains(&row.peer))
            .collect()
    }
    fn connection_lease(&self, peer: PeerId) -> Option<ConnectionLease> {
        if self.neighbours.contains(&peer) {
            self.inner.connection_lease(peer)
        } else {
            None
        }
    }
    fn supports_connection_leases(&self) -> bool {
        true
    }
    async fn dial_addr(&self, _: &str) -> Result<(), TransportError> {
        panic!("connected-only voice fixture must never dial")
    }
    async fn subscribe(&self, _: Topic) -> Result<(), TransportError> {
        Ok(())
    }
    async fn unsubscribe(&self, _: Topic) -> Result<(), TransportError> {
        Ok(())
    }
    async fn publish(&self, _: Topic, _: Bytes) -> Result<(), TransportError> {
        Ok(())
    }
    async fn request(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        self.edge(peer, &data);
        self.inner.request(peer, protocol, data).await
    }
    async fn request_cancellable(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        data: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.edge(peer, &data);
        self.inner
            .request_cancellable(peer, protocol, data, cancellation)
            .await
    }
    async fn request_connected_cancellable(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        data: Bytes,
        cancellation: RequestCancellation,
    ) -> Result<Bytes, TransportError> {
        self.edge(peer, &data);
        self.inner
            .request_connected_cancellable(peer, protocol, data, cancellation)
            .await
    }
    async fn request_connected(
        &self,
        peer: PeerId,
        protocol: ProtocolId,
        data: Bytes,
    ) -> Result<Bytes, TransportError> {
        self.edge(peer, &data);
        self.inner.request_connected(peer, protocol, data).await
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        let event = self.inner.next_event().await;
        if let Some(TransportEvent::Request { from, .. }) = &event {
            assert!(self.neighbours.contains(from));
        }
        event
    }
}
type Node = ChannelSync<Chain, rand_chacha::ChaCha20Rng>;
struct Fixture {
    a: Node,
    b: Node,
    c: Node,
    d: Option<Node>,
    clock: Arc<ManualClock>,
    runtimes: Vec<Arc<Runtime>>,
    requests: Arc<Mutex<Vec<(PeerId, PeerId, u8)>>>,
    control_hops: Arc<Mutex<Vec<(PeerId, PeerId, voice_v2::Nonce)>>>,
    _hub: Arc<catcoms_rt::Hub>,
}
fn key(byte: u8) -> voice_v2::JoinKey {
    voice_v2::JoinKey::new([byte;32], hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()).unwrap()
}
fn activate(
    node: &mut Node,
    byte: u8,
) -> (
    Arc<voice_v2::CallGrant>,
    ActivationOwner,
    ActivationObserver,
) {
    let (grant, owner) = node.prepare_admitted_voice_call(1).unwrap().into_parts();
    let original = owner.observer();
    node.select_admitted_voice_call(&grant, &original)
        .unwrap()
        .commit_native(&grant)
        .unwrap();
    let key = key(byte);
    assert!(node
        .start_admitted_voice_presence(grant.clone(), original.clone(), key.join, key.public_key)
        .unwrap()
        .matches(&original, &grant, key.join, key.public_key));
    (grant, owner, original)
}
fn has_peers(
    node: &mut Node,
    resources: &Arc<voice_v2::resources::Resources>,
    grant: &Arc<voice_v2::CallGrant>,
    original: &ActivationObserver,
) -> bool {
    let permit = voice_v2::presence::PeerSnapshotPermit::new(resources, grant.scope()).unwrap();
    let snapshot = node.admitted_voice_presence_peer_snapshot(grant, original, permit);
    !snapshot.result().unwrap().1.is_empty()
}
async fn bind(a: &mut Node, b: &mut Node) {
    let pending = a.finalize_member_connection(b.local_peer());
    futures::pin_mut!(pending);
    for _ in 0..256 {
        if let Some(result) = pending.as_mut().now_or_never() {
            assert!(result.unwrap());
            return;
        }
        let _ = b.run_once().now_or_never();
        tokio::task::yield_now().await;
    }
    panic!("actual neighbour member finalization did not complete");
}
impl Fixture {
    async fn new() -> Self {
        Self::build(3).await
    }
    async fn build(count: u64) -> Self {
        assert!(
            matches!(count, 3 | 4),
            "bounded three/four-node harness graph"
        );
        let (_, mut original, _) = crate::tests::build_members(count).await;
        crate::tests::converge_and_publish_test_routes(&mut original);
        original[0]
            .initialize_group_policy(GroupMode::PeerToPeer)
            .unwrap();
        original[0].snapshot().unwrap();
        original[0].publish_group_policy().unwrap();
        let policy = original[0].group_policy.as_ref().unwrap().encode();
        for node in original.iter_mut().skip(1) {
            node.on_group_policy(&policy);
        }
        if count == 4 && original[3].device_id() > original[1].device_id() {
            original.swap(1, 3);
        }
        let peers: Vec<_> = original.iter().map(|node| node.local_peer()).collect();
        let snapshots: Vec<_> = original
            .iter_mut()
            .map(|node| node.snapshot().unwrap())
            .collect();
        drop(original);
        let hub = catcoms_rt::Hub::new();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let control_hops = Arc::new(Mutex::new(Vec::new()));
        let clock = Arc::new(ManualClock::new(1000));
        let mut nodes: Vec<Node> = snapshots
            .iter()
            .enumerate()
            .map(|(index, snapshot)| {
                let neighbours = if index == 1 || index == 3 {
                    vec![peers[0], peers[2]]
                } else if count == 4 {
                    vec![peers[1], peers[3]]
                } else {
                    vec![peers[1]]
                };
                ChannelSync::restore(
                    snapshot,
                    Chain {
                        inner: hub.join(peers[index]),
                        neighbours: neighbours.into_iter().collect(),
                        requests: requests.clone(),
                        control_hops: control_hops.clone(),
                    },
                    rand_chacha::ChaCha20Rng::seed_from_u64(index as u64 + 901),
                    Box::new(clock.as_ref().clone()),
                )
                .unwrap()
            })
            .collect();
        let runtimes: Vec<_> = (0..count).map(|_| Runtime::new(clock.clone())).collect();
        for (node, runtime) in nodes.iter_mut().zip(&runtimes) {
            let ticket = runtime.session_ticket();
            runtime.activate_session(&ticket).unwrap();
            node.prepare_voice_actor(
                runtime,
                &ticket,
                991,
                |_| Ok(BTreeSet::from([1, 2])),
                |_| Ok(BTreeSet::new()),
            )
            .unwrap()
            .register()
            .unwrap();
            node.set_voice_routing(crate::VoiceRouting::Forwarding);
            node.catchup_queue.clear();
            node.first_proof_sweep_owed = false;
        }
        let mut iter = nodes.into_iter();
        let (mut a, mut b, mut c) = (
            iter.next().unwrap(),
            iter.next().unwrap(),
            iter.next().unwrap(),
        );
        bind(&mut a, &mut b).await;
        bind(&mut b, &mut c).await;
        let mut d = iter.next();
        if let Some(d) = &mut d {
            bind(&mut a, d).await;
            bind(d, &mut c).await;
        }
        assert!(a.transport.connection_lease(c.local_peer()).is_none());
        assert!(c.transport.connection_lease(a.local_peer()).is_none());
        Self {
            a,
            b,
            c,
            d,
            clock,
            runtimes,
            requests,
            control_hops,
            _hub: hub,
        }
    }
    async fn tick(&mut self, count: usize) {
        for _ in 0..count {
            for node in [&mut self.a, &mut self.b, &mut self.c] {
                for _ in 0..16 {
                    if node.run_once().now_or_never().is_none() {
                        break;
                    }
                }
            }
            if let Some(d) = &mut self.d {
                for _ in 0..16 {
                    if d.run_once().now_or_never().is_none() {
                        break;
                    }
                }
            }
            tokio::task::yield_now().await;
        }
    }
}
async fn bounded(test: impl std::future::Future<Output = ()>) {
    let watchdog = catcoms_rt::SystemClock;
    let timeout = watchdog.sleep(std::time::Duration::from_secs(30));
    futures::pin_mut!(test, timeout);
    assert!(
        matches!(
            futures::future::select(test, timeout).await,
            futures::future::Either::Left(((), _))
        ),
        "sanctioned outer watchdog; protocol time remains ManualClock"
    );
}
fn frame(node: &mut Node, object: &control::Object) -> Vec<u8> {
    let inner = control::wire::EnvelopeRef::Direct(object).encode();
    let mut nonce = [0; 16];
    node.rng.fill_bytes(&mut nonce);
    let key = node.device.public_key_bytes();
    let now = node.voice_v2_clock().now_ms();
    let epoch = node.group.epoch();
    let transcript = catchup_auth_transcript(
        &node.group.group_id(),
        KIND,
        &inner,
        &key,
        now,
        &nonce,
        epoch,
        Some(&node.local_peer()),
    );
    let signature = node.device.sign(&transcript).unwrap();
    encode_authed_request_frame(KIND, &inner, &key, now, &nonce, epoch, &signature)
}

#[tokio::test]
async fn admitted_control_actual_queries_and_support_cross_only_restricted_unjoined_bridge() {
    bounded(async {
        let mut f = Fixture::new().await;
        let (ag, _ao, ap) = activate(&mut f.a, 61);
        let (cg, _co, cp) = activate(&mut f.c, 62);
        let b = f.b.device_id();
        let c = f.c.device_id();
        let ah =
            f.a.ensure_admitted_direct_helper_support(&ag, &ap, b)
                .unwrap();
        let ch =
            f.c.ensure_admitted_direct_helper_support(&cg, &cp, b)
                .unwrap();
        let query = ah.identity().owner_for_test();
        let original = (
            query.object().metadata().id(),
            query.object().hash(),
            query.object().metadata().expires(),
        );
        for _ in 0..24 {
            f.tick(4).await;
            if f.a
                .admitted_direct_helper_support_observation(&ag, &ap, &ah)
                .unwrap()
                == (HelperObservation::Observed { features: 1 })
                && f.c
                    .admitted_direct_helper_support_observation(&cg, &cp, &ch)
                    .unwrap()
                    == (HelperObservation::Observed { features: 1 })
                && has_peers(&mut f.a, &f.runtimes[0].resources(), &ag, &ap)
                && has_peers(&mut f.c, &f.runtimes[2].resources(), &cg, &cp)
            {
                break;
            }
            f.clock.advance_ms(250);
        }
        assert_eq!(
            f.a.admitted_direct_helper_support_observation(&ag, &ap, &ah)
                .unwrap(),
            HelperObservation::Observed { features: 1 }
        );
        assert_eq!(
            f.c.admitted_direct_helper_support_observation(&cg, &cp, &ch)
                .unwrap(),
            HelperObservation::Observed { features: 1 }
        );
        let aq = f.a.ensure_admitted_endpoint_support(&ag, &ap, c).unwrap();
        for _ in 0..20 {
            f.tick(4).await;
            if f.a
                .admitted_endpoint_support_observation(&ag, &ap, &aq)
                .unwrap()
                == (EndpointObservation::Observed { features: 1 })
            {
                break;
            }
            f.clock.advance_ms(250);
        }
        assert_eq!(
            f.a.admitted_endpoint_support_observation(&ag, &ap, &aq)
                .unwrap(),
            EndpointObservation::Observed { features: 1 }
        );
        assert!(
            f.b.voice_runtime
                .as_ref()
                .unwrap()
                .endpoints
                .is_unjoined_for_test(),
            "B joins nothing and owns no watcher/key"
        );
        assert!(
            f.b.voice_transport_actor()
                .unwrap()
                .source()
                .unwrap()
                .is_empty(),
            "Control7 uses no legacy local Source activity"
        );
        let retry =
            f.a.ensure_admitted_direct_helper_support(&ag, &ap, b)
                .unwrap();
        assert!(retry.identity().owner_for_test().same(query));
        assert_eq!(
            (
                query.object().metadata().id(),
                query.object().hash(),
                query.object().metadata().expires()
            ),
            original
        );
        let a = f.a.local_peer();
        let b = f.b.local_peer();
        let c = f.c.local_peer();
        let log = f.requests.lock().unwrap();
        assert!(log
            .iter()
            .any(|&(from, to, kind)| from == a && to == b && kind == KIND));
        assert!(log
            .iter()
            .any(|&(from, to, kind)| from == b && to == c && kind == KIND));
        assert!(log
            .iter()
            .all(|&(from, to, _)| !((from == a && to == c) || (from == c && to == a))));
    })
    .await;
}

#[test]
fn admitted_control_0_measures_real_enclosing_graphs_without_assuming_inherited_fee_headroom() {
    let new = control::inventories_for_test();
    let group = voice_v2::runtime::admitted_control_group_inventory_for_test();
    let counter = voice_v2::resources::admitted_control_counter_inventory_for_test();
    eprintln!("Control7 graphs record/table/ledger {new:?}; enclosing Group/ExistingHelper/ExistingSource/BTreeEntry {group:?}; Counts/HashBucket/Service {counter:?}; wrapper {}",wrapper_graph::<Chain>());
    assert!(new.0 <= 8192 && new.1 <= 4096 && new.2 <= 8192);
    assert!(wrapper_graph::<Chain>() <= 4096);
}

#[tokio::test]
async fn admitted_control_same_original_rotates_to_lower_sorting_unjoined_bridge_when_first_bridge_is_busy(
) {
    bounded(async {
        let mut f = Fixture::build(4).await;
        let (ag, _ao, ap) = activate(&mut f.a, 71);
        let (cg, _co, cp) = activate(&mut f.c, 72);
        let b = f.b.device_id();
        let d = f.d.as_ref().unwrap().device_id();
        assert!(
            d < b,
            "fixture forces the wraparound ordering that used to repeat B"
        );
        let ab =
            f.a.ensure_admitted_direct_helper_support(&ag, &ap, b)
                .unwrap();
        let ad =
            f.a.ensure_admitted_direct_helper_support(&ag, &ap, d)
                .unwrap();
        let cb =
            f.c.ensure_admitted_direct_helper_support(&cg, &cp, b)
                .unwrap();
        let cd =
            f.c.ensure_admitted_direct_helper_support(&cg, &cp, d)
                .unwrap();
        for _ in 0..24 {
            f.tick(4).await;
            let known =
                f.a.admitted_direct_helper_support_observation(&ag, &ap, &ab)
                    .unwrap()
                    == (HelperObservation::Observed { features: 1 })
                    && f.a
                        .admitted_direct_helper_support_observation(&ag, &ap, &ad)
                        .unwrap()
                        == (HelperObservation::Observed { features: 1 })
                    && f.c
                        .admitted_direct_helper_support_observation(&cg, &cp, &cb)
                        .unwrap()
                        == (HelperObservation::Observed { features: 1 })
                    && f.c
                        .admitted_direct_helper_support_observation(&cg, &cp, &cd)
                        .unwrap()
                        == (HelperObservation::Observed { features: 1 });
            if known
                && has_peers(&mut f.a, &f.runtimes[0].resources(), &ag, &ap)
                && has_peers(&mut f.c, &f.runtimes[2].resources(), &cg, &cp)
            {
                break;
            }
            f.clock.advance_ms(250);
        }
        assert_eq!(
            f.a.admitted_direct_helper_support_observation(&ag, &ap, &ad)
                .unwrap(),
            HelperObservation::Observed { features: 1 }
        );
        assert_eq!(
            f.c.admitted_direct_helper_support_observation(&cg, &cp, &cd)
                .unwrap(),
            HelperObservation::Observed { features: 1 }
        );
        let scope = ag.scope();
        let mut held = Vec::new();
        for _ in 0..64 {
            while held.len() < 4 {
                match f.runtimes[1].resources().charge(
                    scope.group,
                    512,
                    voice_v2::resources::ResourceKind::Forward,
                ) {
                    Ok(credit) => held.push(credit),
                    Err(VoiceError::Capacity) => break,
                    Err(error) => panic!("{error:?}"),
                }
            }
            if held.len() == 4 {
                break;
            }
            f.tick(2).await;
        }
        assert_eq!(
            held.len(),
            4,
            "actual B Forward lanes are occupied before Query publication"
        );
        let target = f.c.device_id();
        let query =
            f.a.ensure_admitted_endpoint_support(&ag, &ap, target)
                .unwrap();
        let owner = query.identity().owner_for_test();
        let original = (
            owner.object().metadata().id(),
            owner.object().hash(),
            owner.object().metadata().expires(),
        );
        for _ in 0..20 {
            f.tick(4).await;
            if f.a
                .admitted_endpoint_support_observation(&ag, &ap, &query)
                .unwrap()
                == (EndpointObservation::Observed { features: 1 })
            {
                break;
            }
            f.clock.advance_ms(250);
        }
        assert_eq!(
            f.a.admitted_endpoint_support_observation(&ag, &ap, &query)
                .unwrap(),
            EndpointObservation::Observed { features: 1 }
        );
        assert_eq!(
            (
                owner.object().metadata().id(),
                owner.object().hash(),
                owner.object().metadata().expires()
            ),
            original
        );
        assert!(owner.route_cursor().0 <= 3);
        let (a, b, c, d) = (
            f.a.local_peer(),
            f.b.local_peer(),
            f.c.local_peer(),
            f.d.as_ref().unwrap().local_peer(),
        );
        let hops = f.control_hops.lock().unwrap();
        assert!(
            hops.contains(&(a, b, original.0)),
            "first exact original used B"
        );
        assert!(
            hops.contains(&(a, d, original.0)) && hops.contains(&(d, c, original.0)),
            "SAME original reached C through D after B stayed busy"
        );
        assert!(!hops
            .iter()
            .any(|(from, to, _)| (*from == a && *to == c) || (*from == c && *to == a)));
        assert!(f
            .b
            .voice_runtime
            .as_ref()
            .unwrap()
            .endpoints
            .is_unjoined_for_test());
        assert!(f
            .d
            .as_ref()
            .unwrap()
            .voice_runtime
            .as_ref()
            .unwrap()
            .endpoints
            .is_unjoined_for_test());
        drop(held);
    })
    .await;
}

#[tokio::test]
async fn admitted_control_actual_epoch_and_rejoin_revoke_old_query_and_actual_lower_without_adoption(
) {
    bounded(async {
        let mut f = Fixture::new().await;
        let (grant,_owner,original) = activate(&mut f.a,73);
        let b = f.b.device_id();
        let old = f.a.ensure_admitted_direct_helper_support(&grant,&original,b).unwrap();
        let old_query = old.identity().owner_for_test();
        let m = old_query.object().metadata();
        let replay = control::Object::support(m.scope(),m.epoch(),m.issuer(),m.id(),1,&f.b.device,[181;32],1000,m.expires()).unwrap();
        let pending = f.a.prepare_admitted_controls(false).unwrap().pop().unwrap();
        assert!(pending.authority.authorized());
        let joiner = MlsDevice::generate().unwrap();
        let package = joiner.key_package().unwrap();
        f.a.with_observed_mls_transition(|node| node.group.add_member(&node.device,package)).unwrap();
        assert!(f.a.group.epoch() > m.epoch());
        assert!(!old_query.authorized() && !pending.authority.authorized(),"actual native epoch transition closes both original query and retained Lower immediately");
        assert_eq!(f.a.ensure_admitted_direct_helper_support(&grant,&original,b).err(),Some(VoiceError::Unauthorized));
        f.a.clear_admitted_voice_call(&original).unwrap();
        let (next_grant,_next_owner,next) = activate(&mut f.a,74);
        let fresh = f.a.ensure_admitted_direct_helper_support(&next_grant,&next,b).unwrap();
        let current = fresh.identity().owner_for_test();
        assert_ne!(current.object().metadata().id(),m.id());
        assert_eq!(current.object().metadata().epoch(),f.a.group.epoch());
        // An authenticated outer wrapper at the CURRENT epoch cannot repair the
        // immutable OLD inner control/query identity after a real rejoin.
        let inner = control::wire::EnvelopeRef::Direct(&replay).encode();
        let key = f.b.device.public_key_bytes(); let nonce = [182;16]; let epoch = f.a.group.epoch();
        let transcript = catchup_auth_transcript(&f.b.group.group_id(),KIND,&inner,&key,1000,&nonce,epoch,Some(&f.b.local_peer()));
        let signature = f.b.device.sign(&transcript).unwrap();
        let frame = encode_authed_request_frame(KIND,&inner,&key,1000,&nonce,epoch,&signature);
        drop(f.a.receive_voice_transport(f.b.local_peer(),&frame).unwrap());
        assert_eq!(current.features_at(1000,1000),None);
        assert_eq!(f.a.admitted_direct_helper_support_observation(&next_grant,&next,&old).err(),Some(VoiceError::RetryMismatch));
        drop(pending); f.a.prune_voice_routes();
    }).await;
}

#[tokio::test]
async fn admitted_control_q2_supersedes_before_busy_publication_and_late_q1_cannot_install_support()
{
    bounded(async {
        let mut f = Fixture::new().await;
        let (grant, _owner, original) = activate(&mut f.a, 63);
        let b = f.b.device_id();
        let q1 =
            f.a.ensure_admitted_direct_helper_support(&grant, &original, b)
                .unwrap();
        let first = q1.identity().owner_for_test().clone();
        let m = first.object().metadata();
        let late = control::Object::support(
            m.scope(),
            m.epoch(),
            m.issuer(),
            m.id(),
            1,
            &f.b.device,
            [91; 32],
            1000,
            m.expires(),
        )
        .unwrap();
        let held: Vec<_> = (0..4)
            .map(|_| {
                f.runtimes[0]
                    .resources()
                    .charge(
                        m.scope().group,
                        512,
                        voice_v2::resources::ResourceKind::Forward,
                    )
                    .unwrap()
            })
            .collect();
        let q2 =
            f.a.refresh_admitted_direct_helper_support(&grant, &original, b, q1.identity())
                .unwrap();
        let second = q2.identity().owner_for_test();
        assert!(
            !first.authorized(),
            "q1 closes synchronously even with all actual Forward slots occupied"
        );
        assert_ne!(
            first.object().metadata().id(),
            second.object().metadata().id()
        );
        assert_eq!(
            second.route_cursor().0,
            0,
            "capacity precedes attempt debit"
        );
        let inner = control::wire::EnvelopeRef::Direct(&late).encode();
        let mut nonce = [0; 16];
        f.b.rng.fill_bytes(&mut nonce);
        let key = f.b.device.public_key_bytes();
        let transcript = catchup_auth_transcript(
            &f.b.group.group_id(),
            KIND,
            &inner,
            &key,
            1000,
            &nonce,
            m.epoch(),
            Some(&f.b.local_peer()),
        );
        let signature = f.b.device.sign(&transcript).unwrap();
        let frame =
            encode_authed_request_frame(KIND, &inner, &key, 1000, &nonce, m.epoch(), &signature);
        let before = f.runtimes[0].resources().counts();
        let refusal =
            f.a.receive_voice_transport(f.b.local_peer(), &frame)
                .unwrap();
        assert_eq!(
            second.features_at(1000, 1000),
            None,
            "late q1 cannot create or mutate q2"
        );
        drop(refusal);
        f.a.prune_voice_routes();
        assert_eq!(f.runtimes[0].resources().counts(), before);
        drop(held);
        for _ in 0..20 {
            f.tick(4).await;
            if f.a
                .admitted_direct_helper_support_observation(&grant, &original, &q2)
                .unwrap()
                == (HelperObservation::Observed { features: 1 })
            {
                break;
            }
            f.clock.advance_ms(250);
        }
        assert_eq!(
            f.a.admitted_direct_helper_support_observation(&grant, &original, &q2)
                .unwrap(),
            HelperObservation::Observed { features: 1 }
        );
        assert_eq!(
            f.a.admitted_direct_helper_support_observation(&grant, &original, &q1)
                .err(),
            Some(VoiceError::RetryMismatch)
        );
    })
    .await;
}

#[tokio::test]
async fn admitted_control_duplicate_queries_reuse_original_signed_response_and_flood_stays_bounded()
{
    bounded(async {
        let mut f = Fixture::new().await;
        let context = f.a.voice_v2_context(1).unwrap();
        let query = control::Object::query(
            context.scope,
            context.epoch,
            f.b.device_id(),
            &f.a.device,
            [101; 32],
            1000,
            16_000,
        )
        .unwrap();
        let bytes = frame(&mut f.a, &query);
        let from = f.a.local_peer();
        drop(f.b.receive_voice_transport(from, &bytes).unwrap());
        f.b.prune_voice_routes();
        let ledger = f.b.voice_transport_actor().unwrap().control().unwrap();
        let cached = ledger
            .cached(query.metadata(), query.hash(), false)
            .unwrap()
            .unwrap();
        let identity = (
            cached.object().metadata().id(),
            cached.object().hash(),
            cached.object().metadata().expires(),
        );
        let paid = f.runtimes[1].resources().counts();
        for _ in 0..10 {
            drop(f.b.receive_voice_transport(from, &bytes).unwrap());
            f.b.prune_voice_routes();
            let retry = ledger
                .cached(query.metadata(), query.hash(), false)
                .unwrap()
                .unwrap();
            assert!(retry.same(&cached));
            assert_eq!(
                (
                    retry.object().metadata().id(),
                    retry.object().hash(),
                    retry.object().metadata().expires()
                ),
                identity
            );
            assert_eq!(
                f.runtimes[1].resources().counts(),
                paid,
                "duplicates retain one original response/payer/attempt budget"
            );
        }
        for id in 102..162u8 {
            let query = control::Object::query(
                context.scope,
                context.epoch,
                f.b.device_id(),
                &f.a.device,
                [id; 32],
                1000,
                16_000,
            )
            .unwrap();
            let bytes = frame(&mut f.a, &query);
            drop(f.b.receive_voice_transport(from, &bytes));
            f.b.prune_voice_routes();
        }
        let (source, helper, cache) = ledger.counts_for_test();
        assert!(source <= 32 && cache <= 32 && helper == 0);
        assert!(
            f.runtimes[1]
                .resources()
                .group_counts(context.scope.group)
                .unwrap()
                .0
                <= voice_v2::resources::GROUP_BYTES
        );
        assert!(f
            .b
            .voice_runtime
            .as_ref()
            .unwrap()
            .endpoints
            .is_unjoined_for_test());
    })
    .await;
}

#[tokio::test]
async fn admitted_control_wrong_room_old_decoder_and_expired_original_fail_closed() {
    bounded(async {
        let mut f = Fixture::new().await;
        let (grant, _owner, original) = activate(&mut f.a, 64);
        let b = f.b.device_id();
        let probe =
            f.a.ensure_admitted_direct_helper_support(&grant, &original, b)
                .unwrap();
        let owner = probe.identity().owner_for_test();
        assert!(voice_v2::SignedObject::decode(owner.object().bytes()).is_err());
        let old = control::wire::EnvelopeRef::Direct(owner.object()).encode();
        assert!(
            voice_v2::Envelope::decode(&old).is_err(),
            "Existing helper grammar stays closed to7"
        );
        let context = f.a.voice_v2_context(1).unwrap();
        let wrong = control::Object::query(
            voice_v2::Scope {
                room: 3,
                ..context.scope
            },
            context.epoch,
            b,
            &f.a.device,
            [171; 32],
            1000,
            16_000,
        )
        .unwrap();
        let bytes = frame(&mut f.a, &wrong);
        drop(
            f.b.receive_voice_transport(f.a.local_peer(), &bytes)
                .unwrap(),
        );
        f.b.prune_voice_routes();
        assert!(
            f.b.voice_transport_actor()
                .unwrap()
                .control_if_present()
                .is_none(),
            "wrong directory room precedes control cache construction"
        );
        let before = (
            owner.object().metadata().id(),
            owner.object().metadata().expires(),
            owner.route_cursor(),
        );
        f.clock.advance_ms(15_001);
        assert!(
            !owner.authorized(),
            "checked local deadline has no signed-clock-skew extension"
        );
        assert_eq!(
            f.a.admitted_direct_helper_support_observation(&grant, &original, &probe)
                .unwrap(),
            HelperObservation::Unknown
        );
        let retry =
            f.a.ensure_admitted_direct_helper_support(&grant, &original, b)
                .unwrap();
        assert!(retry.identity().owner_for_test().same(owner));
        assert_eq!(
            (
                owner.object().metadata().id(),
                owner.object().metadata().expires(),
                owner.route_cursor()
            ),
            before,
            "silence/expiry never remints or refunds an original"
        );
        f.a.clear_admitted_voice_call(&original).unwrap();
        assert!(!owner.authorized());
    })
    .await;
}

#[tokio::test]
async fn admitted_control_superseded_actual_owners_keep_affine_occupancy_until_real_return() {
    bounded(async {
        let mut f = Fixture::new().await;
        let (grant, _owner, original) = activate(&mut f.a, 65);
        let b = f.b.device_id();
        let mut current =
            f.a.ensure_admitted_direct_helper_support(&grant, &original, b)
                .unwrap();
        let mut history = Vec::new();
        let ledger = f.a.voice_transport_actor().unwrap().control().unwrap();
        let mut refused = false;
        for _ in 0..64 {
            f.clock.advance_ms(250);
            match f.a.refresh_admitted_direct_helper_support(
                &grant,
                &original,
                b,
                current.identity(),
            ) {
                Ok(next) => {
                    assert!(!current.identity().owner_for_test().authorized());
                    history.push(current);
                    current = next;
                    ledger.collect_expired();
                    f.a.prune_voice_routes();
                }
                Err(VoiceError::Capacity) => {
                    refused = true;
                    break;
                }
                Err(error) => panic!("unexpected refresh refusal: {error:?}"),
            }
        }
        assert!(
            refused && !history.is_empty() && history.len() < 64,
            "fixed infrastructure makes effective capacity lower than64 originals"
        );
        let recovered =
            f.a.ensure_admitted_direct_helper_support(&grant, &original, b)
                .unwrap();
        assert!(
            recovered
                .identity()
                .owner_for_test()
                .same(current.identity().owner_for_test()),
            "failed prepayment leaves exact current q unchanged"
        );
        drop(recovered);
        let before = f.runtimes[0].resources().counts();
        // Superseded aliases are already negative; collection cannot refund them.
        ledger.collect_expired();
        f.a.prune_voice_routes();
        assert_eq!(f.runtimes[0].resources().counts(), before);
        drop(history.remove(0));
        f.a.prune_voice_routes();
        assert!(
            f.runtimes[0].resources().counts().0 < before.0,
            "only actual outside-owner return releases the original quantum"
        );
        f.clock.advance_ms(250);
        let next =
            f.a.refresh_admitted_direct_helper_support(&grant, &original, b, current.identity())
                .unwrap();
        assert!(!current.identity().owner_for_test().authorized());
        assert!(next.identity().owner_for_test().authorized());
    })
    .await;
}

#[tokio::test]
async fn admitted_control_three_original_submissions_exhaust_without_drop_or_silence_refund() {
    bounded(async {
        let mut f = Fixture::new().await;
        let (grant, _owner, original) = activate(&mut f.a, 66);
        let b = f.b.device_id();
        let query =
            f.a.ensure_admitted_direct_helper_support(&grant, &original, b)
                .unwrap();
        let owner = query.identity().owner_for_test();
        let graphs = control::inventories_for_test();
        eprintln!("Control7 record {}/8192; table {}/4096; ledger {}/8192; wrapper {}/4096; scheduler inline {}; completion {}",
            graphs.0,graphs.1,graphs.2,wrapper_graph::<Chain>(),
            std::mem::size_of::<crate::voice_schedule::VoiceSchedule>(),std::mem::size_of::<crate::voice_schedule::VoiceFlight>());
        assert!(graphs.0 <= 8192 && graphs.1 <= 4096 && graphs.2 <= 8192 && wrapper_graph::<Chain>() <= 4096);
        let identity = (
            owner.object().metadata().id(),
            owner.object().hash(),
            owner.object().metadata().expires(),
        );
        for attempt in 1..=3 {
            let mut jobs = f.a.prepare_admitted_controls(false).unwrap();
            assert_eq!(jobs.len(), 1);
            assert_eq!(owner.route_cursor().0, attempt);
            if attempt == 1 {
                let (_send,signal) = tokio::sync::watch::channel(false);
                let execution = jobs.pop().unwrap().execute(signal);
                let actual = std::mem::size_of_val(&execution);
                eprintln!("Control7 actual execute future {actual}; prepared {}",std::mem::size_of::<PendingAdmittedControl<Chain>>());
                assert!(actual <= 2 * std::mem::size_of::<PendingAdmittedControl<Chain>>() + 256);
                drop(execution);
            }
            drop(jobs);
            f.a.prune_voice_routes();
            f.clock.advance_ms(500);
        }
        assert!(f.a.prepare_admitted_controls(false).unwrap().is_empty());
        let retry =
            f.a.ensure_admitted_direct_helper_support(&grant, &original, b)
                .unwrap();
        assert!(retry.identity().owner_for_test().same(owner));
        assert_eq!(owner.route_cursor().0, 3);
        assert_eq!(
            (
                owner.object().metadata().id(),
                owner.object().hash(),
                owner.object().metadata().expires()
            ),
            identity
        );
        assert_eq!(
            f.a.admitted_direct_helper_support_observation(&grant, &original, &query)
                .unwrap(),
            HelperObservation::Unknown
        );
    })
    .await;
}

#[test]
fn admitted_control_service_budget_refuses_an_unowned_group_without_inserting_state() {
    let pool = voice_v2::resources::Resources::default();
    // No unowned synthetic group may create a budget row through this API.
    assert_eq!(
        pool.admit_voice_control([1; 32], 1000),
        Err(VoiceError::Unauthorized)
    );
}
