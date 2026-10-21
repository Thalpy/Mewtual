//! The complete process-pool custody scenario needs its own executable: unrelated parallel
//! actor tests legitimately occupy the same two source slots. Isolation preserves the real
//! global pool, every Busy assertion and the actual blocking worker; it creates no test pool.
//!
//! Compile the exact native executor source to install its private before-read barrier, and
//! also exercise the linked public factory below to pin their shared production pool.
#[path = "../src/file_source.rs"]
// The exact production source exports runtime(); within this private integration-test module
// that public spelling is intentionally retained, while linked public API calls below are real.
#[allow(unreachable_pub)]
mod native_adapter;

use catcoms_rt::RequestAuthority;
use catcoms_rt::{Clock, SystemClock};
use catcoms_storage::{BlobStore, MemoryBlobStore};
use catcoms_sync::file_service::read::SourceReadError;
use catcoms_sync::file_service::read::{
    FileReadExecutor, FileReadFuture, FileReadTask, SourceReadRuntime,
};
use native_adapter::start_worker;
use std::sync::Arc;
use std::sync::Mutex;
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
struct HeldExecutor(Mutex<Option<std::sync::mpsc::Receiver<()>>>);
impl FileReadExecutor for HeldExecutor {
    fn start(&self, task: FileReadTask) -> FileReadFuture {
        let release = self.0.lock().unwrap().take().unwrap();
        start_worker(task, move || {
            let _ = release.recv();
        })
    }
}
#[derive(Debug)]
struct PanickingExecutor;
impl FileReadExecutor for PanickingExecutor {
    fn start(&self, task: FileReadTask) -> FileReadFuture {
        start_worker(task, || panic!("injected native worker failure"))
    }
}

#[tokio::test]
async fn native_blocking_read_retains_capacity_after_observer_and_runtime_drop() {
    let (_live, authority) = lease();
    let mut store = MemoryBlobStore::default();
    let cid = store.put(b"native source ciphertext").unwrap();
    let (release, wait) = std::sync::mpsc::channel();
    let old = SourceReadRuntime::new(Arc::new(HeldExecutor(Mutex::new(Some(wait)))));
    let task = old
        .prepare(b"native/A", &store, &cid, 64, authority.clone())
        .unwrap();
    let other = old
        .prepare(b"native/B", &store, &cid, 64, authority.clone())
        .unwrap();
    let observer = old.start(task);
    drop((observer, old));
    let replacement = catcoms_app::file_source::runtime();
    // The linked public factory and included exact native adapter must use the same dependency
    // static. Copying the source into this test must not accidentally create a second pool.
    let included = native_adapter::runtime();
    assert!(matches!(
        included.prepare(b"native/C", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    assert!(matches!(
        replacement.prepare(b"native/A", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    assert!(matches!(
        replacement.prepare(b"native/C", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    // Retiring another group's unstarted task releases only its own slot. The detached
    // original closure still prevents replacement A from taking a second same-group read.
    drop(other);
    let c = replacement
        .prepare(b"native/C", &store, &cid, 64, authority.clone())
        .unwrap();
    assert!(matches!(
        replacement.prepare(b"native/A", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(c);
    release.send(()).unwrap();
    // A real blocking thread is involved; bound this wait through the sanctioned Clock.
    // Readiness is the actual returned capacity, never a fixed sleep or assumed scheduler turn.
    tokio::select! {
        _ = SystemClock.sleep(std::time::Duration::from_secs(5)) => panic!("native read did not retire"),
        _ = async {
            loop {
                match replacement.prepare(b"native/A", &store, &cid, 64, authority.clone()) {
                    Ok(task) => { drop(task); break; },
                    Err(SourceReadError::Busy) => tokio::task::yield_now().await,
                    Err(error) => panic!("unexpected admission failure: {error}"),
                }
            }
        } => {},
    }
    let task = replacement
        .prepare(b"native/A", &store, &cid, 64, authority.clone())
        .unwrap();
    let completed = replacement.start(task).await.unwrap();
    assert_eq!(
        completed.result().unwrap().unwrap().bytes(),
        b"native source ciphertext"
    );
    assert!(matches!(
        replacement.prepare(b"native/A", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(completed);
    assert!(replacement
        .prepare(b"native/A", &store, &cid, 64, authority.clone())
        .is_ok());

    let (live, cancellable) = lease();
    let (release, wait) = std::sync::mpsc::channel();
    let held = SourceReadRuntime::new(Arc::new(HeldExecutor(Mutex::new(Some(wait)))));
    let task = held
        .prepare(b"native/A", &store, &cid, 64, cancellable)
        .unwrap();
    let observer = held.start(task);
    drop(live);
    release.send(()).unwrap();
    let cancelled = observer.await.unwrap();
    assert!(matches!(cancelled.result(), Err(SourceReadError::Revoked)));
    assert!(matches!(
        replacement.prepare(b"native/A", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(cancelled);
    assert!(replacement
        .prepare(b"native/A", &store, &cid, 64, authority.clone())
        .is_ok());

    let failed = SourceReadRuntime::new(Arc::new(PanickingExecutor));
    let task = failed
        .prepare(b"native/A", &store, &cid, 64, authority.clone())
        .unwrap();
    // The Root exports custody BEFORE starting/awaiting the worker. A panic retires the
    // task but cannot refund responder/proof metadata that the Root still owns.
    let deferred = task.keepalive();
    assert!(failed.start(task).await.is_none());
    assert!(matches!(
        replacement.prepare(b"native/A", &store, &cid, 64, authority.clone()),
        Err(SourceReadError::Busy)
    ));
    drop(deferred);
    assert!(replacement
        .prepare(b"native/A", &store, &cid, 64, authority.clone())
        .is_ok());
}
