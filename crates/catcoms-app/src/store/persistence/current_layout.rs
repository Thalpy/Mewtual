//! Original Current-layout fixed I/O, consumed only by the opaque private physical opener.
//! The old-head pass here is READ ONLY: it does not configure a checkpoint, create a directory,
//! allocate HeadCells, decode history, clean stages or manufacture an empty inventory.
use super::*;
use crate::store::chat_admission::current_lineage::LineageIndex;
use crate::store::critical_namespaces::{
    archive_current_namespace_codec as current_codec,
    current_layout::{
        Declaration, DeclarationParent, MountedCurrentLayout, CURRENT_DECLARATION_SEED,
        CURRENT_MARKER,
    },
};
use crate::store::critical_namespaces::{
    current_layout::{CanonicalFilenameInventoryPolicy, MAX_OPEN_BASE_PATH},
    read_pool::CatalogReadReservation,
};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

// This constructor keeps the existing 4096-byte base-path policy. Its five declaration
// selectors add only closed fixed names; do not spend the filename walk's separate 32KiB
// scratch allowance on silently larger retained declaration paths.
const DECLARATION_PATH_BYTES: usize = MAX_OPEN_BASE_PATH + 256;
fn current_fixed_path(parent: &Path, name: &str) -> Result<PathBuf, AppError> {
    let capacity = parent
        .as_os_str()
        .len()
        .checked_add(name.len() + 1)
        .ok_or_else(critical_namespaces::refused)?;
    if capacity > DECLARATION_PATH_BYTES {
        return Err(critical_namespaces::refused());
    }
    let mut path = PathBuf::with_capacity(capacity);
    path.push(parent);
    path.push(name);
    if path.as_os_str().len() > capacity || path.capacity() > DECLARATION_PATH_BYTES {
        return Err(critical_namespaces::refused());
    }
    Ok(path)
}
fn current_root_path(store: &ServerStore) -> Result<PathBuf, AppError> {
    if store.dir.as_os_str().len() > MAX_OPEN_BASE_PATH {
        return Err(critical_namespaces::refused());
    }
    current_fixed_path(&store.dir, critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
}

impl ServerStore {
    pub(in crate::store) fn require_no_archive_declaration_seed(&self) -> Result<(), AppError> {
        match fs::symlink_metadata(current_fixed_path(
            &self.dir,
            critical_namespaces::ARCHIVE_DECLARATION_SEED,
        )?) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(critical_namespaces::refused()),
        }
    }
    pub(in crate::store) fn read_current_declaration_seed(
        &self,
        fee: &CatalogReadReservation,
    ) -> Result<Option<OpenedArchiveObject>, AppError> {
        let policy = self
            .current_layout_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        fee.validate(&policy.archive.pending.pool, current_codec::DECLARATION_FEE)?;
        let path = current_fixed_path(&self.dir, CURRENT_DECLARATION_SEED)?;
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(metadata)
                if real_current_file(&metadata)
                    && metadata.len() <= current_codec::MAX_DECLARATION_FRAME as u64 => {}
            _ => return Err(critical_namespaces::refused()),
        }
        read_archive_object(&path, current_codec::MAX_DECLARATION_FRAME)
    }
    /// Constructor-only root resource, after BOTH complete catalog sources and independent
    /// declaration have classified. Original descriptor/marker are opened, never regenerated.
    pub(in crate::store) fn mount_existing_current_declaration_root(
        &self,
        descriptor: &critical_namespaces::archive_bootstrap::Descriptor,
    ) -> Result<(), AppError> {
        let policy = self
            .current_layout_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        let fee = policy
            .archive
            .pending
            .pool
            .try_reserve(critical_namespaces::archive_bootstrap::ROOT_METADATA_FEE)?;
        let root = self.open_declared_archive_root(descriptor, fee)?;
        let retired = {
            let mut slot = self
                .archive_bootstrap_root
                .lock()
                .map_err(|_| critical_namespaces::refused())?;
            slot.replace(root)
        };
        drop(retired);
        Ok(())
    }
    pub(in crate::store) fn validate_current_declaration_root(
        &self,
        layout: current_codec::CurrentLayoutData,
    ) -> Result<(), AppError> {
        let guard = self
            .archive_bootstrap_root
            .lock()
            .map_err(|_| critical_namespaces::refused())?;
        let root = guard.as_ref().ok_or_else(critical_namespaces::refused)?;
        validate_current_root(self, root, layout)
    }
    pub(in crate::store) fn check_current_marker_before_old_gates(
        &self,
        layout: current_codec::CurrentLayoutData,
        fee: &CatalogReadReservation,
    ) -> Result<(), AppError> {
        let policy = self
            .current_layout_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        fee.validate(&policy.archive.pending.pool, current_codec::DECLARATION_FEE)?;
        let guard = self
            .archive_bootstrap_root
            .lock()
            .map_err(|_| critical_namespaces::refused())?;
        let root = guard.as_ref().ok_or_else(critical_namespaces::refused)?;
        validate_current_root(self, root, layout)?;
        let path = current_fixed_path(&current_root_path(self)?, CURRENT_MARKER)?;
        let (_file, raw) = read_current_marker(&path)?;
        current_codec::open_marker_data(&self.keys.db_key()?, layout, &raw)?;
        validate_current_root(self, root, layout)
    }
    pub(in crate::store) fn install_current_layout_marker(
        &self,
        layout: current_codec::CurrentLayoutData,
    ) -> Result<(), AppError> {
        let policy = self
            .current_layout_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        let fee = policy
            .archive
            .pending
            .pool
            .try_reserve(current_codec::NAMESPACE_METADATA_FEE)?;
        // Declare possible retired ownership BEFORE Root, so an error after moving the fee
        // into this owner releases Root before its potentially last pool refund.
        let mut next: Option<MountedCurrentLayout>;
        let root_guard = self
            .archive_bootstrap_root
            .lock()
            .map_err(|_| critical_namespaces::refused())?;
        let root = root_guard
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        validate_current_root(self, root, layout)?;
        let path = current_fixed_path(&current_root_path(self)?, CURRENT_MARKER)?;
        let (file, raw) = read_current_marker(&path)?;
        current_codec::open_marker_data(&self.keys.db_key()?, layout, &raw)?;
        let id = catcoms_storage::kept::session_id(&file).map_err(current_io)?;
        file.sync_all().map_err(current_io)?;
        root.directory.sync_all().map_err(current_io)?;
        validate_current_root(self, root, layout)?;
        if !matches!(
            catcoms_storage::kept::session_at(&path, id),
            catcoms_storage::kept::SessionAt::Ours
        ) {
            return Err(critical_namespaces::refused());
        }
        next = Some(MountedCurrentLayout {
            layout,
            marker: file,
            marker_id: id,
            marker_digest: *blake3::hash(&raw).as_bytes(),
            mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            _fee: fee,
        });
        drop(raw);
        {
            let mut current = self
                .current_layout
                .lock()
                .map_err(|_| critical_namespaces::refused())?;
            if let Some(existing) = current.as_ref() {
                validate_current_marker(self, root, existing)?;
            } else {
                *current = next.take();
            }
        }
        drop(root_guard);
        drop(next);
        Ok(())
    }
    pub(in crate::store) fn validate_mounted_current_layout(&self) -> Result<(), AppError> {
        let root_guard = self
            .archive_bootstrap_root
            .lock()
            .map_err(|_| critical_namespaces::refused())?;
        let root = root_guard
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        self.validate_mounted_current_layout_under_archive_root(root)
    }

    /// Repeat the actual mounted namespace proof while the caller ALREADY owns archive
    /// Root. Activation/read consumers acquire admission -> pins -> canonical -> Root;
    /// calling the outer validator here would recursively lock Root. This shares the same
    /// Current-marker/account/file checks as the ordinary opener and grants no row/writer.
    ///
    /// The current-layout mutex is strictly inside Root in both callers. Pool/marker reads
    /// can wait, so consumers must repeat their final original source/name checks afterward.
    pub(in crate::store) fn validate_mounted_current_layout_under_archive_root(
        &self,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        let current = self
            .current_layout
            .lock()
            .map_err(|_| critical_namespaces::refused())?;
        validate_current_marker(
            self,
            root,
            current.as_ref().ok_or_else(critical_namespaces::refused)?,
        )
    }
    /// Sole pre-exposure fixed writer. It consumes only the concrete original prepared/seeded
    /// declaration, not a Caller callback or decoded success flag. No runnable Store escapes.
    pub(in crate::store) fn write_current_layout_declaration(
        &self,
        declaration: &mut Declaration,
        fee: &CatalogReadReservation,
    ) -> Result<(), AppError> {
        use critical_namespaces::{refused, Slot};
        let policy = self.current_layout_policy.as_ref().ok_or_else(refused)?;
        policy.require_persistent_capacity()?;
        fee.validate(&policy.archive.pending.pool, current_codec::DECLARATION_FEE)?;
        self.require_no_archive_declaration_seed()?;
        if *blake3::hash(&declaration.candidate).as_bytes() != declaration.next.framed
            || declaration.next.current_layout != Some(declaration.layout)
        {
            return Err(refused());
        }
        let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let root = root_guard.as_ref().ok_or_else(refused)?;
        validate_current_root(self, root, declaration.layout)?;
        let current_path = current_fixed_path(&self.dir, Slot::Current.name())?;
        let stage_path = current_fixed_path(&self.dir, Slot::Stage.name())?;
        let seed_path = current_fixed_path(&self.dir, CURRENT_DECLARATION_SEED)?;
        let root_path = current_root_path(self)?;
        let marker_path = current_fixed_path(&root_path, CURRENT_MARKER)?;
        let (current, current_hash) =
            read_pending_catalog_hash(&current_path)?.ok_or_else(refused)?;
        let promoted = current_hash == declaration.next.framed;
        if !promoted {
            let DeclarationParent::Original(parent) = &declaration.parent else {
                return Err(refused());
            };
            if current_hash != parent.framed {
                return Err(refused());
            }
            self.current_declaration_fault(211)?;
            ensure_archive_output(
                &seed_path,
                &declaration.seed,
                &mut declaration.seed_file,
                ArchiveOutputMode::Original,
                #[cfg(test)]
                self.critical_namespace_fault.load(Ordering::SeqCst),
                #[cfg(test)]
                212,
            )?;
            sync_directory(&self.dir).map_err(current_io)?;
            self.current_declaration_fault(213)?;
            // Unlike ID2's initial-root transaction, ID3 first settles its marker in the SAME
            // already-complete root BEFORE making any catalog3 stage/promoted namespace.
            ensure_archive_output(
                &marker_path,
                &declaration.marker,
                &mut declaration.marker_file,
                ArchiveOutputMode::SeedRepair,
                #[cfg(test)]
                self.critical_namespace_fault.load(Ordering::SeqCst),
                #[cfg(test)]
                214,
            )?;
            sync_current_namespace_root(root)?;
            validate_current_root(self, root, declaration.layout)?;
            self.current_declaration_fault(215)?;
            ensure_archive_output(
                &stage_path,
                &declaration.candidate,
                &mut declaration.stage_file,
                ArchiveOutputMode::SeedRepair,
                #[cfg(test)]
                self.critical_namespace_fault.load(Ordering::SeqCst),
                #[cfg(test)]
                216,
            )?;
            sync_directory(&self.dir).map_err(current_io)?;
            #[cfg(test)]
            {
                let hook = self
                    .owner_evidence
                    .current_after_namespace_stage
                    .lock()
                    .map_err(|_| refused())?
                    .take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            let (actual, hash) = read_pending_catalog_hash(&current_path)?.ok_or_else(refused)?;
            if hash != parent.framed {
                return Err(refused());
            }
            actual.sync_all().map_err(current_io)?;
            // The independently durable seed must still be the SAME complete original source
            // after stage waits. A byte-equal replacement is not our retained physical object.
            let precommit_seed =
                read_verified_current_seed(&seed_path, declaration)?.ok_or_else(refused)?;
            drop(precommit_seed);
            let (stage_file, stage_raw) =
                read_archive_object(&stage_path, current_codec::MAX_CATALOG_FRAME)?
                    .ok_or_else(refused)?;
            let held_stage = declaration.stage_file.as_ref().ok_or_else(refused)?;
            if stage_raw.as_slice() != declaration.candidate.as_slice()
                || catcoms_storage::kept::session_id(&stage_file).map_err(current_io)?
                    != held_stage.identity.ok_or_else(refused)?
            {
                return Err(refused());
            }
            stage_file.sync_all().map_err(current_io)?;
            drop(stage_raw);
            drop(stage_file);
            validate_current_root(self, root, declaration.layout)?;
            // Marker2 alone does not cover marker3. Recheck the SAME complete held marker
            // after potentially large stage work, immediately before irreversible promotion.
            verify_current_declaration_marker(
                &marker_path,
                &declaration.marker,
                declaration.marker_file.as_ref().ok_or_else(refused)?,
            )?;
            verify_archive_held_name(
                &seed_path,
                declaration.seed_file.as_ref().ok_or_else(refused)?,
            )?;
            verify_archive_held_name(
                &stage_path,
                declaration.stage_file.as_ref().ok_or_else(refused)?,
            )?;
            fs::rename(&stage_path, &current_path).map_err(current_io)?;
            self.current_declaration_fault(217)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            sync_directory(&self.dir)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
        } else {
            // This branch can only re-sync exact CURRENT and an already COMPLETE marker. A
            // missing/partial marker is not recreated even when a complete seed survives.
            ensure_archive_output(
                &marker_path,
                &declaration.marker,
                &mut declaration.marker_file,
                ArchiveOutputMode::SavedExact,
                #[cfg(test)]
                0,
                #[cfg(test)]
                0,
            )?;
            sync_current_namespace_root(root)?;
            if let Some((stage, raw)) =
                read_archive_object(&stage_path, current_codec::MAX_CATALOG_FRAME)?
            {
                if raw.as_slice() != declaration.candidate.as_slice() {
                    return Err(refused());
                }
                stage.sync_all().map_err(current_io)?;
                fs::remove_file(&stage_path).map_err(current_io)?;
            }
            current.sync_all().map_err(current_io)?;
            sync_directory(&self.dir).map_err(current_io)?;
        }
        // Finish potentially large independent-seed work BEFORE the final small marker/name
        // proof. Otherwise a delayed seed read/sync could invalidate that earlier observation
        // while its subsequent unlink destroys the only exact cold repair source.
        let seed_to_remove = read_verified_current_seed(&seed_path, declaration)?;
        #[cfg(test)]
        {
            let hook = self
                .owner_evidence
                .current_after_namespace_seed_verification
                .lock()
                .map_err(|_| refused())?
                .take();
            if let Some(hook) = hook {
                hook();
            }
        }
        let (saved, hash) = read_pending_catalog_hash(&current_path)?.ok_or_else(refused)?;
        if hash != declaration.next.framed {
            return Err(refused());
        }
        saved.sync_all().map_err(current_io)?;
        validate_current_root(self, root, declaration.layout)?;
        // Preserve the independent repair seed if marker3 disappeared/changed while any
        // current/root barrier waited. An AlreadyPromoted retry must never recreate it.
        verify_current_declaration_marker(
            &marker_path,
            &declaration.marker,
            declaration.marker_file.as_ref().ok_or_else(refused)?,
        )?;
        if let Some((seed, id)) = seed_to_remove {
            if catcoms_storage::kept::session_id(&seed).map_err(current_io)? != id
                || !matches!(
                    catcoms_storage::kept::session_at(&seed_path, id),
                    catcoms_storage::kept::SessionAt::Ours
                )
            {
                return Err(refused());
            }
            fs::remove_file(&seed_path).map_err(current_io)?;
            self.current_declaration_fault(218)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
        }
        sync_directory(&self.dir)
            .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))
    }
    fn current_declaration_fault(&self, point: u8) -> Result<(), AppError> {
        #[cfg(test)]
        if self.critical_namespace_fault.load(Ordering::SeqCst) == point {
            return Err(AppError::Io("injected Current declaration stop".into()));
        }
        let _ = point;
        Ok(())
    }
}
fn read_current_marker(path: &Path) -> Result<(File, Zeroizing<Vec<u8>>), AppError> {
    let metadata = fs::symlink_metadata(path).map_err(current_io)?;
    if !real_current_file(&metadata) || metadata.len() > current_codec::MAX_MARKER_FRAME as u64 {
        return Err(critical_namespaces::refused());
    }
    read_archive_object(path, current_codec::MAX_MARKER_FRAME)?
        .ok_or_else(critical_namespaces::refused)
}
fn sync_current_namespace_root(
    root: &critical_namespaces::archive_bootstrap::MountedRoot,
) -> Result<(), AppError> {
    root.directory.sync_all().map_err(current_io)
}
fn read_verified_current_seed(
    path: &Path,
    declaration: &Declaration,
) -> Result<Option<(File, catcoms_storage::kept::SessionId)>, AppError> {
    let Some((file, raw)) = read_archive_object(path, current_codec::MAX_DECLARATION_FRAME)? else {
        return Ok(None);
    };
    let held = declaration
        .seed_file
        .as_ref()
        .ok_or_else(critical_namespaces::refused)?;
    verify_archive_held_name(path, held)?;
    let id = catcoms_storage::kept::session_id(&file).map_err(current_io)?;
    if id != held.identity.ok_or_else(critical_namespaces::refused)?
        || raw.as_slice() != declaration.seed.as_slice()
    {
        return Err(critical_namespaces::refused());
    }
    file.sync_all().map_err(current_io)?;
    verify_archive_held_name(path, held)?;
    Ok(Some((file, id)))
}
fn verify_current_declaration_marker(
    path: &Path,
    expected: &[u8],
    held: &critical_namespaces::pending_join::CreatedStage,
) -> Result<(), AppError> {
    verify_archive_held_name(path, held)?;
    let (file, raw) = read_current_marker(path)?;
    if catcoms_storage::kept::session_id(&file).map_err(current_io)?
        != held.identity.ok_or_else(critical_namespaces::refused)?
        || raw.as_slice() != expected
    {
        return Err(critical_namespaces::refused());
    }
    verify_archive_held_name(path, held)
}
fn validate_current_root(
    store: &ServerStore,
    root: &critical_namespaces::archive_bootstrap::MountedRoot,
    layout: current_codec::CurrentLayoutData,
) -> Result<(), AppError> {
    if store.current_layout_policy.is_none()
        || layout.vault != root.descriptor.vault
        || layout.root != root.descriptor.root
        || root.negative.is_cancelled()
        || !Arc::ptr_eq(&root.mount, &store.registry_mount())
    {
        return Err(critical_namespaces::refused());
    }
    validate_archive_root_session(&current_root_path(store)?, root)
}
fn validate_current_marker(
    store: &ServerStore,
    root: &critical_namespaces::archive_bootstrap::MountedRoot,
    current: &MountedCurrentLayout,
) -> Result<(), AppError> {
    let policy = store
        .current_layout_policy
        .as_ref()
        .ok_or_else(critical_namespaces::refused)?;
    current._fee.validate(
        &policy.archive.pending.pool,
        current_codec::NAMESPACE_METADATA_FEE,
    )?;
    if current.negative.is_cancelled() || !Arc::ptr_eq(&current.mount, &store.registry_mount()) {
        return Err(critical_namespaces::refused());
    }
    validate_current_root(store, root, current.layout)?;
    let path = current_fixed_path(&current_root_path(store)?, CURRENT_MARKER)?;
    if catcoms_storage::kept::session_id(&current.marker).map_err(current_io)? != current.marker_id
        || !matches!(
            catcoms_storage::kept::session_at(&path, current.marker_id),
            catcoms_storage::kept::SessionAt::Ours
        )
    {
        return Err(critical_namespaces::refused());
    }
    let (file, raw) = read_current_marker(&path)?;
    if catcoms_storage::kept::session_id(&file).map_err(current_io)? != current.marker_id
        || *blake3::hash(&raw).as_bytes() != current.marker_digest
    {
        return Err(critical_namespaces::refused());
    }
    current_codec::open_marker_data(&store.keys.db_key()?, current.layout, &raw)?;
    validate_current_root(store, root, current.layout)
}

const PATH_BYTES: usize = 32 * 1024 + 256;
const COMPONENT_BYTES: usize = 64 * 1024;
// Windows uses fixed native find-data + a root path; Unix DirEntry/file_name have two name
// copies. Sum both named phases, including UTF16/path-conversion overlap conservatively.
// Native OS/C iterator buffers and allocator/RSS remain explicitly outside this invoice.
pub(in crate::store) const CURRENT_FILENAME_SCRATCH_FEE: usize = 3 * PATH_BYTES
    + 2 * COMPONENT_BYTES
    + std::mem::size_of::<fs::ReadDir>()
    + std::mem::size_of::<fs::DirEntry>()
    + 4 * std::mem::size_of::<fs::Metadata>()
    + std::mem::size_of::<CurrentCanonicalFilenameAssociation>()
    + std::mem::size_of::<blake3::Hasher>()
    + 4 * 1024; // fixed error/hex/native-width conversion headers, not another directory list

#[derive(Clone, Copy)]
enum AssociationKind {
    Known,
    Unavailable,
}

/// Constructor-only read-only walk result. Private fields prevent caller status/data from
/// installing complete classification. This is NEGATIVE metadata, not a root/Current token.
pub(in crate::store) struct CurrentCanonicalFilenameAssociation {
    index: Arc<()>,
    generation: Cell<u64>,
    unavailable: Cell<bool>,
    directory: RefCell<Option<(File, catcoms_storage::kept::SessionId)>>,
    mount: Arc<()>,
    negative: catcoms_rt::RequestCancellation,
}
impl CurrentCanonicalFilenameAssociation {
    pub(in crate::store) fn validate_original_directory(
        &self,
        index: &LineageIndex,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        if !self.matches_index(index, store) {
            return Err(critical_namespaces::refused());
        }
        let path = old_directory_path(store)?;
        let directory = self
            .directory
            .try_borrow()
            .map_err(|_| critical_namespaces::refused())?;
        match directory.as_ref() {
            Some((held, id)) => {
                let metadata = fs::symlink_metadata(&path).map_err(current_io)?;
                // SessionAt is file-only. A fixed directory needs the actual opened named
                // handle plus held/named IDs, with the existing no-reparse directory helper.
                let named = open_archive_directory(&path)?;
                if !real_current_directory(&metadata)
                    || catcoms_storage::kept::session_id(held).map_err(current_io)? != *id
                    || catcoms_storage::kept::session_id(&named).map_err(current_io)? != *id
                {
                    return Err(critical_namespaces::refused());
                }
                Ok(())
            }
            None => match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                // The actual known-key file writer may adopt a cooperative configure-created
                // directory before its first file. A vacancy guard itself never adopts one.
                _ => Err(critical_namespaces::refused()),
            },
        }
    }

    pub(in crate::store) fn belongs_to_index(
        &self,
        index: &LineageIndex,
        store: &ServerStore,
    ) -> bool {
        index.same_original(store)
            && !self.negative.is_cancelled()
            && Arc::ptr_eq(&self.mount, &store.registry_mount())
            && Arc::ptr_eq(&self.index, index.identity_data())
            && self.generation.get() == index.generation_data()
    }
    pub(in crate::store) fn matches_index(
        &self,
        index: &LineageIndex,
        store: &ServerStore,
    ) -> bool {
        !self.unavailable.get() && self.belongs_to_index(index, store)
    }
    /// Preserve a complete walk only through an ACTUAL admitted known-key file effect. When
    /// owner-state was initially absent, hold its newly created original directory before the
    /// first file. Failure closes future V2 vacancy and cannot block unrelated legitimate V1.
    pub(in crate::store) fn observe_known_directory(
        &self,
        index: &LineageIndex,
        store: &ServerStore,
        path: &Path,
    ) {
        if self.unavailable.get() {
            return;
        }
        let observed = (|| -> Result<(), AppError> {
            if !self.belongs_to_index(index, store) || path != old_directory_path(store)? {
                return Err(critical_namespaces::refused());
            }
            let metadata = fs::symlink_metadata(path).map_err(current_io)?;
            if !real_current_directory(&metadata) {
                return Err(critical_namespaces::refused());
            }
            let mut held = self
                .directory
                .try_borrow_mut()
                .map_err(|_| critical_namespaces::refused())?;
            if let Some((file, id)) = held.as_ref() {
                if catcoms_storage::kept::session_id(file).map_err(current_io)? != *id
                    || !current_directory_at(path, *id)?
                {
                    return Err(critical_namespaces::refused());
                }
            } else {
                let file = open_archive_directory(path)?;
                let id = catcoms_storage::kept::session_id(&file).map_err(current_io)?;
                if !current_directory_at(path, id)? {
                    return Err(critical_namespaces::refused());
                }
                *held = Some((file, id));
            }
            Ok(())
        })();
        if observed.is_err() {
            self.unavailable.set(true);
        }
    }
    /// Only an actual same-index known-key effect may preserve a complete-walk invariant.
    /// It cannot turn an incomplete/unknown walk into Known or attach a replacement index.
    pub(in crate::store) fn advance_known(&self, identity: &Arc<()>, old: u64, next: u64) {
        if !Arc::ptr_eq(&self.index, identity)
            || self.generation.get() != old
            || next != old.saturating_add(1)
        {
            self.unavailable.set(true);
            return;
        }
        self.generation.set(next);
    }
}

fn current_io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}
fn old_directory_path(store: &ServerStore) -> Result<PathBuf, AppError> {
    if store.dir.as_os_str().len() > MAX_OPEN_BASE_PATH {
        return Err(critical_namespaces::refused());
    }
    let capacity = store
        .dir
        .as_os_str()
        .len()
        .checked_add(32)
        .ok_or_else(critical_namespaces::refused)?;
    let mut path = PathBuf::with_capacity(capacity);
    path.push(&store.dir);
    path.push("owner-state");
    if path.as_os_str().len() > capacity || path.capacity() > PATH_BYTES {
        return Err(critical_namespaces::refused());
    }
    Ok(path)
}
fn real_current_directory(metadata: &fs::Metadata) -> bool {
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// `kept::session_at` deliberately answers only for regular files and returns Other for
/// directories. Prove a directory name with the actual directory opener instead; it rejects
/// links/reparse points and compares the original in-session object number, never contents.
fn current_directory_at(
    path: &Path,
    expected: catcoms_storage::kept::SessionId,
) -> Result<bool, AppError> {
    let named = open_archive_directory(path)?;
    Ok(catcoms_storage::kept::session_id(&named).map_err(current_io)? == expected)
}

fn real_current_file(metadata: &fs::Metadata) -> bool {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    {
        true
    }
}
fn old_filename_key(name: &std::ffi::OsStr) -> Option<[u8; 32]> {
    let name = name.to_str()?;
    let encoded = name
        .strip_suffix(".bin")
        .or_else(|| name.strip_suffix(".replace.tmp"))?;
    if encoded.len() != 64
        || !encoded
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut key = [0; 32];
    hex::decode_to_slice(encoded, &mut key).ok()?;
    Some(key)
}
fn association(
    store: &ServerStore,
    index: &LineageIndex,
    kind: AssociationKind,
    directory: Option<File>,
    id: Option<catcoms_storage::kept::SessionId>,
) -> CurrentCanonicalFilenameAssociation {
    CurrentCanonicalFilenameAssociation {
        index: index.identity_data().clone(),
        generation: Cell::new(index.generation_data()),
        unavailable: Cell::new(matches!(kind, AssociationKind::Unavailable)),
        directory: RefCell::new(directory.zip(id)),
        mount: store.registry_mount(),
        negative: store.chat_mount.observer(),
    }
}

impl ServerStore {
    /// Exactly one bounded startup pass against ACTUAL loaded admission keys. Caller already
    /// holds that index's admission writer and pays scratch before entering it. Both suffixes
    /// count as files; no dedup/map allocation is hidden in this negative association scan.
    pub(in crate::store) fn inspect_current_canonical_filenames(
        &self,
        index: &LineageIndex,
        policy: CanonicalFilenameInventoryPolicy,
        scratch: &CatalogReadReservation,
    ) -> Result<CurrentCanonicalFilenameAssociation, AppError> {
        let configured = self
            .current_layout_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        scratch.validate(
            &configured.archive.pending.pool,
            CURRENT_FILENAME_SCRATCH_FEE,
        )?;
        if !index.same_original(self) {
            return Err(critical_namespaces::refused());
        }
        let parent = old_directory_path(self)?;
        match fs::symlink_metadata(&parent) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(association(self, index, AssociationKind::Known, None, None))
            }
            Ok(metadata) if real_current_directory(&metadata) => {}
            _ => {
                return Ok(association(
                    self,
                    index,
                    AssociationKind::Unavailable,
                    None,
                    None,
                ))
            }
        }
        let directory = match open_archive_directory(&parent) {
            Ok(file) => file,
            Err(_) => {
                return Ok(association(
                    self,
                    index,
                    AssociationKind::Unavailable,
                    None,
                    None,
                ))
            }
        };
        let id = catcoms_storage::kept::session_id(&directory).map_err(current_io)?;
        let iter = match fs::read_dir(&parent) {
            Ok(iter) => iter,
            Err(_) => {
                return Ok(association(
                    self,
                    index,
                    AssociationKind::Unavailable,
                    Some(directory),
                    Some(id),
                ))
            }
        };
        let mut examined = 0usize;
        let mut known = true;
        for entry in iter {
            examined = match examined.checked_add(1).filter(|n| *n <= policy.max_files) {
                Some(n) => n,
                None => {
                    known = false;
                    break;
                }
            };
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    known = false;
                    break;
                }
            };
            let name = entry.file_name();
            if name.len() > COMPONENT_BYTES {
                known = false;
                break;
            }
            let key = match old_filename_key(&name) {
                Some(key) => key,
                None => {
                    known = false;
                    break;
                }
            };
            if !index.contains_key_data(&key) {
                known = false;
                break;
            }
            // Do not call entry.path(): its root.clone().join can grow outside the checked
            // requested path capacity. Names have now passed the exact closed ASCII grammar.
            let capacity = parent
                .as_os_str()
                .len()
                .checked_add(name.len() + 1)
                .ok_or_else(critical_namespaces::refused)?;
            let mut path = PathBuf::with_capacity(capacity);
            path.push(&parent);
            path.push(&name);
            if path.as_os_str().len() > capacity || path.capacity() > PATH_BYTES {
                known = false;
                break;
            }
            match fs::symlink_metadata(&path) {
                Ok(metadata)
                    if real_current_file(&metadata) && metadata.len() <= policy.max_file_bytes => {}
                _ => {
                    known = false;
                    break;
                }
            }
        }
        if !current_directory_at(&parent, id).unwrap_or(false) || !index.same_original(self) {
            known = false;
        }
        Ok(association(
            self,
            index,
            if known {
                AssociationKind::Known
            } else {
                AssociationKind::Unavailable
            },
            Some(directory),
            Some(id),
        ))
    }
}

#[cfg(test)]
#[path = "current_layout_directory_identity_tests.rs"]
mod directory_identity_tests;
