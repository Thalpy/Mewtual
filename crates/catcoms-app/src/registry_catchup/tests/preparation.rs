use super::*;

fn cold(f: &mut Fixture) -> ServerRegistryPageProvider {
    f.server
        .begin_registry_page_provider(&f.store, SERVER, f.key.bucket())
        .unwrap()
}

#[tokio::test]
async fn registry_idle_runtime_caches_release_all_four_slots_for_another_studio_source() {
    // Private pool keeps this resource-lifetime regression deterministic beside parallel
    // suites. It is the same semaphore/job/source ownership used by the global process pool.
    let pool = Arc::new(Semaphore::new(4));
    let mut held = Vec::new();
    for _ in 0..4 {
        let mut f = Fixture::new();
        f.edit(1);
        let mut provider = cold(&mut f);
        let job = f
            .server
            .begin_registry_page_preparation_with(&f.store, &mut provider, &pool)
            .unwrap()
            .unwrap();
        let result = job.rebuild().await.unwrap();
        f.server
            .finish_registry_page_preparation(&f.store, &mut provider, result)
            .unwrap();
        let receiver = crate::studio::StudioReceiver::retaining_registry_for_test(provider, 31_000);
        held.push((f, receiver));
    }
    assert_eq!(pool.available_permits(), 0);
    for (f, receiver) in &mut held {
        f.clock.advance_ms(29_999);
        receiver
            .run(&mut f.server, &mut f.store, SERVER, None)
            .unwrap();
    }
    assert_eq!(pool.available_permits(), 0, "no premature refund");
    for (f, receiver) in &mut held {
        f.clock.advance_ms(1);
        receiver
            .run(&mut f.server, &mut f.store, SERVER, None)
            .unwrap();
    }
    assert_eq!(
        pool.available_permits(),
        4,
        "idle pass needs no UI access or new peer request"
    );
    // A fifth server can now reserve before capturing its real saved Studio graph.
    let mut f = Fixture::new();
    let channel = crate::channel_id("general").to_be_bytes();
    f.server
        .studio_transaction(
            &mut f.store,
            SERVER,
            crate::studio::StudioRequest::Create {
                channel,
                object: [4; 16],
                nonce: [5; 16],
                title: "fifth".into(),
                ts: 1000,
            },
        )
        .unwrap();
    let permit = pool.clone().try_acquire_owned().unwrap();
    let target = catcoms_replication::studio::StudioTarget::Flipnote {
        channel,
        object: [4; 16],
    };
    let capture = f
        .server
        .sync
        .with_registry_context(|g, d, _, _| f.store.capture_studio_source(SERVER, g, target, d))
        .unwrap()
        .unwrap();
    let prepared = capture.rebuild().unwrap();
    assert!(f
        .server
        .sync
        .with_registry_context(|g, d, _, _| f.store.install_prepared_studio_source(g, d, prepared))
        .unwrap());
    drop(permit);
    assert_eq!(pool.available_permits(), 4);
}

#[tokio::test]
async fn registry_page_split_preparation_allows_saves_and_rejects_superseded_jobs() {
    let mut f = Fixture::new();
    f.edit(1);
    let pool = Arc::new(Semaphore::new(4));
    let mut provider = cold(&mut f);
    let job = f
        .server
        .begin_registry_page_preparation_with(&f.store, &mut provider, &pool)
        .unwrap()
        .unwrap();
    // No store or Server borrow escapes capture. An ordinary durable save can proceed while
    // the detached job exists; finishing that job must not install its obsolete projection.
    f.edit(2);
    let result = job.rebuild().await.unwrap();
    assert!(
        !f.server
            .attach_registry_page_preparation_if_current(&f.store, &mut provider, result)
            .unwrap(),
        "healthy saved-source replacement is a discarded result, not a storage fault"
    );
    let older = f
        .server
        .begin_registry_page_preparation_with(&f.store, &mut provider, &pool)
        .unwrap()
        .unwrap();
    let newer = f
        .server
        .begin_registry_page_preparation_with(&f.store, &mut provider, &pool)
        .unwrap()
        .unwrap();
    let result = newer.rebuild().await.unwrap();
    f.server
        .finish_registry_page_preparation(&f.store, &mut provider, result)
        .unwrap();
    let result = older.rebuild().await.unwrap();
    assert!(f
        .server
        .finish_registry_page_preparation(&f.store, &mut provider, result)
        .unwrap_err()
        .to_string()
        .contains("was replaced"));
    // A rejected old completion cannot evict the successfully installed newer source.
    assert!(provider.has_prepared_source());
    assert_eq!(f.page(&mut provider, None).operations.len(), 2);
    assert_eq!(pool.available_permits(), 3);
    f.edit(3);
    assert!(
        !f.server
            .registry_page_preparation_is_warm(&f.store, &mut provider)
            .unwrap(),
        "healthy rewrite of an attached graph requests preparation, not a storage pause"
    );
    assert!(!provider.has_prepared_source());
    assert_eq!(pool.available_permits(), 4);
}

#[tokio::test]
async fn registry_page_prepared_pages_reuse_one_rebuild_and_refuse_oversized_file() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut f = Fixture::new();
    f.edit(1);
    let mut provider = cold(&mut f);
    let rebuilt = Arc::new(AtomicUsize::new(0));
    let count = rebuilt.clone();
    f.server
        .prepare_registry_page_source_with(
            &f.store,
            &mut provider,
            &Arc::new(Semaphore::new(4)),
            move |capture| {
                count.fetch_add(1, Ordering::SeqCst);
                capture.rebuild()
            },
        )
        .await
        .unwrap();
    let full_loads = crate::store::registry_full_loads_for_test();
    for _ in 0..3 {
        assert_eq!(f.page(&mut provider, None).operations.len(), 1);
    }
    assert_eq!(crate::store::registry_full_loads_for_test(), full_loads);
    assert_eq!(rebuilt.load(Ordering::SeqCst), 1);
    let path = f.file();
    // A sparse oversized fixture tests metadata rejection without allocating its contents.
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(
            catcoms_replication::registry_epoch::MAX_REGISTRY_EPOCH_SNAPSHOT_BYTES as u64 + 2048,
        )
        .unwrap();
    assert!(f.serve(&mut provider, None).is_err());
    assert!(!provider.has_prepared_source());
    assert_eq!(rebuilt.load(Ordering::SeqCst), 1);
}

#[test]
fn registry_page_preparation_is_explicit_and_refresh_preserves_cursor() {
    let mut f = Fixture::new();
    for n in 0..33 {
        f.edit(n);
    }
    let mut provider = cold(&mut f);
    assert!(!provider.has_prepared_source());
    assert!(f
        .serve(&mut provider, None)
        .unwrap_err()
        .to_string()
        .contains("local preparation"));
    f.prepare(&mut provider);
    let first = f.page(&mut provider, None);
    let cursor = first.next.unwrap();
    f.edit(34);
    assert!(f.serve(&mut provider, Some(cursor.as_bytes())).is_err());
    assert!(!provider.has_prepared_source());
    f.prepare(&mut provider);
    // Refresh preserves the MAC key and the frozen prefix: the appended 34th op is not part
    // of this cursor. A fresh request will include it. No duplicate prefix/restart is necessary.
    let next = f.page(&mut provider, Some(cursor.as_bytes()));
    assert_eq!(next.operations.len(), 1);
    assert!(next.next.is_none());
}

#[test]
fn registry_page_preparation_detects_gate_only_fault_and_missing_source() {
    let mut f = Fixture::new();
    f.edit(1);
    let mut provider = f.begin();
    for close in [7, 8] {
        f.server.sync.with_registry_context(|g, d, _, r| {
            let state = f
                .store
                .load_registry_epoch(SERVER, g, f.key.bucket(), d)
                .unwrap()
                .unwrap();
            assert_eq!(state.op_count(), 1);
            let seed = state.projection().unwrap().checkpoint([close; 32]).unwrap();
            let receipt = catcoms_replication::Receipt::sign(
                registry_document(&g.group_id(), f.key.bucket()).unwrap(),
                0,
                [close; 32],
                seed.change_hash(),
                0,
                catcoms_replication::InheritedCheckpoint::EpochZero,
                d,
            )
            .unwrap();
            f.store
                .seal_registry_epoch(SERVER, g, f.key.bucket(), d, receipt, 0, r, &mut f.budget)
                .unwrap();
        });
        assert!(f
            .serve(&mut provider, None)
            .unwrap_err()
            .to_string()
            .contains("local preparation"));
        f.prepare(&mut provider);
    }
    // The whole authenticated wrapper changed, despite identical history. A freshly prepared
    // Fault must still take the core's refusal path, never serve the former Open source.
    assert!(f.serve(&mut provider, None).is_err());
    std::fs::remove_file(f.file()).unwrap();
    assert!(f.serve(&mut provider, None).is_err());
    assert!(!provider.has_prepared_source());
}

#[tokio::test]
async fn registry_page_preparation_rejects_changed_source_before_attachment() {
    let mut f = Fixture::new();
    f.edit(1);
    let path = f.file();
    let old = std::fs::read(&path).unwrap();
    f.edit(2);
    let new = std::fs::read(&path).unwrap();
    std::fs::write(&path, old).unwrap();
    let mut provider = cold(&mut f);
    let pool = Arc::new(Semaphore::new(4));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let task = f.server.prepare_registry_page_source_with(
        &f.store,
        &mut provider,
        &pool,
        move |capture| {
            let _ = started_tx.send(());
            let _ = release_rx.recv();
            capture.rebuild()
        },
    );
    let (result, ()) = tokio::join!(task, async {
        started_rx.await.unwrap();
        // A different, valid authenticated source arrives while reconstruction is detached.
        std::fs::write(path, new).unwrap();
        release_tx.send(()).unwrap();
    });
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("local preparation"));
    assert!(!provider.has_prepared_source());
    assert_eq!(pool.available_permits(), 4);
}

#[tokio::test]
async fn registry_page_preparation_cancelled_workers_remain_charged_across_remount() {
    use std::future::Future;
    use std::task::Poll;
    let mut f = Fixture::new();
    f.edit(1);
    let pool = Arc::new(Semaphore::new(MAX_PREPARED_REGISTRY_SOURCES));
    // Production has one process pool across all mounts and Server instances.
    assert!(Arc::ptr_eq(preparation_pool(), preparation_pool()));
    let mut releases: Vec<std::sync::mpsc::Sender<()>> = Vec::new();
    for remaining in (0..4).rev() {
        let mut provider = cold(&mut f);
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        releases.push(release_tx);
        let mut task = Box::pin(f.server.prepare_registry_page_source_with(
            &f.store,
            &mut provider,
            &pool,
            move |capture| {
                // Dropping the sender on assertion/panic also releases this worker; a failed
                // regression must not strand a blocking thread during runtime shutdown.
                let _ = release_rx.recv();
                capture.rebuild()
            },
        ));
        std::future::poll_fn(|cx| {
            assert!(task.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(task);
        assert_eq!(pool.available_permits(), remaining);
        assert!(!provider.has_prepared_source());
        drop(provider);
        drop(f.store);
        f.store = ServerStore::open(f.root.path(), b"page-store", &mut rng()).unwrap();
    }
    let mut provider = cold(&mut f);
    let error = f
        .server
        .prepare_registry_page_source_with(
            &f.store,
            &mut provider,
            &pool,
            RegistrySourceCapture::rebuild,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("capacity exhausted"));
    drop(releases);
    // Wait for actual worker result destruction, not merely dropped awaiting futures.
    drop(pool.clone().acquire_many_owned(4).await.unwrap());
    assert!(f
        .server
        .prepare_registry_page_source_with(
            &f.store,
            &mut provider,
            &pool,
            RegistrySourceCapture::rebuild,
        )
        .await
        .unwrap());
    assert_eq!(pool.available_permits(), 3);
    // A successful prepared result retains its permit until dropped, too.
    drop(provider);
    assert_eq!(pool.available_permits(), 4);
}
