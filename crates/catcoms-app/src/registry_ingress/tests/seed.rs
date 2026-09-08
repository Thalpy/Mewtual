//! Actual joined members use the vault provider and both kind-21/22 network routes. The
//! Fetch alone leaves the receiver untouched; explicit installation preserves provisional work.
use super::*;
use crate::registry_catchup::{RegistryReceiveState, ServerRegistryReceive};
use crate::registry_seed::{
    ServerRegistrySeedDiscovery, ServerRegistrySeedFetch, ServerRegistrySeedWatch,
};
use crate::store::RegistryAdoptionOutcome;
use automerge::{
    transaction::{CommitOptions, Transactable},
    ActorId, AutoCommit, ROOT,
};
use catcoms_replication::{CheckpointSeed, CloseRecord, EpochPhase, SealedOp, SignedOp};

struct SeedFixture {
    p: Pair,
    key: PointerKey,
    document: LogicalDocument,
    receipt: Receipt,
    seed: CheckpointSeed,
    pass: ServerRegistrySeedFetch,
    seed_watch: ServerRegistrySeedWatch,
    queued_epoch_zero: Option<(ServerRegistryWatch, ServerRegistryReceive)>,
}

async fn fetched_fixture(queue_epoch_zero: bool) -> SeedFixture {
    let mut p = Pair::new().await;
    let key = (0u32..)
        .map(|n| PointerKey::new(DocType::StudioObject, n.to_be_bytes().to_vec()).unwrap())
        .find(|k| k.bucket() != p.key.bucket())
        .unwrap();
    let bucket = key.bucket();
    let document = registry_document(&p.alice.group_id(), bucket).unwrap();
    let id = epoch_zero_id(DocType::DocRegistry, &document.logical_key);
    let (receipt, close, seed) = p.alice.sync.with_registry_context(|g, d, _, r| {
        let mut writer =
            AutoCommit::new().with_actor(ActorId::from(d.device_id().as_bytes().to_vec()));
        for n in 0..10u8 {
            let op = RegistryOp::Put {
                key: key.clone(),
                epoch: u64::from(n),
            }
            .domain_op(&g.group_id(), [n; 16])
            .unwrap();
            for (field, value) in [("bucket", u64::from(bucket)), ("epoch", 0), ("v", 1)] {
                writer.put(ROOT, field, value).unwrap();
            }
            writer.put(ROOT, "kind", "registry").unwrap();
            writer
                .put(ROOT, "key", hex::encode(&document.logical_key))
                .unwrap();
            writer
                .put(
                    ROOT,
                    format!("p/0010/{}", hex::encode(key.logical_key())),
                    u64::from(n),
                )
                .unwrap();
            writer
                .put(
                    ROOT,
                    format!("_p1/op/{}", hex::encode(op.id(&d.device_id()))),
                    1u64,
                )
                .unwrap();
            // Real signed bytes meet the production close threshold without 10,000 disk edits.
            writer.commit_with(CommitOptions::default().with_message("x".repeat(220_000)));
            let signed = SignedOp::sign_domain(
                d,
                DocType::DocRegistry,
                id,
                writer.get_last_local_change().unwrap().raw_bytes().to_vec(),
                &op,
            )
            .unwrap();
            let sealed = SealedOp::seal(&signed, g, d, r).unwrap();
            p.alice_store
                .ingest_registry_epoch(SERVER, g, bucket, d, &sealed, r, &mut p.alice_budget)
                .unwrap();
        }
        let close = CloseRecord::sign(
            &document,
            id,
            0,
            writer.get_heads().into_iter().map(|h| h.0).collect(),
            d,
        )
        .unwrap();
        let state = p
            .alice_store
            .load_registry_epoch(SERVER, g, bucket, d)
            .unwrap()
            .unwrap();
        let seed = state
            .projection()
            .unwrap()
            .checkpoint(close.hash())
            .unwrap();
        let receipt = Receipt::sign(
            document.clone(),
            0,
            close.hash(),
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap();
        p.alice_store
            .prepare_epoch_owner_receipt(SERVER, receipt.clone(), g, 0, r, &mut p.alice_budget)
            .unwrap();
        p.alice_store
            .seal_registry_epoch(
                SERVER,
                g,
                bucket,
                d,
                receipt.clone(),
                0,
                r,
                &mut p.alice_budget,
            )
            .unwrap();
        (receipt, close.encode(), seed)
    });
    let (proof, tick) = tokio::join!(
        p.bob
            .sync
            .request_catchup(p.alice.local_peer(), DocType::Wiki, 125),
        p.alice.sync_once()
    );
    proof.unwrap();
    tick.unwrap();
    let head_watch = p.alice.watch_registry_head(&p.alice_store, SERVER, bucket);
    let seed_watch = p.alice.watch_registry_seed(&p.alice_store, SERVER, bucket);
    let permit = p
        .alice
        .prepare_owner_head_snapshot(&p.alice_store, SERVER)
        .unwrap();
    let (answer, ()) = tokio::join!(
        p.bob
            .discover_registry_seed(&p.bob_store, SERVER, p.alice.local_peer(), bucket),
        async {
            p.alice.sync_once().await.unwrap();
            p.alice
                .serve_registry_head_step(
                    &mut p.alice_store,
                    &head_watch,
                    Some(&permit),
                    &mut p.alice_budget,
                )
                .unwrap()
                .unwrap();
        }
    );
    let mut pass = match answer.unwrap().unwrap() {
        ServerRegistrySeedDiscovery::Selected(pass) => pass,
        _ => panic!("owner proof"),
    };
    let (answer, ()) = tokio::join!(
        p.bob
            .fetch_registry_seed_step(&mut pass, p.alice.local_peer()),
        async {
            p.alice.sync_once().await.unwrap();
            p.alice
                .serve_registry_seed_step(&mut p.alice_store, &seed_watch, &mut p.alice_budget)
                .unwrap()
                .unwrap();
        }
    );
    assert!(
        !answer.unwrap(),
        "the receipted next seed is not installed yet"
    );
    // Retain an actual old-epoch page, not merely an idle receive handle. After installation
    // this must still target epoch zero; fetching it must not have created a local document.
    let queued_epoch_zero = if queue_epoch_zero {
        let watch = p
            .bob
            .watch_registry_epoch(&p.bob_store, SERVER, bucket)
            .unwrap();
        p.bob.flush_registry_subscriptions().await.unwrap();
        let owner_watch = p
            .alice
            .watch_registry_epoch(&p.alice_store, SERVER, bucket)
            .unwrap();
        p.alice.flush_registry_subscriptions().await.unwrap();
        let mut receive = p
            .bob
            .begin_registry_receive(
                &mut p.bob_store,
                &watch,
                p.alice.local_peer(),
                &mut p.bob_budget,
            )
            .unwrap();
        let mut provider = p
            .alice
            .begin_registry_page_provider(&p.alice_store, SERVER, bucket)
            .unwrap();
        let (result, ()) = tokio::join!(p.bob.fetch_registry_receive_step(&mut receive), async {
            p.alice.sync_once().await.unwrap();
            p.alice
                .serve_registry_request_step(&p.alice_store, &mut provider, &owner_watch)
                .unwrap()
                .unwrap();
        });
        assert_eq!(result.unwrap(), RegistryReceiveState::PageReady);
        assert!(receive.progress().received_operations > 0);
        assert_eq!(receive.progress().saved_pages, 0);
        Some((watch, receive))
    } else {
        None
    };
    p.alice.sync.with_registry_context(|g, d, clock, r| {
        p.alice_store
            .install_registry_checkpoint(
                SERVER,
                g,
                bucket,
                d,
                &receipt.encode(),
                &close,
                0,
                clock,
                r,
                &mut p.alice_budget,
                &mut p.alice_intents,
            )
            .unwrap();
    });
    p.clock.advance_ms(1000);
    let (answer, ()) = tokio::join!(
        p.bob
            .fetch_registry_seed_step(&mut pass, p.alice.local_peer()),
        async {
            p.alice.sync_once().await.unwrap();
            p.alice
                .serve_registry_seed_step(&mut p.alice_store, &seed_watch, &mut p.alice_budget)
                .unwrap()
                .unwrap();
        }
    );
    assert!(answer.unwrap());
    assert!(p.bob.registry_seed_ready(&p.bob_store, SERVER, &pass));
    assert!(!p.bob.registry_seed_ready(&p.alice_store, SERVER, &pass));
    assert!(!p.bob.registry_seed_ready(&p.bob_store, SERVER + 1, &pass));
    p.bob
        .sync
        .with_registry_seed(&pass.inner, |_, _, _, selected| {
            assert_eq!(selected.checkpoint.bytes(), seed.bytes())
        })
        .unwrap();
    p.bob.sync.with_registry_context(|g, d, _, _| {
        assert!(p
            .bob_store
            .load_registry_epoch(SERVER, g, bucket, d)
            .unwrap()
            .is_none());
        assert_eq!(
            p.bob_store
                .load_epoch_recovery(SERVER, &document)
                .unwrap()
                .retained()
                .count(),
            0
        );
    });
    p.alice.unwatch_registry_head(&head_watch).unwrap();
    SeedFixture {
        p,
        key,
        document,
        receipt,
        seed,
        pass,
        seed_watch,
        queued_epoch_zero,
    }
}

#[tokio::test]
async fn registry_seed_server_fetches_rotated_vault_checkpoint_without_installing_on_newcomer() {
    let SeedFixture {
        mut p, seed_watch, ..
    } = fetched_fixture(false).await;
    assert!(p
        .alice
        .serve_registry_seed_step(&mut p.bob_store, &seed_watch, &mut p.bob_budget)
        .is_err());
    p.alice.unwatch_registry_seed(&seed_watch).unwrap();
}

async fn rediscover(p: &mut Pair, bucket: u8) -> ServerRegistrySeedFetch {
    p.clock.advance_ms(1000);
    let watch = p.alice.watch_registry_head(&p.alice_store, SERVER, bucket);
    let permit = p
        .alice
        .prepare_owner_head_snapshot(&p.alice_store, SERVER)
        .unwrap();
    let (answer, ()) = tokio::join!(
        p.bob
            .discover_registry_seed(&p.bob_store, SERVER, p.alice.local_peer(), bucket),
        async {
            p.alice.sync_once().await.unwrap();
            p.alice
                .serve_registry_head_step(
                    &mut p.alice_store,
                    &watch,
                    Some(&permit),
                    &mut p.alice_budget,
                )
                .unwrap()
                .unwrap();
        }
    );
    p.alice.unwatch_registry_head(&watch).unwrap();
    match answer.unwrap().unwrap() {
        ServerRegistrySeedDiscovery::Selected(pass) => pass,
        _ => panic!("fresh owner proof required"),
    }
}
async fn refetch(
    p: &mut Pair,
    pass: &mut ServerRegistrySeedFetch,
    watch: &ServerRegistrySeedWatch,
) {
    let (result, ()) = tokio::join!(
        p.bob.fetch_registry_seed_step(pass, p.alice.local_peer()),
        async {
            p.alice.sync_once().await.unwrap();
            p.alice
                .serve_registry_seed_step(&mut p.alice_store, watch, &mut p.alice_budget)
                .unwrap()
                .unwrap();
        }
    );
    assert!(result.unwrap());
}

#[tokio::test]
async fn registry_seed_install_joined_peer_preserves_provisional_edit_then_catches_up() {
    let SeedFixture {
        mut p,
        key,
        document,
        receipt,
        seed,
        pass: old_pass,
        seed_watch,
        queued_epoch_zero,
    } = fetched_fixture(true).await;
    let bucket = key.bucket();
    let (_, mut intents) = inventory(&mut p.bob_store, &p.bob.group_id());
    let op = RegistryOp::Put {
        key: key.clone(),
        epoch: 42,
    }
    .domain_op(&p.bob.group_id(), [200; 16])
    .unwrap();
    let epoch0 = epoch_zero_id(DocType::DocRegistry, &document.logical_key);
    p.bob.sync.with_registry_context(|g, d, _, r| {
        p.bob_store
            .edit_registry_epoch(
                SERVER,
                g,
                bucket,
                epoch0,
                d,
                op.clone(),
                r,
                &mut p.bob_budget,
                &mut intents,
            )
            .unwrap();
    });
    let (old_watch, mut old_receive) = queued_epoch_zero.unwrap();
    let mut pass = rediscover(&mut p, bucket).await;
    assert!(p
        .bob
        .install_registry_seed_step(&mut p.bob_store, SERVER, &old_pass, &mut p.bob_budget)
        .is_err());
    let (outcome, state) = p
        .bob
        .install_registry_seed_step(&mut p.bob_store, SERVER, &pass, &mut p.bob_budget)
        .unwrap();
    assert_eq!(outcome, RegistryAdoptionOutcome::AwaitingSeed);
    assert_eq!(
        (state.epoch(), state.phase(), state.op_count()),
        (0, EpochPhase::Closing, 1)
    );
    refetch(&mut p, &mut pass, &seed_watch).await;
    let (outcome, state) = p
        .bob
        .install_registry_seed_step(&mut p.bob_store, SERVER, &pass, &mut p.bob_budget)
        .unwrap();
    assert_eq!(outcome, RegistryAdoptionOutcome::Installed);
    assert_eq!(state.doc_id(), seed.origin().doc_id());
    assert_eq!(state.projection().unwrap().pointers[&key], 9);
    let recovery = p.bob_store.load_epoch_recovery(SERVER, &document).unwrap();
    let typed = catcoms_replication::registry::RegistryRecovery::from_snapshot(
        recovery.retained().next().unwrap(),
        &document,
        bucket,
    )
    .unwrap();
    assert_eq!(typed.projection().pointers[&key], 42);
    assert_eq!(
        p.bob_store
            .load_epoch_intents(SERVER, &document)
            .unwrap()
            .pending()
            .count(),
        1
    );
    // A pass/watch captured for epoch zero cannot save into the installed epoch. Rewatching
    // is explicit; installation does not silently rebind queued work to a different document.
    assert!(p
        .bob
        .begin_registry_receive(
            &mut p.bob_store,
            &old_watch,
            p.alice.local_peer(),
            &mut p.bob_budget
        )
        .is_err());
    let progress = old_receive.progress();
    assert_eq!(old_receive.state(), RegistryReceiveState::PageReady);
    let error = p
        .bob
        .persist_registry_receive_step(&mut p.bob_store, &mut old_receive, &mut p.bob_budget)
        .unwrap_err();
    assert!(error.to_string().contains("epoch"), "{error}");
    assert_eq!(old_receive.progress().saved_pages, 0);
    assert_eq!(old_receive.progress().accepted, 0);
    assert_eq!(
        old_receive.progress().received_pages,
        progress.received_pages
    );
    assert_eq!(
        old_receive.progress().persist_attempts,
        progress.persist_attempts + 1
    );
    assert_eq!(old_receive.state(), RegistryReceiveState::Paused);
    old_receive.retry();
    assert_eq!(
        old_receive.state(),
        RegistryReceiveState::PageReady,
        "the unsaved page is retained, not consumed into its continuation"
    );
    p.bob.sync.with_registry_context(|g, d, _, _| {
        let saved = p
            .bob_store
            .load_registry_epoch(SERVER, g, bucket, d)
            .unwrap()
            .unwrap();
        assert_eq!(saved.doc_id(), seed.origin().doc_id());
        assert_eq!(saved.op_count(), 0);
        assert_eq!(saved.projection().unwrap().pointers[&key], 9);
    });
    let after = p.bob_store.load_epoch_recovery(SERVER, &document).unwrap();
    assert_eq!(after.retained().count(), 1);
    assert_eq!(
        after.retained().next().unwrap().id().unwrap(),
        recovery.retained().next().unwrap().id().unwrap()
    );
    let watch = p
        .bob
        .watch_registry_epoch(&p.bob_store, SERVER, bucket)
        .unwrap();
    p.bob.flush_registry_subscriptions().await.unwrap();
    let alice_watch = p
        .alice
        .watch_registry_epoch(&p.alice_store, SERVER, bucket)
        .unwrap();
    p.alice.flush_registry_subscriptions().await.unwrap();
    let next = RegistryOp::Put {
        key: key.clone(),
        epoch: 12,
    }
    .domain_op(&p.alice.group_id(), [201; 16])
    .unwrap();
    p.alice.sync.with_registry_context(|g, d, _, r| {
        p.alice_store
            .edit_registry_epoch(
                SERVER,
                g,
                bucket,
                seed.origin().doc_id(),
                d,
                next,
                r,
                &mut p.alice_budget,
                &mut p.alice_intents,
            )
            .unwrap();
    });
    let mut provider = p
        .alice
        .begin_registry_page_provider(&p.alice_store, SERVER, bucket)
        .unwrap();
    let mut receive = p
        .bob
        .begin_registry_receive(
            &mut p.bob_store,
            &watch,
            p.alice.local_peer(),
            &mut p.bob_budget,
        )
        .unwrap();
    let (result, ()) = tokio::join!(p.bob.fetch_registry_receive_step(&mut receive), async {
        p.alice.sync_once().await.unwrap();
        p.alice
            .serve_registry_request_step(&p.alice_store, &mut provider, &alice_watch)
            .unwrap()
            .unwrap();
    });
    assert_eq!(result.unwrap(), RegistryReceiveState::PageReady);
    assert_eq!(
        p.bob
            .persist_registry_receive_step(&mut p.bob_store, &mut receive, &mut p.bob_budget)
            .unwrap(),
        RegistryReceiveState::PrefixComplete
    );
    let (outcome, state) = p
        .bob
        .install_registry_seed_step(&mut p.bob_store, SERVER, &pass, &mut p.bob_budget)
        .unwrap();
    assert_eq!(outcome, RegistryAdoptionOutcome::AlreadyInstalled);
    assert_eq!(state.projection().unwrap().pointers[&key], 12);
    assert_eq!(state.op_count(), 1);
    assert_eq!(receipt.seed_change_hash, seed.change_hash());
}

#[tokio::test]
async fn registry_seed_install_refuses_expired_foreign_mount_server_and_replaced_runtime() {
    let SeedFixture {
        mut p,
        key,
        document,
        pass,
        seed_watch,
        ..
    } = fetched_fixture(false).await;
    assert!(p
        .bob
        .install_registry_seed_step(&mut p.alice_store, SERVER, &pass, &mut p.alice_budget)
        .is_err());
    assert!(p
        .bob
        .install_registry_seed_step(&mut p.bob_store, SERVER + 1, &pass, &mut p.bob_budget)
        .is_err());
    p.clock.advance_ms(60_001);
    assert!(p
        .bob
        .install_registry_seed_step(&mut p.bob_store, SERVER, &pass, &mut p.bob_budget)
        .is_err());
    let mut fresh = rediscover(&mut p, key.bucket()).await;
    refetch(&mut p, &mut fresh, &seed_watch).await;
    let snapshot = p.bob.snapshot().unwrap();
    p.bob = Server::restore(
        &snapshot,
        p.hub.join(PeerId::from_u64(99)),
        rng(),
        Box::new(p.clock.clone()),
        "bob",
    )
    .unwrap();
    assert!(p
        .bob
        .install_registry_seed_step(&mut p.bob_store, SERVER, &fresh, &mut p.bob_budget)
        .is_err());
    p.bob.sync.with_registry_context(|g, d, _, _| {
        assert!(p
            .bob_store
            .load_registry_epoch(SERVER, g, key.bucket(), d)
            .unwrap()
            .is_none());
    });
    assert_eq!(
        p.bob_store
            .load_epoch_recovery(SERVER, &document)
            .unwrap()
            .retained()
            .count(),
        0
    );
}
