use super::*;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[tokio::test]
async fn healthy_solo_studio_access_and_idle_work_do_not_default_to_paused() {
    let clock = ManualClock::new(1000);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(952),
        Box::new(clock.clone()),
        "solo",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(
        root.path(),
        b"solo-sync",
        &mut ChaCha20Rng::seed_from_u64(953),
    )
    .unwrap();
    let channel = crate::channel_id("general").to_be_bytes();
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            7,
            Some(StudioRequest::Read {
                target: StudioTarget::Index { channel },
            }),
        )
        .unwrap();
    assert!(!receiver.take_sync_notice().unwrap().paused);
    receiver
        .run(
            &mut server,
            &mut store,
            7,
            Some(StudioRequest::Create {
                channel,
                object: [0xd1; 16],
                nonce: [0xd2; 16],
                title: "Solo drawing".into(),
                ts: 1000,
            }),
        )
        .unwrap();
    for _ in 0..24 {
        clock.advance_ms(5000);
        receiver.run(&mut server, &mut store, 7, None).unwrap();
        if let Some(job) = receiver.detach(&mut server) {
            receiver.complete(&mut server, job.run(None).await);
        }
        assert!(
            !receiver.take_pause_notice(),
            "normal solo maintenance must not pause"
        );
    }
    assert!(!receiver.paused);
}

#[tokio::test]
async fn a_real_solo_source_failure_reports_ordered_pause_and_only_successful_access_resumes() {
    let clock = ManualClock::new(1000);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(954),
        Box::new(clock.clone()),
        "solo",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(
        root.path(),
        b"solo-sync-failure",
        &mut ChaCha20Rng::seed_from_u64(955),
    )
    .unwrap();
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [0xd3; 16],
    };
    let mut receiver = StudioReceiver::default();
    receiver
        .run(
            &mut server,
            &mut store,
            7,
            Some(StudioRequest::Create {
                channel: target.channel(),
                object: [0xd3; 16],
                nonce: [0xd4; 16],
                title: "Saved drawing".into(),
                ts: 1000,
            }),
        )
        .unwrap();
    receiver.take_sync_notice();
    let path = server
        .sync
        .with_registry_context(|group, _, _, _| store.studio_source_path_for_test(7, group, target))
        .unwrap();
    assert!(path.starts_with(root.path()));
    let original = std::fs::read(&path).unwrap();
    // Inject damage only into this test's owned sealed record, not a model pause flag.
    std::fs::write(&path, b"damaged sealed source").unwrap();
    for _ in 0..24 {
        clock.advance_ms(5000);
        if receiver.run(&mut server, &mut store, 7, None).is_err() {
            break;
        }
        if let Some(job) = receiver.detach(&mut server) {
            receiver.complete(&mut server, job.run(None).await);
        }
    }
    let paused = receiver
        .take_sync_notice()
        .expect("actual background failure must report its pause");
    assert!(paused.paused);
    assert_eq!(paused.cause, Some(StudioSyncPauseCause::Background));
    assert!(!paused.detail.as_ref().unwrap().is_empty());
    assert!(paused.detail.unwrap().chars().count() <= 512);
    assert!(receiver
        .run(
            &mut server,
            &mut store,
            7,
            Some(StudioRequest::Read { target })
        )
        .is_err());
    assert!(receiver.paused);
    assert!(
        receiver.take_sync_notice().is_none(),
        "failed Retry cannot claim resume"
    );
    std::fs::write(&path, original).unwrap();
    receiver
        .run(
            &mut server,
            &mut store,
            7,
            Some(StudioRequest::Read { target }),
        )
        .unwrap();
    let resumed = receiver.take_sync_notice().unwrap();
    assert!(!resumed.paused);
    assert!(resumed.sequence > paused.sequence);
    assert!(resumed.cause.is_none());
    assert!(resumed.detail.is_none());
}
