use super::*;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn replay_pending_requires_its_own_current_watch_not_another_healthy_watch() {
    let mut rng = ChaCha20Rng::seed_from_u64(181);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng.clone(),
        Box::new(ManualClock::new(1000)),
        "owner",
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"replay-watch", &mut rng).unwrap();
    let mut receiver = StudioReceiver::default();
    for n in [1, 2] {
        receiver
            .run(
                &mut server,
                &mut store,
                83,
                Some(StudioRequest::Read {
                    target: StudioTarget::Flipnote {
                        channel: crate::channel_id("general").to_be_bytes(),
                        object: [n; 16],
                    },
                }),
            )
            .unwrap();
    }
    let (first, epoch) = &receiver.watches[0];
    receiver.replay.active = Some(Pass {
        target: first.target,
        epoch: *epoch,
        order: VecDeque::new(),
        manual: BTreeSet::new(),
        history_ids: vec![],
    });
    assert!(receiver.replay.pending(&receiver.watches, 1000, |w| server
        .sync
        .studio_watch_is_current(&w.inner)));
    server.unwatch_studio_epoch(first).unwrap();
    assert!(server
        .sync
        .studio_watch_is_current(&receiver.watches[1].0.inner));
    assert!(!receiver.replay.pending(&receiver.watches, 1000, |w| server
        .sync
        .studio_watch_is_current(&w.inner)));
}
