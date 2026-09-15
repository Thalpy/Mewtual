use super::*;
use crate as app;
#[path = "../../../tests/support/studio_inspection.rs"]
mod fixture;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::time::Duration;

#[tokio::test]
async fn studio_inspection_cancelled_worker_and_retained_result_keep_original_capacity() {
    let root = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(74);
    let store = ServerStore::open(root.path(), b"capacity", &mut rng).unwrap();
    let clock = ManualClock::new(1000);
    let mut server = Server::found(
        Hub::new().join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng,
        Box::new(clock.clone()),
        "owner",
    )
    .unwrap();
    let target = StudioTarget::Index {
        channel: crate::channel_id("general").to_be_bytes(),
    };
    let pool = Arc::new(tokio::sync::Semaphore::new(4));
    let mut jobs = Vec::new();
    for _ in 0..4 {
        jobs.push(
            server
                .begin_inspection_with_pool(&store, 7, target, &pool)
                .unwrap(),
        );
    }
    assert!(server
        .begin_inspection_with_pool(&store, 7, target, &pool)
        .is_err());
    let queued = jobs.pop().unwrap();
    drop(queued);
    assert_eq!(pool.available_permits(), 1);
    let job = jobs.pop().unwrap();
    let (started, running) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let cancelled = tokio::spawn(job.rebuild_with(move |capture| {
        started.send(()).unwrap();
        wait.recv().unwrap();
        capture.rebuild()
    }));
    running.await.unwrap();
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    assert_eq!(
        pool.available_permits(),
        1,
        "cancelled waiter refunded running inspection"
    );
    let prepared = jobs.pop().unwrap().rebuild().await.unwrap();
    assert_eq!(pool.available_permits(), 1);
    let mut result = server
        .finish_studio_inspection(&store, 7, target, prepared)
        .unwrap();
    let handoff = result.begin_delivery(Arc::new(clock.clone()));
    let delivery = result.delivery();
    drop(result);
    assert_eq!(
        pool.available_permits(),
        1,
        "native guard lost retained output capacity"
    );
    clock.advance_ms(5000);
    assert!(!delivery.is_current());
    assert_eq!(
        pool.available_permits(),
        1,
        "expiry refunded a retained result"
    );
    drop(delivery);
    drop(handoff);
    assert_eq!(pool.available_permits(), 2);
    drop(jobs);
    assert_eq!(pool.available_permits(), 3);
    release.send(()).unwrap();
    let all = tokio::time::timeout(Duration::from_secs(10), pool.clone().acquire_many_owned(4))
        .await
        .unwrap()
        .unwrap();
    drop(all);
    assert_eq!(pool.available_permits(), 4);
}

#[tokio::test]
async fn studio_inspection_paused_real_draft_allows_actor_checkpoint_progress() {
    let f = fixture::InspectionFixture::new(true).await;
    assert!(!f.group.is_empty());
    assert!(!f.device.to_string().is_empty());
    let snapshot = f.actor.snapshot().await.unwrap();
    let mut verifier = Server::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(9)),
        ChaCha20Rng::seed_from_u64(79),
        Box::new(f.clock.clone()),
        "verifier",
    )
    .unwrap();
    let other = StudioTarget::Flipnote {
        channel: f.target.channel(),
        object: [8; 16],
    };
    verifier.sync.with_registry_context(|g, d, _, _| {
        crate::store::fill_studio_epoch_fixture(
            f.store.try_lock().unwrap().as_mut().unwrap(),
            7,
            g,
            d,
            other,
        );
    });
    let before = f.records();
    let job = f.capture().await;
    assert!(
        Arc::ptr_eq(
            job.permit.semaphore(),
            crate::registry_catchup::preparation_pool()
        ),
        "inspection must use the existing shared preparation pool"
    );
    let (started, running) = oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    // The release sender is dropped on failure too, so an assertion cannot hang a blocking worker.
    let worker = tokio::spawn(job.rebuild_with(move |capture| {
        started.send(()).unwrap();
        let _ = wait.recv();
        capture.rebuild()
    }));
    running.await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        f.actor
            .studio_begin(StudioRequest::Read { target: other })
            .await
            .unwrap()
            .execute(StudioVaultLease::new(
                f.store.clone().try_lock_owned().unwrap(),
                7,
                (),
            ))
            .await
            .unwrap();
        for _ in 0..60 {
            f.clock.advance_ms(1000);
            f.actor
                .studio_receive_begin()
                .await
                .unwrap()
                .execute(StudioVaultLease::new(
                    f.store.clone().try_lock_owned().unwrap(),
                    7,
                    (),
                ))
                .await
                .unwrap();
            let installed = verifier.sync.with_registry_context(|g, d, _, _| {
                let held = f.store.try_lock().unwrap();
                let store = held.as_ref().unwrap();
                store
                    .load_studio_epoch(7, g, other, d)
                    .unwrap()
                    .is_some_and(|s| s.epoch() == 1 && s.op_count() == 0)
            });
            if installed {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("authoritative checkpoint stalled behind detached inspection");
    })
    .await
    .expect("actor custody stayed with detached inspection");
    assert!(
        !worker.is_finished(),
        "checkpoint progress depended on worker completion"
    );
    assert_ne!(
        f.records(),
        before,
        "checkpoint must cross actual durable writer"
    );
    release.send(()).unwrap();
    let prepared = worker.await.unwrap().unwrap();
    let StudioControlResponse::OverlayInspection(read) = f
        .control(StudioControlAction::FinishOverlayInspection(Box::new(
            prepared,
        )))
        .await
        .unwrap()
    else {
        panic!("not an inspection")
    };
    read.inspect(|target, prepared, draft| {
        assert_eq!(target, f.target);
        assert!(!prepared);
        let draft = draft.unwrap();
        assert_eq!(draft.basis(), f.basis);
        assert_eq!(draft.projection(), &f.expected);
    })
    .unwrap();
    drop(read);
    f.shutdown().await;
}
