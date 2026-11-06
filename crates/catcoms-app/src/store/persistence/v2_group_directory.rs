//! Exact original group-directory I/O paired with the concrete original task, registry and
//! actual admission guards. No decoded record constructs Saved.
use super::*;
use crate::store::critical_namespaces::{
    archive_current_namespace_codec::CurrentLayoutData,
    current_layout::{OriginalV2GenesisGroupObservation, PreparedV2GenesisGroupNamespace},
    read_pool::CatalogReadReservation,
};
use catcoms_storage::kept::SessionId;
use catcoms_sync::owner_v2::BootstrapSourceId;
use std::sync::{Arc, Mutex};

#[cfg(test)]
mod cold_read;
#[cfg(test)]
pub(in crate::store) use cold_read::cold_group_read_entries_for_test;
#[cfg(test)]
pub(in crate::store) use cold_read::{
    OpenedColdV2GroupDirectory, COLD_V2_GROUP_DIRECTORY_READ_BYTES,
};

const MAX_GROUP_PATH_BYTES: usize =
    crate::store::critical_namespaces::current_layout::MAX_OPEN_BASE_PATH + 256;

enum DirectoryProgress {
    Absent,
    // A successful mkdir followed by a failed first open has no held identity. Preserve it;
    // path equality alone cannot turn a later object into our original directory.
    CreatedWithoutHandle,
    Created {
        file: File,
        identity: Option<SessionId>,
    },
    Saved {
        file: File,
        identity: SessionId,
    },
}

/// Inert shared output, installed in Store before the first mkdir. It contains neither a
/// sender/life nor a source graph/key/Store/session. Private enum states record ACTUAL local
/// syscall/barrier outcomes only; they cannot be decoded, set by callers or issue Current.
pub(in crate::store) struct V2GenesisDirectoryCustody {
    path: PathBuf,
    layout: CurrentLayoutData,
    source: BootstrapSourceId,
    mount: Arc<()>,
    progress: Mutex<DirectoryProgress>,
    // LAST even when only Store's inert registry entry survives the original task.
    _fee: Arc<CatalogReadReservation>,
}

/// Actual immutable directory+parent-barrier receipt. The concrete engine alone constructs
/// it from its SAME saved held File. No receipt back-edge retains the mutable output cell.
pub(in crate::store) struct SavedV2GroupDirectory {
    path: PathBuf,
    file: File,
    identity: SessionId,
    layout: CurrentLayoutData,
    source: BootstrapSourceId,
    mount: Arc<()>,
    task: OriginalV2GenesisGroupObservation,
    // LAST: path/File/weak task observation drain before the actual shared task payment.
    _fee: Arc<CatalogReadReservation>,
}
impl std::fmt::Debug for SavedV2GroupDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedV2GroupDirectory")
            .finish_non_exhaustive()
    }
}

/// Only named Rust buffers/metadata, not native/allocator/RSS or a whole-host allowance.
/// Task preparation adds its OWN stamp/life/registry metadata and weak original observers.
pub(in crate::store) const V2_GROUP_DIRECTORY_METADATA_BYTES: usize = std::mem::size_of::<V2GenesisDirectoryCustody>()
        + std::mem::size_of::<SavedV2GroupDirectory>()
        + std::mem::size_of::<DirectoryProgress>()
        + 4 * MAX_GROUP_PATH_BYTES // output, receipt, old-parent and one joined old-name probe
        + 32 * 1024 // native fixed-path conversion, separately bounded from retained paths
        + 4 * std::mem::size_of::<fs::Metadata>()
        + 4 * std::mem::size_of::<File>()
        + std::mem::size_of::<blake3::Hasher>()
        + 2 * std::mem::size_of::<String>() + 64 + 128 // fixed hex key + suffixed filename
        + 2 * 66 // original lower-hex group selector and its fixed g- prefix
        + 3 * (2 * std::mem::size_of::<usize>()); // custody, receipt and fee Arc headers

fn original_group_path(
    store: &ServerStore,
    layout: CurrentLayoutData,
    group: &[u8],
) -> Result<PathBuf, AppError> {
    use critical_namespaces::{current_group, refused, ARCHIVE_BOOTSTRAP_ROOT};
    if store.dir.as_os_str().len() > critical_namespaces::current_layout::MAX_OPEN_BASE_PATH {
        return Err(refused());
    }
    let suffix = current_group::group_suffix(&layout.vault, &layout.root, group)?;
    let mut name = [0; 66];
    name[..2].copy_from_slice(b"g-");
    name[2..].copy_from_slice(&suffix);
    let name = std::str::from_utf8(&name).map_err(|_| refused())?;
    let capacity = store
        .dir
        .as_os_str()
        .len()
        .checked_add(ARCHIVE_BOOTSTRAP_ROOT.len() + 1 + name.len() + 1)
        .filter(|n| *n <= MAX_GROUP_PATH_BYTES)
        .ok_or_else(refused)?;
    let mut path = PathBuf::with_capacity(capacity);
    path.push(&store.dir);
    path.push(ARCHIVE_BOOTSTRAP_ROOT);
    path.push(name);
    if path.as_os_str().len() > capacity || path.capacity() > MAX_GROUP_PATH_BYTES {
        return Err(refused());
    }
    Ok(path)
}
fn directory_io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}
fn require_same_directory(path: &Path, held: &File, identity: SessionId) -> Result<(), AppError> {
    // kept::session_at is a FILE-only name proof and deliberately classifies directories as
    // Other. Open the fixed named directory with the actual no-reparse directory helper and
    // compare both held and named IDs instead; never weaken file proofs to cover this case.
    let named = open_archive_directory(path)?;
    if !archive_real_directory(&fs::symlink_metadata(path).map_err(directory_io)?)
        || catcoms_storage::kept::session_id(held).map_err(directory_io)? != identity
        || catcoms_storage::kept::session_id(&named).map_err(directory_io)? != identity
    {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}
impl ServerStore {
    /// DATA/resource preparation before admission/pins/canonical/Root. The path is selected
    /// solely from actual mounted Store and original bounded source, never caller path text.
    pub(in crate::store) fn prepare_v2_genesis_directory_output(
        &self,
        source: &GenesisSourceDataRef<'_>,
        layout: CurrentLayoutData,
        fee: Arc<CatalogReadReservation>,
    ) -> Result<Arc<V2GenesisDirectoryCustody>, AppError> {
        let policy = self
            .current_layout_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        fee.validate(
            &policy.archive.pending.pool,
            V2_GROUP_DIRECTORY_METADATA_BYTES,
        )?;
        source.require_original_negative(self)?;
        source
            .seed()
            .check_opened_source_data(source.source().data())?;
        if source.data().context.spec_data().vault != layout.vault {
            return Err(critical_namespaces::refused());
        }
        let path = original_group_path(self, layout, source.data().context.spec_data().group)?;
        Ok(Arc::new(V2GenesisDirectoryCustody {
            path,
            layout,
            source: source.data().source,
            mount: self.registry_mount(),
            progress: Mutex::new(DirectoryProgress::Absent),
            _fee: fee,
        }))
    }

    /// Actual first-directory effects under the SAME original vacant-row/task/pins/canonical/
    /// Root closure. The private output lock is acquired BEFORE mkdir; no lock/allocation can
    /// fail between successful creation and recording CreatedWithoutHandle.
    pub(in crate::store) fn write_v2_genesis_group_directory(
        &self,
        guard: &critical_namespaces::current_layout::CreatingV2GenesisDirectoryGuard<'_, '_>,
    ) -> Result<(), AppError> {
        use critical_namespaces::refused;
        let task = guard.task();
        let output = task.directory_output();
        output.require_same_data(self, guard.source(), task)?;
        guard.validate_source_only()?;
        self.validate_mounted_current_layout_under_archive_root(guard.archive_root())?;
        let mut progress = output.progress.try_lock().map_err(|_| refused())?;
        if matches!(*progress, DirectoryProgress::CreatedWithoutHandle) {
            // No retained directory identity exists. Preserve the fixed object and debt;
            // opening an equal path on retry would be an unsupported ownership adoption.
            return Err(refused());
        }
        if matches!(*progress, DirectoryProgress::Absent) {
            match fs::symlink_metadata(&output.path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(refused()),
            }
            guard.validate_source_only()?;
            self.validate_mounted_current_layout_under_archive_root(guard.archive_root())?;
            fs::create_dir(&output.path).map_err(directory_io)?;
            *progress = DirectoryProgress::CreatedWithoutHandle;
            self.v2_group_directory_fault(251)?;
            let file = open_created_group_directory(&output.path)?;
            // Cache BEFORE metadata/session identity/sync. If any later query fails, the
            // next retry may inspect this SAME File, never reopen by its name.
            *progress = DirectoryProgress::Created {
                file,
                identity: None,
            };
            self.v2_group_directory_fault(252)?;
        }
        match &mut *progress {
            DirectoryProgress::Created { file, identity } => {
                if identity.is_none() {
                    if !archive_real_directory(&file.metadata().map_err(directory_io)?) {
                        return Err(refused());
                    }
                    *identity =
                        Some(catcoms_storage::kept::session_id(file).map_err(directory_io)?);
                }
                let original = identity.ok_or_else(refused)?;
                require_same_directory(&output.path, file, original)?;
                guard.validate_source_only()?;
                self.validate_mounted_current_layout_under_archive_root(guard.archive_root())?;
                file.sync_all().map_err(directory_io)?;
                self.v2_group_directory_fault(253)?;
                guard
                    .archive_root()
                    .directory
                    .sync_all()
                    .map_err(directory_io)?;
                self.v2_group_directory_fault(254)?;
                self.validate_mounted_current_layout_under_archive_root(guard.archive_root())?;
                #[cfg(test)]
                run_final_directory_hook(
                    &self.owner_evidence.before_v2_group_final_directory_proof,
                )?;
                require_same_directory(&output.path, file, original)?;
                guard.validate_source_only()?;
            }
            DirectoryProgress::Saved { file, identity } => {
                self.validate_mounted_current_layout_under_archive_root(guard.archive_root())?;
                #[cfg(test)]
                run_final_directory_hook(
                    &self.owner_evidence.before_v2_group_final_directory_proof,
                )?;
                require_same_directory(&output.path, file, *identity)?;
                guard.validate_source_only()?;
                return Ok(());
            }
            DirectoryProgress::Absent | DirectoryProgress::CreatedWithoutHandle => {
                return Err(refused())
            }
        }
        // All required syscalls and final originals have completed. Recording immutable saved
        // DATA is infallible; collection/Arc/path allocation happens AFTER global guards drain.
        let created = std::mem::replace(&mut *progress, DirectoryProgress::CreatedWithoutHandle);
        if let DirectoryProgress::Created {
            file,
            identity: Some(identity),
        } = created
        {
            *progress = DirectoryProgress::Saved { file, identity };
        } else {
            // Exhaustive local invariant protection; no unexpected state may be called Saved.
            *progress = created;
            return Err(refused());
        }
        Ok(())
    }

    fn v2_group_directory_fault(&self, point: u8) -> Result<(), AppError> {
        #[cfg(test)]
        if self.critical_namespace_fault.load(Ordering::SeqCst) == point {
            return Err(AppError::Io(
                "injected original group-directory stop".into(),
            ));
        }
        let _ = point;
        Ok(())
    }
}
/// Unlike the generic directory opener, do NOT query metadata after opening but before
/// returning the File: the caller must first place it in the original inert output cell.
fn open_created_group_directory(path: &Path) -> Result<File, AppError> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(directory_io)
    }
    #[cfg(unix)]
    {
        File::open(path).map_err(directory_io)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(critical_namespaces::refused())
    }
}
impl V2GenesisDirectoryCustody {
    #[cfg(test)]
    pub(in crate::store) fn path_for_test(&self) -> &Path {
        &self.path
    }
    pub(in crate::store) fn require_same_data(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        task.retains_original(store, source)?;
        if !Arc::ptr_eq(&self.mount, &store.registry_mount())
            || self.layout != task.layout_data()
            || self.source != task.source_id_data()
            || !std::ptr::eq(self, task.directory_output().as_ref())
        {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }

    /// Collect only the actual saved original outcome AFTER the global namespace guards.
    /// The first clone/path allocation can fail without losing the Store-retained File/state.
    /// The receipt retains no back-edge into this cell and cannot manufacture source liveness.
    pub(in crate::store) fn collect_saved_directory(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<Arc<SavedV2GroupDirectory>, AppError> {
        use critical_namespaces::refused;
        if !std::ptr::eq(self, task.directory_output().as_ref())
            || self.layout != task.layout_data()
            || self.source != task.source_id_data()
        {
            return Err(refused());
        }
        let progress = self.progress.try_lock().map_err(|_| refused())?;
        let DirectoryProgress::Saved { file, identity } = &*progress else {
            return Err(refused());
        };
        let cloned = file.try_clone().map_err(directory_io)?;
        let path = self.path.clone();
        if path.capacity() > MAX_GROUP_PATH_BYTES {
            return Err(refused());
        }
        Ok(Arc::new(SavedV2GroupDirectory {
            path,
            file: cloned,
            identity: *identity,
            layout: self.layout,
            source: self.source,
            mount: self.mount.clone(),
            task: task.observe_original(),
            _fee: self._fee.clone(),
        }))
    }
}
impl SavedV2GroupDirectory {
    /// Cached exact-original comparison for Source/Scope receipt validation. No locks, I/O,
    /// SDK/encoding, allocation or pool operations; it grants no present file durability.
    pub(in crate::store) fn retains_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        self.task.require_same_original(task)?;
        task.retains_original(store, source)?;
        if !Arc::ptr_eq(&self.mount, &store.registry_mount())
            || self.layout != task.layout_data()
            || self.source != task.source_id_data()
            || self.path != task.directory_output().path
        {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }

    /// Immutable borrowed physical DATA only, available after the genuine covering barrier.
    pub(in crate::store) fn directory_data(&self) -> (&Path, &File, SessionId) {
        (&self.path, &self.file, self.identity)
    }
    pub(in crate::store) fn validate_under_archive_root(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        let policy = store
            .current_layout_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        self._fee.validate(
            &policy.archive.pending.pool,
            V2_GROUP_DIRECTORY_METADATA_BYTES,
        )?;
        self.retains_original(store, source, task)?;
        store.validate_mounted_current_layout_under_archive_root(root)?;
        source.source().validate_under_archive_root(store, root)?;
        store.validate_mounted_current_layout_under_archive_root(root)?;
        #[cfg(test)]
        run_final_directory_hook(
            &store
                .owner_evidence
                .before_v2_saved_group_final_directory_proof,
        )?;
        // Relevant last physical proof: no later fee/source/ID3 I/O may outlive directory
        // replacement before the caller retires the RAM exclusion or writes its first seed.
        require_same_directory(&self.path, &self.file, self.identity)?;
        self.retains_original(store, source, task)
    }
}

#[cfg(test)]
fn run_final_directory_hook(
    hook: &Mutex<Option<Box<dyn FnOnce() + Send>>>,
) -> Result<(), AppError> {
    let callback = hook
        .lock()
        .map_err(|_| critical_namespaces::refused())?
        .take();
    if let Some(callback) = callback {
        callback();
    }
    Ok(())
}
