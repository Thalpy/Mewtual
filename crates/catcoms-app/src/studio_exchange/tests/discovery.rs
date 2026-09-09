//! Joined-member tests of the actual head/seed wire and recovery-first vault consumers. Owner
//! receipts here are explicitly prepared fixtures; automatic close/receipt issuance is Gate 4.
use super::*;
use crate::store::epoch_budget::{EpochStorageBudget, StorageScope};
use crate::store::StudioAdoptionOutcome;
use crate::studio_exchange::discovery::{
    ServerCheckpointDiscovery, ServerCheckpointFetch, ServerStudioCheckpointWatch,
};
use catcoms_replication::{CheckpointSeed, EpochPhase, InheritedCheckpoint, Receipt};
use catcoms_sync::checkpoint_exchange::CheckpointTarget;

fn prepared_checkpoint(
    p: &mut Pair,
    selected_target: StudioTarget,
) -> (Receipt, CheckpointSeed, u128) {
    let logical = selected_target.document(&p.alice.group_id()).unwrap();
    p.alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Apply {
                target: selected_target,
                epoch_id: epoch_zero_id(logical.doc_type, &logical.logical_key),
                nonce: [1; 16],
                body: title(1, "checkpoint title").body,
            },
        )
        .unwrap();
    let (receipt, seed) = p.alice.sync.with_registry_context(|g, d, _, _| {
        let source = p
            .a_store
            .load_studio_epoch(SERVER, g, selected_target, d)
            .unwrap()
            .unwrap();
        let mut projection = source.projection().unwrap();
        let StudioProjection::Flipnote(ref mut art) = projection else {
            panic!("art");
        };
        art.epoch = 0;
        let seed = projection.checkpoint([6; 32]).unwrap();
        let receipt = Receipt::sign(
            selected_target.document(&g.group_id()).unwrap(),
            0,
            [6; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            d,
        )
        .unwrap();
        (receipt, seed)
    });
    let mut b = budget(&mut p.alice, &mut p.a_store);
    let id = p.alice.sync.with_registry_context(|g, d, _, r| {
        let (outcome, state) = p
            .a_store
            .adopt_studio_checkpoint(
                SERVER,
                g,
                selected_target,
                d,
                &receipt,
                Some(seed.bytes()),
                0,
                &p.clock,
                r,
                &mut b,
            )
            .unwrap();
        assert_eq!(outcome, StudioAdoptionOutcome::Installed);
        let id = state.doc_id();
        p.a_store.retain_studio_source(g, d, state);
        id
    });
    let mut scan = p.a_store.scan_epoch_storage_with_studio().unwrap();
    while !scan.step().unwrap().complete {}
    let inventory = scan.finish().unwrap();
    p.alice.sync.with_registry_context(|g, _, _, r| {
        let mut b = EpochStorageBudget::from_inventory(
            StorageScope::new(SERVER, &g.group_id()).unwrap(),
            inventory.records_for_server(SERVER, &g.group_id()).unwrap(),
        )
        .unwrap();
        p.a_store
            .prepare_epoch_owner_receipt(SERVER, receipt.clone(), g, 0, r, &mut b)
            .unwrap();
    });
    (receipt, seed, id)
}

async fn discover(
    p: &mut Pair,
    watch: &ServerStudioCheckpointWatch,
    selected_target: StudioTarget,
) -> ServerCheckpointFetch {
    let snapshot = p
        .alice
        .prepare_owner_head_snapshot(&p.a_store, SERVER)
        .unwrap();
    let mut b = budget(&mut p.alice, &mut p.a_store);
    let attempt = p
        .bob
        .prepare_checkpoint_discovery(
            &p.b_store,
            SERVER,
            p.alice.local_peer(),
            CheckpointTarget::Studio(selected_target),
        )
        .unwrap();
    let (completed, ()) = tokio::join!(attempt.fetch(), async {
        while p
            .alice
            .serve_studio_head_step(&mut p.a_store, watch, Some(&snapshot), &mut b)
            .unwrap()
            .is_none()
        {
            p.alice.sync_once().await.unwrap();
        }
    });
    let Some(ServerCheckpointDiscovery::Selected(pass)) = p
        .bob
        .complete_checkpoint_discovery(&p.b_store, SERVER, completed)
        .unwrap()
    else {
        panic!("current owner selection");
    };
    pass
}

async fn fetch(
    p: &mut Pair,
    watch: &ServerStudioCheckpointWatch,
    pass: &mut ServerCheckpointFetch,
) {
    let mut b = budget(&mut p.alice, &mut p.a_store);
    let pending = p
        .bob
        .prepare_checkpoint_seed_fetch(pass, p.alice.local_peer())
        .unwrap()
        .unwrap();
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        while p
            .alice
            .serve_studio_seed_step(&mut p.a_store, watch, &mut b)
            .unwrap()
            .is_none()
        {
            p.alice.sync_once().await.unwrap();
        }
    });
    assert!(p
        .bob
        .complete_checkpoint_seed_fetch(pass, completed)
        .unwrap());
}

#[tokio::test]
async fn studio_discovery_actual_join_bootstrap_checkpoint_recovery_tail_and_restart() {
    let mut p = Pair::new().await;
    let (receipt, _, id) = prepared_checkpoint(&mut p, target());
    let scope = CheckpointTarget::Studio(target());
    // Inviter/Welcome is only a candidate; never disclose the logical Studio key on that basis.
    assert!(p
        .bob
        .prepare_checkpoint_discovery(&p.b_store, SERVER, p.alice.local_peer(), scope)
        .is_err());
    assert!(p.bob.sync.observed_owner_tenure_start().is_none());
    // This is the same authenticated directory catch-up used by the desktop join command,
    // not promote_member_peer_bound or a fixture-only proof API.
    let (bootstrap, tick) = tokio::join!(
        p.bob.request_channel_index_catchup(p.alice.local_peer()),
        p.alice.sync_once()
    );
    bootstrap.unwrap();
    tick.unwrap();
    let local = title(2, "my excluded provisional title");
    p.bob
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Apply {
                target: target(),
                epoch_id: epoch_zero_id(receipt.document.doc_type, &receipt.document.logical_key),
                nonce: local.nonce,
                body: local.body,
            },
        )
        .unwrap();
    let tail = title(3, "open epoch tail");
    p.alice
        .studio_transaction(
            &mut p.a_store,
            SERVER,
            StudioRequest::Apply {
                target: target(),
                epoch_id: id,
                nonce: tail.nonce,
                body: tail.body,
            },
        )
        .unwrap();
    let watch = p
        .alice
        .watch_studio_checkpoint(&p.a_store, SERVER, target())
        .unwrap();
    let mut pass = discover(&mut p, &watch, target()).await;
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let (outcome, sealed) = p
        .bob
        .install_studio_seed_step(&mut p.b_store, SERVER, &pass, &mut b)
        .unwrap();
    assert_eq!(outcome, StudioAdoptionOutcome::AwaitingSeed);
    assert_eq!(sealed.phase(), EpochPhase::Closing);
    assert_eq!(sealed.op_count(), 1);
    fetch(&mut p, &watch, &mut pass).await;
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let (outcome, saved) = p
        .bob
        .install_studio_seed_step(&mut p.b_store, SERVER, &pass, &mut b)
        .unwrap();
    assert_eq!(outcome, StudioAdoptionOutcome::Installed);
    assert_eq!(
        (saved.epoch(), saved.op_count(), saved.doc_id()),
        (1, 0, id)
    );
    p.bob
        .sync
        .with_registry_context(|g, d, _, _| p.b_store.retain_studio_source(g, d, saved));
    let recovery = p
        .b_store
        .load_epoch_recovery(SERVER, &receipt.document)
        .unwrap();
    assert_eq!(recovery.retained().count(), 1);
    assert_eq!(
        p.b_store
            .load_epoch_intents(SERVER, &receipt.document)
            .unwrap()
            .pending()
            .len(),
        1
    );
    let old = &p.watch;
    p.bob.unwatch_studio_epoch(old).unwrap();
    p.watch = p
        .bob
        .watch_studio_epoch(&p.b_store, SERVER, target())
        .unwrap();
    let provider_watch = p
        .alice
        .watch_studio_epoch(&p.a_store, SERVER, target())
        .unwrap();
    let mut provider = p.alice.studio_page_provider(&p.a_store, SERVER);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let mut pages = p
        .bob
        .begin_studio_receive(&mut p.b_store, &p.watch, p.alice.local_peer(), &mut b)
        .unwrap();
    let attempt = p
        .bob
        .prepare_studio_receive_step(&mut pages)
        .unwrap()
        .unwrap();
    let (completed, ()) = tokio::join!(attempt.fetch(), async {
        p.alice.sync_once().await.unwrap();
        p.alice
            .serve_studio_request_step(&mut p.a_store, &mut provider, &provider_watch)
            .unwrap()
            .unwrap();
    });
    p.bob
        .complete_studio_receive_step(&mut pages, completed)
        .unwrap();
    let mut b = budget(&mut p.bob, &mut p.b_store);
    p.bob
        .persist_studio_receive_step(&mut p.b_store, &mut pages, &mut b)
        .unwrap();
    assert_eq!(pages.progress().accepted, 1);
    let before = p.state().unwrap().projection().unwrap();
    drop(p.b_store);
    p.b_store = open(p.b_root.path());
    assert_eq!(p.state().unwrap().projection().unwrap(), before);
    assert_eq!(p.state().unwrap().epoch(), 1);
    assert_eq!(
        p.b_store
            .load_epoch_recovery(SERVER, &receipt.document)
            .unwrap()
            .retained()
            .count(),
        1
    );
}

#[tokio::test]
async fn studio_discovery_install_rechecks_mount_server_and_channel_after_head_and_seed() {
    use automerge::{transaction::Transactable, ROOT};
    for after_seed in [false, true] {
        for change in ["mount", "server", "channel"] {
            let mut p = pages::proven_pair().await;
            p.alice.open_channel_index().await.unwrap();
            p.bob.open_channel_index().await.unwrap();
            let channel = p.alice.create_channel("checkpoint-test").await.unwrap().id;
            p.bob.create_channel("checkpoint-test").await.unwrap();
            let selected_target = StudioTarget::Flipnote {
                channel: channel.to_be_bytes(),
                object: [7; 16],
            };
            let (receipt, _, _) = prepared_checkpoint(&mut p, selected_target);
            let watch = p
                .alice
                .watch_studio_checkpoint(&p.a_store, SERVER, selected_target)
                .unwrap();
            let mut pass = discover(&mut p, &watch, selected_target).await;
            if after_seed {
                fetch(&mut p, &watch, &mut pass).await;
            }
            match change {
                "mount" => {
                    drop(p.b_store);
                    p.b_store = open(p.b_root.path());
                }
                "channel" => {
                    p.bob
                        .sync
                        .post(
                            catcoms_wire::DocType::ChannelIndex,
                            crate::CHANNEL_INDEX_DOC,
                            |d| d.delete(ROOT, format!("{channel:032x}")),
                        )
                        .await
                        .unwrap();
                }
                _ => {}
            }
            let server = if change == "server" {
                SERVER + 1
            } else {
                SERVER
            };
            let mut b = budget(&mut p.bob, &mut p.b_store);
            assert!(p
                .bob
                .install_studio_seed_step(&mut p.b_store, server, &pass, &mut b)
                .is_err());
            assert!(p
                .bob
                .sync
                .with_registry_context(|g, d, _, _| {
                    p.b_store.load_studio_epoch(SERVER, g, selected_target, d)
                })
                .unwrap()
                .is_none());
            assert_eq!(
                p.b_store
                    .load_epoch_recovery(SERVER, &receipt.document)
                    .unwrap()
                    .retained()
                    .count(),
                0
            );
        }
    }
}

#[tokio::test]
async fn studio_discovery_post_handoff_failure_keeps_exact_receipt_for_republication() {
    for after in [false, true] {
        let mut p = pages::proven_pair().await;
        let (receipt, _, _) = prepared_checkpoint(&mut p, target());
        let watch = p
            .alice
            .watch_studio_checkpoint(&p.a_store, SERVER, target())
            .unwrap();
        let snapshot = p
            .alice
            .prepare_owner_head_snapshot(&p.a_store, SERVER)
            .unwrap();
        let mut b = budget(&mut p.alice, &mut p.a_store);
        let attempt = p
            .bob
            .prepare_checkpoint_discovery(
                &p.b_store,
                SERVER,
                p.alice.local_peer(),
                CheckpointTarget::Studio(target()),
            )
            .unwrap();
        let (completed, ()) = tokio::join!(attempt.fetch(), async {
            p.alice.sync_once().await.unwrap();
            assert!(p
                .alice
                .serve_studio_head_with_completion(
                    &mut p.a_store,
                    &watch,
                    Some(&snapshot),
                    &mut b,
                    |store, server, receipt, rng, budget| store
                        .complete_studio_head_with_test_failure(
                            server, receipt, rng, budget, after
                        )
                )
                .is_err());
        });
        // An error after local reply handoff cannot unsend the authenticated owner proof.
        assert!(matches!(
            p.bob
                .complete_checkpoint_discovery(&p.b_store, SERVER, completed)
                .unwrap(),
            Some(ServerCheckpointDiscovery::Selected(_))
        ));
        assert!(b.requires_reconciliation());
        let journal = p
            .a_store
            .load_epoch_owner_receipts(SERVER, &receipt.document)
            .unwrap();
        assert_eq!(
            journal.pending().or_else(|| journal.published()),
            Some(&receipt)
        );
        p.clock.advance_ms(2000);
        let pass = discover(&mut p, &watch, target()).await;
        p.bob
            .sync
            .with_checkpoint_seed_selection(&pass.inner, |_, _, _, s| {
                assert_eq!(s.receipt, &receipt)
            })
            .unwrap();
        assert!(p
            .a_store
            .load_epoch_owner_receipts(SERVER, &receipt.document)
            .unwrap()
            .pending()
            .is_none());
    }
}
