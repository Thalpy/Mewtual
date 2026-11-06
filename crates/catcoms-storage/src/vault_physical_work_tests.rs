//! Actual OS-lock lifetime tests. Authored UNRUN while compilation is paused. No source/key/
//! authority permit is fabricated: the worker owns only a real output File and original loan.
use super::*;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

#[test]
fn physical_work_lease_keeps_same_os_lock_until_last_guard_or_loan_drains() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<VaultPhysicalWorkLease>();
    let directory = tempfile::tempdir().unwrap();
    let guard = acquire_vault_session(directory.path()).unwrap();
    let first = guard.retain_for_physical_work();
    let second = guard.retain_for_physical_work();
    assert_eq!(
        std::mem::size_of::<VaultPhysicalWorkLease>(),
        std::mem::size_of::<Arc<File>>()
    );
    assert_eq!(format!("{guard:?}"), "VaultSessionGuard { .. }");
    assert_eq!(format!("{first:?}"), "VaultPhysicalWorkLease { .. }");
    drop(guard);
    assert!(matches!(
        acquire_vault_session(directory.path()),
        Err(StorageError::VaultBusy)
    ));
    drop(first);
    assert!(matches!(
        acquire_vault_session(directory.path()),
        Err(StorageError::VaultBusy)
    ));
    drop(second);
    let guard = acquire_vault_session(directory.path()).unwrap();
    let lease = guard.retain_for_physical_work();
    drop(lease);
    assert!(
        matches!(
            acquire_vault_session(directory.path()),
            Err(StorageError::VaultBusy)
        ),
        "loan-first drop must not unlock its still-live original guard"
    );
    drop(guard);
    drop(acquire_vault_session(directory.path()).unwrap());
}

/// Sender Drop releases a parked worker during any assertion panic; the worker's receive is
/// bounded too. Join on unwind prevents temporary-directory cleanup racing a detached writer.
struct RunningIo {
    release: Option<Sender<()>>,
    done: Receiver<Result<(), String>>,
    join: Option<JoinHandle<()>>,
}
/// Field drop order also protects panic unwinding, unlike merely declaring a local loan next
/// to a File captured in the closure environment (that environment may drain later).
struct PhysicalWork {
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
        // Construct before channel allocation/spawn so even setup failure drains the File
        // before the loan. The closure captures ONE work object, never independent fields.
        let work = PhysicalWork {
            output,
            _session: lease,
        };
        let (started_tx, started) = channel();
        let (release, advance) = channel();
        let (done_tx, done) = channel();
        let join = std::thread::Builder::new()
            .name("vault-session-physical-work".into())
            .spawn(move || {
                let mut work = work;
                let result = (|| -> Result<(), String> {
                    work.output
                        .write_all(b"before-")
                        .map_err(|error| error.to_string())?;
                    started_tx.send(()).map_err(|error| error.to_string())?;
                    advance
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|error| error.to_string())?;
                    work.output
                        .write_all(b"after")
                        .map_err(|error| error.to_string())?;
                    work.output.sync_all().map_err(|error| error.to_string())?;
                    Ok(())
                })();
                drop(work);
                let _ = done_tx.send(result);
            })
            .expect("spawn actual physical worker");
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
fn already_started_real_write_can_finish_after_guard_drop_without_allowing_remount() {
    let directory = tempfile::tempdir().unwrap();
    let guard = acquire_vault_session(directory.path()).unwrap();
    let path = directory.path().join("physical-work-output");
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let (worker, started) = RunningIo::start(output, guard.retain_for_physical_work());
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(guard);
    assert!(
        matches!(
            acquire_vault_session(directory.path()),
            Err(StorageError::VaultBusy)
        ),
        "old admitted continuation still owns the SAME OS lock"
    );
    worker.drain();
    assert_eq!(std::fs::read(path).unwrap(), b"before-after");
    drop(acquire_vault_session(directory.path()).unwrap());
}

#[test]
fn physical_work_loan_preserves_existing_separate_short_vault_transaction_behavior() {
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0x5e551ea5e);
    drop(open_or_create_vault(directory.path(), b"resource-only loan", &mut rng).unwrap());
    let guard = acquire_vault_session(directory.path()).unwrap();
    let lease = guard.retain_for_physical_work();
    drop(guard);
    // These public key-vault helpers have ALWAYS used the DISTINCT short transaction lock;
    // a resource loan is not a replacement mount or a new authorization boundary for them.
    verify_vault_passphrase(directory.path(), b"resource-only loan").unwrap();
    assert!(matches!(
        acquire_vault_session(directory.path()),
        Err(StorageError::VaultBusy)
    ));
    drop(lease);
    drop(acquire_vault_session(directory.path()).unwrap());
}
