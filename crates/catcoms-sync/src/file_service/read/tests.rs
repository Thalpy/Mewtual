use super::*;
use catcoms_rt::{Clock, ManualClock};
use catcoms_storage::{FsBlobStore, MemoryBlobStore};
use tokio::sync::watch;

#[derive(Debug)]
struct Authority(watch::Receiver<bool>);
#[async_trait::async_trait]
impl RequestAuthority for Authority {
    fn authorized(&self) -> bool {
        *self.0.borrow() && self.0.has_changed().is_ok()
    }
    async fn revoked(&self) {
        let mut signal = self.0.clone();
        while self.authorized() {
            if signal.changed().await.is_err() {
                return;
            }
        }
    }
}
fn lease() -> (watch::Sender<bool>, Arc<dyn RequestAuthority>) {
    let (live, signal) = watch::channel(true);
    (live, Arc::new(Authority(signal)))
}

#[derive(Debug)]
struct NotStarted;
impl FileReadExecutor for NotStarted {
    fn start(&self, _: FileReadTask) -> FileReadFuture {
        panic!("preparation must not start a worker")
    }
}
fn isolated() -> SourceReadRuntime {
    SourceReadRuntime {
        pool: Arc::new(SourcePool::default()),
        executor: Arc::new(NotStarted),
    }
}
fn held() -> (MemoryBlobStore, Cid) {
    let mut store = MemoryBlobStore::default();
    let cid = store.put(b"ciphertext held at a source").unwrap();
    (store, cid)
}

#[test]
fn source_tasks_and_returned_results_keep_exact_full_group_and_process_capacity() {
    let (_live, authority) = lease();
    let runtime = isolated();
    let (store, cid) = held();
    let first = runtime
        .prepare(b"same-prefix/A", &store, &cid, 64, authority.clone())
        .unwrap();
    assert!(matches!(
        runtime.prepare(b"same-prefix/A", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    let other = runtime
        .prepare(b"same-prefix/B", &store, &cid, 64, authority.clone())
        .unwrap();
    assert!(matches!(
        runtime.prepare(b"C", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));

    let completed = first.run();
    let read = completed.result().unwrap().unwrap();
    assert!(read.is_current());
    assert_eq!(read.bytes(), b"ciphertext held at a source");
    assert!(matches!(
        runtime.prepare(b"same-prefix/A", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(other);
    let third = runtime
        .prepare(b"C", &store, &cid, 64, authority.clone())
        .unwrap();
    assert!(matches!(
        runtime.prepare(b"same-prefix/A", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(completed);
    let next = runtime
        .prepare(b"same-prefix/A", &store, &cid, 64, authority.clone())
        .unwrap();
    drop((third, next));
    assert!(runtime.pool.0.lock().unwrap().is_empty());
}

#[test]
fn deferred_source_metadata_and_actual_worker_have_independent_last_owner_custody() {
    let (_live, authority) = lease();
    let runtime = isolated();
    let (store, cid) = held();
    let prepare = || runtime.prepare(b"A", &store, &cid, 64, authority.clone());
    let task = prepare().unwrap();
    let deferred = task.keepalive();
    assert_eq!(format!("{deferred:?}"), "SourceReadCustody { .. }");
    drop(deferred);
    assert!(matches!(prepare(), Err(SourceReadError::Busy)));
    let deferred = task.keepalive();
    let completed = task.run();
    assert!(matches!(prepare(), Err(SourceReadError::Busy)));
    drop(completed);
    assert!(matches!(prepare(), Err(SourceReadError::Busy)));
    drop(deferred);

    let task = prepare().unwrap();
    let deferred = task.keepalive();
    let completed = Arc::new(task.run());
    let actual_response: catcoms_rt::SharedRequestKeepalive = completed.clone();
    drop((deferred, completed));
    assert!(matches!(prepare(), Err(SourceReadError::Busy)));
    drop(actual_response);
    assert!(prepare().is_ok());
}

#[test]
fn preparation_bounds_and_storage_errors_return_only_the_unstarted_reservation() {
    let (_live, authority) = lease();
    let runtime = isolated();
    let (store, cid) = held();
    for (group, max) in [
        (&[][..], 64),
        (&[1; 257][..], 64),
        (b"valid".as_slice(), MAX_SOURCE_CHUNK + 1),
    ] {
        assert!(matches!(
            runtime.prepare(group, &store, &cid, max, authority.clone()),
            Err(SourceReadError::Bounds)
        ));
        assert!(runtime.pool.0.lock().unwrap().is_empty());
    }
    assert!(matches!(
        runtime.prepare(b"A", &store, &cid, 1, authority.clone()),
        Err(SourceReadError::Storage(StorageError::BlobSizeLimit))
    ));
    assert!(runtime.pool.0.lock().unwrap().is_empty());
    let read = runtime
        .prepare(b"A", &store, &cid, 64, authority.clone())
        .unwrap();
    assert_eq!(runtime.pool.0.lock().unwrap().len(), 1);
    drop(read);
    assert!(runtime.pool.0.lock().unwrap().is_empty());
    assert!(BlobReadPlan::memory_bound(MAX_SOURCE_CHUNK).unwrap() <= SOURCE_READ_BYTES);
    assert!(BlobReadPlan::memory_bound(MAX_SOURCE_CHUNK).unwrap() > 32 * 1024 * 1024);
}

#[test]
fn returned_storage_epoch_and_failed_or_absent_results_remain_owned() {
    let (_live, authority) = lease();
    let runtime = isolated();
    let (mut store, cid) = held();
    let completed = runtime
        .prepare(b"A", &store, &cid, 64, authority.clone())
        .unwrap()
        .run();
    store.delete(&cid).unwrap();
    assert!(!completed.result().unwrap().unwrap().is_current());
    assert!(matches!(
        runtime.prepare(b"A", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(completed);
    let absent = runtime
        .prepare(b"A", &store, &cid, 64, authority.clone())
        .unwrap()
        .run();
    assert!(absent.result().unwrap().is_none());
    assert!(matches!(
        runtime.prepare(b"A", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(absent);
    assert!(runtime
        .prepare(b"A", &store, &cid, 64, authority.clone())
        .is_ok());

    let dir = tempfile::tempdir().unwrap();
    let mut disk = FsBlobStore::open(dir.path()).unwrap();
    let cid = disk.put(b"disk ciphertext").unwrap();
    let task = runtime
        .prepare(b"A", &disk, &cid, 64, authority.clone())
        .unwrap();
    std::fs::write(dir.path().join(cid.to_hex()), b"corrupt disk content").unwrap();
    let failed = task.run();
    assert!(matches!(
        failed.result(),
        Err(SourceReadError::Storage(StorageError::CidMismatch))
    ));
    assert!(matches!(
        runtime.prepare(b"A", &disk, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(failed);
    assert!(runtime
        .prepare(b"A", &disk, &cid, 64, authority.clone())
        .is_ok());
}

#[test]
fn replacing_runtime_and_executor_cannot_refill_process_source_slots() {
    let (_live, authority) = lease();
    // All other unit tests use isolated pools; this specifically exercises public construction.
    let original = SourceReadRuntime::new(Arc::new(NotStarted));
    let (store, cid) = held();
    let a = original
        .prepare(b"public/source/A", &store, &cid, 64, authority.clone())
        .unwrap();
    let b = original
        .prepare(b"public/source/B", &store, &cid, 64, authority.clone())
        .unwrap();
    drop(original);
    let replacement = SourceReadRuntime::new(Arc::new(NotStarted));
    assert!(matches!(
        replacement.prepare(b"public/source/C", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(a);
    assert!(replacement
        .prepare(b"public/source/C", &store, &cid, 64, authority.clone())
        .is_ok());
    drop(b);
}

#[test]
fn revoked_prepare_and_queued_read_refuse_before_any_storage_operation() {
    let runtime = isolated();
    let (store, cid) = held();
    let (live, authority) = lease();
    let task = runtime
        .prepare(b"A", &store, &cid, 64, authority.clone())
        .unwrap();
    drop(live); // the task/result's Arc does not retain the publisher or Root
    assert!(matches!(
        runtime.prepare(b"B", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Revoked)
    ));
    let completed = task.run_with(|_| panic!("revoked queued work must never enter read I/O"));
    assert!(matches!(completed.result(), Err(SourceReadError::Revoked)));
    let (_replacement, fresh) = lease();
    assert!(matches!(
        runtime.prepare(b"A", &store, &cid, 64, fresh.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(completed);
    assert!(runtime.prepare(b"A", &store, &cid, 64, fresh).is_ok());
    assert!(
        !authority.authorized(),
        "replacement cannot revive the retired observer"
    );
}

#[test]
fn revocation_during_admitted_read_suppresses_late_borrow_and_keeps_real_result_charged() {
    let runtime = isolated();
    let (store, cid) = held();
    let (live, authority) = lease();
    let task = runtime
        .prepare(b"A", &store, &cid, 64, authority.clone())
        .unwrap();
    let completed = task.run_with(|plan| {
        let result = plan.run();
        live.send_replace(false);
        result
    });
    assert!(
        completed.result.as_ref().unwrap().is_some(),
        "blocking I/O had already admitted and returned actual bytes"
    );
    assert!(matches!(completed.result(), Err(SourceReadError::Revoked)));
    let (_replacement, fresh) = lease();
    assert!(matches!(
        runtime.prepare(b"A", &store, &cid, 64, fresh.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(completed);
    let completed = runtime
        .prepare(b"A", &store, &cid, 64, fresh)
        .unwrap()
        .run();
    // The scope remains live; this source storage fence is independently current.
    assert!(completed.result().unwrap().unwrap().is_current());
}

#[derive(Debug)]
struct DeadlineAuthority(Arc<ManualClock>, u64);
#[async_trait::async_trait]
impl RequestAuthority for DeadlineAuthority {
    fn authorized(&self) -> bool {
        self.0.monotonic_ms() < self.1
    }
    async fn revoked(&self) {
        while self.authorized() {
            self.0
                .sleep(std::time::Duration::from_millis(
                    self.1.saturating_sub(self.0.monotonic_ms()),
                ))
                .await;
        }
    }
}
#[test]
fn exact_original_deadline_refuses_queued_read_before_operation_entry() {
    let runtime = isolated();
    let (store, cid) = held();
    let clock = Arc::new(ManualClock::new(1000));
    let authority: Arc<dyn RequestAuthority> = Arc::new(DeadlineAuthority(clock.clone(), 1010));
    let task = runtime.prepare(b"A", &store, &cid, 64, authority).unwrap();
    clock.advance_ms(10);
    let completed = task.run_with(|_| panic!("expired queued read must not enter I/O"));
    assert!(matches!(completed.result(), Err(SourceReadError::Revoked)));
}

#[test]
fn final_storage_observation_survives_reply_work_retirement_but_never_store_retirement() {
    let (live, authority) = lease();
    let runtime = isolated();
    let (mut store, cid) = held();
    let completed = Arc::new(
        runtime
            .prepare(b"A", &store, &cid, 64, authority)
            .unwrap()
            .run(),
    );
    assert!(completed.storage_is_current());
    assert!(completed.result().unwrap().is_some());
    live.send_replace(false);
    assert!(matches!(completed.result(), Err(SourceReadError::Revoked)));
    assert!(
        completed.storage_is_current(),
        "final storage observation must exclude the retired work/reply lifetime"
    );
    store.delete(&cid).unwrap();
    assert!(
        !completed.storage_is_current(),
        "Forget/delete before send cannot escape storage fencing"
    );
    let (_next_live, next) = lease();
    assert!(matches!(
        runtime.prepare(b"A", &store, &cid, 64, next),
        Err(SourceReadError::Busy)
    ));
    assert_eq!(
        runtime.pool.0.lock().unwrap().len(),
        1,
        "revocation does not refund held completion custody"
    );
    let actual_response: catcoms_rt::SharedRequestKeepalive = completed.clone();
    drop(completed);
    assert_eq!(
        runtime.pool.0.lock().unwrap().len(),
        1,
        "actual response custody can retain the opaque whole completion"
    );
    drop(actual_response);
    assert!(runtime.pool.0.lock().unwrap().is_empty());
    let (_live, authority) = lease();
    let absent = runtime
        .prepare(b"A", &store, &cid, 64, authority)
        .unwrap()
        .run();
    assert!(absent.result().unwrap().is_none());
    assert!(
        !absent.storage_is_current(),
        "an absence observation is not a current stored-copy claim"
    );
}
