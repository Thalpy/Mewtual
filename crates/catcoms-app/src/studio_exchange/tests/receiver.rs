use super::*;
use crate::studio::StudioReceiver;

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
