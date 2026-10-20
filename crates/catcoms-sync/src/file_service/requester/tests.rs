use super::*;
use crate::file_service::read::{
    FileReadExecutor, FileReadFuture, FileReadTask, SourceReadRuntime,
};
use catcoms_rt::{ManualClock, MemNetwork};
use futures::FutureExt;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

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
struct UnusedExecutor;
impl FileReadExecutor for UnusedExecutor {
    fn start(&self, _: FileReadTask) -> FileReadFuture {
        panic!("requester must not start source storage")
    }
}
#[derive(Debug)]
struct ImmediateExecutor;
impl FileReadExecutor for ImmediateExecutor {
    fn start(&self, task: FileReadTask) -> FileReadFuture {
        async move { Some(task.run()) }.boxed()
    }
}
struct Fixture {
    roots: Vec<Member>,
    clock: Arc<ManualClock>,
    native: watch::Sender<bool>,
    caller: watch::Sender<bool>,
    catalog: BridgeCatalog,
    reference: FileRef,
    ciphertext: Vec<u8>,
}
impl Fixture {
    /// Only three live native owners, with the helper and sole source after 512 offline leaves.
    /// Building the native MLS state is setup work; every subsequent C discovery step is paged.
    fn late_source() -> Self {
        let c = MlsDevice::generate().unwrap();
        let mut cg = ServerGroup::create(&c).unwrap();
        let group_id = cg.group_id();
        for index in 0u64..512 {
            let offline = MlsDevice::generate().unwrap();
            let mut nonce = [0; 16];
            nonce[..8].copy_from_slice(&index.to_be_bytes());
            cg.add_member(
                &c,
                offline.key_package_for_invite(&group_id, nonce).unwrap(),
            )
            .unwrap();
        }
        let b = MlsDevice::generate().unwrap();
        let added_b = cg
            .add_member(&c, b.key_package_for_invite(&group_id, [0xB1; 16]).unwrap())
            .unwrap();
        let mut bg = ServerGroup::join(&b, &added_b.welcome).unwrap();
        let a = MlsDevice::generate().unwrap();
        let added_a = cg
            .add_member(&c, a.key_package_for_invite(&group_id, [0xA1; 16]).unwrap())
            .unwrap();
        bg.process_incoming(&b, &added_a.commit).unwrap();
        let ag = ServerGroup::join(&a, &added_a.welcome).unwrap();
        assert!(cg.member_leaf_index(&a.device_id()).unwrap() > 512);
        assert!(cg.member_leaf_index(&b.device_id()).unwrap() > 512);
        let hub = catcoms_rt::Hub::new();
        let clock = Arc::new(ManualClock::new(1_000));
        let mut roots = vec![(a, ag), (b, bg), (c, cg)]
            .into_iter()
            .enumerate()
            .map(|(i, (dev, group))| {
                let libp2p = libp2p::identity::Keypair::ed25519_from_bytes([i as u8 + 1; 32])
                    .unwrap()
                    .public()
                    .to_peer_id();
                ChannelSync::new(
                    hub.join(catcoms_net::phase0_peer_id(&libp2p)),
                    group,
                    dev,
                    ChaCha20Rng::seed_from_u64(i as u64 + 1),
                    Box::new(clock.as_ref().clone()),
                )
            })
            .collect::<Vec<_>>();
        crate::tests::converge_and_publish_test_routes(&mut roots);
        roots[2]
            .initialize_group_policy(GroupMode::PeerToPeer)
            .unwrap();
        roots[2].publish_group_policy().unwrap();
        let mut frame = vec![CTRL_GROUP_POLICY];
        frame.extend_from_slice(&roots[2].admission_policy().unwrap().unwrap().encode());
        let founder = roots[2].local_peer();
        for root in roots.iter_mut().take(2) {
            root.on_control(founder, &frame);
        }
        let helper_peer = roots[1].local_peer();
        let helper_id = roots[1].device.device_id();
        crate::tests::prove_live_member(&mut roots[2], helper_peer, helper_id);
        let (native, session) = watch::channel(false);
        roots[2].set_native_file_service(Some(super::super::NativeFileService {
            reads: SourceReadRuntime::testing(Arc::new(UnusedExecutor)),
            session: RequestCancellation::new(session, None),
            actor: Arc::new(Live),
        }));
        let (caller, signal) = watch::channel(false);
        let catalog = roots[2]
            .begin_file_bridge_catalog(RequestCancellation::new(signal, None), Arc::new(Live))
            .unwrap();
        let (reference, ciphertext) = roots[0]
            .seal_file(b"sole late source", "text/plain")
            .unwrap();
        Self {
            roots,
            clock,
            native,
            caller,
            catalog,
            reference,
            ciphertext,
        }
    }
    async fn new() -> Self {
        Self::with_members(3).await
    }
    async fn with_members(count: u64) -> Self {
        let (_, mut roots, ids) = crate::tests::build_members(count).await;
        crate::tests::converge_and_publish_test_routes(&mut roots);
        roots[0]
            .initialize_group_policy(GroupMode::PeerToPeer)
            .unwrap();
        roots[0].publish_group_policy().unwrap();
        let mut frame = vec![CTRL_GROUP_POLICY];
        frame.extend_from_slice(&roots[0].admission_policy().unwrap().unwrap().encode());
        let founder = roots[0].local_peer();
        for root in roots.iter_mut().skip(1) {
            root.on_control(founder, &frame);
        }
        let b = roots[1].local_peer();
        crate::tests::prove_live_member(&mut roots[2], b, ids[1]);
        let clock = Arc::new(ManualClock::new(1_000));
        roots[2].clock = clock.clone();
        let (native, session) = watch::channel(false);
        roots[2].set_native_file_service(Some(super::super::NativeFileService {
            reads: SourceReadRuntime::testing(Arc::new(UnusedExecutor)),
            session: RequestCancellation::new(session, None),
            actor: Arc::new(Live),
        }));
        let (caller, signal) = watch::channel(false);
        let catalog = roots[2]
            .begin_file_bridge_catalog(RequestCancellation::new(signal, None), Arc::new(Live))
            .unwrap();
        let (reference, ciphertext) = roots[0]
            .seal_file(b"original encrypted file", "text/plain")
            .unwrap();
        Self {
            roots,
            clock,
            native,
            caller,
            catalog,
            reference,
            ciphertext,
        }
    }
    fn choice(&mut self) -> BridgeChoice {
        for _ in 0..16 {
            match self.roots[2]
                .next_file_bridge_choice(
                    &mut self.catalog,
                    &[bridge_reference_variant(&self.reference).unwrap()],
                )
                .unwrap()
            {
                BridgeStep::Ready(choice) => return choice,
                BridgeStep::Yield | BridgeStep::Backoff => {}
                BridgeStep::Exhausted => panic!("helper/source choices unexpectedly exhausted"),
            }
        }
        panic!("bounded bootstrap did not find the connected helper")
    }
    fn prepare(&mut self, choice: &BridgeChoice) -> PreparedBridgePage {
        self.roots[2]
            .prepare_file_bridge_page(&self.catalog, choice, &self.reference, 0, MAX_PAGE_BYTES)
            .unwrap()
    }
    fn response(&self, grant: &[u8]) -> Vec<u8> {
        let grant = Grant::parse(grant).unwrap();
        let source = SourcePage::issue(
            &grant,
            SourceStatus::Present,
            self.ciphertext.len() as u32,
            &self.ciphertext,
            &self.roots[0].device,
        )
        .unwrap();
        let source = SourcePage::parse(&source, &grant).unwrap();
        HelperPage::issue(
            &grant,
            HelperStatus::Found,
            Some(&source),
            &self.roots[1].device,
        )
        .unwrap()
    }

    fn enable_real_service(&mut self, service_clock: Arc<ManualClock>) {
        let admission = super::super::admission::AdmissionRuntime::testing(service_clock);
        let ids: Vec<_> = self
            .roots
            .iter()
            .map(|r| (r.local_peer(), r.device.device_id()))
            .collect();
        for (index, root) in self.roots.iter_mut().enumerate() {
            for (other, &(peer, device)) in ids.iter().enumerate() {
                // C has only the authenticated B route; A is not eligible as its helper.
                if other != index && (index != 2 || other == 1) {
                    crate::tests::prove_live_member(root, peer, device);
                }
            }
            root.file_service.admission = admission.clone();
            root.set_native_file_service(Some(super::super::NativeFileService {
                reads: SourceReadRuntime::testing(Arc::new(ImmediateExecutor)),
                session: RequestCancellation::new(self.native.subscribe(), None),
                actor: Arc::new(Live),
            }));
        }
        self.roots[0].put_blob(&self.ciphertext).unwrap();
    }

    /// Drive actual kind29 dispatch at B and kind30 dispatch/storage at A. The C response is
    /// produced by those owners, rather than fabricating signed pages at the requester boundary.
    async fn wire_page(&mut self, choice: &BridgeChoice, offset: u32) -> BlobPageOutcome {
        self.clock.advance_ms(REQUEST_INTERVAL_MS);
        let prepared = self.roots[2]
            .prepare_file_bridge_page(
                &self.catalog,
                choice,
                &self.reference,
                offset,
                MAX_PAGE_BYTES,
            )
            .unwrap();
        let pending = self.roots[2]
            .commit_file_bridge_page(&mut self.catalog, prepared)
            .unwrap();
        let (_cancel, receiver) = watch::channel(false);
        let request = pending.fetch(RequestCancellation::new(receiver, None));
        let request = std::pin::pin!(request);
        let (ab, c) = self.roots.split_at_mut(2);
        let (a, b) = ab.split_at_mut(1);
        let completed = tokio::time::timeout(Duration::from_secs(10), async {
            let mut request = request;
            loop {
                tokio::select! {
                    result = &mut request => break result,
                    result = a[0].run_once() => { result.unwrap(); },
                    result = b[0].run_once() => { result.unwrap(); },
                }
            }
        })
        .await
        .expect("actual helper/source service stalled");
        c[0].authenticate_blob_page(completed).unwrap()
    }
}

#[tokio::test]
async fn replaced_helper_connection_discards_old_page_and_retries_same_sources() {
    let mut f = Fixture::new().await;
    let choice = f.choice();
    let prepared = f.prepare(&choice);
    let context = f.prepare(&choice).context;
    let response = f.response(&context.grant);
    let effect = context.effect_scope();
    let old_grant = Grant::parse(&prepared.frame).unwrap();
    let old_sources = old_grant.sources().to_vec();
    let old_nonce = old_grant.nonce();
    let deadline = prepared.deadline();
    let pending = f.roots[2]
        .commit_file_bridge_page(&mut f.catalog, prepared)
        .unwrap();
    // Same PeerId, new actual connection incarnation. No descriptor parity gate is required.
    f.roots[1].transport = Arc::new(f.roots[1].transport.rejoin());
    assert_eq!(
        f.roots[2].blob_effect_status(&effect),
        BlobEffectStatus::RouteChanged
    );
    assert!(matches!(
        f.roots[2].authenticate_bridge_page(context, Ok(response.into())),
        Ok(BlobPageOutcome::RouteChanged)
    ));
    drop(pending);
    f.clock.advance_ms(REQUEST_INTERVAL_MS);
    let refreshed = f.roots[2]
        .refresh_file_bridge_choice(&mut f.catalog, &choice)
        .unwrap();
    let retry = f.prepare(&refreshed);
    let grant = Grant::parse(&retry.frame).unwrap();
    assert_eq!(grant.sources(), old_sources);
    assert_ne!(grant.nonce(), old_nonce);
    assert!(f.clock.monotonic_ms() < deadline);
    let response = f.response(&retry.frame);
    assert!(matches!(
        f.roots[2].authenticate_bridge_page(retry.context, Ok(response.into())),
        Ok(BlobPageOutcome::Page(_))
    ));
    assert!(f.roots[2].get_blob(&f.reference.ciphertext_cid).is_none());
}

#[tokio::test]
async fn original_native_caller_and_deadline_retirement_win_over_changed_connection() {
    for fault in ["native", "caller", "deadline"] {
        let mut f = Fixture::new().await;
        let choice = f.choice();
        let prepared = f.prepare(&choice);
        let response = f.response(&prepared.frame);
        let effect = prepared.context.effect_scope();
        f.roots[1].transport = Arc::new(f.roots[1].transport.rejoin());
        match fault {
            "native" => {
                f.native.send_replace(true);
            }
            "caller" => {
                f.caller.send_replace(true);
            }
            "deadline" => f.clock.set_ms(prepared.deadline()),
            _ => unreachable!(),
        }
        assert_eq!(
            f.roots[2].blob_effect_status(&effect),
            BlobEffectStatus::Retired,
            "{fault}"
        );
        assert!(
            matches!(
                f.roots[2].authenticate_bridge_page(prepared.context, Ok(response.into())),
                Err(SyncError::Unauthorized)
            ),
            "{fault}"
        );
    }
}

#[tokio::test]
async fn unstarted_route_loss_preserves_proposal_and_fresh_d_precedes_waiting_for_b() {
    let mut f = Fixture::with_members(4).await;
    let b = f.roots[1].device.device_id();
    let d = f.roots[3].device.device_id();
    let d_peer = f.roots[3].local_peer();
    let choice = f.choice();
    assert_eq!(choice.0.helper.device, b);
    let prepared = f.prepare(&choice);
    let descriptor = f.roots[2].peer_records.remove(&b).unwrap();
    assert!(matches!(
        f.roots[2].refresh_file_bridge_choice(&mut f.catalog, &choice),
        Err(BridgeError::Unavailable)
    ));
    assert!(matches!(
        f.roots[2].prepare_file_bridge_page(&f.catalog, &choice, &f.reference, 0, MAX_PAGE_BYTES),
        Err(BridgeError::Unavailable)
    ));
    assert!(matches!(
        f.roots[2].commit_file_bridge_page(&mut f.catalog, prepared),
        Err(BridgeError::Unavailable)
    ));
    assert!(
        !f.catalog.committed,
        "local refusal cannot consume the source cursor"
    );
    // D enters within the existing four-helper bound; this does not claim fifth-helper rotation.
    crate::tests::prove_live_member(&mut f.roots[2], d_peer, d);
    let mut selected_d = false;
    for _ in 0..16 {
        match f.roots[2]
            .next_file_bridge_choice(&mut f.catalog, &[choice.variant()])
            .unwrap()
        {
            BridgeStep::Ready(next) => {
                assert_eq!(next.0.helper.device, d);
                selected_d = true;
                break;
            }
            BridgeStep::Yield | BridgeStep::Backoff => {}
            BridgeStep::Exhausted => panic!("missing B cannot hide usable D"),
        }
    }
    assert!(selected_d);
    f.roots[2].peer_records.insert(b, descriptor);
    assert!(
        f.roots[2]
            .refresh_file_bridge_choice(&mut f.catalog, &choice)
            .is_err(),
        "old B choice cannot revive after D selection"
    );
}

#[tokio::test]
async fn original_native_scope_and_both_exact_signatures_precede_ciphertext_admission() {
    let mut f = Fixture::new().await;
    let choice = f.choice();
    for fault in ["signature", "other-grant", "ok"] {
        let prepared = f.prepare(&choice);
        let scope = prepared.context.scope.clone();
        let mut response = f.response(&prepared.frame);
        if fault == "signature" {
            *response.last_mut().unwrap() ^= 1;
        }
        let context = if fault == "other-grant" {
            f.prepare(&choice).context
        } else {
            prepared.context
        };
        let result = f.roots[2].authenticate_bridge_page(context, Ok(Bytes::from(response)));
        if fault == "ok" {
            let BlobPageOutcome::Page(page) = result.unwrap() else {
                panic!("expected ciphertext page")
            };
            assert_eq!(page.bytes, f.ciphertext);
            assert_eq!(page.offset, 0);
            assert!(!page.more);
            assert!(scope.authorized());
            f.native.send_replace(true);
            assert!(
                !scope.authorized(),
                "returned authority must retire before later AEAD/storage effects"
            );
        } else {
            assert!(result.is_err(), "{fault}");
        }
        assert!(
            f.roots[2].get_blob(&f.reference.ciphertext_cid).is_none(),
            "authentication must never write cache"
        );
    }
}

#[tokio::test]
async fn local_capacity_refusal_preserves_proposal_and_deadline_is_frozen_before_debit() {
    let mut f = Fixture::new().await;
    let choice = f.choice();
    let pressure: Vec<_> = (0..128)
        .map(|_| f.roots[2].file_service.routes.source().unwrap())
        .collect();
    assert!(matches!(
        f.roots[2].prepare_file_bridge_page(&f.catalog, &choice, &f.reference, 0, MAX_PAGE_BYTES),
        Err(BridgeError::Busy)
    ));
    drop(pressure);
    let same = f.choice();
    assert!(choice.0.same_as(&same.0));
    let prepared = f.prepare(&same);
    assert_eq!(prepared.deadline(), 13_000);
    f.clock.advance_ms(12_001);
    assert!(f.roots[2]
        .commit_file_bridge_page(&mut f.catalog, prepared)
        .is_err());
    assert!(
        !f.catalog.committed,
        "an unstarted expired preparation cannot spend source coverage"
    );
}

#[tokio::test]
async fn dispatched_batch_retry_keeps_sources_with_fresh_grant_and_descriptor_change_retires_old_work(
) {
    let mut f = Fixture::new().await;
    let choice = f.choice();
    let first = f.prepare(&choice);
    let original = first.frame.clone();
    let request = f.roots[2]
        .commit_file_bridge_page(&mut f.catalog, first)
        .unwrap();
    f.clock.advance_ms(REQUEST_INTERVAL_MS);
    let retry = f.prepare(&choice);
    let old = Grant::parse(&original).unwrap();
    let new = Grant::parse(&retry.frame).unwrap();
    assert_eq!(old.sources(), new.sources());
    assert_eq!(old.window(), new.window());
    assert_ne!(
        old.nonce(),
        new.nonce(),
        "ambiguous retry is fresh paid work, not a replay"
    );
    let scope = retry.context.scope.clone();
    f.roots[2]
        .peer_records
        .get_mut(&choice.0.helper.device)
        .unwrap()
        .seq += 1;
    f.roots[2].reconcile_file_requesters();
    assert!(!scope.authorized());
    assert!(f.roots[2]
        .commit_file_bridge_page(&mut f.catalog, retry)
        .is_err());
    drop(request);
    f.caller.send_replace(true);
    assert!(f.roots[2]
        .next_file_bridge_choice(&mut f.catalog, &[[1; 32]])
        .is_err());
}

#[tokio::test]
async fn actual_requester_helper_and_source_admit_after_effective_wall_rollback() {
    let mut f = Fixture::new().await;
    let service_clock = Arc::new(ManualClock::new(10_000));
    f.enable_real_service(service_clock.clone());
    let choice = f.choice();
    service_clock.set_wall_ms(100_000);
    assert_eq!(
        f.roots[2].file_service.admission.request_time_ms().unwrap(),
        100_000
    );
    service_clock.set_wall_ms(1_000);
    service_clock.advance_ms(100);
    let BlobPageOutcome::Page(page) = f.wire_page(&choice, 0).await else {
        panic!("fresh C grant must survive wall rollback at real B/A admission");
    };
    assert_eq!(page.bytes, f.ciphertext);
    assert!(!page.more);
    assert!(
        f.roots[1].get_blob(&f.reference.ciphertext_cid).is_none(),
        "B only forwards encrypted pages"
    );
    assert!(
        f.roots[2].get_blob(&f.reference.ciphertext_cid).is_none(),
        "C auth has no storage effect"
    );
}

#[tokio::test]
async fn moving_on_after_admission_retires_old_choice_even_while_next_catalog_turn_yields() {
    let mut f = Fixture::new().await;
    let old = f.choice();
    let prepared = f.prepare(&old);
    drop(
        f.roots[2]
            .commit_file_bridge_page(&mut f.catalog, prepared)
            .unwrap(),
    );
    assert!(matches!(
        f.roots[2]
            .next_file_bridge_choice(&mut f.catalog, &[old.variant()],)
            .unwrap(),
        BridgeStep::Yield
    ));
    assert!(f.roots[2]
        .prepare_file_bridge_page(&f.catalog, &old, &f.reference, 0, MAX_PAGE_BYTES,)
        .is_err());
    assert!(f.roots[2]
        .refresh_file_bridge_choice(&mut f.catalog, &old)
        .is_err());
}

#[tokio::test]
async fn actual_temporarily_busy_sole_source_is_revisited_after_fresh_coverage() {
    let mut f = Fixture::new().await;
    f.enable_real_service(Arc::new(ManualClock::new(10_000)));
    let choice = f.choice();
    let native = f.roots[0].file_service.native.clone().unwrap();
    let held = native
        .reads
        .prepare(
            &f.roots[0].group.group_id(),
            &*f.roots[0].blobs,
            &f.reference.ciphertext_cid,
            f.ciphertext.len(),
            Arc::new(Live),
        )
        .unwrap();
    assert!(matches!(
        f.wire_page(&choice, 0).await,
        BlobPageOutcome::Unavailable
    ));
    f.roots[2].file_bridge_unavailable(&mut f.catalog, &choice);
    drop(held);
    let mut backed_off = false;
    for _ in 0..16 {
        match f.roots[2]
            .next_file_bridge_choice(&mut f.catalog, &[choice.variant()])
            .unwrap()
        {
            BridgeStep::Backoff => backed_off = true,
            BridgeStep::Yield => {}
            BridgeStep::Ready(retry) => {
                assert!(
                    backed_off,
                    "pressure retry must follow bounded sweep backoff"
                );
                assert_eq!(
                    retry.0.sources[0].device_id(),
                    choice.0.sources[0].device_id()
                );
                let BlobPageOutcome::Page(page) = f.wire_page(&retry, 0).await else {
                    panic!("healthy source must remain reachable after temporary pressure");
                };
                assert_eq!(page.bytes, f.ciphertext);
                return;
            }
            BridgeStep::Exhausted => panic!("temporary pressure is not permanent absence"),
        }
    }
    panic!("bounded retry sweep did not revisit the sole source");
}

#[tokio::test]
async fn actual_bridge_reaches_sole_holder_beyond_512_offline_members_without_per_batch_backoff() {
    let mut f = Fixture::late_source();
    let service_clock = Arc::new(ManualClock::new(10_000));
    f.enable_real_service(service_clock.clone());
    let variant = bridge_reference_variant(&f.reference).unwrap();
    let mut unavailable = 0;
    for _ in 0..800 {
        match f.roots[2]
            .next_file_bridge_choice(&mut f.catalog, &[variant])
            .unwrap()
        {
            BridgeStep::Yield => {}
            BridgeStep::Backoff => panic!("fresh late source must be reached before a retry sweep"),
            BridgeStep::Exhausted => panic!("connected sole holder omitted from native catalog"),
            BridgeStep::Ready(choice) => {
                service_clock.advance_ms(REQUEST_INTERVAL_MS);
                match f.wire_page(&choice, 0).await {
                    BlobPageOutcome::Unavailable => {
                        unavailable += 1;
                        f.roots[2].file_bridge_unavailable(&mut f.catalog, &choice);
                    }
                    BlobPageOutcome::Page(page) => {
                        assert_eq!(
                            unavailable, 128,
                            "all offline source batches were actually served by B"
                        );
                        assert_eq!(page.bytes, f.ciphertext);
                        assert!(
                            f.clock.monotonic_ms() < 60_000,
                            "late source is reachable within the existing quiet bound"
                        );
                        assert!(f.roots[1].get_blob(&f.reference.ciphertext_cid).is_none());
                        return;
                    }
                    _ => panic!("unexpected actual B/A outcome"),
                }
            }
        }
    }
    panic!("bounded catalog failed to reach late helper/source");
}

#[tokio::test]
async fn pacing_is_shared_by_prepared_requests_and_expiry_never_renews_prepared_authority() {
    let mut f = Fixture::new().await;
    let choice = f.choice();
    let first = f.prepare(&choice);
    let second = f.prepare(&choice);
    let pending = f.roots[2]
        .commit_file_bridge_page(&mut f.catalog, first)
        .unwrap();
    assert!(matches!(
        f.roots[2].commit_file_bridge_page(&mut f.catalog, second),
        Err(BridgeError::Busy)
    ));
    assert!(matches!(
        f.roots[2].prepare_file_bridge_page(&f.catalog, &choice, &f.reference, 0, MAX_PAGE_BYTES,),
        Err(BridgeError::Busy)
    ));
    f.clock.advance_ms(REQUEST_INTERVAL_MS);
    let prepared = f.prepare(&choice);
    let deadline = prepared.deadline();
    f.clock.set_ms(deadline);
    assert!(matches!(
        f.roots[2].commit_file_bridge_page(&mut f.catalog, prepared),
        Err(BridgeError::Stale)
    ));
    drop(pending);
}

#[tokio::test]
async fn full_pacing_table_refuses_before_coverage_spend_and_expired_rows_can_be_reused() {
    let mut f = Fixture::new().await;
    let choice = f.choice();
    f.roots[2].file_service.requester_ready = (0..REQUEST_PACING_ROWS)
        .map(|i| {
            (
                DeviceId::from_public_key_bytes(&[i as u8; 32]),
                1_000 + REQUEST_INTERVAL_MS,
            )
        })
        .collect();
    assert!(matches!(
        f.roots[2].prepare_file_bridge_page(&f.catalog, &choice, &f.reference, 0, MAX_PAGE_BYTES,),
        Err(BridgeError::Busy)
    ));
    assert!(!f.catalog.committed);
    let same = f.choice();
    assert!(same.0.same_as(&choice.0));
    f.clock.advance_ms(REQUEST_INTERVAL_MS);
    drop(f.prepare(&same));
    assert!(f.roots[2].file_service.requester_ready.is_empty());
}

#[tokio::test]
async fn foreign_or_retired_unsupported_observation_cannot_disable_another_current_read() {
    let mut f = Fixture::new().await;
    let foreign = f.choice();
    f.catalog = f.roots[2]
        .begin_file_bridge_catalog(
            RequestCancellation::new(f.caller.subscribe(), None),
            Arc::new(Live),
        )
        .unwrap();
    let current = f.choice();
    f.roots[2].file_bridge_unsupported(&mut f.catalog, &foreign);
    assert!(f.choice().0.same_as(&current.0));
    f.roots[2].file_bridge_unsupported(&mut f.catalog, &current);
    assert!(f.roots[2]
        .prepare_file_bridge_page(&f.catalog, &current, &f.reference, 0, MAX_PAGE_BYTES,)
        .is_err());
}

#[tokio::test]
async fn variant_replacement_before_bootstrap_yield_cannot_revive_an_unstarted_choice() {
    let mut f = Fixture::new().await;
    let old = f.choice();
    let prepared = f.prepare(&old);
    assert!(matches!(
        f.roots[2]
            .next_file_bridge_choice(&mut f.catalog, &[[9; 32]])
            .unwrap(),
        BridgeStep::Yield
    ));
    assert!(f.roots[2]
        .prepare_file_bridge_page(&f.catalog, &old, &f.reference, 0, MAX_PAGE_BYTES)
        .is_err());
    assert!(f.roots[2]
        .commit_file_bridge_page(&mut f.catalog, prepared)
        .is_err());
    let reintroduced = f.choice();
    assert!(!old.0.same_as(&reintroduced.0));
    assert_eq!(
        old.0.sources[0].device_id(),
        reintroduced.0.sources[0].device_id(),
        "retiring an unstarted variant cannot spend source coverage"
    );
}
