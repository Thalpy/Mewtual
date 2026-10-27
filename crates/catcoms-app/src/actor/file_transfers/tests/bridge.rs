//! Actual C/B/A signed bridge pages at the actor's final AEAD/storage boundary. The test
//! selects the initial bridge candidate explicitly; production selection has separate TCP tests.
use super::*;

type Member = Server<MemNetwork, ChaCha20Rng>;

async fn served<F: Future>(request: F, server: &mut Member) -> F::Output {
    tokio::pin!(request);
    loop {
        tokio::select! {
            result = &mut request => return result,
            result = server.sync_once() => { result.unwrap(); },
        }
    }
}

async fn fixture() -> (Member, Member, Member, ManualClock, Cid, Vec<u8>) {
    let hub = Hub::new();
    let clock = ManualClock::new(1_000);
    let peers: Vec<_> = (70..73)
        .map(|seed| {
            let key = libp2p::identity::Keypair::ed25519_from_bytes([seed; 32]).unwrap();
            catcoms_net::phase0_peer_id(&key.public().to_peer_id())
        })
        .collect();
    let mut a = Server::found(
        hub.join(peers[0]),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(70),
        Box::new(clock.clone()),
        "A",
    )
    .unwrap();
    a.subscribe_control().await.unwrap();
    a.open_files().await.unwrap();
    let mut joined: Vec<Member> = Vec::new();
    for (index, peer) in peers.iter().enumerate().skip(1) {
        let invite = a.mint_invite([index as u8; 16], u64::MAX, vec![]).unwrap();
        let joining = Server::join(
            hub.join(*peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(index as u64 + 70),
            Box::new(clock.clone()),
            format!("member{index}"),
            peers[0],
            &invite,
        );
        tokio::pin!(joining);
        let mut member = loop {
            tokio::select! {
                result = &mut joining => break result.unwrap(),
                result = a.sync_once() => { result.unwrap(); },
                _ = async {
                    if let Some(previous) = joined.first_mut() { previous.sync_once().await.unwrap(); }
                    else { std::future::pending::<()>().await; }
                } => {},
            }
        };
        member.subscribe_control().await.unwrap();
        joined.push(member);
        eprintln!("bridge fixture: admitted member {index}");
    }
    let mut c = joined.pop().unwrap();
    let mut b = joined.pop().unwrap();
    while b.epoch() != a.epoch() {
        tokio::select! {
            result = b.sync_once() => { result.unwrap(); },
            result = a.sync_once() => { result.unwrap(); },
        }
    }
    eprintln!("bridge fixture: roster converged");
    for node in [&mut a, &mut b, &mut c] {
        node.sync.publish_self_record(Vec::new(), 1).unwrap();
    }
    let records: Vec<_> = [&a, &b, &c]
        .into_iter()
        .map(|node| node.sync.self_record().unwrap().clone())
        .collect();
    for node in [&mut a, &mut b, &mut c] {
        for record in &records {
            node.sync.ingest_peer_record(record.clone());
        }
    }
    assert!(served(b.sync.finalize_member_connection(peers[0]), &mut a)
        .await
        .unwrap());
    assert!(served(c.sync.finalize_member_connection(peers[1]), &mut b)
        .await
        .unwrap());
    eprintln!("bridge fixture: both bridge edges authenticated");
    let data: Vec<_> = (0..150_000).map(|i| (i % 229) as u8).collect();
    let cid = a
        .add_file("pages.bin", "application/octet-stream", "", &data)
        .await
        .unwrap();
    b.open_files().await.unwrap();
    c.open_files().await.unwrap();
    served(b.request_files_catchup(peers[0]), &mut a)
        .await
        .unwrap();
    served(c.request_files_catchup(peers[1]), &mut b)
        .await
        .unwrap();
    eprintln!("bridge fixture: file directory recovered");
    assert!(c.file_head(&cid).is_some());
    (a, b, c, clock, cid, data)
}

async fn next(
    transfers: &mut FileTransfers,
    a: &mut Member,
    b: &mut Member,
    clock: &ManualClock,
) -> TransferStep {
    bounded(async {
        // This integration boundary uses the real process service's admission clock. Progress
        // the actor's injected timer while its owned backoff waits, instead of leaving ManualClock
        // frozen and mistaking an intentional pacing delay for a stalled bridge.
        let service_clock = catcoms_rt::SystemClock;
        let mut tick = service_clock.sleep(Duration::from_millis(20));
        loop {
            tokio::select! {
                step = transfers.next() => return step.unwrap(),
                result = a.sync_once() => { result.unwrap(); },
                result = b.sync_once() => { result.unwrap(); },
                _ = tick.as_mut() => {
                    clock.advance_ms(20);
                    tick = service_clock.sleep(Duration::from_millis(20));
                }
            }
        }
    })
    .await
}

#[tokio::test]
async fn final_bridge_aead_route_loss_retains_prefix_without_storage_verdict_or_progress_refill() {
    for (bad_key, cancel) in [(false, false), (true, false), (false, true), (true, true)] {
        let (mut a, mut b, mut c, clock, cid, data) = bounded(fixture()).await;
        if bad_key {
            add_bad_wrapped_variant(&mut c, &cid).await;
        }
        let directory = tempfile::tempdir().unwrap();
        attach_kept(&mut c, directory.path());
        let mut transfers = isolated_transfers();
        let (session, native) = watch::channel(false);
        for node in [&mut a, &mut b, &mut c] {
            node.sync.set_native_file_service(Some(
                catcoms_sync::file_service::NativeFileService {
                    reads: crate::file_source::runtime(),
                    session: RequestCancellation::new(native.clone(), None),
                    actor: transfers.authority(),
                },
            ));
        }
        let (caller, receiver) = watch::channel(false);
        let (reply, mut result) = oneshot::channel();
        transfers.keep(
            &mut c,
            cid.as_bytes().to_vec(),
            Some(RequestCancellation::new(receiver, None)),
            reply,
        );
        let TransferStep::Local(mut job) = next(&mut transfers, &mut a, &mut b, &clock).await
        else {
            panic!("initial owned Keep step")
        };
        let reference = job.keep.as_ref().unwrap().manifest.chunks[0].clone();
        let variant = bridge_reference_variant(&reference).unwrap();
        let mut catalog = c
            .sync
            .begin_file_bridge_catalog(job.cancel.clone().unwrap(), job.root.clone())
            .unwrap();
        let choice = loop {
            match c
                .sync
                .next_file_bridge_choice(&mut catalog, &[variant])
                .unwrap()
            {
                BridgeStep::Ready(choice) => break choice,
                BridgeStep::Yield | BridgeStep::Backoff => {}
                BridgeStep::Exhausted => panic!("current B route is available"),
            }
        };
        assert_eq!(choice.helper_peer(), b.sync.local_peer());
        job.tried_providers.extend(c.sync.blob_fetch_peers());
        job.candidates = Some(VecDeque::from([ReadCandidate::Bridge(reference, choice)]));
        job.bridge = Some(catalog);
        let original = c.sync.kept_files().transfers[0].clone();
        transfers.advance(&mut c, *job);
        // Retain at least two genuine signed pages before the final page is opened by AEAD.
        let mut final_step = loop {
            let step = next(&mut transfers, &mut a, &mut b, &clock).await;
            if matches!(&step, TransferStep::Network(attempt) if attempt.job.pages >= 2
                && attempt.job.staged.len() + catcoms_sync::MIN_BLOB_PAGE >= kept_ciphertext_bound(&attempt.reference).unwrap())
            {
                break step;
            }
            clock.advance_ms(200);
            transfers.complete(&mut c, step);
        };
        let TransferStep::Network(attempt) = &mut final_step else {
            unreachable!()
        };
        assert!(attempt.result.is_ok());
        let prefix = attempt.job.staged.clone();
        let pages = attempt.job.pages;
        let deadline = attempt.job.deadline;
        attempt.job.bridge_retries = 2;
        let route = c
            .sync
            .transport()
            .connection_lease(b.sync.local_peer())
            .unwrap();
        let caller_at_effect = caller.clone();
        transfers.before_storage_admission = Some(Box::new(move || {
            route.revoke();
            if cancel {
                caller_at_effect.send_replace(true);
            }
        }));
        clock.advance_ms(200);
        transfers.complete(&mut c, final_step);
        assert!(
            transfers.before_storage_admission.is_none(),
            "actual final AEAD boundary reached"
        );
        assert!(c.sync.blob_cids().is_empty());
        let retained = c.sync.kept_files().transfers[0].clone();
        assert_eq!(retained.plan, original.plan);
        assert_eq!(retained.intent, original.intent);
        assert_eq!(retained.grant, original.grant);
        assert!(!retained.blocked && retained.verified_chunks == 0);
        if cancel {
            assert!(bounded(result)
                .await
                .unwrap()
                .unwrap_err()
                .contains("cancelled"));
        } else {
            let TransferStep::Local(job) = next(&mut transfers, &mut a, &mut b, &clock).await
            else {
                panic!("retry yields original owned state")
            };
            assert_eq!(
                job.staged, prefix,
                "retry keeps the exact previously accepted offset"
            );
            assert_eq!(job.pages, pages);
            assert_eq!(
                job.deadline, deadline,
                "discarded final bytes cannot refresh the quiet bound"
            );
            assert_eq!(job.bridge_retries, 2);
            assert!(
                job.rejected_variants.is_empty(),
                "stale bad AEAD cannot reject a current variant"
            );
            transfers.complete(&mut c, TransferStep::Local(job));
            // The original good reference completes through a fresh captured B route. The bad
            // reference may now receive a CURRENT bad-key verdict and fall back normally.
            bounded(async {
                loop {
                    tokio::select! {
                        // Accept sends the terminal reply after removing the final task. Both
                        // branches can therefore be ready: observe that reply before polling an
                        // empty JoinSet. Closed/error replies still fail the unwraps below.
                        biased;
                        value = &mut result => { value.unwrap().unwrap(); break; },
                        step = next(&mut transfers, &mut a, &mut b, &clock) => {
                            clock.advance_ms(200);
                            transfers.complete(&mut c, step);
                        },
                    }
                }
            })
            .await;
            assert!(c.sync.kept_files().files[0].checked);
            assert_eq!(c.fetch_file_chunk(&cid, 0).await.unwrap().0, data);
        }
        drop(session);
    }
}
