//! New paid cold bootstrap DATA, never a reconstructed warm source/arm or Current. Every
//! filesystem operation here is read-only. The full read/opener/page comparisons run outside
//! Root; final original small-control/named-object proof runs after all that work.
use super::*;
use catcoms_storage::private_chat_page::MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES;
use catcoms_sync::owner_v2::{
    BootstrapSourceContextData, BootstrapSourceId, OpenedBootstrapSourceData,
};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::Weak;
mod genesis_seed;
pub(in crate::store) mod v2_genesis_activation;
pub(in crate::store) use genesis_seed::{GenesisCurrentOwnerRef, GenesisSourceDataRef};
pub use genesis_seed::{
    GenesisSeedBudget, GenesisSeedRefusal, PreparedColdV2GenesisCurrent, PreparedV2GenesisCurrent,
    RetainedBootstrapGenesisSeed, ReturnedBootstrapSourceData, UnexposedColdV2GenesisCurrent,
    UnexposedV2GenesisCurrent,
};

const MAX_PATH: usize = 32 * 1024;
const FIXED_FEE: usize = 1024 * 1024;
const READ_FEE: usize = 3 * catcoms_sync::owner_v2::MAX_BOOTSTRAP_SOURCE_FRAME_BYTES + FIXED_FEE;
#[cfg(test)]
thread_local! {
    static READ_BUFFER_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct ReadFile {
    file: File,
    id: catcoms_storage::kept::SessionId,
    len: usize,
}
struct ReadControl {
    object: ReadFile,
    frame: Zeroizing<Vec<u8>>,
}
/// Paths have a checked bound; all other names derive from these two original paths and
/// the authenticated invocation. No caller path, scan, key, session or sender is retained.
struct ReadCapture {
    identity: Arc<()>,
    root: PathBuf,
    target: PathBuf,
    root_directory: File,
    root_id: catcoms_storage::kept::SessionId,
    target_directory: File,
    target_id: catcoms_storage::kept::SessionId,
    root_marker: ReadControl,
    origin: ReadControl,
    debit: ReadControl,
    terminal: ReadControl,
    source: ReadFile,
    records: ReadFile,
    manifest: ReadFile,
    invocation: founder::InvocationData,
    context: BootstrapSourceContextData,
    source_id: BootstrapSourceId,
    mount: Arc<()>,
    negative: catcoms_rt::RequestCancellation,
    root_negative: catcoms_rt::RequestCancellation,
    consumed: AtomicBool,
    // LAST: all Files, exact frames, context/paths and returned opened DATA drain first.
    fee: Arc<critical_namespaces::read_pool::CatalogReadReservation>,
}

/// Retained authenticated source plus both separate genesis artifacts, with its actual
/// ORIGINAL read payment. This is historical private DATA only, not a latest-host selector,
/// SavedTerminal, Fresh, SDK admission, Current or native/UI disclosure lease. No Serialize.
pub struct RetainedBootstrapSourceData {
    // This includes the complete fixed-size frontier metadata. Prepay its Box in READ_FEE
    // before the opener, keeping later owning refusals small WITHOUT allocating an error
    // after a reservation failure. The SAME Box moves through every downstream refusal.
    opened: Box<OpenedBootstrapSourceData>,
    capture: Arc<ReadCapture>,
}
const _: () =
    assert!(std::mem::size_of::<RetainedBootstrapSourceData>() <= 2 * std::mem::size_of::<usize>());
/// Weak original READ identity for a retained group task. Matching whole ciphertext alone
/// cannot swap that task to a different paid capture/held-object set. This observation keeps
/// no DATA, fee, sender, key, Store or session alive and grants no positive permission.
#[derive(Clone)]
pub(in crate::store) struct OriginalBootstrapReadObservation {
    identity: Weak<()>,
}
/// Conservative named control-block/observer allowance, not a physical RSS promise. Any
/// owning task which can outlive the actual read must pay this separately: a Weak keeps its
/// tiny identity allocation alive after the source fee drains, never the ReadCapture itself.
pub(in crate::store) const ORIGINAL_BOOTSTRAP_READ_OBSERVATION_BYTES: usize =
    std::mem::size_of::<OriginalBootstrapReadObservation>() + 64;
impl OriginalBootstrapReadObservation {
    pub(in crate::store) fn require_same_original(
        &self,
        source: &RetainedBootstrapSourceData,
    ) -> Result<(), AppError> {
        let original = self
            .identity
            .upgrade()
            .ok_or_else(critical_namespaces::refused)?;
        if !Arc::ptr_eq(&original, &source.capture.identity) {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }
}
impl std::fmt::Debug for RetainedBootstrapSourceData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetainedBootstrapSourceData")
            .finish_non_exhaustive()
    }
}
impl RetainedBootstrapSourceData {
    /// Original physical lifetime observation ONLY. Safe while an actual writer already
    /// holds Root: no host writer, fee/pool access, filesystem, codec/SDK or new owner is involved.
    /// Full original control/name/resource validation remains separately mandatory before I/O.
    pub(in crate::store) fn require_original_negative(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        self.capture.require_open()?;
        if !Arc::ptr_eq(&self.capture.mount, &store.registry_mount()) {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }

    pub(in crate::store) fn observe_original(&self) -> OriginalBootstrapReadObservation {
        OriginalBootstrapReadObservation {
            identity: Arc::downgrade(&self.capture.identity),
        }
    }
    /// Borrow only while the SAME actual source-read fee remains retained. A downstream
    /// seed/SDK builder must separately pay/retain its own graph and dependent metadata copies.
    /// This DATA accessor cannot authorize a host, write or IPC disclosure after UI lock.
    pub fn data(&self) -> &OpenedBootstrapSourceData {
        &self.opened
    }

    /// Reobserve the original physical capture before a later consumer. This performs only
    /// bounded small-control/object checks; it never reruns large verification under Root.
    pub fn validate_original(&self, store: &ServerStore) -> Result<(), AppError> {
        self.capture.validate_policy(store)?;
        let root_guard = store
            .archive_bootstrap_root
            .lock()
            .map_err(|_| critical_namespaces::refused())?;
        let root = root_guard
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        self.validate_under_archive_root(store, root)
    }
    /// Non-relocking variant for a concrete writer ALREADY holding this Store's archive
    /// Root after admission/pins. This validates DATA, never Current or publication.
    pub(in crate::store) fn validate_under_archive_root(
        &self,
        store: &ServerStore,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        self.capture.validate_against_root(store, root)
    }
}

struct ReadWork {
    // Only this real bounded running read owns a key. No OS lease is needed: no create,
    // write, sync, unlink, repair or queued mutation can happen, including its Drop paths.
    key: Zeroizing<[u8; 32]>,
    #[cfg(test)]
    before_read: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(test)]
    after_open: Option<Box<dyn FnOnce() + Send>>,
    // LAST: key/continuations drain before the capture's possibly last real fee owner.
    capture: Arc<ReadCapture>,
}
fn io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

/// No implicit Vec growth, including the EOF probe. One exact preallocated buffer holds at
/// most expected+1 bytes; oversized/truncated/replaced files refuse before parsing/decryption.
fn read_exact_frame(object: &ReadFile, max: usize) -> Result<Zeroizing<Vec<u8>>, AppError> {
    use critical_namespaces::refused;
    use std::io::{Seek, SeekFrom};
    if object.len == 0 || object.len > max {
        return Err(refused());
    }
    #[cfg(test)]
    READ_BUFFER_ENTRIES.with(|entries| entries.set(entries.get() + 1));
    let capacity = object.len.checked_add(1).ok_or_else(refused)?;
    let mut file = object.file.try_clone().map_err(io)?;
    file.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut bytes = Zeroizing::new(vec![0; capacity]);
    let mut used = 0usize;
    while used < capacity {
        let read = file.read(&mut bytes[used..]).map_err(io)?;
        if read == 0 {
            break;
        }
        used = used
            .checked_add(read)
            .filter(|n| *n <= capacity)
            .ok_or_else(refused)?;
    }
    if used != object.len {
        return Err(refused());
    }
    bytes.truncate(used);
    Ok(bytes)
}
fn open_read_file(path: &Path, max: usize) -> Result<ReadFile, AppError> {
    use critical_namespaces::refused;
    let metadata = fs::symlink_metadata(path).map_err(io)?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > max as u64 {
        return Err(refused());
    }
    let file = File::open(path).map_err(io)?;
    let metadata = file.metadata().map_err(io)?;
    let len = usize::try_from(metadata.len()).map_err(|_| refused())?;
    if !metadata.file_type().is_file() || len == 0 || len > max {
        return Err(refused());
    }
    let id = catcoms_storage::kept::session_id(&file).map_err(io)?;
    let object = ReadFile { file, id, len };
    verify_read_name(path, &object)?;
    Ok(object)
}
fn verify_read_name(path: &Path, object: &ReadFile) -> Result<(), AppError> {
    if catcoms_storage::kept::session_id(&object.file).map_err(io)? != object.id
        || !matches!(
            catcoms_storage::kept::session_at(path, object.id),
            catcoms_storage::kept::SessionAt::Ours
        )
        || object.file.metadata().map_err(io)?.len() != object.len as u64
    {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}
fn read_control(path: &Path, max: usize) -> Result<ReadControl, AppError> {
    let object = open_read_file(path, max)?;
    let frame = read_exact_frame(&object, max)?;
    verify_read_name(path, &object)?;
    Ok(ReadControl { object, frame })
}
fn verify_control(path: &Path, original: &ReadControl) -> Result<(), AppError> {
    verify_read_name(path, &original.object)?;
    let current = read_control(path, original.frame.len())?;
    if current.object.id != original.object.id
        || current.frame.as_slice() != original.frame.as_slice()
    {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}
fn require_absent(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(critical_namespaces::refused()),
    }
}
fn control_path(
    root: &Path,
    invocation: &founder::InvocationData,
    slot: FounderControlSlot,
) -> Result<PathBuf, AppError> {
    let name = invocation.name();
    let name = std::str::from_utf8(&name).map_err(|_| critical_namespaces::refused())?;
    Ok(root.join(format!("{}{}", name, slot.suffix())))
}

impl ReadCapture {
    fn require_open(&self) -> Result<(), AppError> {
        if self.negative.is_cancelled() || self.root_negative.is_cancelled() {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }
    fn verify_selected_files(&self) -> Result<(), AppError> {
        for (path, held, id) in [
            (&self.root, &self.root_directory, self.root_id),
            (&self.target, &self.target_directory, self.target_id),
        ] {
            let observed = open_archive_directory(path)?;
            if catcoms_storage::kept::session_id(held).map_err(io)? != id
                || catcoms_storage::kept::session_id(&observed).map_err(io)? != id
            {
                return Err(critical_namespaces::refused());
            }
        }
        verify_control(
            &self
                .root
                .join(critical_namespaces::archive_bootstrap::ROOT_MARKER),
            &self.root_marker,
        )?;
        verify_control(&self.target.join("origin"), &self.origin)?;
        verify_control(
            &control_path(&self.root, &self.invocation, FounderControlSlot::Debit)?,
            &self.debit,
        )?;
        verify_control(
            &control_path(&self.root, &self.invocation, FounderControlSlot::Intent)?,
            &self.terminal,
        )?;
        require_absent(&control_path(
            &self.root,
            &self.invocation,
            FounderControlSlot::Stage,
        )?)?;
        require_absent(&control_path(
            &self.root,
            &self.invocation,
            FounderControlSlot::Seed,
        )?)?;
        // LAST object proof; no hooks, quota or source comparers may follow these names.
        verify_read_name(&self.target.join("source.bin"), &self.source)?;
        verify_read_name(&self.target.join("records.bin"), &self.records)?;
        verify_read_name(&self.target.join("manifest.bin"), &self.manifest)?;
        self.require_open()
    }
    fn validate_policy(&self, store: &ServerStore) -> Result<(), AppError> {
        use critical_namespaces::refused;
        self.require_open()?;
        let policy = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?;
        policy.source_read.as_ref().ok_or_else(refused)?;
        self.fee.validate(&policy.pending.pool, READ_FEE)
    }
    fn validate_against_root(
        &self,
        store: &ServerStore,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        use critical_namespaces::refused;
        // The already-borrowed Root caller cannot rely on a resource observation sampled
        // before its own writer waits. Repeat it, then prove the actual files afterward.
        self.validate_policy(store)?;
        if !Arc::ptr_eq(&self.mount, &store.registry_mount())
            || !Arc::ptr_eq(&self.mount, &root.mount)
            || root.directory_id != self.root_id
            || root.marker_id != self.root_marker.object.id
            || root.descriptor.vault != self.invocation.vault
            || root.descriptor.root != self.invocation.root
        {
            return Err(refused());
        }
        validate_archive_root_session(&self.root, root)?;
        self.verify_selected_files()?;
        if root.negative.is_cancelled() {
            return Err(refused());
        }
        Ok(())
    }
}

impl ReadWork {
    fn run(self) -> Result<RetainedBootstrapSourceData, AppError> {
        use critical_namespaces::refused;
        self.capture
            .consumed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        #[cfg(test)]
        if let Some(hook) = self.before_read {
            hook();
        }
        self.capture.require_open()?;
        let raw = read_exact_frame(
            &self.capture.source,
            catcoms_sync::owner_v2::MAX_BOOTSTRAP_SOURCE_FRAME_BYTES,
        )?;
        let opened = catcoms_sync::owner_v2::open_bootstrap_source_data(
            &self.key,
            &self.capture.context,
            self.capture.source_id,
            &raw,
        )?;
        drop(raw);
        #[cfg(test)]
        if let Some(hook) = self.after_open {
            hook();
        }
        self.capture.require_open()?;
        let view = opened.view_data();
        for (file, expected) in [
            (&self.capture.records, view.records_frame),
            (&self.capture.manifest, view.manifest_frame),
        ] {
            if file.len != expected.len() {
                return Err(refused());
            }
            let frame = read_exact_frame(file, MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES)?;
            if frame.as_slice() != expected {
                return Err(refused());
            }
            self.capture.require_open()?;
        }
        // Read-only final physical proof, then a later Store-specific original check before
        // the public helper returns. All full reads/opener work has already drained.
        self.capture.verify_selected_files()?;
        Ok(RetainedBootstrapSourceData {
            opened: Box::new(opened),
            capture: self.capture,
        })
    }
}

impl ServerStore {
    /// Read one exact closed historical source after a physical open configured for paid
    /// source DATA. No SDK, repair, host selection, Fresh, Current or native publication occurs.
    pub fn read_saved_bootstrap_source_data(
        &self,
        group: &[u8],
        signer: [u8; 32],
    ) -> Result<RetainedBootstrapSourceData, AppError> {
        let work = self.capture_bootstrap_source_read(group, signer)?;
        // The entire Root guard from capture has drained before any large I/O or AEAD.
        let data = work.run()?;
        data.validate_original(self)?;
        Ok(data)
    }

    fn capture_bootstrap_source_read(
        &self,
        group: &[u8],
        signer: [u8; 32],
    ) -> Result<ReadWork, AppError> {
        use critical_namespaces::{archive_bootstrap as archive, refused};
        if group.is_empty() || group.len() > 256 || self.dir.as_os_str().len() > MAX_PATH {
            return Err(refused());
        }
        let policy = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
        policy.source_read.as_ref().ok_or_else(refused)?;
        validate_fixed_invoice()?;
        // Before ANY small/large data buffer, not a scalar promise decoded from a terminal.
        // Keep this original fee alias OUTSIDE Root so early refusals cannot drop its last
        // owner/reenter the pool while a global writer is held.
        let fee = Arc::new(policy.pending.pool.try_reserve(READ_FEE)?);
        let key = Zeroizing::new(self.keys.db_key()?);
        let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let root = root_guard.as_ref().ok_or_else(refused)?;
        let root_path = self.dir.join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT);
        validate_archive_root_session(&root_path, root)?;
        if root.negative.is_cancelled() {
            return Err(refused());
        }
        let name = archive::target_name(&root.descriptor.vault, group, &signer)?;
        let target = root_path.join(std::str::from_utf8(&name).map_err(|_| refused())?);
        let target_directory = open_archive_directory(&target)?;
        let target_id = catcoms_storage::kept::session_id(&target_directory).map_err(io)?;
        let origin = read_control(&target.join("origin"), founder::MAX_TARGET_MARKER_FRAME)?;
        let (invocation, debit_id) = founder::open_target_marker_locator_data(
            &key,
            &origin.frame,
            root.descriptor.vault,
            root.descriptor.root,
            group,
            signer,
        )?;
        require_absent(&control_path(
            &root_path,
            &invocation,
            FounderControlSlot::Stage,
        )?)?;
        require_absent(&control_path(
            &root_path,
            &invocation,
            FounderControlSlot::Seed,
        )?)?;
        let debit = read_control(
            &control_path(&root_path, &invocation, FounderControlSlot::Debit)?,
            founder::MAX_DEBIT_FRAME,
        )?;
        if founder::DebitId::of(&debit.frame) != debit_id {
            return Err(refused());
        }
        let actual_debit = founder::open_debit_data(&key, invocation, &debit.frame)?;
        let marker =
            founder::open_target_marker_data(&key, &actual_debit, debit_id, &origin.frame)?;
        let terminal = read_control(
            &control_path(&root_path, &invocation, FounderControlSlot::Intent)?,
            founder::MAX_INTENT_FRAME,
        )?;
        let data = founder::open_intent_data(&key, &actual_debit, debit_id, &terminal.frame)?;
        data.matches_marker(&marker, founder::TargetMarkerId::of(&origin.frame))?;
        let founder::IntentKind::SourceSaved {
            context,
            source: source_id,
            ..
        } = data.kind
        else {
            return Err(refused());
        };
        let source = open_read_file(
            &target.join("source.bin"),
            catcoms_sync::owner_v2::MAX_BOOTSTRAP_SOURCE_FRAME_BYTES,
        )?;
        let records = open_read_file(
            &target.join("records.bin"),
            MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES,
        )?;
        let manifest = open_read_file(
            &target.join("manifest.bin"),
            MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES,
        )?;
        let root_marker = read_control(
            &root_path.join(archive::ROOT_MARKER),
            archive::MAX_MARKER_FRAMED,
        )?;
        if root_marker.object.id != root.marker_id
            || *blake3::hash(&root_marker.frame).as_bytes() != root.marker_digest
        {
            return Err(refused());
        }
        let capture = Arc::new(ReadCapture {
            identity: Arc::new(()),
            root: root_path,
            target,
            root_directory: root.directory.try_clone().map_err(io)?,
            root_id: root.directory_id,
            target_directory,
            target_id,
            root_marker,
            origin,
            debit,
            terminal,
            source,
            records,
            manifest,
            invocation,
            context,
            source_id,
            mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            root_negative: root.negative.clone(),
            consumed: AtomicBool::new(false),
            fee: fee.clone(),
        });
        // No full producer/SDK operation follows this exact final selected-file proof.
        capture.verify_selected_files()?;
        drop(root_guard);
        drop(fee);
        Ok(ReadWork {
            capture,
            key,
            #[cfg(test)]
            before_read: None,
            #[cfg(test)]
            after_open: None,
        })
    }
}

fn validate_fixed_invoice() -> Result<(), AppError> {
    // Actual three-buffer opener peak + a SERIAL external page read, named nested public
    // codec scratch, four bounded path-copy overlaps, exact small controls and metadata.
    let named = catcoms_sync::owner_v2::BOOTSTRAP_SOURCE_NESTED_DATA_INVOICE
        + MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES
        + 1
        // A cloned PathBuf followed by join/push may double its requested capacity.
        // Pay four simultaneous complete paths at that peak, not just their byte lengths.
        + 2 * 4 * (MAX_PATH + 256)
        + 2 * critical_namespaces::archive_bootstrap::MAX_MARKER_FRAMED
        + 4 * founder::MAX_TARGET_MARKER_FRAME
        + 4 * founder::MAX_INTENT_FRAME
        + 4 * founder::MAX_DEBIT_FRAME
        + std::mem::size_of::<ReadCapture>()
        + std::mem::size_of::<ReadWork>()
        + std::mem::size_of::<RetainedBootstrapSourceData>()
        + std::mem::size_of::<OpenedBootstrapSourceData>() // prepaid retained metadata Box
        + ORIGINAL_BOOTSTRAP_READ_OBSERVATION_BYTES
        + 8 * 1024;
    if named > FIXED_FEE {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
