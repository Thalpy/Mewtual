//! Actual Store destruction while an admitted physical continuation runs. This fixture has
//! no source/Current/publication authority: only a real File and the same resource-only loan.
//! Authored UNRUN under the compilation/test/build pause.
use super::*;
use catcoms_storage::VaultPhysicalWorkLease;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

struct RunningIo {
    release: Option<Sender<()>>,
    done: Receiver<Result<(), String>>,
    join: Option<JoinHandle<()>>,
}
struct PhysicalWork {
    // Structural field order is essential even on panic: a captured File plus a loan local
    // would otherwise drop the local loan before the closure environment's output File.
    output: File,
    _session: VaultPhysicalWorkLease,
}
impl Drop for RunningIo {
    fn drop(&mut self) {
        drop(self.release.take());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
impl RunningIo {
    fn start(output: File, lease: VaultPhysicalWorkLease) -> (Self, Receiver<()>) {
        let work = PhysicalWork {
            output,
            _session: lease,
        };
        let (started_tx, started) = channel();
        let (release, advance) = channel();
        let (done_tx, done) = channel();
        let join = std::thread::Builder::new()
            .name("store-physical-session".into())
            .spawn(move || {
                // Resource owner is LAST to drain: no result/file/cleanup can outlive this loan.
                // It contains no Store, keys, creator sender, positive scope or Current capability.
                let mut work = work;
                let result = (|| -> Result<(), String> {
                    work.output
                        .write_all(b"admitted-")
                        .map_err(|error| error.to_string())?;
                    started_tx.send(()).map_err(|error| error.to_string())?;
                    advance
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|error| error.to_string())?;
                    work.output
                        .write_all(b"finished")
                        .map_err(|error| error.to_string())?;
                    work.output.sync_all().map_err(|error| error.to_string())?;
                    Ok(())
                })();
                drop(work);
                let _ = done_tx.send(result);
            })
            .expect("spawn actual physical work");
        (
            Self {
                release: Some(release),
                done,
                join: Some(join),
            },
            started,
        )
    }
    fn drain(mut self) {
        self.release.take().unwrap().send(()).unwrap();
        self.done
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        self.join.take().unwrap().join().unwrap();
    }
}

#[test]
fn dropped_store_revokes_mount_but_admitted_physical_work_blocks_successor_until_drain() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x51e5510);
    let store = ServerStore::open(directory.path(), b"physical session fixture", &mut rng).unwrap();
    let negative = store.chat_mount.observer();
    let output_path = directory.path().join("physical-session-test-output");
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .unwrap();
    let (worker, started) = RunningIo::start(output, store._session.retain_for_physical_work());
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(store);
    assert!(
        negative.is_cancelled(),
        "the physical resource must not retain Store authority"
    );
    assert!(
        matches!(
            ServerStore::open(
                directory.path(),
                b"wrong secret must not be checked",
                &mut rng
            ),
            Err(AppError::Storage(StorageError::VaultBusy))
        ),
        "actual constructor must fail before unseal or cold repair"
    );
    worker.drain();
    assert_eq!(fs::read(output_path).unwrap(), b"admitted-finished");
    let next = ServerStore::open(directory.path(), b"physical session fixture", &mut rng).unwrap();
    assert!(!next.chat_mount.observer().is_cancelled());
    assert!(
        negative.is_cancelled(),
        "a new mount cannot renew the old observer"
    );
}
