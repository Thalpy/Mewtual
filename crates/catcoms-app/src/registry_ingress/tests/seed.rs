//! Actual joined members use the vault provider and both kind-21/22 network routes. The
//! receiver stays untouched: obtaining bytes is not permission to discard provisional edits.
use super::*;
use crate::registry_seed::ServerRegistrySeedDiscovery;
use automerge::{
    transaction::{CommitOptions, Transactable},
    ActorId, AutoCommit, ROOT,
};
use catcoms_replication::{CloseRecord, SealedOp, SignedOp};

#[tokio::test]
async fn registry_seed_server_fetches_rotated_vault_checkpoint_without_installing_on_newcomer() {
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
    assert!(p
        .alice
        .serve_registry_seed_step(&mut p.bob_store, &seed_watch, &mut p.bob_budget)
        .is_err());
    p.alice.unwatch_registry_seed(&seed_watch).unwrap();
}
