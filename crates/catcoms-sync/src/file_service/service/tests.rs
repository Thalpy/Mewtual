use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use futures::StreamExt;
use rand_chacha::ChaCha20Rng;
use std::sync::Mutex;
use tokio::sync::oneshot;

type Member = ChannelSync<MemNetwork, ChaCha20Rng>;
#[derive(Debug)]
struct Live;
#[async_trait::async_trait]
impl RequestAuthority for Live {
    fn authorized(&self) -> bool {
        true
    }
    async fn revoked(&self) {
        std::future::pending().await
    }
}
#[derive(Debug)]
struct Held(Mutex<Option<(FileReadTask, oneshot::Sender<FileReadCompletion>)>>);
impl FileReadExecutor for Held {
    fn start(&self, task: FileReadTask) -> FileReadFuture {
        let (send, receive) = oneshot::channel();
        assert!(self.0.lock().unwrap().replace((task, send)).is_none());
        async move { receive.await.ok() }.boxed()
    }
}
impl Held {
    fn finish(&self) {
        let (task, send) = self.0.lock().unwrap().take().unwrap();
        let _ = send.send(task.run());
    }
}
struct Fixture {
    hub: Arc<Hub>,
    roots: Vec<Member>,
    clock: Arc<ManualClock>,
    executor: Arc<Held>,
    native: NativeFileService,
    account: watch::Sender<bool>,
    actor: watch::Sender<bool>,
    cid: Cid,
    ciphertext: Vec<u8>,
    nonce: u8,
}
#[derive(Debug)]
struct Actor(watch::Receiver<bool>);
#[async_trait::async_trait]
impl RequestAuthority for Actor {
    fn authorized(&self) -> bool {
        *self.0.borrow() && self.0.has_changed().is_ok()
    }
    async fn revoked(&self) {
        let mut signal = self.0.clone();
        while self.authorized() {
            if signal.changed().await.is_err() {
                return;
            }
        }
    }
}
impl Fixture {
    async fn new() -> Self {
        let (hub, mut roots, _) = crate::tests::build_members(3).await;
        crate::tests::converge_and_publish_test_routes(&mut roots);
        roots[0]
            .initialize_group_policy(GroupMode::PeerToPeer)
            .unwrap();
        roots[0].publish_group_policy().unwrap();
        let pin = roots[0].admission_policy().unwrap().unwrap();
        let mut frame = vec![CTRL_GROUP_POLICY];
        frame.extend_from_slice(&pin.encode());
        let a = roots[0].local_peer();
        for root in roots.iter_mut().skip(1) {
            root.on_control(a, &frame);
        }
        assert!(roots.iter().all(Member::policy_allows_service));
        let b = roots[1].local_peer();
        let bid = roots[1].device.device_id();
        crate::tests::prove_live_member(&mut roots[0], b, bid);
        let clock = Arc::new(ManualClock::new(10_000));
        let executor = Arc::new(Held(Mutex::new(None)));
        let (account, session) = watch::channel(false);
        let (actor, observe) = watch::channel(true);
        let native = NativeFileService {
            reads: SourceReadRuntime::testing(executor.clone()),
            session: RequestCancellation::new(session, None),
            actor: Arc::new(Actor(observe)),
        };
        roots[0].file_service.admission = AdmissionRuntime::testing(clock.clone());
        roots[0].set_native_file_service(Some(native.clone()));
        let ciphertext = vec![0xA7; MAX_PAGE_BYTES + 123];
        let cid = roots[0].blobs.put(&ciphertext).unwrap();
        Self {
            hub,
            roots,
            clock,
            executor,
            native,
            account,
            actor,
            cid,
            ciphertext,
            nonce: 0,
        }
    }
    fn frame(&mut self) -> (Vec<u8>, Vec<u8>) {
        self.nonce += 1;
        let grant = GrantFields {
            group: &self.roots[0].group.group_id(),
            epoch: self.roots[0].epoch(),
            helper: self.roots[1].device.device_id(),
            helper_peer: self.roots[1].local_peer(),
            requester_peer: self.roots[2].local_peer(),
            window: Window {
                cid: self.cid,
                max_cipher_bytes: self.ciphertext.len() as u32,
                offset: 0,
                length: MAX_PAGE_BYTES as u32,
            },
            sources: &[self.roots[0].device.device_id()],
            created_ms: self.clock.now_ms(),
            nonce: [self.nonce; 16],
        }
        .issue(&self.roots[2].device)
        .unwrap();
        let delivery = Delivery::issue(
            &Grant::parse(&grant).unwrap(),
            self.roots[0].device.device_id(),
            self.roots[0].local_peer(),
            &self.roots[1].device,
        )
        .unwrap();
        (grant, delivery)
    }
    fn queue(&mut self, delivery: &[u8]) -> catcoms_rt::ResponderRx {
        let (responder, receive) = Responder::channel();
        let from = self.roots[1].local_peer();
        self.roots[0].queue_file_source(from, delivery, responder);
        receive
    }
    fn assert_busy(&self) {
        assert!(matches!(
            self.native.reads.prepare(
                &self.roots[0].group.group_id(),
                &*self.roots[0].blobs,
                &self.cid,
                self.ciphertext.len(),
                Arc::new(Live)
            ),
            Err(SourceReadError::Busy)
        ));
    }
    async fn complete(&mut self) {
        let done = self.roots[0].file_service.jobs.next().await.unwrap();
        self.roots[0].complete_file_service(done);
    }
}

#[tokio::test]
async fn connected_source_page_without_voice_keeps_actual_response_custody_and_storage_gate() {
    let mut f = Fixture::new().await;
    assert!(f.roots[0].voice_runtime.is_none());
    let (grant, delivery) = f.frame();
    let receive = f.queue(&delivery);
    f.assert_busy();
    // Cancellation of an outer owner poll leaves the actual task, nonce and responder on Root.
    {
        let mut next = std::pin::pin!(f.roots[0].file_service.jobs.next());
        assert!(futures::poll!(next.as_mut()).is_pending());
    }
    f.executor.finish();
    // File completion gets its own rotating owner turn even with ready transport events.
    f.roots[0].recovery_turn = 3;
    assert!(f.roots[0].next_recovery_or_event().await.is_none());
    let response = receive.recv_owned().await.unwrap();
    let page = SourcePage::parse(response.data(), &Grant::parse(&grant).unwrap()).unwrap();
    assert_eq!(page.status(), SourceStatus::Present);
    assert_eq!(page.page(), &f.ciphertext[..MAX_PAGE_BYTES]);
    assert!(page.verify_signature());
    assert!(
        response.is_current(),
        "reply-wait retirement cannot retire final effect"
    );
    f.assert_busy();
    let driver = response.keepalive().unwrap();
    f.roots[0].blobs.delete(&f.cid).unwrap();
    assert!(
        !response.is_current(),
        "Forget before final driver admission refuses Present"
    );
    drop(response);
    f.assert_busy();
    drop(driver);
    assert!(f
        .native
        .reads
        .prepare(
            &f.roots[0].group.group_id(),
            &*f.roots[0].blobs,
            &f.cid,
            f.ciphertext.len(),
            Arc::new(Live)
        )
        .is_ok());
}

#[tokio::test]
async fn source_native_actor_descriptor_connection_and_original_membership_never_refresh_old_work()
{
    for boundary in 0..5 {
        let mut f = Fixture::new().await;
        let (_, delivery) = f.frame();
        let receive = f.queue(&delivery);
        assert_eq!(f.roots[0].file_service.jobs.len(), 1);
        match boundary {
            0 => {
                f.account.send_replace(true);
            }
            1 => {
                f.actor.send_replace(false);
            }
            2 => {
                f.roots[1].publish_self_record(vec![], 2).unwrap();
                let updated = f.roots[1].self_record().unwrap().clone();
                f.roots[0].ingest_peer_record(updated);
            }
            3 => {
                let _replacement = f.hub.join(f.roots[1].local_peer());
            }
            4 => {
                let target = f.roots[2].device.device_id();
                let root = &mut f.roots[0];
                root.group.stage_remove(&root.device, &target).unwrap();
                root.with_observed_mls_transition(|root| {
                    root.group.merge_staged_self(&root.device)
                })
                .unwrap();
            }
            _ => unreachable!(),
        }
        // Install fresh native owners after freeze/unlock. They cannot revive the old capture.
        let (account, session) = watch::channel(false);
        let (actor, observe) = watch::channel(true);
        f.roots[0].set_native_file_service(Some(NativeFileService {
            reads: f.native.reads.clone(),
            session: RequestCancellation::new(session, None),
            actor: Arc::new(Actor(observe)),
        }));
        f.roots[0].reconcile_file_service_routes();
        f.complete().await;
        assert!(
            receive.recv().await.is_none(),
            "retired boundary {boundary} returned a page"
        );
        f.assert_busy(); // detached actual worker still owns its charge
        f.executor.finish();
        let _ = (account, actor);
    }
}

#[tokio::test]
async fn source_read_timeout_is_signed_unavailable_without_refunding_actual_worker_or_replaying_nonce(
) {
    let mut f = Fixture::new().await;
    let (grant, delivery) = f.frame();
    let receive = f.queue(&delivery);
    f.clock.advance_ms(READ_MS);
    f.complete().await;
    let response = receive.recv_owned().await.unwrap();
    let page = SourcePage::parse(response.data(), &Grant::parse(&grant).unwrap()).unwrap();
    assert_eq!(page.status(), SourceStatus::Unavailable);
    assert!(page.verify_signature());
    f.assert_busy();
    assert!(
        f.queue(&delivery).recv().await.is_none(),
        "nonce cannot rerun after local timeout"
    );
    f.executor.finish();
    f.assert_busy(); // response, not only the worker, owns the SAME credit
    drop(response);
    assert!(f
        .native
        .reads
        .prepare(
            &f.roots[0].group.group_id(),
            &*f.roots[0].blobs,
            &f.cid,
            f.ciphertext.len(),
            Arc::new(Live)
        )
        .is_ok());
}

#[tokio::test]
async fn queued_present_keeps_exact_two_second_read_deadline_after_reply_receive() {
    let mut f = Fixture::new().await;
    let (_, delivery) = f.frame();
    let receive = f.queue(&delivery);
    f.executor.finish();
    f.complete().await;
    let response = receive.recv_owned().await.unwrap();
    assert!(response.is_current());
    f.clock.advance_ms(READ_MS);
    assert!(
        !response.is_current(),
        "queue/receive cannot extend the admitted read effect deadline"
    );
    f.assert_busy();
    drop(response);
    assert!(f
        .native
        .reads
        .prepare(
            &f.roots[0].group.group_id(),
            &*f.roots[0].blobs,
            &f.cid,
            f.ciphertext.len(),
            Arc::new(Live)
        )
        .is_ok());
}

#[tokio::test]
async fn already_retired_storage_refuses_first_poll_of_present_authority_wait() {
    let mut f = Fixture::new().await;
    let (_, delivery) = f.frame();
    let _receive = f.queue(&delivery);
    f.executor.finish();
    let Completion::Source(completed) = f.roots[0].file_service.jobs.next().await.unwrap() else {
        panic!("source completion")
    };
    let CompletedSource { result, flow } = *completed;
    let authority = PresentAuthority {
        scope: flow.scope,
        deadline: flow.read_deadline,
        response: Arc::new(SourceResponse {
            completed: result,
            _flow: flow.custody,
        }),
    };
    assert!(authority.authorized());
    f.roots[0].blobs.delete(&f.cid).unwrap();
    assert!(!authority.authorized());
    let wait = authority.revoked();
    let mut wait = std::pin::pin!(wait);
    assert!(futures::poll!(wait.as_mut()).is_ready());
    f.assert_busy();
}

#[tokio::test]
async fn source_refuses_missing_native_legacy_policy_and_unbound_sender_before_start() {
    for boundary in 0..3 {
        let mut f = Fixture::new().await;
        let (_, delivery) = f.frame();
        match boundary {
            0 => f.roots[0].set_native_file_service(None),
            1 => f.roots[0].group_policy_active = false,
            2 => {
                for proof in &mut f.roots[0].member_peers {
                    proof.bound = false;
                }
            }
            _ => unreachable!(),
        }
        assert!(f.queue(&delivery).recv().await.is_none());
        assert!(f.executor.0.lock().unwrap().is_none());
        assert!(f.roots[0].file_service.jobs.is_empty());
    }
}

#[tokio::test]
async fn actual_source_dispatch_returns_through_owner_queue_without_opening_a_document() {
    let mut f = Fixture::new().await;
    let (grant, delivery) = f.frame();
    let transport = f.roots[1].transport.clone();
    let target = f.roots[0].local_peer();
    let request = transport.request(target, ProtocolId(RR_PROTOCOL), Bytes::from(delivery));
    let mut request = std::pin::pin!(request);
    assert!(futures::poll!(request.as_mut()).is_pending());
    // The actual dispatch sees ordinary queued transport events as well. Its file handler must
    // install the responder on Root without awaiting the deliberately held native read.
    while f.roots[0].file_service.jobs.is_empty() {
        f.roots[0].run_once().await.unwrap();
    }
    assert!(futures::poll!(request.as_mut()).is_pending());
    f.executor.finish();
    f.roots[0].recovery_turn = 3;
    assert!(f.roots[0].next_recovery_or_event().await.is_none());
    let bytes = request.await.unwrap();
    let page = SourcePage::parse(&bytes, &Grant::parse(&grant).unwrap()).unwrap();
    assert_eq!(page.status(), SourceStatus::Present);
    assert!(page.verify_signature());
}

#[tokio::test]
async fn source_capacity_and_absence_are_distinct_and_root_drop_does_not_refund_worker() {
    let mut f = Fixture::new().await;
    let held = f
        .native
        .reads
        .prepare(
            &f.roots[0].group.group_id(),
            &*f.roots[0].blobs,
            &f.cid,
            f.ciphertext.len(),
            Arc::new(Live),
        )
        .unwrap();
    let (grant, delivery) = f.frame();
    let bytes = f.queue(&delivery).recv().await.unwrap();
    assert_eq!(
        SourcePage::parse(&bytes, &Grant::parse(&grant).unwrap())
            .unwrap()
            .status(),
        SourceStatus::Unavailable
    );
    assert!(f.executor.0.lock().unwrap().is_none());
    drop(held);
    f.roots[0].blobs.delete(&f.cid).unwrap();
    let (grant, delivery) = f.frame();
    let receive = f.queue(&delivery);
    f.executor.finish();
    f.complete().await;
    let response = receive.recv_owned().await.unwrap();
    assert_eq!(
        SourcePage::parse(response.data(), &Grant::parse(&grant).unwrap())
            .unwrap()
            .status(),
        SourceStatus::Absent
    );
    assert!(
        response.is_current(),
        "a past absence observation needs no positive storage fence"
    );
    drop(response);

    let (_, delivery) = f.frame();
    let receive = f.queue(&delivery);
    let group = f.roots[0].group.group_id();
    drop(f.roots.remove(0));
    assert!(receive.recv().await.is_none());
    let store = MemoryBlobStore::default();
    assert!(matches!(
        f.native
            .reads
            .prepare(&group, &store, &f.cid, f.ciphertext.len(), Arc::new(Live)),
        Err(SourceReadError::Busy)
    ));
    f.executor.finish();
    assert!(f
        .native
        .reads
        .prepare(&group, &store, &f.cid, f.ciphertext.len(), Arc::new(Live))
        .is_ok());
}

impl Fixture {
    fn enable_helper(&mut self) {
        let a = self.roots[0].local_peer();
        let aid = self.roots[0].device.device_id();
        let c = self.roots[2].local_peer();
        let cid = self.roots[2].device.device_id();
        crate::tests::prove_live_member(&mut self.roots[1], a, aid);
        crate::tests::prove_live_member(&mut self.roots[1], c, cid);
        self.roots[1].file_service.admission = AdmissionRuntime::testing(self.clock.clone());
        self.roots[1].file_service.helpers = HelperCapacity::testing();
        self.roots[1].set_native_file_service(Some(self.native.clone()));
    }
    fn queue_helper(&mut self, grant: &[u8]) -> catcoms_rt::ResponderRx {
        let (send, receive) = Responder::channel();
        let c = self.roots[2].local_peer();
        self.roots[1].queue_file_helper(c, grant, send);
        receive
    }
    async fn complete_helper(&mut self) {
        let done = self.roots[1].file_service.jobs.next().await.unwrap();
        self.roots[1].complete_file_service(done);
    }
}

#[tokio::test]
async fn helper_and_source_actual_owner_paths_preserve_both_signatures_and_final_source_scope() {
    for retire in 0..3 {
        let mut f = Fixture::new().await;
        f.enable_helper();
        let (grant, _) = f.frame();
        let receive = f.queue_helper(&grant);
        assert!(
            futures::poll!(std::pin::pin!(f.roots[1].file_service.jobs.next()).as_mut())
                .is_pending()
        );
        while f.roots[0].file_service.jobs.is_empty() {
            f.roots[0].run_once().await.unwrap();
        }
        f.executor.finish();
        f.complete().await;
        f.complete_helper().await;
        let response = receive.recv_owned().await.unwrap();
        let page = HelperPage::parse(response.data(), &Grant::parse(&grant).unwrap()).unwrap();
        assert_eq!(page.status(), HelperStatus::Found);
        assert!(page.verify_signature());
        let source = page.source().unwrap();
        assert_eq!(source.source(), f.roots[0].device.device_id());
        assert!(source.verify_signature());
        assert_eq!(source.page(), &f.ciphertext[..MAX_PAGE_BYTES]);
        assert!(
            response.is_current(),
            "reply consumption cannot cancel a healthy Found"
        );
        match retire {
            0 => {
                f.clock.advance_ms(READ_MS);
            }
            1 => {
                f.roots[0].publish_self_record(vec![], 2).unwrap();
                let record = f.roots[0].self_record().unwrap().clone();
                f.roots[1].ingest_peer_record(record);
            }
            2 => {
                f.account.send_replace(true);
            }
            _ => unreachable!(),
        }
        f.roots[1].reconcile_file_service_routes();
        assert!(
            !response.is_current(),
            "Found escaped retired selected-source boundary {retire}"
        );
        assert!(matches!(
            f.roots[1]
                .file_service
                .helpers
                .reserve(&f.roots[1].group.group_id(), f.roots[2].device.device_id()),
            Err(HelperCapacityError::Busy)
        ));
    }
}

struct HelperFixture {
    roots: Vec<Member>,
    clock: Arc<ManualClock>,
    sources: Vec<usize>,
    callers: Vec<usize>,
    _account: watch::Sender<bool>,
}
impl HelperFixture {
    async fn new(source_count: usize, caller_count: usize) -> Self {
        let (_, mut roots, _) =
            crate::tests::build_members((1 + source_count + caller_count) as u64).await;
        crate::tests::converge_and_publish_test_routes(&mut roots);
        roots[0]
            .initialize_group_policy(GroupMode::PeerToPeer)
            .unwrap();
        roots[0].publish_group_policy().unwrap();
        let pin = roots[0].admission_policy().unwrap().unwrap();
        let mut frame = vec![CTRL_GROUP_POLICY];
        frame.extend_from_slice(&pin.encode());
        let helper = roots[0].local_peer();
        for root in roots.iter_mut().skip(1) {
            root.on_control(helper, &frame);
        }
        let routes: Vec<_> = roots
            .iter()
            .skip(1)
            .map(|r| (r.local_peer(), r.device.device_id()))
            .collect();
        for (peer, device) in routes {
            crate::tests::prove_live_member(&mut roots[0], peer, device);
        }
        let clock = Arc::new(ManualClock::new(20_000));
        roots[0].file_service.admission = AdmissionRuntime::testing(clock.clone());
        roots[0].file_service.helpers = HelperCapacity::testing();
        let (account, session) = watch::channel(false);
        roots[0].set_native_file_service(Some(NativeFileService {
            reads: SourceReadRuntime::testing(Arc::new(Held(Mutex::new(None)))),
            session: RequestCancellation::new(session, None),
            actor: Arc::new(Live),
        }));
        let mut sources: Vec<_> = (1..=source_count).collect();
        sources.sort_by_key(|i| roots[*i].device.device_id());
        Self {
            roots,
            clock,
            sources,
            callers: (source_count + 1..source_count + 1 + caller_count).collect(),
            _account: account,
        }
    }
    fn grant(&self, caller: usize, nonce: u8) -> Vec<u8> {
        let sources: Vec<_> = self
            .sources
            .iter()
            .map(|i| self.roots[*i].device.device_id())
            .collect();
        GrantFields {
            group: &self.roots[0].group.group_id(),
            epoch: self.roots[0].epoch(),
            helper: self.roots[0].device.device_id(),
            helper_peer: self.roots[0].local_peer(),
            requester_peer: self.roots[caller].local_peer(),
            window: Window {
                cid: Cid::of(&[42; 256]),
                max_cipher_bytes: 256,
                offset: 0,
                length: 256,
            },
            sources: &sources,
            created_ms: self.clock.now_ms(),
            nonce: [nonce; 16],
        }
        .issue(&self.roots[caller].device)
        .unwrap()
    }
    fn queue(&mut self, caller: usize, grant: &[u8]) -> catcoms_rt::ResponderRx {
        let (send, receive) = Responder::channel();
        let peer = self.roots[caller].local_peer();
        self.roots[0].queue_file_helper(peer, grant, send);
        receive
    }
    async fn start_lowers(&mut self) {
        let mut next = std::pin::pin!(self.roots[0].file_service.jobs.next());
        assert!(futures::poll!(next.as_mut()).is_pending());
    }
    async fn lower(&self, source: usize) -> (Bytes, Responder) {
        loop {
            if let TransportEvent::Request {
                data, responder, ..
            } = self.roots[source].transport.next_event().await.unwrap()
            {
                if data.first() == Some(&catcoms_rt::FILE_SOURCE_KIND) {
                    return (data, responder);
                }
            }
        }
    }
    async fn complete(&mut self) {
        let done = self.roots[0].file_service.jobs.next().await.unwrap();
        self.roots[0].complete_file_service(done);
    }
}

#[tokio::test]
async fn four_parallel_helper_chains_reach_fourth_source_while_all_prior_lowers_remain_owned() {
    let mut f = HelperFixture::new(4, 4).await;
    let callers = f.callers.clone();
    let grants: Vec<_> = callers
        .iter()
        .enumerate()
        .map(|(i, caller)| f.grant(*caller, i as u8))
        .collect();
    let mut responses = Vec::new();
    for (caller, grant) in callers.iter().zip(&grants) {
        responses.push(f.queue(*caller, grant));
    }
    assert_eq!(f.roots[0].file_service.jobs.len(), 4);
    let mut actual_lowers = Vec::new();
    for attempt in 0..3 {
        f.start_lowers().await;
        for _ in 0..4 {
            actual_lowers.push(f.lower(f.sources[attempt]).await);
        }
        f.clock.advance_ms(READ_MS);
        for _ in 0..4 {
            f.complete().await;
        }
        assert_eq!(f.roots[0].file_service.jobs.len(), 4);
    }
    f.start_lowers().await;
    for _ in 0..4 {
        let (frame, responder) = f.lower(f.sources[3]).await;
        let delivery = Delivery::parse(&frame).unwrap();
        let source = SourcePage::issue(
            delivery.grant(),
            SourceStatus::Present,
            256,
            &[42; 256],
            &f.roots[f.sources[3]].device,
        )
        .unwrap();
        responder.respond(Bytes::from(source));
    }
    for _ in 0..4 {
        f.complete().await;
    }
    let mut final_responses = Vec::new();
    for (receive, grant) in responses.into_iter().zip(&grants) {
        let response = receive.recv_owned().await.unwrap();
        let page = HelperPage::parse(response.data(), &Grant::parse(grant).unwrap()).unwrap();
        assert_eq!(page.status(), HelperStatus::Found);
        assert_eq!(
            page.source().unwrap().source(),
            f.roots[f.sources[3]].device.device_id()
        );
        final_responses.push(response);
    }
    assert_eq!(actual_lowers.len(), 12);
    drop(final_responses);
    for caller in &callers {
        assert!(
            matches!(
                f.roots[0].file_service.helpers.reserve(
                    &f.roots[0].group.group_id(),
                    f.roots[*caller].device.device_id()
                ),
                Err(HelperCapacityError::Busy)
            ),
            "timed-out actual lower responders still own each original chain"
        );
    }
    drop(actual_lowers);
    for caller in callers {
        assert!(f.roots[0]
            .file_service
            .helpers
            .reserve(
                &f.roots[0].group.group_id(),
                f.roots[caller].device.device_id()
            )
            .is_ok());
    }
}

#[tokio::test]
async fn helper_rejects_another_named_source_and_only_all_signed_absent_produces_absent() {
    for outcome in 0..4 {
        let mut f = HelperFixture::new(2, 1).await;
        let caller = f.callers[0];
        let grant = f.grant(caller, 1);
        let receive = f.queue(caller, &grant);
        f.start_lowers().await;
        let (frame, responder) = f.lower(f.sources[0]).await;
        let delivery = Delivery::parse(&frame).unwrap();
        let (status, signer, total, page): (_, _, _, &[u8]) = if outcome == 0 {
            (SourceStatus::Present, f.sources[1], 256, &[42; 256]) // valid named signer, wrong selected A
        } else if outcome == 1 {
            (SourceStatus::Unavailable, f.sources[0], 0, &[])
        } else {
            (SourceStatus::Absent, f.sources[0], 0, &[])
        };
        if outcome == 3 {
            responder.respond(Bytes::new());
        }
        // released old peer, no service
        else {
            let source = SourcePage::issue(
                delivery.grant(),
                status,
                total,
                page,
                &f.roots[signer].device,
            )
            .unwrap();
            responder.respond(Bytes::from(source));
        }
        f.complete().await;
        assert_eq!(
            f.roots[0].file_service.jobs.len(),
            1,
            "wrong source cannot finish Found"
        );
        f.start_lowers().await;
        let (frame, responder) = f.lower(f.sources[1]).await;
        let delivery = Delivery::parse(&frame).unwrap();
        let source = SourcePage::issue(
            delivery.grant(),
            SourceStatus::Absent,
            0,
            &[],
            &f.roots[f.sources[1]].device,
        )
        .unwrap();
        responder.respond(Bytes::from(source));
        f.complete().await;
        let bytes = receive.recv().await.unwrap();
        assert_eq!(
            HelperPage::parse(&bytes, &Grant::parse(&grant).unwrap())
                .unwrap()
                .status(),
            if outcome == 2 {
                HelperStatus::Absent
            } else {
                HelperStatus::Unavailable
            }
        );
    }
}

#[tokio::test]
async fn unused_source_route_loss_can_rotate_but_reply_cancellation_keeps_actual_lower_custody() {
    let mut f = HelperFixture::new(2, 1).await;
    let first = f.roots[f.sources[0]].device.device_id();
    f.roots[0].member_peers.retain(|p| p.device != first);
    let caller = f.callers[0];
    let grant = f.grant(caller, 1);
    let receive = f.queue(caller, &grant);
    f.start_lowers().await;
    let (frame, responder) = f.lower(f.sources[1]).await;
    let delivery = Delivery::parse(&frame).unwrap();
    let source = SourcePage::issue(
        delivery.grant(),
        SourceStatus::Present,
        256,
        &[42; 256],
        &f.roots[f.sources[1]].device,
    )
    .unwrap();
    responder.respond(Bytes::from(source));
    f.complete().await;
    let response = receive.recv_owned().await.unwrap();
    assert!(
        response.is_current(),
        "unused source loss cannot retire a successful selected source"
    );
    assert_eq!(
        HelperPage::parse(response.data(), &Grant::parse(&grant).unwrap())
            .unwrap()
            .status(),
        HelperStatus::Found
    );
    drop(response);

    let grant = f.grant(caller, 2);
    let receive = f.queue(caller, &grant);
    f.start_lowers().await;
    let actual_lower = f.lower(f.sources[1]).await;
    drop(receive);
    f.complete().await;
    assert!(matches!(
        f.roots[0].file_service.helpers.reserve(
            &f.roots[0].group.group_id(),
            f.roots[caller].device.device_id()
        ),
        Err(HelperCapacityError::Busy)
    ));
    drop(actual_lower);
    assert!(f.roots[0]
        .file_service
        .helpers
        .reserve(
            &f.roots[0].group.group_id(),
            f.roots[caller].device.device_id()
        )
        .is_ok());
}

#[tokio::test]
async fn final_helper_response_refusal_cannot_refund_original_source_bytes_or_their_clones() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct SourceBuffer {
        data: Vec<u8>,
        pool: HelperCapacity,
        group: Vec<u8>,
        original: DeviceId,
        dropped: Arc<AtomicBool>,
        charged_at_drop: Arc<AtomicBool>,
    }
    impl AsRef<[u8]> for SourceBuffer {
        fn as_ref(&self) -> &[u8] {
            &self.data
        }
    }
    impl Drop for SourceBuffer {
        fn drop(&mut self) {
            self.charged_at_drop.store(
                matches!(
                    self.pool.reserve(&self.group, self.original),
                    Err(HelperCapacityError::Busy)
                ),
                Ordering::SeqCst,
            );
            self.dropped.store(true, Ordering::SeqCst);
        }
    }
    #[derive(Debug)]
    struct Refuse;
    impl catcoms_rt::ResponseBudget for Refuse {
        fn retain(self: Arc<Self>, _: usize) -> Result<SharedRequestKeepalive, TransportError> {
            Err(TransportError::NoResponse)
        }
    }
    for keep_clone in [false, true] {
        let mut f = HelperFixture::new(1, 1).await;
        let caller = f.callers[0];
        let grant = f.grant(caller, 1);
        let (responder, receive) =
            Responder::channel_owned(Arc::new(()), Arc::new(Live), Arc::new(Refuse));
        let peer = f.roots[caller].local_peer();
        f.roots[0].queue_file_helper(peer, &grant, responder);
        f.start_lowers().await;
        let (frame, responder) = f.lower(f.sources[0]).await;
        let delivery = Delivery::parse(&frame).unwrap();
        let data = SourcePage::issue(
            delivery.grant(),
            SourceStatus::Present,
            256,
            &[42; 256],
            &f.roots[f.sources[0]].device,
        )
        .unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let charged_at_drop = Arc::new(AtomicBool::new(false));
        responder.respond(Bytes::from_owner(SourceBuffer {
            data,
            pool: f.roots[0].file_service.helpers.clone(),
            group: f.roots[0].group.group_id(),
            original: f.roots[caller].device.device_id(),
            dropped: dropped.clone(),
            charged_at_drop: charged_at_drop.clone(),
        }));
        let Completion::Helper(completed) = f.roots[0].file_service.jobs.next().await.unwrap()
        else {
            panic!("helper completion")
        };
        let clone = if keep_clone {
            completed.response_clone_for_test()
        } else {
            None
        };
        f.roots[0].complete_file_service(Completion::Helper(completed));
        assert!(receive.recv().await.is_none());
        if keep_clone {
            assert!(!dropped.load(Ordering::SeqCst));
            assert!(matches!(
                f.roots[0].file_service.helpers.reserve(
                    &f.roots[0].group.group_id(),
                    f.roots[caller].device.device_id()
                ),
                Err(HelperCapacityError::Busy)
            ));
        }
        drop(clone);
        assert!(dropped.load(Ordering::SeqCst));
        assert!(
            charged_at_drop.load(Ordering::SeqCst),
            "final refusal released capacity before the original source buffer"
        );
        assert!(f.roots[0]
            .file_service
            .helpers
            .reserve(
                &f.roots[0].group.group_id(),
                f.roots[caller].device.device_id()
            )
            .is_ok());
    }
}
