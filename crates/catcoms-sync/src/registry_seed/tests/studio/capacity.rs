use super::*;

fn occupy_previews(client: &mut Node) -> Vec<ProvisionalCheckpointCapacity> {
    let held = (0..3)
        .map(|_| client.reserve_provisional_checkpoint_capacity().unwrap())
        .collect();
    assert!(client.reserve_provisional_checkpoint_capacity().is_err());
    held
}

#[tokio::test]
async fn checkpoint_capacity_three_provisional_reservations_allow_both_authoritative_seed_classes()
{
    let (mut owner, mut client, clock) = pair().await;
    let held = occupy_previews(&mut client);
    // Drive the existing authenticated path for each class, without releasing a preview slot
    // or changing membership. This tests capacity, not actor scheduling or installation.
    for target in [target(true), CheckpointTarget::Registry(4)] {
        let (receipt, seed) = if let CheckpointTarget::Registry(bucket) = target {
            seed(&owner, bucket)
        } else {
            studio_seed(&owner, target)
        };
        let mut pass = selected(&mut owner, &mut client, target, &receipt).await;
        assert!(client.reserve_provisional_checkpoint_capacity().is_err());
        assert!(client
            .prepare_checkpoint_discovery(owner.local_peer(), CheckpointTarget::Registry(5))
            .is_err());
        let completed =
            seed_response(&mut owner, &mut client, &mut pass, seed.bytes().to_vec()).await;
        assert!(client
            .complete_checkpoint_seed(&mut pass, completed)
            .unwrap());
        client
            .with_checkpoint_seed_selection(&pass, |_, _, _, selection| {
                assert_eq!(selection.target, target);
                assert_eq!(selection.receipt, &receipt);
                assert_eq!(selection.checkpoint.unwrap().bytes(), seed.bytes());
            })
            .unwrap();
        assert!(client.docs.is_empty());
        drop(pass);
        clock.advance_ms(2000);
    }
    assert!(client.reserve_provisional_checkpoint_capacity().is_err());
    drop(held);
}

#[tokio::test]
async fn checkpoint_capacity_reserved_hint_releases_before_provisional_retry() {
    let (mut owner, mut client, _) = pair().await;
    let mut held = occupy_previews(&mut client);
    let target = target(true);
    let (receipt, _) = studio_seed(&owner, target);
    let watch = owner.watch_checkpoint_head(target).unwrap();
    let pending = client
        .prepare_checkpoint_discovery(owner.local_peer(), target)
        .unwrap();
    let (completed, ()) = tokio::join!(pending.fetch(), async {
        owner.run_once().await.unwrap();
        owner
            .serve_receipt_head(&watch, None, |_, _, _, _| {
                Ok::<_, ()>(receipt_head::ReceiptHeadSelection {
                    receipt: Some(receipt.clone()),
                    prove: false,
                })
            })
            .unwrap()
            .unwrap()
            .unwrap();
    });
    assert!(client
        .prepare_checkpoint_discovery(owner.local_peer(), target)
        .is_err());
    let Some(RegistrySeedDiscovery::Hint(hint)) =
        client.complete_checkpoint_discovery(completed).unwrap()
    else {
        panic!("expected authenticated hint without owner selection");
    };
    assert_eq!(hint.receipt.as_ref(), Some(&receipt));
    assert!(hint.proof.is_none());
    // Even retaining the returned raw hint cannot retain the authoritative reservation.
    let retry = client
        .prepare_checkpoint_discovery(owner.local_peer(), target)
        .unwrap();
    assert!(client.reserve_provisional_checkpoint_capacity().is_err());
    held.pop();
    let provisional = client.reserve_provisional_checkpoint_capacity().unwrap();
    assert!(client.reserve_provisional_checkpoint_capacity().is_err());
    assert!(client.docs.is_empty());
    drop((hint, retry, provisional, held));
}

#[tokio::test]
async fn checkpoint_capacity_reserved_cancelled_head_and_seed_wait_for_transport_release() {
    for fetching in [false, true] {
        let (mut owner, mut client, clock) = pair().await;
        let held = occupy_previews(&mut client);
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
        clock.advance_ms(FETCH_MS);
        assert!(client
            .prepare_checkpoint_discovery(owner.local_peer(), target)
            .is_err());
        assert!(client.reserve_provisional_checkpoint_capacity().is_err());
        // The unserved lower request still owns the fourth slot after caller drop and expiry.
        owner.run_once().await.unwrap();
        assert!(client
            .prepare_checkpoint_discovery(owner.local_peer(), target)
            .is_ok());
        assert!(client.reserve_provisional_checkpoint_capacity().is_err());
        drop(held);
    }
}

#[tokio::test]
async fn checkpoint_capacity_completed_seed_and_failed_preparation_preserve_accounting() {
    let (mut owner, mut client, clock) = pair().await;
    let held = occupy_previews(&mut client);
    let target = target(true);
    // A failed, unauthenticated preparation must release its provisional-independent slot.
    assert!(client
        .prepare_checkpoint_discovery(PeerId::from_u64(999), target)
        .is_err());
    let (receipt, seed) = studio_seed(&owner, target);
    let mut pass = selected(&mut owner, &mut client, target, &receipt).await;
    let completed = seed_response(&mut owner, &mut client, &mut pass, seed.bytes().to_vec()).await;
    drop(pass);
    clock.advance_ms(FETCH_MS);
    assert!(client
        .prepare_checkpoint_discovery(owner.local_peer(), target)
        .is_err());
    drop(completed);
    assert!(client
        .prepare_checkpoint_discovery(owner.local_peer(), target)
        .is_ok());
    assert!(client.reserve_provisional_checkpoint_capacity().is_err());
    drop(held);
}
