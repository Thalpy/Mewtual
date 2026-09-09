use super::*;
use crate::studio::StudioReceiver;

#[tokio::test]
async fn studio_receiver_reuses_large_active_source_across_successive_remote_edits() {
    let mut p = Pair::new().await;
    let history = p.alice.sync.with_registry_context(|group, device, _, _| {
        crate::store::save_studio_source_fixture(&mut p.a_store, SERVER, group, device, target())
    });
    // Establish realistic held history through authenticated network packets and the existing
    // durable adapter. The measured path below starts with an ordinary explicit Read.
    for sealed in history {
        p.alice
            .sync
            .publish_local_studio_once(target(), sealed.doc_id, sealed)
            .await
            .unwrap();
        p.bob.sync_once().await.unwrap();
        assert_eq!(p.receive().unwrap().unwrap().admission, Admission::Accepted);
    }
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    // A populated Index must be loaded independently, as the UI refreshes both views on every
    // StudioUpdated. Neither an Index save nor its later list reads may evict the open art.
    let index = StudioTarget::Index { channel: channel() };
    let logical = index.document(&p.bob.group_id()).unwrap();
    let creator = p.bob.sync.with_registry_context(|_, d, _, _| d.device_id());
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Apply {
                target: index,
                epoch_id: epoch_zero_id(logical.doc_type, &logical.logical_key),
                nonce: [17; 16],
                body: IndexOp::PutObject {
                    object: [7; 16],
                    kind: StudioKind::Flipnote,
                    title: "large opened art".into(),
                    created_by: creator,
                    ts: 1000,
                    expiry: StudioExpiry::Never,
                }
                .encode()
                .unwrap(),
            }),
        )
        .unwrap();
    p.bob.flush_studio_subscriptions().await.unwrap();
    for n in 4..=5 {
        let (view, _) = receiver
            .run(
                &mut p.bob,
                &mut p.b_store,
                SERVER,
                Some(StudioRequest::Read { target: index }),
            )
            .unwrap();
        let StudioProjection::Index(view) = view.view.unwrap().projection else {
            panic!("expected independently verified Index view");
        };
        assert_eq!(view.objects.len(), 1);
        let op = title(n, &format!("remote after large history {n}"));
        p.save(&op);
        p.send(op).await.unwrap();
        p.bob.sync_once().await.unwrap();
        let restores = crate::store::studio_full_restores_for_test();
        assert_eq!(
            receiver
                .run(&mut p.bob, &mut p.b_store, SERVER, None)
                .unwrap()
                .1,
            Some(target())
        );
        assert_eq!(
            crate::store::studio_full_restores_for_test(),
            restores,
            "a warm remote edit cannot restore the full history"
        );
        receiver
            .run(
                &mut p.bob,
                &mut p.b_store,
                SERVER,
                Some(StudioRequest::Read { target: target() }),
            )
            .unwrap();
        assert_eq!(
            crate::store::studio_full_restores_for_test(),
            restores,
            "the timeline refresh must reuse the same authenticated source too"
        );
        assert_eq!(p.state().unwrap().op_count(), n as usize);
    }
}

#[tokio::test]
async fn studio_receiver_warm_large_unrelated_history_accepts_small_target() {
    let mut p = Pair::new().await;
    let mut receiver = StudioReceiver::default();
    let read = || Some(StudioRequest::Read { target: target() });
    receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, read())
        .unwrap();
    p.bob.flush_studio_subscriptions().await.unwrap();
    let op = title(1, "small active flipnote");
    let epoch_id = p.save(&op);
    p.send(op).await.unwrap();
    p.bob.sync_once().await.unwrap();
    let path = crate::store::save_inventory_fixture(&mut p.b_store);
    let before = std::fs::read(&path).unwrap();
    assert!(receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .err()
        .unwrap()
        .to_string()
        .contains("cold byte limit"));
    assert!(receiver.take_pause_notice());
    assert!(
        p.b_store.load_server(SERVER).is_err(),
        "cold refusal precedes snapshot write"
    );

    // Read resumes but does not inventory the whole vault. A normal explicit Save warms the
    // exact unrelated record through its existing complete scan, preserving the held packet.
    receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, read())
        .unwrap();
    let local = title(2, "local title");
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Apply {
                target: target(),
                epoch_id,
                nonce: local.nonce,
                body: local.body,
            }),
        )
        .unwrap();
    assert!(receiver.pending(&p.bob));
    assert_eq!(
        receiver
            .run(&mut p.bob, &mut p.b_store, SERVER, None)
            .unwrap()
            .1,
        Some(target())
    );
    assert!(p.state().is_some());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "unrelated history remains untouched"
    );
}

#[tokio::test]
async fn studio_receiver_read_preserves_queue_accepts_once_and_reopens() {
    let mut p = Pair::new().await;
    let mut receiver = StudioReceiver::default();
    let read = || Some(StudioRequest::Read { target: target() });
    receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, read())
        .unwrap();
    p.bob.flush_studio_subscriptions().await.unwrap();
    let op = title(1, "remote title");
    p.save(&op);
    p.send(op.clone()).await.unwrap();
    p.bob.sync_once().await.unwrap();
    assert!(receiver.pending(&p.bob));
    receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, read())
        .unwrap();
    assert!(
        receiver.pending(&p.bob),
        "reopening the same watch must not discard traffic"
    );
    let (_, updated) = receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .unwrap();
    assert_eq!(updated, Some(target()));
    assert!(!receiver.pending(&p.bob));
    assert!(p.state().is_some());
    let snapshot = p.b_store.load_server(SERVER).unwrap();
    let mut reopened = Server::restore(
        &snapshot,
        Net::new(Hub::new().join(PeerId::from_u64(3))),
        rng(),
        Box::new(ManualClock::new(1000)),
        "bob",
    )
    .unwrap();
    assert!(reopened
        .studio_transaction(
            &mut p.b_store,
            SERVER,
            StudioRequest::Read { target: target() }
        )
        .unwrap()
        .is_some());
    p.send(op).await.unwrap();
    p.bob.sync_once().await.unwrap();
    let (_, updated) = receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .unwrap();
    assert_eq!(
        updated, None,
        "duplicates cannot repaint as new remote edits"
    );
}

#[tokio::test]
async fn studio_receiver_large_unrelated_source_pauses_until_explicit_access() {
    let mut p = Pair::new().await;
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    p.bob.flush_studio_subscriptions().await.unwrap();
    let op = title(1, "held edit");
    p.save(&op);
    p.send(op).await.unwrap();
    p.bob.sync_once().await.unwrap();
    let oversized = p
        .b_root
        .path()
        .join("servers")
        .join(format!("{}.registry-epoch", "00".repeat(32)));
    std::fs::write(&oversized, vec![0; 256 * 1024 + 1]).unwrap();
    assert!(receiver
        .run(&mut p.bob, &mut p.b_store, SERVER, None)
        .err()
        .unwrap()
        .to_string()
        .contains("byte limit"));
    assert!(!receiver.pending(&p.bob));
    assert!(receiver.take_pause_notice());
    assert!(!receiver.take_pause_notice());
    assert!(
        p.b_store.load_server(SERVER).is_err(),
        "scan refused before snapshot I/O"
    );
    std::fs::remove_file(oversized).unwrap();
    for _ in 0..3 {
        assert_eq!(
            receiver
                .run(&mut p.bob, &mut p.b_store, SERVER, None)
                .unwrap()
                .1,
            None
        );
        assert!(p.b_store.load_server(SERVER).is_err());
    }
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    assert!(
        receiver.pending(&p.bob),
        "explicit access retries, preserving queued packet"
    );
    assert_eq!(
        receiver
            .run(&mut p.bob, &mut p.b_store, SERVER, None)
            .unwrap()
            .1,
        Some(target())
    );
}

#[tokio::test]
async fn studio_receiver_replaced_mount_revokes_before_snapshot_or_scan() {
    let mut p = Pair::new().await;
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut p.bob,
            &mut p.b_store,
            SERVER,
            Some(StudioRequest::Read { target: target() }),
        )
        .unwrap();
    p.bob.flush_studio_subscriptions().await.unwrap();
    let op = title(1, "old mount");
    p.save(&op);
    p.send(op).await.unwrap();
    p.bob.sync_once().await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut replacement = open(root.path());
    assert!(receiver
        .run(&mut p.bob, &mut replacement, SERVER, None)
        .is_err());
    assert!(!receiver.pending(&p.bob));
    assert!(replacement.load_server(SERVER).is_err());
    assert!(!receiver.take_pause_notice());
}

#[tokio::test]
async fn studio_receiver_recent_watch_rail_evicts_oldest_without_creating_sources() {
    let mut p = Pair::new().await;
    let mut receiver = StudioReceiver::default();
    for i in 7..=23 {
        receiver
            .run(
                &mut p.bob,
                &mut p.b_store,
                SERVER,
                Some(StudioRequest::Read {
                    target: StudioTarget::Flipnote {
                        channel: channel(),
                        object: [i; 16],
                    },
                }),
            )
            .unwrap();
    }
    p.bob.flush_studio_subscriptions().await.unwrap();
    let op = title(1, "evicted object");
    p.save(&op);
    assert!(
        p.send(op).await.is_err(),
        "evicted object must have no subscriber"
    );
    assert!(!receiver.pending(&p.bob));
    assert!(p.state().is_none());
    assert!(p.b_store.load_server(SERVER).is_err());
}
