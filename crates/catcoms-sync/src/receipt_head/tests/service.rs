use super::*;

#[tokio::test]
async fn epoch_service_unopened_head_is_prepaid_without_registering_a_watch() {
    let (mut n, _) = node();
    assert!(
        enqueue(&mut n, 4).recv().await.is_none(),
        "disabled by default"
    );
    n.enable_epoch_service();
    let rx = enqueue(&mut n, 4);
    let interest = n.reserve_epoch_service_interest().unwrap();
    assert!(n.receipt_heads.watches.is_empty());
    assert!(
        n.reserve_epoch_service_interest().is_none(),
        "one right per request"
    );
    // No second service charge is allowed after preparation. Even exhausted debt cannot
    // strand this exact warmed request behind a stream of fresh cold interests.
    assert!(n.receipt_heads.service.as_mut().unwrap().charge(1000, 2, 4));
    assert!(n.receipt_heads.service.as_mut().unwrap().charge(1000, 2, 4));
    assert!(n.receipt_heads.service.as_mut().unwrap().charge(1000, 2, 4));
    assert!(!n.receipt_heads.service.as_mut().unwrap().charge(1000, 2, 4));
    let snapshot = n
        .prepare_receipt_head_snapshot(|_, _| Ok::<_, ()>(()))
        .unwrap()
        .unwrap();
    let receipt = receipt(&n, 4);
    let served = n
        .serve_epoch_head_interest(&interest, Some(&snapshot), |_, _, _, _| {
            selection(receipt.clone(), true)
        })
        .unwrap()
        .unwrap()
        .unwrap();
    let ReceiptHeadServed::Owner(handoff) = served else {
        panic!("proof handoff")
    };
    assert_eq!(
        n.with_receipt_head_handoff(handoff, |r, _| r.hash())
            .unwrap(),
        receipt.hash()
    );
    assert!(rx.recv().await.is_some());
    assert!(!n.epoch_service_interest_is_current(&interest));
}

#[tokio::test]
async fn epoch_service_expiry_replacement_and_reenable_do_not_reuse_preparation_or_debt() {
    let (mut n, clock) = node();
    n.enable_epoch_service();
    let old_rx = enqueue(&mut n, 4);
    let old = n.reserve_epoch_service_interest().unwrap();
    clock.advance_ms(5000);
    let rx = enqueue(&mut n, 4);
    assert!(old_rx.recv().await.is_none());
    assert!(!n.epoch_service_interest_is_current(&old));
    assert!(n
        .serve_epoch_head_interest(&old, None, |_, _, _, _| -> Result<_, ()> {
            panic!("old token read replacement source")
        })
        .is_err());
    let new = n.reserve_epoch_service_interest().unwrap();
    n.disable_epoch_service();
    assert!(rx.recv().await.is_none());
    n.enable_epoch_service();
    assert!(!n.epoch_service_interest_is_current(&new));
    let _rx = enqueue(&mut n, 4);
    let _right = n.reserve_epoch_service_interest().unwrap();
    n.disable_epoch_service();
    n.enable_epoch_service();
    assert!(
        enqueue(&mut n, 4).recv().await.is_none(),
        "enable cannot forgive requester debt"
    );
}

#[tokio::test]
async fn epoch_service_membership_or_watch_replacement_revokes_exact_interest() {
    for mode in ["watch", "membership", "runtime"] {
        let (mut n, clock) = node();
        n.enable_epoch_service();
        if mode == "watch" {
            n.watch_registry_head(4);
        }
        let _rx = enqueue(&mut n, 4);
        let interest = n.reserve_epoch_service_interest().unwrap();
        match mode {
            "watch" => {
                n.watch_registry_head(4);
            }
            "membership" => {
                let peer = MlsDevice::generate().unwrap();
                n.with_observed_mls_transition(|n| {
                    n.group.add_member(&n.device, peer.key_package().unwrap())
                })
                .unwrap();
            }
            _ => {
                n = Node::restore(
                    &n.snapshot().unwrap(),
                    Hub::new().join(PeerId::from_u64(2)),
                    ChaCha20Rng::seed_from_u64(8),
                    Box::new(clock),
                )
                .unwrap();
                n.enable_epoch_service();
            }
        }
        assert!(!n.epoch_service_interest_is_current(&interest), "{mode}");
        assert!(n
            .serve_epoch_head_interest(&interest, None, |_, _, _, _| -> Result<_, ()> {
                panic!("revoked source read")
            })
            .is_err());
    }
}
