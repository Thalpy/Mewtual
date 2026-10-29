//! Actual admitted Control7 over the existing blocked TCP fixture. These tests
//! prove protocol observations, never SDP, RTC, key agreement or audible media.
use super::*;
use crate::voice_authority::AdmittedPublishedKey;
use crate::voice_call_activation::Handle;
use catcoms_app::{
    VoiceAdmittedSupportPublisher, VoiceAdmittedSupportResult, VoiceAdmittedSupportWork,
    VoiceEndpointError,
};
use catcoms_sync::voice_v2::admitted_control::{
    DirectHelperSupportObservation as HelperObservation, DirectHelperSupportProbe,
    EndpointSupportObservation as EndpointObservation, EndpointSupportQuery,
};
use catcoms_sync::voice_v2::call_activation::ActivationRpcPermit;
use catcoms_sync::voice_v2::presence::PeerSnapshotPermit;
use catcoms_sync::voice_v2::resources::{Credit, ResourceKind};
use catcoms_sync::voice_v2::{Body, VoiceError};

/// Framing-only diagnostic, NOT a signature check or a second protocol adapter.
/// Hash the complete borrowed signed object exactly as production Object::hash.
/// All authority comes from Root's independently authenticated observation.
pub(super) fn unverified_control_hash(frame: &[u8]) -> Option<[u8; 32]> {
    if frame.len() > 4096 {
        return None;
    }
    let mut frame = catcoms_wire::Decoder::new(frame);
    if frame.get_u8().ok()? != 28 {
        return None;
    }
    let inner = frame.get_bytes().ok()?;
    let mut envelope = catcoms_wire::Decoder::new(inner);
    if envelope.get_u8().ok()? != 2 || !(1..=3).contains(&envelope.get_u8().ok()?) {
        return None;
    }
    let signed = envelope.get_bytes().ok()?;
    let mut object = catcoms_wire::Decoder::new(signed);
    let mut header = catcoms_wire::Decoder::new(object.get_bytes().ok()?);
    if header.get_str().ok()? != "mewtual/voice/object/v2"
        || header.get_u8().ok()? != 2
        || header.get_u8().ok()? != 7
    {
        return None;
    }
    Some(*blake3::hash(signed).as_bytes())
}

const HARNESS_BYTES: usize = 4096;
struct Funding {
    _credit: Credit,
}
struct FundingOwner(Option<Arc<Funding>>);
impl Clone for FundingOwner {
    fn clone(&self) -> Self {
        Self(Some(self.0.as_ref().unwrap().clone()))
    }
}
impl Drop for FundingOwner {
    fn drop(&mut self) {
        if let Some(funding) = self.0.take().and_then(Arc::into_inner) {
            drop(funding);
        }
    }
}
#[derive(Default)]
struct Slot {
    work: Option<VoiceAdmittedSupportWork>,
    result: Option<VoiceAdmittedSupportResult>,
    refusal: Option<VoiceEndpointError>,
    uncertain: bool,
}
struct CellBody {
    slot: StdMutex<Slot>,
    changed: tokio::sync::Notify,
}
/// Every alias carries the same outside sponsor; no raw Arc/Weak escapes.
/// Cancellation cannot refund an accepted Actor's still-owned publisher/cell.
struct Cell {
    body: Option<Arc<CellBody>>,
    funding: FundingOwner,
}
impl Clone for Cell {
    fn clone(&self) -> Self {
        Self {
            body: Some(self.body.as_ref().unwrap().clone()),
            funding: self.funding.clone(),
        }
    }
}
impl std::ops::Deref for Cell {
    type Target = CellBody;
    fn deref(&self) -> &CellBody {
        self.body.as_deref().unwrap()
    }
}
impl Drop for Cell {
    fn drop(&mut self) {
        drop(self.body.take());
    }
}
impl Cell {
    fn new(handle: &Handle) -> Self {
        // Full body/Arc/funding headers, publication overlap, fixed publisher,
        // result, aliases and an explicit1024-byte future allowance. This is a
        // test-harness invoice, never a new production/native payer claim.
        let invoice = 2 * std::mem::size_of::<CellBody>()
            + 4 * std::mem::size_of::<Cell>()
            + 2 * std::mem::size_of::<Publisher>()
            + 2 * std::mem::size_of::<Funding>()
            + std::mem::size_of::<VoiceAdmittedSupportResult>()
            + 8 * std::mem::size_of::<usize>()
            + 1024
            + 256;
        assert!(
            invoice <= HARNESS_BYTES,
            "complete TCP support harness {invoice}/{HARNESS_BYTES}"
        );
        static INVOICE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        INVOICE.get_or_init(|| {
            eprintln!("complete TCP Support harness {invoice}/{HARNESS_BYTES}");
            invoice
        });
        let credit = handle.original().reserve_retained(HARNESS_BYTES).unwrap();
        let funding = FundingOwner(Some(Arc::new(Funding { _credit: credit })));
        Self {
            body: Some(Arc::new(CellBody {
                slot: StdMutex::new(Slot::default()),
                changed: tokio::sync::Notify::new(),
            })),
            funding,
        }
    }
    async fn ready(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let slot = self.slot.lock().unwrap();
                if slot.result.is_some() || slot.refusal.is_some() || slot.uncertain {
                    return;
                }
            }
            changed.await;
        }
    }
}
struct Publisher {
    cell: Cell,
    finished: bool,
}
impl Drop for Publisher {
    fn drop(&mut self) {
        if !self.finished {
            self.cell.slot.lock().unwrap().uncertain = true;
            self.cell.changed.notify_waiters();
        }
    }
}
impl VoiceAdmittedSupportPublisher for Publisher {
    fn accepted(&mut self, work: VoiceAdmittedSupportWork) -> Result<(), VoiceEndpointError> {
        let mut slot = self.cell.slot.lock().unwrap();
        assert!(slot.work.is_none());
        slot.work = Some(work);
        Ok(())
    }
    fn completed(
        mut self: Box<Self>,
        result: VoiceAdmittedSupportResult,
        work: VoiceAdmittedSupportWork,
    ) {
        let mut slot = self.cell.slot.lock().unwrap();
        assert!(slot.work.as_ref().unwrap().is_same(&work));
        slot.result = Some(result);
        drop(slot);
        self.finished = true;
        self.cell.changed.notify_waiters();
    }
    fn refused(
        mut self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceAdmittedSupportWork>,
        payment: Option<ActivationRpcPermit>,
    ) {
        let mut slot = self.cell.slot.lock().unwrap();
        if let Some(work) = work {
            assert!(slot.work.as_ref().unwrap().is_same(&work));
        }
        slot.refusal = Some(error);
        drop(slot);
        drop(payment);
        self.finished = true;
        self.cell.changed.notify_waiters();
    }
}
enum Request {
    Helper(DeviceId),
    Endpoint(DeviceId),
    HelperObservation(DirectHelperSupportProbe),
    EndpointObservation(EndpointSupportQuery),
    Peers(PeerSnapshotPermit),
}
/// Construct payment BEFORE the future/Box graph. The returned future retains
/// the same paid cell even if an outer watchdog loses its observing waiter.
fn rpc(handle: &Handle, request: Request) -> impl Future<Output = VoiceAdmittedSupportResult> + '_ {
    let cell = Cell::new(handle);
    let future = async move {
        assert!(
            handle.active(),
            "exact native selected original must remain current"
        );
        let original = handle.original().clone();
        let grant = handle.grant().unwrap();
        let payment = original.reserve_rpc().unwrap();
        let control = handle.publication_control().unwrap();
        let publisher = Box::new(Publisher {
            cell: cell.clone(),
            finished: false,
        });
        let queued = match request {
            Request::Helper(peer) => control.try_queue_ensure_admitted_direct_helper_support(
                grant, original, peer, payment, publisher,
            ),
            Request::Endpoint(peer) => control.try_queue_ensure_admitted_endpoint_support(
                grant, original, peer, payment, publisher,
            ),
            Request::HelperObservation(query) => control
                .try_queue_observe_admitted_direct_helper_support(
                    grant, original, query, payment, publisher,
                ),
            Request::EndpointObservation(query) => control
                .try_queue_observe_admitted_endpoint_support(
                    grant, original, query, payment, publisher,
                ),
            Request::Peers(permit) => control.try_queue_admitted_presence_peer_snapshot(
                grant, original, permit, payment, publisher,
            ),
        };
        let work = queued.unwrap();
        cell.ready().await;
        work.wait_retired().await;
        let mut slot = cell.slot.lock().unwrap();
        assert!(
            !slot.uncertain && slot.refusal.is_none(),
            "actual Support RPC failed: {:?}",
            slot.refusal
        );
        slot.result.take().expect("whole actual Root result")
    };
    let actual = std::mem::size_of_val(&future);
    assert!(
        actual <= 1024,
        "actual native Support observer future {actual}/1024"
    );
    static FUTURE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    FUTURE.get_or_init(|| {
        eprintln!("actual native Support observer future {actual}/1024");
        actual
    });
    future
}
struct Endpoint {
    handle: Handle,
    key: AdmittedPublishedKey,
}
fn point() -> [u8; 65] {
    hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap()
}
async fn activate(scenario: &Scenario, index: usize) -> Endpoint {
    let state = &scenario.online(index).state;
    let controller = bounded(
        "native admitted controller preparation",
        prepare_voice_controller_inner(state),
    )
    .await
    .unwrap();
    bounded(
        "native admitted controller activation",
        activate_voice_owner_inner(state, controller.clone()),
    )
    .await
    .unwrap();
    let owner = bounded(
        "actual admitted call preparation",
        crate::voice_admitted_call::prepare_inner(
            state,
            controller,
            1,
            channel_id("general").to_string(),
        ),
    )
    .await
    .unwrap();
    bounded(
        "actual admitted call activation",
        crate::voice_admitted_call::activate_inner(state, owner.clone()),
    )
    .await
    .unwrap();
    crate::voice_admitted_call::acknowledge_inner(state, &owner)
        .await
        .unwrap();
    let executor = [9; 32];
    let admission = crate::voice_admitted_key::admit_inner(state, &owner, executor)
        .await
        .unwrap();
    bounded(
        "actual admitted on-curve point publication",
        crate::voice_admitted_key::publish_inner(
            state,
            &owner,
            executor,
            &admission.permit,
            point(),
        ),
    )
    .await
    .unwrap();
    crate::voice_admitted_key::acknowledge_inner(state, &owner, executor).unwrap();
    crate::voice_admitted_key::settle_inner(state, &owner, executor)
        .await
        .unwrap();
    let mut authority = state.voice_authority.lock().unwrap();
    let handle = authority.admitted_activation(&owner).unwrap();
    let key = authority.admitted_published_key(&handle, executor).unwrap();
    assert!(handle.active());
    Endpoint { handle, key }
}
async fn end(scenario: &Scenario, index: usize, endpoint: Endpoint) {
    let state = &scenario.online(index).state;
    let Endpoint { handle, key } = endpoint;
    drop(key);
    crate::end_voice_owner_inner(state, handle.id()).await;
    bounded(
        "admitted activation actual native task return",
        handle.wait_execution_return(),
    )
    .await;
    crate::voice_admitted_call::collect_retired(state);
}
fn assert_unjoined(scenario: &Scenario, index: usize, group: [u8; 32]) {
    let running = scenario.online(index);
    assert!(running
        .state
        .voice_authority
        .lock()
        .unwrap()
        .has_no_call_or_key_for_test());
    // Instantaneous actual charged-watcher oracle, not proof of all historical
    // behavior. Combine it with the path that never invokes a helper join/watch.
    let pool = crate::voice_runtime(&running.state).resources();
    let mut positions = Vec::new();
    for _ in 0..8 {
        positions.push(
            pool.charge(group, 512, ResourceKind::Watcher)
                .expect("unjoined helper has all eight actual Watcher positions free"),
        );
    }
    assert!(matches!(
        pool.charge(group, 512, ResourceKind::Watcher),
        Err(VoiceError::Capacity)
    ));
    drop(positions);
}
async fn fresh(
    scenario: &Scenario,
    index: usize,
    endpoint: &Endpoint,
    other: DeviceId,
    key: &AdmittedPublishedKey,
) -> bool {
    let permit = PeerSnapshotPermit::new(
        &crate::voice_runtime(&scenario.online(index).state).resources(),
        endpoint.handle.original().scope(),
    )
    .unwrap();
    let _waiter = permit.rpc_waiter().unwrap();
    let VoiceAdmittedSupportResult::PresencePeers(snapshot) =
        rpc(&endpoint.handle, Request::Peers(permit)).await
    else {
        panic!("typed admitted peers result");
    };
    let (_, peers) = snapshot.result().unwrap();
    peers.iter().any(|fresh| fresh.announcement().issuer()==other && matches!(fresh.announcement().body(),Body::Announcement { key:announced,.. } if announced.join==key.join() && announced.key_ref==catcoms_sync::voice_v2::key_reference(&key.point())))
}
async fn helper(handle: &Handle, peer: DeviceId) -> DirectHelperSupportProbe {
    let VoiceAdmittedSupportResult::DirectHelperProbe(Ok(probe)) =
        rpc(handle, Request::Helper(peer)).await
    else {
        panic!("actual helper probe creation");
    };
    probe
}
async fn observed_helper(handle: &Handle, probe: &DirectHelperSupportProbe) -> bool {
    let VoiceAdmittedSupportResult::DirectHelperObservation(result) =
        rpc(handle, Request::HelperObservation(probe.clone())).await
    else {
        panic!("typed helper observation");
    };
    result.unwrap() == (HelperObservation::Observed { features: 1 })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admitted_control_signed_endpoint_support_uses_blocked_tcp_and_same_query_replacement() {
    let mut scenario = Scenario::new(false, isolate().await);
    scenario.admit_prefix(4).await; // Not six admissions followed by hidden eligible peers.
    scenario.rules.drop_gossip.store(true, Ordering::Release);
    for index in 0..4 {
        super::voice_live::enable(&scenario, index).await;
    }
    let mut endpoints: [Option<Endpoint>; 2] = [None, None];
    let result=crate::voice_handoff::integration::webcrypto::catch_fixture(async {
        endpoints[0]=Some(activate(&scenario,1).await);
        endpoints[1]=Some(activate(&scenario,2).await);
        let a=endpoints[0].as_ref().unwrap();let c=endpoints[1].as_ref().unwrap();
        let ids:Vec<_>=(0..4).map(|index|scenario.clients[index].identity.as_ref().unwrap().1).collect();
        let (b,d)=if ids[0]>ids[3] {(0,3)} else {(3,0)};
        assert!(ids[d]<ids[b]);
        let group=a.handle.original().scope().group;
        assert_unjoined(&scenario,b,group);assert_unjoined(&scenario,d,group);
        bounded("both endpoints have actual connected helper bindings",async {
            loop {
                let mut ready=true;
                for endpoint in [1,2] {
                    let online=scenario.online(endpoint).actor.online_members().await;
                    for bridge in [b,d] {
                        ready &= online.contains(&fingerprint(&ids[bridge]))
                            && scenario.online(endpoint).actor.member_transport_peer(ids[bridge]).await==Some(scenario.rules.peers[bridge]);
                    }
                }
                if ready { break; }
                SystemClock.sleep(Duration::from_millis(20)).await;
            }
        }).await;
        bounded("original admitted watcher proves exact native joins",async {
            loop {
                if fresh(&scenario,1,a,ids[2],&c.key).await && fresh(&scenario,2,c,ids[1],&a.key).await { break; }
                SystemClock.sleep(Duration::from_millis(20)).await;
            }
        }).await;
        // Table insertion order determines first choice; sorting IDs alone does
        // not. Explicitly install B before the lower-sorting replacement D.
        let ab=helper(&a.handle,ids[b]).await;let ad=helper(&a.handle,ids[d]).await;
        let cb=helper(&c.handle,ids[b]).await;let cd=helper(&c.handle,ids[d]).await;
        bounded("four actual signed direct-helper observations",async {
            loop {
                if observed_helper(&a.handle,&ab).await && observed_helper(&a.handle,&ad).await && observed_helper(&c.handle,&cb).await && observed_helper(&c.handle,&cd).await { break; }
                SystemClock.sleep(Duration::from_millis(20)).await;
            }
        }).await;
        let pool=crate::voice_runtime(&scenario.online(b).state).resources();
        let held=bounded("all four actual B Forward positions",async {
            loop {
                let mut held=Vec::new();
                for _ in 0..4 {
                    match pool.charge(group,512,ResourceKind::Forward) { Ok(credit)=>held.push(credit),Err(VoiceError::Capacity)=>break,Err(error)=>panic!("Forward prepayment: {error:?}") }
                }
                if held.len()==4 { break held; }
                drop(held);SystemClock.sleep(Duration::from_millis(20)).await;
            }
        }).await;
        { let mut trace=scenario.rules.trace.lock().unwrap();trace.control_submissions.clear();trace.control_evicted=0; }
        let VoiceAdmittedSupportResult::EndpointQuery(Ok(query))=rpc(&a.handle,Request::Endpoint(ids[2])).await else { panic!("actual endpoint query creation"); };
        let original=query.identity().metadata();
        let VoiceAdmittedSupportResult::EndpointQuery(Ok(recovered))=rpc(&a.handle,Request::Endpoint(ids[2])).await else { panic!("same original ensure recovery"); };
        assert!(query.identity().is_same(recovered.identity()));
        assert_eq!(original,recovered.identity().metadata());
        let clock=crate::voice_runtime(&scenario.online(1).state).clock();
        // ONE original local deadline covers every poll and both TCP bridges.
        tokio::select! {
            _=async {
                loop {
                    let VoiceAdmittedSupportResult::EndpointObservation(result)=rpc(&a.handle,Request::EndpointObservation(query.clone())).await else { panic!("typed endpoint observation"); };
                    match result.unwrap() {
                        EndpointObservation::Observed {features}=>{assert_eq!(features,1);break;},
                        EndpointObservation::Unknown=>SystemClock.sleep(Duration::from_millis(20)).await,
                    }
                }
            }=>{},
            _=clock.sleep(Duration::from_millis(original.local_deadline_ms.saturating_sub(clock.monotonic_ms())))=>panic!("original admitted Query expired before authenticated TCP endpoint Support"),
        }
        assert_eq!(query.identity().metadata(),original);
        assert!(clock.monotonic_ms()<original.local_deadline_ms);
        let trace=scenario.rules.trace.lock().unwrap();
        assert_eq!(trace.control_evicted,0,"bounded trace must retain this whole acceptance interval");
        let hops=&trace.control_submissions;
        for edge in [(1,b),(1,d),(d,2)] { assert!(hops.iter().any(|(from,to,hash)|(*from,*to)==edge && *hash==original.signed_hash),"SAME signed query must traverse {edge:?}"); }
        assert!(hops.iter().filter(|(from,_,hash)|*from==1 && *hash==original.signed_hash).count()<=3);
        let reply_hash=hops.iter().find_map(|(from,to,hash)|(*from==2 && *to==d && *hash!=original.signed_hash
            && hops.iter().any(|(next_from,next_to,next_hash)|*next_from==d && *next_to==1 && next_hash==hash)).then_some(*hash))
            .expect("the same signed endpoint reply must return C→D→A; trace alone still proves no acceptance");
        assert!(hops.iter().filter(|(from,_,hash)|*from==2 && *hash==reply_hash).count()<=3);
        drop(trace);
        scenario.rules.assert_graph();
        assert_unjoined(&scenario,b,group);assert_unjoined(&scenario,d,group);
        drop(held); // B remains blocked until the actual signed endpoint observation.
    }).await;
    // Return all opaque Query/snapshot/RPC owners before native End, then await
    // every actual Actor and TCP driver even when the acceptance assertion fails.
    let mut failure = result.err();
    for (slot, index) in [(0, 1), (1, 2)] {
        if let Some(endpoint) = endpoints[slot].take() {
            if let Err(error) = crate::voice_handoff::integration::webcrypto::catch_fixture(end(
                &scenario, index, endpoint,
            ))
            .await
            {
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
    }
    if let Err(error) =
        crate::voice_handoff::integration::webcrypto::catch_fixture(scenario.close(&[0, 1, 2, 3]))
            .await
    {
        if failure.is_none() {
            failure = Some(error);
        }
    }
    if let Some(error) = failure {
        std::panic::resume_unwind(error);
    }
}
