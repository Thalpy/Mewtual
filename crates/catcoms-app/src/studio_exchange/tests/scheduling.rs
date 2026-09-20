//! Real actor scheduling under retained preview pressure and a transport-only owner return.
use super::*;
use crate::studio::{StudioPreviewDelivery, StudioRead, StudioVaultLease};
use crate::studio_exchange::provisional::ServerPreparedProvisionalStudioSeed;
use catcoms_replication::{
    registry_epoch::catchup::{RegistryOpPage, RegistryPageOutcome},
    studio::StudioEpoch,
    CheckpointSeed, EpochPhase, InheritedCheckpoint, Receipt, SealedOp,
};
use catcoms_rt::Clock;
use catcoms_sync::{
    checkpoint_exchange::CheckpointTarget, epoch_service::EpochServiceKind,
    receipt_head::ReceiptHeadSelection,
};
use std::{sync::Weak, time::Duration};
use tokio::sync::Mutex;

struct Running {
    actor: crate::ServerActor,
    store: Arc<Mutex<Option<ServerStore>>>,
    task: tokio::task::JoinHandle<()>,
    drain: tokio::task::JoinHandle<()>,
}
impl Running {
    fn new(node: Node, store: ServerStore) -> Self {
        let (actor, mut events, task) = crate::spawn(node);
        let drain = tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                assert!(!matches!(event.event, crate::AppEvent::StudioReceivePaused));
            }
        });
        Self {
            actor,
            store: Arc::new(Mutex::new(Some(store))),
            task,
            drain,
        }
    }
    async fn request(
        &self,
        request: Option<StudioRequest>,
        cancellation: Option<RequestCancellation>,
    ) -> Option<StudioRead> {
        let ready = match request {
            Some(request) => self.actor.studio_begin(request).await.unwrap(),
            None => self.actor.studio_receive_begin().await.unwrap(),
        };
        let mut lease =
            StudioVaultLease::new(self.store.clone().try_lock_owned().unwrap(), SERVER, ());
        if let Some(cancellation) = cancellation {
            lease = lease.with_cancellation(cancellation);
        }
        ready.execute_read(lease).await.unwrap()
    }
    async fn stop(self) -> ServerStore {
        self.actor.shutdown().await;
        self.task.await.unwrap();
        self.drain.await.unwrap();
        let store = self.store.lock().await.take().unwrap();
        store
    }
}

struct Hint {
    target: StudioTarget,
    receipt: Receipt,
    seed: CheckpointSeed,
    tail: SealedOp,
    projection: StudioProjection,
}
fn hint(node: &mut Node, target: StudioTarget) -> Hint {
    node.sync.with_registry_context(|g, d, _, rng| {
        let source = StudioEpoch::new(g, target, d.device_id()).unwrap();
        let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
        let logical = target.document(&g.group_id()).unwrap();
        let receipt = Receipt::sign(
            logical.clone(),
            0,
            [7; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap();
        let mut source = StudioEpoch::from_checkpoint(
            g,
            target,
            d.device_id(),
            receipt.clone(),
            0,
            seed.bytes(),
        )
        .unwrap();
        let body = match target {
            StudioTarget::Index { .. } => IndexOp::PutObject {
                object: [7; 16],
                kind: StudioKind::Flipnote,
                title: "unconfirmed peer history".into(),
                created_by: d.device_id(),
                ts: 1,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap(),
            _ => FlipnoteOp::SetHeader(FlipnoteHeader::Title("unconfirmed peer history".into()))
                .encode()
                .unwrap(),
        };
        let tail = source
            .edit_or_reseal(
                d,
                g,
                rng,
                &DomainOp {
                    doc_type: logical.doc_type,
                    logical_key: logical.logical_key,
                    body,
                    nonce: [9; 16],
                },
                1,
            )
            .unwrap();
        Hint {
            target,
            receipt,
            seed,
            tail,
            projection: source.projection().unwrap(),
        }
    })
}

async fn turn(
    owner: &Running,
    client: &Running,
    clock: &ManualClock,
    cancellation: Option<RequestCancellation>,
    wait_parse: bool,
) {
    tokio::join!(
        owner.request(None, None),
        client.request(None, cancellation)
    );
    owner.actor.wait_studio_preparation().await;
    if wait_parse {
        client.actor.wait_studio_preparation().await;
    }
    // Let detached network completions reach the actor before advancing simulated deadlines.
    tokio::time::sleep(Duration::from_millis(5)).await;
    clock.advance_ms(250);
}

async fn retain_delivery(
    client: &Running,
    target: StudioTarget,
    expected: &StudioProjection,
) -> (
    StudioPreviewDelivery,
    Weak<ServerPreparedProvisionalStudioSeed>,
) {
    let (cancel, signal) = tokio::sync::watch::channel(false);
    let read = client
        .request(
            Some(StudioRequest::Read { target }),
            Some(RequestCancellation::new(signal, None)),
        )
        .await;
    let Some(StudioRead::AwaitingTenureReceipt(preview)) = read else {
        panic!("real actor preview required")
    };
    preview
        .inspect(|_, projection| assert_eq!(projection, expected))
        .unwrap();
    let seed = Arc::downgrade(&preview.seed);
    let delivery = preview.delivery();
    drop(preview);
    // Model a cancelled native conversion that still owns its data, without a five-second
    // clock jump or clearing the actor's ready cache. Custody lasts until delivery is dropped.
    cancel.send_replace(true);
    client.actor.studio_scheduling_for_test(None).await;
    assert!(!delivery.is_current());
    (delivery, seed)
}

#[tokio::test]
async fn studio_actor_owner_return_installs_both_classes_with_three_retained_previews() {
    tokio::time::timeout(Duration::from_secs(90), owner_return(Pressure::Ready))
        .await
        .unwrap();
}

#[tokio::test]
async fn studio_actor_owner_return_installs_both_classes_with_cancelled_preview_parser() {
    tokio::time::timeout(Duration::from_secs(90), owner_return(Pressure::Parser))
        .await
        .unwrap();
}

#[tokio::test]
async fn studio_actor_owner_return_installs_both_classes_with_cancelled_preview_transport() {
    tokio::time::timeout(Duration::from_secs(90), owner_return(Pressure::Transport))
        .await
        .unwrap();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pressure {
    Ready,
    Parser,
    Transport,
}

async fn owner_return(pressure: Pressure) {
    let ready_count = if pressure == Pressure::Ready { 3 } else { 2 };
    let mut p = Pair::new().await;
    let targets = [
        StudioTarget::Index { channel: channel() },
        target(),
        StudioTarget::Flipnote {
            channel: channel(),
            object: [8; 16],
        },
        StudioTarget::Flipnote {
            channel: channel(),
            object: [9; 16],
        },
    ];
    let owner_peer = p.alice.local_peer();
    let provider_peer = p.bob.local_peer();
    p.bob.subscribe_control().await.unwrap();
    let wire = Net::new(p.hub.join(PeerId::from_u64(3)));
    let invite = p.alice.mint_invite([44; 16], u64::MAX, vec![]).unwrap();
    let (joined, tick) = tokio::join!(
        Node::join(
            wire.clone(),
            MlsDevice::generate().unwrap(),
            rng(),
            Box::new(p.clock.clone()),
            "capacity-bound reader",
            owner_peer,
            &invite
        ),
        p.alice.sync_once()
    );
    tick.unwrap();
    let mut client = joined.unwrap();
    while p.bob.epoch() != p.alice.epoch() {
        p.bob.sync_once().await.unwrap();
    }
    p.alice.open_channel_index().await.unwrap();
    p.bob.open_channel_index().await.unwrap();
    for peer in [&mut p.alice, &mut p.bob] {
        tokio::select! {
            result = client.request_channel_index_catchup(peer.local_peer()) => { result.unwrap(); },
            _ = async { loop { peer.sync_once().await.unwrap(); } } => unreachable!(),
        }
    }
    assert_eq!(client.sync.studio_page_peers().len(), 2);
    let group_epoch = client.epoch();
    let members = client.members();
    let owner_id = p.alice.device_id();
    assert!(p.alice.is_owner());
    assert!(!p.bob.is_owner() && !client.is_owner());
    let (_, _, studio_id) = super::discovery::prepared_checkpoint(&mut p, targets[0]);
    let bucket = super::unopened::prepared_registry(&mut p, targets[0], false);
    let (studio_expected, registry_id, registry_expected) =
        p.alice.sync.with_registry_context(|g, d, _, _| {
            let studio = p
                .a_store
                .load_studio_epoch(SERVER, g, targets[0], d)
                .unwrap()
                .unwrap();
            let registry = p
                .a_store
                .load_registry_epoch(SERVER, g, bucket, d)
                .unwrap()
                .unwrap();
            (
                studio.projection().unwrap(),
                registry.doc_id(),
                registry.projection().unwrap(),
            )
        });
    let hints: Vec<_> = targets
        .into_iter()
        .map(|target| hint(&mut p.alice, target))
        .collect();
    let expected: Vec<_> = hints.iter().map(|h| h.projection.clone()).collect();
    let mut provider = p.bob;
    provider.sync.enable_epoch_service();
    let served_hints = Arc::new(AtomicUsize::new(0));
    let served = served_hints.clone();
    let provider_task = tokio::spawn(async move {
        loop {
            provider.sync_once().await.unwrap();
            while let Some(interest) = provider.sync.reserve_epoch_service_interest() {
                let selected = hints
                    .iter()
                    .find(|h| interest.target() == CheckpointTarget::Studio(h.target));
                match interest.kind() {
                    EpochServiceKind::Head => {
                        provider
                            .sync
                            .serve_epoch_head_interest(&interest, None, |_, _, _, _| {
                                Ok::<_, ()>(ReceiptHeadSelection {
                                    receipt: selected.map(|h| h.receipt.clone()),
                                    prove: false,
                                })
                            })
                            .unwrap()
                            .unwrap()
                            .unwrap();
                        if selected.is_some_and(|h| h.target == targets[3]) {
                            served.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    EpochServiceKind::Seed => {
                        provider
                            .sync
                            .serve_epoch_seed_interest(&interest, |_, _, _, _| {
                                Ok::<_, ()>(selected.map(|h| h.seed.bytes().to_vec()))
                            })
                            .unwrap()
                            .unwrap()
                            .unwrap();
                    }
                    EpochServiceKind::Page => {
                        provider
                            .sync
                            .serve_epoch_page_interest(&interest, |_, _, _, _| {
                                Ok::<_, ()>(match selected {
                                    Some(h) if interest.doc_id() == Some(h.tail.doc_id) => {
                                        RegistryPageOutcome::Page(RegistryOpPage {
                                            operations: vec![h.tail.clone()],
                                            next: None,
                                        })
                                    }
                                    _ => RegistryPageOutcome::CheckpointRequired,
                                })
                            })
                            .unwrap()
                            .unwrap()
                            .unwrap();
                    }
                }
            }
        }
    });
    let root = tempfile::tempdir().unwrap();
    let client_store = open(root.path());
    let snapshot = client.snapshot().unwrap();
    let mut verifier = Node::restore(
        &snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(99))),
        rng(),
        Box::new(p.clock.clone()),
        "read-only vault verifier",
    )
    .unwrap();
    *wire.hidden_peer.lock().unwrap() = Some(owner_peer);
    assert_eq!(client.sync.studio_page_peers(), vec![provider_peer]);
    let owner = Running::new(p.alice, p.a_store);
    let client = Running::new(client, client_store);
    eprintln!("{pressure:?}: joined, authenticated, owner hidden");
    let mut deliveries = Vec::new();
    let mut seeds = Vec::new();
    for i in 0..ready_count {
        client
            .request(Some(StudioRequest::Read { target: targets[i] }), None)
            .await;
        let mut ready = false;
        for _ in 0..160 {
            turn(&owner, &client, &p.clock, None, true).await;
            let observation = client.actor.studio_scheduling_for_test(None).await;
            if observation.0.contains(&targets[i]) {
                ready = true;
                break;
            }
        }
        assert!(
            ready,
            "preview {i} never became ready; hint: {:?}",
            client.actor.observed_studio_hint_for_test()
        );
        eprintln!(
            "{pressure:?}: preview {i} ready at {}",
            p.clock.monotonic_ms()
        );
        let (delivery, seed) = retain_delivery(&client, targets[i], &expected[i]).await;
        deliveries.push(delivery);
        seeds.push(seed);
    }
    let mut release_parser = None;
    if pressure != Pressure::Ready {
        let (entered, mut entry) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        if pressure == Pressure::Parser {
            client
                .actor
                .studio_scheduling_for_test(Some((entered, released)))
                .await;
            release_parser = Some(release);
        } else {
            wire.hold_seed.store(true, Ordering::SeqCst);
        }
        let (cancel, signal) = tokio::sync::watch::channel(false);
        let cancellation = RequestCancellation::new(signal, None);
        client
            .request(
                Some(StudioRequest::Read { target: targets[2] }),
                Some(cancellation.clone()),
            )
            .await;
        let mut paused = false;
        for _ in 0..160 {
            turn(&owner, &client, &p.clock, Some(cancellation.clone()), false).await;
            if entry.try_recv().is_ok() || wire.held_seed.lock().unwrap().is_some() {
                paused = true;
                break;
            }
        }
        assert!(
            paused,
            "the actual actor {pressure:?} never reached its barrier"
        );
        assert_eq!(client.actor.studio_scheduling_for_test(None).await.1, 0);
        cancel.send_replace(true);
        client.actor.wait_studio_preparation().await;
        // Process cancellation through the actor without releasing lower transport custody.
        for _ in 0..4 {
            turn(&owner, &client, &p.clock, None, true).await;
        }
        if pressure == Pressure::Transport {
            let held = wire.held_seed.lock().unwrap();
            let (bytes, cancellation) = held.as_ref().expect("transport still owns the request");
            assert_eq!(bytes.first(), Some(&25));
            assert!(
                cancellation.is_cancelled(),
                "the original request waiter must be cancelled"
            );
            assert!(cancellation.keepalive().is_some());
        }
    }
    let before = client.actor.studio_scheduling_for_test(None).await;
    assert_eq!(before.0.len(), ready_count);
    assert_eq!(
        before.1, 0,
        "three real preview custodians must occupy eligible capacity"
    );
    let original_expiry = || {
        seeds.iter().any(|seed| {
            seed.upgrade()
                .is_some_and(|seed| seed.unconfirmed_is_unexpired())
        })
    };
    assert!(original_expiry());
    // A fourth watched target supplies authoritative Hint pressure. It cannot reserve a new
    // provisional slot while the native deliveries / cancelled parser still retain the three.
    client
        .request(Some(StudioRequest::Read { target: targets[3] }), None)
        .await;
    let mut fourth_hint = false;
    for _ in 0..80 {
        turn(&owner, &client, &p.clock, None, true).await;
        assert_eq!(client.actor.studio_scheduling_for_test(None).await.1, 0);
        if client
            .actor
            .observed_studio_hint_for_test()
            .is_some_and(|h| h.target == CheckpointTarget::Studio(targets[3]))
        {
            fourth_hint = true;
            break;
        }
    }
    assert!(fourth_hint && served_hints.load(Ordering::SeqCst) > 0);
    assert!(!client
        .actor
        .studio_scheduling_for_test(None)
        .await
        .0
        .contains(&targets[3]));
    assert!(
        original_expiry(),
        "fixture must not expire all original previews before owner return"
    );
    {
        let held = client.store.lock().await;
        verifier.sync.with_registry_context(|g, d, _, _| {
            let store = held.as_ref().unwrap();
            assert!(
                store
                    .load_studio_epoch(SERVER, g, targets[0], d)
                    .unwrap()
                    .is_none(),
                "Hints cannot install Studio history"
            );
            assert!(
                store
                    .load_registry_epoch(SERVER, g, bucket, d)
                    .unwrap()
                    .is_none(),
                "Hints cannot install Registry history"
            );
        });
    }
    // Only transport reachability changes. No MLS edit, rejoin, restart, unwatch or
    // user Read occurs after this edge; both installations must follow ordinary idle turns.
    eprintln!(
        "{pressure:?}: fourth Hint observed; owner returning at {}",
        p.clock.monotonic_ms()
    );
    *wire.hidden_peer.lock().unwrap() = None;
    let start = p.clock.monotonic_ms();
    let (mut studio_installed, mut registry_installed) = (false, false);
    // Passes actually consumed. The budget is exactly 160 turns of 250 ms, so this and the
    // injected elapsed time are the same quantity seen two ways; both are reported because the
    // failure hypothesis is about *turns that accomplished nothing*, not about wall time.
    let mut passes_used = 0;
    for pass in 0..160 {
        passes_used = pass + 1;
        turn(&owner, &client, &p.clock, None, true).await;
        assert_eq!(
            client.actor.studio_scheduling_for_test(None).await.1,
            0,
            "custody refunded at pass {pass}"
        );
        let held = client.store.lock().await;
        (studio_installed, registry_installed) =
            verifier.sync.with_registry_context(|g, d, _, _| {
                let store = held.as_ref().unwrap();
                let studio = store.load_studio_epoch(SERVER, g, targets[0], d).unwrap();
                let registry = store.load_registry_epoch(SERVER, g, bucket, d).unwrap();
                (
                    studio.is_some_and(|s| {
                        s.doc_id() == studio_id
                            && s.phase() == EpochPhase::Open
                            && s.projection().unwrap() == studio_expected
                    }),
                    registry.is_some_and(|r| {
                        r.doc_id() == registry_id
                            && r.epoch() == 1
                            && r.projection().unwrap() == registry_expected
                    }),
                )
            });
        if studio_installed && registry_installed {
            break;
        }
    }
    // One assertion reporting both classes, because two sequential ones stop at the first: the
    // Studio failure hid Registry's state in every trace collected so far.
    //
    // The pass count is the discriminator. The loop's budget is 160 turns and a healthy run uses
    // 132 (Parser, Transport) or 91 (Ready), so there are 28 spare passes in the tight variants.
    // `turn` sleeps 5 ms of real time and then advances 250 ms of injected time, and 5 ms is not
    // a completion barrier: under load a pass can return with its awaited work not yet landed,
    // accomplishing nothing while still spending 250 ms of budget. If a failing run reports 160
    // passes consumed, that is the mechanism, and no semaphore instrumentation is needed. If it
    // reports far fewer, the loop broke early and the cause is elsewhere.
    let injected = p.clock.monotonic_ms() - start;
    assert!(
        studio_installed && registry_installed,
        "{pressure:?}: install incomplete — studio={studio_installed} registry={registry_installed},          passes={passes_used}/160, injected={injected}/40000 ms since owner return"
    );
    assert!(injected <= 40_000);
    eprintln!(
        "{pressure:?}: both classes installed after {} ms",
        p.clock.monotonic_ms() - start
    );
    assert!(
        original_expiry(),
        "installation cannot depend on expiry of every original preview"
    );
    let after = client.actor.snapshot().await.unwrap();
    let mut check = Node::restore(
        &after,
        Net::new(Hub::new().join(PeerId::from_u64(98))),
        rng(),
        Box::new(p.clock.clone()),
        "membership verifier",
    )
    .unwrap();
    assert_eq!(check.epoch(), group_epoch);
    assert_eq!(check.members(), members);
    check
        .sync
        .with_registry_context(|g, _, _, _| assert_eq!(g.designated_committer(), Some(owner_id)));
    drop(release_parser);
    wire.held_seed.lock().unwrap().take();
    drop(deliveries);
    let store = client.stop().await;
    drop(store);
    let reopened = open(root.path());
    verifier.sync.with_registry_context(|g, d, _, _| {
        assert_eq!(
            reopened
                .load_studio_epoch(SERVER, g, targets[0], d)
                .unwrap()
                .unwrap()
                .doc_id(),
            studio_id
        );
        assert_eq!(
            reopened
                .load_registry_epoch(SERVER, g, bucket, d)
                .unwrap()
                .unwrap()
                .doc_id(),
            registry_id
        );
    });
    drop(owner.stop().await);
    provider_task.abort();
    assert!(provider_task.await.unwrap_err().is_cancelled());
}
