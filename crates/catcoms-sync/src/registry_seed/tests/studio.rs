use super::*;
use catcoms_replication::studio::{StudioEpoch, StudioTarget};
mod capacity;

fn target(art: bool) -> CheckpointTarget {
    CheckpointTarget::Studio(if art {
        StudioTarget::Flipnote {
            channel: [3; 16],
            object: [7; 16],
        }
    } else {
        StudioTarget::Index { channel: [3; 16] }
    })
}
fn studio_seed(owner: &Node, target: CheckpointTarget) -> (Receipt, CheckpointSeed) {
    let CheckpointTarget::Studio(studio) = target else {
        panic!("studio fixture");
    };
    let source = StudioEpoch::new(&owner.group, studio, owner.device.device_id()).unwrap();
    let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
    let receipt = Receipt::sign(
        target.document(&owner.group.group_id()).unwrap(),
        0,
        [7; 32],
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &owner.device,
    )
    .unwrap();
    (receipt, seed)
}
async fn head_response(
    owner: &mut Node,
    client: &mut Node,
    target: CheckpointTarget,
    receipt: &Receipt,
) -> CompletedCheckpointDiscovery {
    let watch = owner.watch_checkpoint_head(target).unwrap();
    let snapshot = owner
        .prepare_receipt_head_snapshot(|_, _| Ok::<_, ()>(()))
        .unwrap()
        .unwrap();
    let request = client
        .prepare_checkpoint_discovery(owner.local_peer(), target)
        .unwrap();
    let (completed, ()) = tokio::join!(request.fetch(), async {
        owner.run_once().await.unwrap();
        owner
            .serve_receipt_head(&watch, Some(&snapshot), |_, _, _, _| {
                Ok::<_, ()>(receipt_head::ReceiptHeadSelection {
                    receipt: Some(receipt.clone()),
                    prove: true,
                })
            })
            .unwrap()
            .unwrap()
            .unwrap();
    });
    completed
}
async fn selected(
    owner: &mut Node,
    client: &mut Node,
    target: CheckpointTarget,
    receipt: &Receipt,
) -> RegistrySeedFetch {
    let completed = head_response(owner, client, target, receipt).await;
    let Some(RegistrySeedDiscovery::Selected(pass)) =
        client.complete_checkpoint_discovery(completed).unwrap()
    else {
        panic!("selected");
    };
    pass
}
async fn seed_response(
    owner: &mut Node,
    client: &mut Node,
    pass: &mut RegistrySeedFetch,
    raw: Vec<u8>,
) -> CompletedCheckpointSeed {
    let watch = owner.watch_checkpoint_seed(pass.target()).unwrap();
    let pending = client
        .prepare_checkpoint_seed(pass, owner.local_peer())
        .unwrap()
        .unwrap();
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        owner.run_once().await.unwrap();
        owner
            .serve_registry_seed(&watch, |_, _, _, _| Ok::<_, ()>(Some(raw)))
            .unwrap()
            .unwrap()
            .unwrap();
    });
    completed
}

#[test]
fn studio_seed_wire_exact_scope_and_kind_rejects_aliases() {
    for art in [false, true] {
        let q = ScopedQuery {
            target: target(art),
            doc_id: 99,
            hash: [5; 32],
        };
        let bytes = encode_scoped_query(&q, &[8; 16]).unwrap();
        let mut golden = vec![1, 0, if art { 16 } else { 15 }, 0, 0, 0, 16];
        golden.extend([3; 16]);
        golden.extend([0, 0, 0, 16]);
        golden.extend(if art { [7; 16] } else { [0; 16] });
        golden.extend(99u128.to_be_bytes());
        golden.extend([0, 0, 0, 32]);
        golden.extend([5; 32]);
        assert_eq!(bytes, golden);
        assert_eq!(
            decode_scoped_query(KIND_STUDIO_SEED, &bytes, &[8; 16]).unwrap(),
            q
        );
        assert!(decode_scoped_query(KIND_REGISTRY_SEED, &bytes, &[8; 16]).is_err());
        for len in 0..bytes.len() {
            assert!(decode_scoped_query(KIND_STUDIO_SEED, &bytes[..len], &[8; 16]).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_scoped_query(KIND_STUDIO_SEED, &trailing, &[8; 16]).is_err());
        if !art {
            let mut alias = bytes.clone();
            alias[27] = 1;
            assert!(decode_scoped_query(KIND_STUDIO_SEED, &alias, &[8; 16]).is_err());
        }
    }
}

#[tokio::test]
async fn studio_seed_detached_discovery_and_fetch_bind_index_art_and_channel() {
    for art in [false, true] {
        let (mut owner, mut client, clock) = pair().await;
        let target = target(art);
        let (receipt, seed) = studio_seed(&owner, target);
        let mut pass = selected(&mut owner, &mut client, target, &receipt).await;
        assert_eq!(pass.target(), target);
        assert!(!pass.is_fetched());
        assert!(client
            .with_registry_seed_selection(&pass, |_, _, _, _| panic!("wrong typed consumer"))
            .is_err());
        let response =
            seed_response(&mut owner, &mut client, &mut pass, seed.bytes().to_vec()).await;
        assert!(client
            .complete_checkpoint_seed(&mut pass, response)
            .unwrap());
        client
            .with_checkpoint_seed_selection(&pass, |_, _, _, s| {
                assert_eq!(s.target, target);
                assert_eq!(s.receipt, &receipt);
                assert_eq!(s.checkpoint.unwrap().bytes(), seed.bytes());
            })
            .unwrap();
        if art {
            // Object logical ids do not contain their channel. An authentic receipt therefore
            // still needs the typed seed check; channel substitution cannot render its content.
            clock.advance_ms(2000);
            let other = CheckpointTarget::Studio(StudioTarget::Flipnote {
                channel: [4; 16],
                object: [7; 16],
            });
            let mut wrong = selected(&mut owner, &mut client, other, &receipt).await;
            let response =
                seed_response(&mut owner, &mut client, &mut wrong, seed.bytes().to_vec()).await;
            assert!(client
                .complete_checkpoint_seed(&mut wrong, response)
                .is_err());
            assert!(!wrong.is_fetched());
        }
    }
}

#[tokio::test]
async fn studio_seed_detached_head_and_fetch_reject_older_completion_after_new_preparation() {
    let (mut owner, mut client, clock) = pair().await;
    let target = target(true);
    let (receipt, seed) = studio_seed(&owner, target);
    let old = head_response(&mut owner, &mut client, target, &receipt).await;
    clock.advance_ms(2000);
    let new = head_response(&mut owner, &mut client, target, &receipt).await;
    assert!(client.complete_checkpoint_discovery(old).is_err());
    let Some(RegistrySeedDiscovery::Selected(mut pass)) =
        client.complete_checkpoint_discovery(new).unwrap()
    else {
        panic!("new selection");
    };
    let old = seed_response(&mut owner, &mut client, &mut pass, seed.bytes().to_vec()).await;
    clock.advance_ms(2000);
    let new = seed_response(&mut owner, &mut client, &mut pass, seed.bytes().to_vec()).await;
    assert!(client.complete_checkpoint_seed(&mut pass, old).is_err());
    assert!(!pass.is_fetched());
    assert!(client.complete_checkpoint_seed(&mut pass, new).unwrap());
}

#[tokio::test]
async fn studio_seed_detached_slots_share_registry_and_follow_unpolled_and_completed_jobs() {
    let (mut owner, mut client, clock) = pair().await;
    let target = target(true);
    let (receipt, seed) = studio_seed(&owner, target);
    let mut pass = selected(&mut owner, &mut client, target, &receipt).await;
    let completed = seed_response(&mut owner, &mut client, &mut pass, seed.bytes().to_vec()).await;
    drop(pass);
    let mut held = Vec::new();
    for bucket in 0..3 {
        held.push(
            client
                .prepare_checkpoint_discovery(
                    owner.local_peer(),
                    CheckpointTarget::Registry(bucket),
                )
                .unwrap(),
        );
    }
    assert!(client
        .prepare_checkpoint_discovery(owner.local_peer(), target)
        .is_err());
    // Expiry revokes authority, not memory ownership. Three unpolled discoveries and one
    // completed seed response occupy all four retained slots after the original pass dies.
    clock.advance_ms(FETCH_MS);
    assert!(client
        .prepare_checkpoint_discovery(owner.local_peer(), target)
        .is_err());
    drop(completed);
    assert!(client
        .prepare_checkpoint_discovery(owner.local_peer(), target)
        .is_ok());
    drop(held);
}

#[tokio::test]
async fn studio_seed_cancelled_discovery_and_fetch_keep_retained_capacity_until_driver_release() {
    for fetching in [false, true] {
        let (mut owner, mut client, _) = pair().await;
        let target = target(true);
        if fetching {
            let (receipt, _) = studio_seed(&owner, target);
            let mut pass = selected(&mut owner, &mut client, target, &receipt).await;
            let pending = client
                .prepare_checkpoint_seed(&mut pass, owner.local_peer())
                .unwrap()
                .unwrap();
            let mut future = Box::pin(pending.fetch());
            assert!(futures::poll!(future.as_mut()).is_pending());
            drop(future);
            drop(pass);
        } else {
            let pending = client
                .prepare_checkpoint_discovery(owner.local_peer(), target)
                .unwrap();
            let mut future = Box::pin(pending.fetch());
            assert!(futures::poll!(future.as_mut()).is_pending());
            drop(future);
        }
        let mut held = Vec::new();
        for bucket in 0..3 {
            held.push(
                client
                    .prepare_checkpoint_discovery(
                        owner.local_peer(),
                        CheckpointTarget::Registry(bucket),
                    )
                    .unwrap(),
            );
        }
        assert_eq!(
            client
                .registry_seeds
                .retained
                .iter()
                .filter(|slot| slot.strong_count() > 0)
                .count(),
            4,
            "retained capacity must survive caller drop independently of the head-outbound limit"
        );
        assert!(client
            .prepare_checkpoint_discovery(owner.local_peer(), target)
            .is_err());
        owner.run_once().await.unwrap();
        assert!(client
            .prepare_checkpoint_discovery(owner.local_peer(), target)
            .is_ok());
        drop(held);
    }
}
