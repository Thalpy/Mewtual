//! Concrete native execution for the sync service's affine full-source read. No read is started
//! by constructing this adapter; new bridge dispatch is separately gated in the sync owner.
use std::sync::Arc;

use catcoms_sync::file_service::read::{
    FileReadExecutor, FileReadFuture, FileReadTask, SourceReadRuntime,
};

#[derive(Debug)]
struct NativeExecutor;
impl FileReadExecutor for NativeExecutor {
    fn start(&self, task: FileReadTask) -> FileReadFuture {
        start_worker(task, || {})
    }
}

// The hook is empty in production. Narrow parent visibility lets the isolated integration test
// compile this exact source and hold the native closure before storage I/O, proving custody
// without depending on a slow disk, a fixed sleep or exclusive access in the lib-test process.
// No external crate can call this helper or bypass FileReadTask's original shared reservation.
pub(super) fn start_worker(
    task: FileReadTask,
    before_read: impl FnOnce() + Send + 'static,
) -> FileReadFuture {
    // spawn_blocking owns the task, including its capacity, before any observer can await
    // or drop the JoinHandle. Dropping that handle does not abort a queued/running closure.
    let worker = tokio::task::spawn_blocking(move || {
        before_read();
        task.run()
    });
    Box::pin(async move { worker.await.ok() })
}

/// Native callers share the source pool even across separate adapter/Root constructions.
/// This is an executor factory, not activation, consent or authorization to serve files.
pub fn runtime() -> SourceReadRuntime {
    SourceReadRuntime::new(Arc::new(NativeExecutor))
}
