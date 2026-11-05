//! A single original, durably debited bootstrap-source pass. Large I/O never holds the
//! admission/pins/canonical/Root/registry locks. The long output mutex cannot be awaited by
//! their writers: result collection uses try_lock. No decoded count reconstructs this Work.
use super::*;
use catcoms_storage::VaultPhysicalWorkLease;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};
mod terminal;
pub(in crate::store) use terminal::{
    OriginalSourceTerminal, PreparedBootstrapSourceTerminal, SavedBootstrapSourceTerminal,
};

const PREPARED: u8 = 0;
const RUNNING: u8 = 1;
const DRAINED: u8 = 2;
const ABANDONED: u8 = 3;
const MAX_WORK_PATH_BYTES: usize = 32 * 1024;

/// Requested fixed metadata separate from the four full-frame source/opener buffers. This
/// includes all <=10 retained small journal events (initial,8 arms,terminal), bounded path-copy
/// overlap, Arc/control
/// holders and output objects. It is not an allocator, thread-stack or process RSS claim.
pub(super) fn validate_metadata_invoice(store: &ServerStore) -> Result<(), AppError> {
    // Two stored paths plus two simultaneous marker/control paths. join/push may double
    // each cloned byte capacity; byte length alone is not a path allocation invoice.
    let fixed = 2 * 4 * (MAX_WORK_PATH_BYTES + 256)
        + 10 * (founder::MAX_INTENT_FRAME
            + founder::MAX_TRANSITION_FRAME
            + founder::MAX_TARGET_MARKER_FRAME
            + 512
            + std::mem::size_of::<FounderSourcePreparedJournal>())
        + std::mem::size_of::<FounderSourceDebt>()
        + std::mem::size_of::<FounderSourcePayload>()
        + std::mem::size_of::<OriginalSourceWriteWork>()
        + std::mem::size_of::<FounderSourceOutput>()
        + 9 * (std::mem::size_of::<FounderSourceRun>() + 128)
        + std::mem::size_of::<OriginalSourceTerminal>()
        + std::mem::size_of::<PreparedBootstrapSourceTerminal>()
        + std::mem::size_of::<SavedBootstrapSourceTerminal>()
        + 16 * 1024;
    if fixed > 512 * 1024 || store.dir.as_os_str().len() > MAX_WORK_PATH_BYTES {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}

pub(super) struct FounderSourceRun {
    operation: Arc<()>,
    state: AtomicU8,
}
/// End notification is structurally AFTER captured file/output cleanup. Publishing Abandoned
/// in Work::drop itself would run BEFORE fields drain and could race a successor pass with
/// that cleanup. This small owner retains no files, key, quota or positive permission.
struct SourceRunEnd(Arc<FounderSourceRun>);
impl Drop for SourceRunEnd {
    fn drop(&mut self) {
        if self.0.state.load(Ordering::Acquire) != DRAINED {
            self.0.state.store(ABANDONED, Ordering::Release);
        }
    }
}
/// Idle debt retains the SAME files, not a key, session lease, task or source publisher.
/// Only a genuine running Work may mutate these files. Its lease outlives this mutex guard
/// and all in-flight syscalls; dropping Store cannot enable a competing physical opener.
pub(super) struct FounderSourceOutput {
    seed: Option<critical_namespaces::pending_join::CreatedStage>,
    source: Option<critical_namespaces::pending_join::CreatedStage>,
    records: Option<critical_namespaces::pending_join::CreatedStage>,
    manifest: Option<critical_namespaces::pending_join::CreatedStage>,
    completed: Option<CompletedSourceFiles>,
}
impl FounderSourceOutput {
    pub(super) fn new() -> Self {
        Self {
            seed: None,
            source: None,
            records: None,
            manifest: None,
            completed: None,
        }
    }
}
struct CompletedSourceFiles {
    run: Arc<FounderSourceRun>,
    lengths: [u64; 4],
}

/// Actual historical writer outcome. This is not a proof that an old SP is still current:
/// every consumer must also reselect the actual current journal and original root/target.
pub(in crate::store) struct SavedBootstrapSourceFiles {
    cell: Arc<Mutex<FounderSourceDebt>>,
    run: Arc<FounderSourceRun>,
}
impl std::fmt::Debug for SavedBootstrapSourceFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedBootstrapSourceFiles")
            .finish_non_exhaustive()
    }
}

struct PhysicalSourcePaths {
    root: PathBuf,
    target: PathBuf,
    root_directory: File,
    root_id: catcoms_storage::kept::SessionId,
    root_marker: File,
    root_marker_id: catcoms_storage::kept::SessionId,
    root_digest: [u8; 32],
    target_directory: File,
    target_id: catcoms_storage::kept::SessionId,
    origin: File,
    origin_id: catcoms_storage::kept::SessionId,
    origin_digest: [u8; 32],
}
impl PhysicalSourcePaths {
    fn validate(&self) -> Result<(), AppError> {
        use critical_namespaces::{archive_bootstrap, refused};
        for (path, held, expected) in [
            (&self.root, &self.root_directory, self.root_id),
            (&self.target, &self.target_directory, self.target_id),
        ] {
            let observed = open_archive_directory(path)?;
            if catcoms_storage::kept::session_id(held).map_err(io)? != expected
                || catcoms_storage::kept::session_id(&observed).map_err(io)? != expected
            {
                return Err(refused());
            }
        }
        for (path, held, expected, digest, limit) in [
            (
                self.root.join(archive_bootstrap::ROOT_MARKER),
                &self.root_marker,
                self.root_marker_id,
                self.root_digest,
                archive_bootstrap::MAX_MARKER_FRAMED,
            ),
            (
                self.target.join("origin"),
                &self.origin,
                self.origin_id,
                self.origin_digest,
                founder::MAX_TARGET_MARKER_FRAME,
            ),
        ] {
            let (observed, bytes) = read_archive_object(&path, limit)?.ok_or_else(refused)?;
            if catcoms_storage::kept::session_id(held).map_err(io)? != expected
                || catcoms_storage::kept::session_id(&observed).map_err(io)? != expected
                || *blake3::hash(&bytes).as_bytes() != digest
            {
                return Err(refused());
            }
        }
        Ok(())
    }
}
fn io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

/// Constructed before debit, consumed only after the actual small arm save. No Store,
/// Server, transport, live-source sender or positive Current capability is retained.
/// Bounded, already-paid read/AEAD allocations occur INSIDE the spent pass; allocation or
/// I/O refusal does not refund it. The temporary dbkey copy is not a source-only key: the
/// existing DATA opener also verifies nested private pages. It never enters idle debt.
pub(in crate::store) struct OriginalSourceWriteWork {
    paths: PhysicalSourcePaths,
    output: Arc<Mutex<FounderSourceOutput>>,
    run: Arc<FounderSourceRun>,
    negative: catcoms_rt::RequestCancellation,
    root_negative: catcoms_rt::RequestCancellation,
    key: Zeroizing<[u8; 32]>,
    #[cfg(test)]
    fault: u8,
    #[cfg(test)]
    before_io: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    after_seed: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    before_pages: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    _run_end: SourceRunEnd,
    // The shared original invoice outlives every named Work/output/control allocation.
    payload: Arc<FounderSourcePayload>,
    // LAST. In particular, closing captured File handles and dropping output ownership can
    // never happen after a new mount obtains this SAME physical vault lock.
    _session: VaultPhysicalWorkLease,
}
impl OriginalSourceWriteWork {
    #[cfg(test)]
    pub(in crate::store) fn set_before_io(&self, hook: Option<Box<dyn FnOnce() + Send>>) {
        *self.before_io.lock().expect("test hook lock") = hook;
    }
    #[cfg(test)]
    pub(in crate::store) fn set_after_seed(&self, hook: Box<dyn FnOnce() + Send>) {
        *self.after_seed.lock().expect("test hook lock") = Some(hook);
    }
    #[cfg(test)]
    pub(in crate::store) fn set_before_pages(&self, hook: Box<dyn FnOnce() + Send>) {
        *self.before_pages.lock().expect("test hook lock") = Some(hook);
    }
    fn require_original(&self) -> Result<(), AppError> {
        if self.negative.is_cancelled() || self.root_negative.is_cancelled() {
            return Err(critical_namespaces::refused());
        }
        self.paths.validate()?;
        if self.negative.is_cancelled() || self.root_negative.is_cancelled() {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }
    #[cfg(test)]
    fn fault(&self, point: u8) -> Result<(), AppError> {
        if self.fault == point {
            Err(io("injected original source pass fault"))
        } else {
            Ok(())
        }
    }
    /// Exactly one real pass. No global lock is acquired anywhere in this method, including
    /// its error/Drop paths. Output ownership is released before the caller may reenter Root.
    pub(in crate::store) fn run(self) -> Result<(), AppError> {
        self.run
            .state
            .compare_exchange(PREPARED, RUNNING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| critical_namespaces::refused())?;
        let mut output = self
            .output
            .lock()
            .map_err(|_| critical_namespaces::refused())?;
        #[cfg(test)]
        if let Some(hook) = self
            .before_io
            .lock()
            .map_err(|_| critical_namespaces::refused())?
            .take()
        {
            hook();
        }
        self.require_original()?;
        if output.completed.is_some() {
            return Err(critical_namespaces::refused());
        }
        let source = self
            .payload
            .source
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        let frame = source.as_bytes();
        // Both full verification reads below drain before the three-buffer DATA opener.
        // The complete independently selected seed then permits exact destination repair.
        ensure_large_source_output(
            &self,
            &self.paths.target.join("source.seed"),
            frame,
            &mut output.seed,
            LargeOutputMode::Original,
            #[cfg(test)]
            self.fault,
            #[cfg(test)]
            160,
            #[cfg(test)]
            161,
        )?;
        sync_directory(&self.paths.target).map_err(io)?;
        #[cfg(test)]
        if let Some(hook) = self
            .after_seed
            .lock()
            .map_err(|_| critical_namespaces::refused())?
            .take()
        {
            hook();
        }
        #[cfg(test)]
        self.fault(162)?;
        self.require_original()?;
        ensure_large_source_output(
            &self,
            &self.paths.target.join("source.bin"),
            frame,
            &mut output.source,
            LargeOutputMode::Seeded,
            #[cfg(test)]
            self.fault,
            #[cfg(test)]
            0,
            #[cfg(test)]
            163,
        )?;
        sync_directory(&self.paths.target).map_err(io)?;
        #[cfg(test)]
        self.fault(164)?;
        self.require_original()?;
        let opened = catcoms_sync::owner_v2::open_bootstrap_source_data(
            &self.key,
            source.context_data(),
            source.id(),
            frame,
        )?;
        let view = opened.view_data();
        #[cfg(test)]
        if let Some(hook) = self
            .before_pages
            .lock()
            .map_err(|_| critical_namespaces::refused())?
            .take()
        {
            hook();
        }
        // Offline source decryption, hashing and nested signature verification may wait.
        // Recheck AFTER it, before the first page effect, rather than lending an earlier
        // successful observation to work that only became ready during that wait.
        self.require_original()?;
        ensure_large_source_output(
            &self,
            &self.paths.target.join("records.bin"),
            view.records_frame,
            &mut output.records,
            LargeOutputMode::Seeded,
            #[cfg(test)]
            self.fault,
            #[cfg(test)]
            0,
            #[cfg(test)]
            165,
        )?;
        #[cfg(test)]
        self.fault(166)?;
        self.require_original()?;
        ensure_large_source_output(
            &self,
            &self.paths.target.join("manifest.bin"),
            view.manifest_frame,
            &mut output.manifest,
            LargeOutputMode::Seeded,
            #[cfg(test)]
            self.fault,
            #[cfg(test)]
            0,
            #[cfg(test)]
            167,
        )?;
        #[cfg(test)]
        self.fault(168)?;
        sync_directory(&self.paths.target).map_err(io)?;
        sync_directory(&self.paths.root).map_err(io)?;
        #[cfg(test)]
        self.fault(169)?;
        self.require_original()?;
        let lengths = [
            frame.len() as u64,
            frame.len() as u64,
            view.records_frame.len() as u64,
            view.manifest_frame.len() as u64,
        ];
        validate_output_names(&self.paths.target, &output, lengths)?;
        // Only this actual writer can install the historical completion. The original
        // Root/selected-current/source guard is still mandatory before returning Saved.
        output.completed = Some(CompletedSourceFiles {
            run: self.run.clone(),
            lengths,
        });
        self.run.state.store(DRAINED, Ordering::Release);
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum LargeOutputMode {
    Original,
    Seeded,
}

/// Closed bootstrap destinations only. First source seed uses strict create_new, even for
/// equal foreign bytes. A verified complete original seed permits destination/page prefixes.
/// A held handle is installed BEFORE identity or writing can fail, including a zero-byte file.
fn ensure_large_source_output(
    work: &OriginalSourceWriteWork,
    path: &Path,
    expected: &[u8],
    held: &mut Option<critical_namespaces::pending_join::CreatedStage>,
    mode: LargeOutputMode,
    #[cfg(test)] fault: u8,
    #[cfg(test)] zero_point: u8,
    #[cfg(test)] short_point: u8,
) -> Result<(), AppError> {
    use critical_namespaces::refused;
    use std::io::{Seek, SeekFrom};
    if expected.is_empty()
        || expected.len() > catcoms_sync::owner_v2::MAX_BOOTSTRAP_SOURCE_FRAME_BYTES
    {
        return Err(refused());
    }
    if held.is_none() {
        work.require_original()?;
        let file = match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(io)?,
            Ok(metadata)
                if matches!(mode, LargeOutputMode::Seeded)
                    && metadata.file_type().is_file()
                    && metadata.len() <= expected.len() as u64 =>
            {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)
                    .map_err(io)?
            }
            _ => return Err(refused()),
        };
        *held = Some(critical_namespaces::pending_join::CreatedStage {
            file,
            identity: None,
        });
        #[cfg(test)]
        if zero_point != 0 && fault == zero_point {
            return Err(io("injected empty source seed"));
        }
    }
    let held = held.as_mut().ok_or_else(refused)?;
    if held.identity.is_none() {
        held.identity = Some(catcoms_storage::kept::session_id(&held.file).map_err(io)?);
    }
    verify_archive_held_name(path, held)?;
    held.file.seek(SeekFrom::Start(0)).map_err(io)?;
    // A stack buffer bounds comparison scratch independently of malicious file length. The
    // arm pays ALL full reads, including an already complete file on a later failed pass.
    let length = held.file.metadata().map_err(io)?.len();
    if length > expected.len() as u64 {
        return Err(refused());
    }
    let mut offset = 0usize;
    let mut buffer = [0u8; 8192];
    while offset < expected.len() {
        let count = held.file.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        let next = offset
            .checked_add(count)
            .filter(|n| *n <= expected.len())
            .ok_or_else(refused)?;
        if buffer[..count] != expected[offset..next] {
            return Err(refused());
        }
        offset = next;
    }
    if offset as u64 != length {
        return Err(refused());
    }
    verify_archive_held_name(path, held)?;
    if offset != expected.len() {
        // Prefix comparison may itself read the complete large source. Cancellation/root
        // replacement during that read must precede this NEW write, not only final exposure.
        work.require_original()?;
        held.file.seek(SeekFrom::Start(offset as u64)).map_err(io)?;
        #[cfg(test)]
        if short_point != 0 && fault == short_point {
            let end = offset + (expected.len() - offset) / 2;
            held.file.write_all(&expected[offset..end]).map_err(io)?;
            return Err(io("injected short source output"));
        }
        held.file.write_all(&expected[offset..]).map_err(io)?;
    }
    work.require_original()?;
    held.file.sync_all().map_err(io)?;
    verify_archive_held_name(path, held)
}

fn validate_output_names(
    target: &Path,
    output: &FounderSourceOutput,
    lengths: [u64; 4],
) -> Result<(), AppError> {
    for ((name, held), length) in [
        ("source.seed", &output.seed),
        ("source.bin", &output.source),
        ("records.bin", &output.records),
        ("manifest.bin", &output.manifest),
    ]
    .into_iter()
    .zip(lengths)
    {
        let held = held.as_ref().ok_or_else(critical_namespaces::refused)?;
        verify_archive_held_name(&target.join(name), held)?;
        if held.file.metadata().map_err(io)?.len() != length {
            return Err(critical_namespaces::refused());
        }
    }
    Ok(())
}

fn target_path(store: &ServerStore, actual: &FounderSourceDebt) -> Result<PathBuf, AppError> {
    let spec = actual.context.spec_data();
    let name =
        critical_namespaces::archive_bootstrap::target_name(&spec.vault, spec.group, &spec.signer)?;
    Ok(store
        .dir
        .join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(std::str::from_utf8(&name).map_err(|_| critical_namespaces::refused())?))
}

impl ServerStore {
    /// Resolve an interrupted SMALL successor before preparing another pass. Its abandoned
    /// Work is never revived. A failed/ambiguous arm may therefore consume an allowance unit
    /// without large I/O; it still cannot reset/refund the original persistent counter.
    pub(in crate::store) fn prepare_original_bootstrap_source_write<T, R>(
        &self,
        captured: &CapturedFounderInvocation,
        guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        registered: &RegisteredBootstrapSource,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<OriginalSourceWriteWork, AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: rand_core::CryptoRngCore,
    {
        use critical_namespaces::refused;
        registered.validate_original(self, captured, guard)?;
        let needs_repair = {
            let actual = registered.cell.lock().map_err(|_| refused())?;
            let journal = actual.prepared.as_ref().ok_or_else(refused)?;
            if let Some(run) = &journal.execution {
                if run.state.load(Ordering::Acquire) != ABANDONED {
                    return Err(refused());
                }
            }
            journal.saved.is_none()
        };
        // This is bounded SMALL recovery, never an old large-pass refund/reconstruction.
        if needs_repair {
            self.save_original_source_prepared(captured, guard, registered, rng)?;
        }
        let key = Zeroizing::new(self.keys.db_key()?);
        let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let root = root_guard.as_ref().ok_or_else(refused)?;
        let registry = self.founder_sources.lock().map_err(|_| refused())?;
        if !registry
            .slots
            .iter()
            .flatten()
            .any(|entry| Arc::ptr_eq(entry, &registered.cell))
        {
            return Err(refused());
        }
        let mut actual = registered.cell.lock().map_err(|_| refused())?;
        validate_registered_source_under_root(self, captured, guard, &actual, root)?;
        let parent = actual.prepared.as_ref().ok_or_else(refused)?;
        require_saved_source_journal(parent)?;
        if parent.data.io_spent >= captured.data().io_allowance {
            return Err(refused());
        }
        if parent
            .execution
            .as_ref()
            .is_some_and(|run| run.state.load(Ordering::Acquire) != ABANDONED)
        {
            return Err(refused());
        }
        // Do not wait for the long file-I/O mutex under ANY global writer, even if an earlier
        // state observation claimed the old Work had stopped. Poison/Busy preserves the debt.
        let output = actual.output.try_lock().map_err(|_| refused())?;
        if output.completed.is_some() {
            return Err(refused());
        }
        drop(output);
        let target = target_path(self, &actual)?;
        let intent = actual.founding.debt.lock().map_err(|_| refused())?;
        let original_target = intent.target.as_ref().ok_or_else(refused)?;
        let original_origin = intent.marker_file.as_ref().ok_or_else(refused)?;
        let paths = PhysicalSourcePaths {
            root: self.dir.join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT),
            target,
            root_directory: root.directory.try_clone().map_err(io)?,
            root_id: root.directory_id,
            root_marker: root.marker.try_clone().map_err(io)?,
            root_marker_id: root.marker_id,
            root_digest: root.marker_digest,
            target_directory: original_target
                .directory
                .as_ref()
                .ok_or_else(refused)?
                .try_clone()
                .map_err(io)?,
            target_id: original_target.identity.ok_or_else(refused)?,
            origin: original_origin.file.try_clone().map_err(io)?,
            origin_id: original_origin.identity.ok_or_else(refused)?,
            origin_digest: *blake3::hash(&intent.marker).as_bytes(),
        };
        drop(intent);
        paths.validate()?;
        let operation = Arc::new(());
        let run = Arc::new(FounderSourceRun {
            operation: operation.clone(),
            state: AtomicU8::new(PREPARED),
        });
        let parent = actual.prepared.as_ref().ok_or_else(refused)?;
        let data = founder::IntentData {
            invocation: parent.data.invocation,
            debit: parent.data.debit,
            sequence: parent.data.sequence.checked_add(1).ok_or_else(refused)?,
            parent: Some(founder::IntentId::of(
                parent.candidate.as_ref().ok_or_else(refused)?,
            )),
            io_spent: parent.data.io_spent.checked_add(1).ok_or_else(refused)?,
            kind: parent.data.kind.clone(),
        };
        data.successor(
            Some((&parent.data, data.parent.ok_or_else(refused)?)),
            captured.data(),
            guard.debit_id(),
        )?;
        let mut next = Box::new(FounderSourcePreparedJournal {
            candidate: None,
            seed: None,
            marker: Zeroizing::new(parent.marker.to_vec()),
            data,
            operation,
            admitted: None,
            saved: None,
            seed_file: None,
            stage_file: None,
            parent: None,
            execution: Some(run.clone()),
            terminal: None,
        });
        // All quota/metadata/output/path/session resources precede the irreversible small
        // debit. No quota is obtained by the worker. Already-paid crypto buffers still may
        // allocate/refuse inside that spent pass; there is deliberately no attempt refund.
        let work = OriginalSourceWriteWork {
            paths,
            output: actual.output.clone(),
            run: run.clone(),
            negative: captured.observer().clone(),
            root_negative: root.negative.clone(),
            key,
            payload: actual.payload.as_ref().ok_or_else(refused)?.clone(),
            #[cfg(test)]
            fault: self.critical_namespace_fault.load(Ordering::SeqCst),
            #[cfg(test)]
            before_io: Mutex::new(None),
            #[cfg(test)]
            after_seed: Mutex::new(None),
            #[cfg(test)]
            before_pages: Mutex::new(None),
            _run_end: SourceRunEnd(run),
            _session: self._session.retain_for_physical_work(),
        };
        recheck_source_capture_after_waits(
            self,
            &captured.original,
            root,
            &actual.founding,
            actual.prepared.as_ref(),
        )?;
        next.parent = actual.prepared.take().map(Box::new);
        actual.prepared = Some(*next);
        drop(actual);
        drop(registry);
        drop(root_guard);
        // Losing this return cannot revive the Work: Drop marks it abandoned, while Root
        // retains the complete candidate/seed and the persistent charge for exact repair.
        self.save_original_source_prepared(captured, guard, registered, rng)?;
        Ok(work)
    }

    /// Recover ONLY an actual completed writer outcome, never from file equality or a
    /// parsed saved count. A lost local reply may reobserve this without large verification.
    pub(in crate::store) fn completed_original_bootstrap_source_write<T, R>(
        &self,
        captured: &CapturedFounderInvocation,
        guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        registered: &RegisteredBootstrapSource,
    ) -> Result<Option<SavedBootstrapSourceFiles>, AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: rand_core::CryptoRngCore,
    {
        use critical_namespaces::refused;
        registered.validate_original(self, captured, guard)?;
        let run = {
            let actual = registered.cell.lock().map_err(|_| refused())?;
            let Some(run) = actual
                .prepared
                .as_ref()
                .and_then(|journal| journal.execution.as_ref())
            else {
                return Ok(None);
            };
            if run.state.load(Ordering::Acquire) != DRAINED {
                return Ok(None);
            }
            run.clone()
        };
        let saved = SavedBootstrapSourceFiles {
            cell: registered.cell.clone(),
            run,
        };
        saved.validate_original(self, captured, guard, registered)?;
        Ok(Some(saved))
    }
}
impl SavedBootstrapSourceFiles {
    pub(in crate::store) fn validate_original<T, R>(
        &self,
        store: &ServerStore,
        captured: &CapturedFounderInvocation,
        guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        registered: &RegisteredBootstrapSource,
    ) -> Result<(), AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: rand_core::CryptoRngCore,
    {
        use critical_namespaces::refused;
        if !Arc::ptr_eq(&self.cell, &registered.cell) {
            return Err(refused());
        }
        let root_guard = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let root = root_guard.as_ref().ok_or_else(refused)?;
        let registry = store.founder_sources.lock().map_err(|_| refused())?;
        if !registry
            .slots
            .iter()
            .flatten()
            .any(|entry| Arc::ptr_eq(entry, &self.cell))
        {
            return Err(refused());
        }
        let actual = self.cell.lock().map_err(|_| refused())?;
        let journal = actual.prepared.as_ref().ok_or_else(refused)?;
        require_saved_source_journal(journal)?;
        if !Arc::ptr_eq(&journal.operation, &self.run.operation)
            || !journal
                .execution
                .as_ref()
                .is_some_and(|run| Arc::ptr_eq(run, &self.run))
            || self.run.state.load(Ordering::Acquire) != DRAINED
        {
            return Err(refused());
        }
        validate_registered_source_under_root(store, captured, guard, &actual, root)?;
        let output = actual.output.try_lock().map_err(|_| refused())?;
        let completed = output.completed.as_ref().ok_or_else(refused)?;
        if !Arc::ptr_eq(&completed.run, &self.run) {
            return Err(refused());
        }
        #[cfg(test)]
        if let Some(hook) = store
            .founder_before_source_files_saved
            .lock()
            .map_err(|_| refused())?
            .take()
        {
            hook();
        }
        recheck_source_capture_after_waits(
            store,
            &captured.original,
            root,
            &actual.founding,
            actual.prepared.as_ref(),
        )?;
        // No new writer may begin while this same cell is held. All source/current/Root
        // comparers and final hooks precede the LAST output-object proof. No hook/comparer/
        // quota wait follows it; only cheap original negatives may precede the return.
        // ALL full-frame read/hash/AEAD work was in the spent physical pass.
        validate_output_names(&target_path(store, &actual)?, &output, completed.lengths)?;
        if captured.original.negative.is_cancelled() || root.negative.is_cancelled() {
            return Err(refused());
        }
        Ok(())
    }
}
