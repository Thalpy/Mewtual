//! Concrete negative group namespace locks paired with the original task/directory writer,
//! actual admission roles and unexposed Current caller.
//! A registry entry is exclusion DATA; it grants neither a scope nor a canonical writer.
use super::*;
use crate::store::chat_admission::{
    current_lineage::CurrentFilenameAssociation,
    v2_current::{V2CurrentAdmission, V2InitializingAdmission, VacantV2GenesisAdmission},
};
use crate::store::critical_namespaces::{
    archive_current_namespace_codec::CurrentLayoutData,
    current_layout::{group_task::GroupTaskLife, PreparedV2GenesisGroupNamespace},
    read_pool::{CatalogReadReservation, CriticalCatalogReadPool},
};
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::persistence::V2GenesisDirectoryCustody;

// The real task-free cold capture is test-only until its separate owner/writer caller lands.
#[cfg(test)]
pub(in crate::store) mod cold_read;

const MAX_PENDING_GROUPS: usize = 16;
const MAX_GROUP_BYTES: usize = 256;

struct PendingGroupEntry {
    group: [u8; MAX_GROUP_BYTES],
    length: u16,
    layout: CurrentLayoutData,
    source: catcoms_sync::owner_v2::BootstrapSourceId,
    mount: Arc<()>,
    original: Weak<GroupTaskLife>,
    custody: Arc<V2GenesisDirectoryCustody>,
    // LAST: fixed binding/control references drain before the independent slot payment.
    _fee: CatalogReadReservation,
}
impl PendingGroupEntry {
    fn group(&self) -> &[u8] {
        &self.group[..self.length as usize]
    }
    fn require_same_task(
        &self,
        store: &ServerStore,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.mount, &store.registry_mount())
            || self.group() != task.group()
            || self.layout != task.layout_data()
            || self.source != task.source_id_data()
            || !Weak::ptr_eq(&self.original, &task.original_life())
            || !Arc::ptr_eq(&self.custody, task.directory_output())
        {
            return Err(refused());
        }
        Ok(())
    }
}
struct PendingGroupBuffer {
    slots: Box<[Option<PendingGroupEntry>; MAX_PENDING_GROUPS]>,
    // LAST: the real fixed backing, every inert slot/custody and its own fee drain first.
    _fee: CatalogReadReservation,
}

/// Fixed sixteen unresolved original directory operations, not sixteen lifetime groups.
/// A cancelled/expired weak task NEVER erases a slot: its independently paid output can
/// still be owed. Only a genuine same permanent-directory barrier receipt can remove it.
#[derive(Default)]
pub(in crate::store::owner_evidence) struct PendingGroupRegistry {
    buffer: Option<PendingGroupBuffer>,
}

/// Entirely prepared outside all host writers. If another genuine first operation installs
/// its prepaid buffer first, the unused one remains in this owner until OUTSIDE guards.
pub(in crate::store) struct PreparedGroupRegistration {
    buffer: Option<PendingGroupBuffer>,
    entry: Option<PendingGroupEntry>,
}

/// Actual retired inert DATA, held by the original task after the permanent fence handoff.
/// Drop this only outside admission/pins/canonical/Root; it can refund actual pool payments.
pub(in crate::store) struct RetiredGroupEntry {
    _entry: PendingGroupEntry,
}

impl PreparedGroupRegistration {
    pub(in crate::store) fn new(
        store: &ServerStore,
        pool: &CriticalCatalogReadPool,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<Self, AppError> {
        let buffer_fee = pool.try_reserve(
            std::mem::size_of::<[Option<PendingGroupEntry>; MAX_PENDING_GROUPS]>()
                + std::mem::size_of::<PendingGroupBuffer>(),
        )?;
        let buffer = PendingGroupBuffer {
            slots: Box::new(std::array::from_fn(|_| None)),
            _fee: buffer_fee,
        };
        let entry_fee = pool.try_reserve(2 * std::mem::size_of::<PendingGroupEntry>())?;
        if task.group().is_empty() || task.group().len() > MAX_GROUP_BYTES {
            return Err(refused());
        }
        let mut group = [0; MAX_GROUP_BYTES];
        group[..task.group().len()].copy_from_slice(task.group());
        Ok(Self {
            buffer: Some(buffer),
            entry: Some(PendingGroupEntry {
                group,
                length: task.group().len() as u16,
                layout: task.layout_data(),
                source: task.source_id_data(),
                mount: store.registry_mount(),
                original: task.original_life(),
                custody: task.directory_output().clone(),
                _fee: entry_fee,
            }),
        })
    }
}
impl PendingGroupRegistry {
    /// Bounded query under the actual pins owner. No expiry/filter/cleanup is performed.
    pub(in crate::store::owner_evidence) fn excludes_group(&self, group: &[u8]) -> bool {
        self.buffer.as_ref().is_some_and(|buffer| {
            buffer
                .slots
                .iter()
                .flatten()
                .any(|entry| entry.group() == group)
        })
    }
    fn require_same_task(
        &self,
        store: &ServerStore,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        let entry = self
            .buffer
            .as_ref()
            .ok_or_else(refused)?
            .slots
            .iter()
            .flatten()
            .find(|entry| entry.group() == task.group())
            .ok_or_else(refused)?;
        entry.require_same_task(store, task)
    }
    /// Call ONLY after the concrete vacant namespace/source/Root checks under admission and
    /// pins. Every fallible condition precedes both transfers; Store never owns task life.
    fn install(
        &mut self,
        store: &ServerStore,
        task: &PreparedV2GenesisGroupNamespace,
        prepared: &mut PreparedGroupRegistration,
    ) -> Result<(), AppError> {
        prepared
            .entry
            .as_ref()
            .ok_or_else(refused)?
            .require_same_task(store, task)?;
        if self.excludes_group(task.group()) {
            return Err(refused());
        }
        let slot = match self.buffer.as_ref() {
            Some(buffer) => buffer
                .slots
                .iter()
                .position(Option::is_none)
                .ok_or_else(refused)?,
            None if prepared.buffer.is_some() => 0,
            None => return Err(refused()),
        };
        // No fallible call, allocation, callback or last-fee destruction after this point.
        if self.buffer.is_none() {
            self.buffer = prepared.buffer.take();
        }
        if let Some(buffer) = self.buffer.as_mut() {
            buffer.slots[slot] = prepared.entry.take();
        }
        Ok(())
    }
}

/// Closed concrete roles only; no public trait, status Boolean or caller callback can
/// substitute a Current2 record for Init1 or a vacant row for a Head writer's admission.
enum OriginalAdmission<'guard, 'writer> {
    Vacant(&'guard VacantV2GenesisAdmission<'writer>),
    Initializing(&'guard V2InitializingAdmission<'writer>),
    Current(&'guard V2CurrentAdmission<'writer>),
}
impl OriginalAdmission<'_, '_> {
    fn store(&self) -> &ServerStore {
        match self {
            Self::Vacant(a) => a.store(),
            Self::Initializing(a) => a.store(),
            Self::Current(a) => a.store(),
        }
    }
    fn source(&self) -> &GenesisSourceDataRef<'_> {
        match self {
            Self::Vacant(a) => a.source(),
            Self::Initializing(a) => a.source(),
            Self::Current(a) => a.source(),
        }
    }
    fn validate_task(&self, task: &PreparedV2GenesisGroupNamespace) -> Result<(), AppError> {
        match self {
            Self::Vacant(a) => a.require_vacant_lineage_for_own_group_task(task),
            Self::Initializing(a) => a.require_initializing_lineage_for_own_group_task(task),
            Self::Current(a) => a.require_current_lineage_for_own_group_task(task),
        }
    }
}

struct OriginalOwnerNamespace<'guard, 'writer> {
    // The role borrow physically keeps the actual admission writer alive through BOTH
    // subordinate mutex guards. A raw Store borrow is deliberately insufficient.
    admission: OriginalAdmission<'guard, 'writer>,
    task: &'guard PreparedV2GenesisGroupNamespace,
    pins: MutexGuard<'guard, super::super::pins::Runtime>,
    pool: MutexGuard<'guard, Runtime>,
    association: CurrentFilenameAssociation<'guard>,
    identifier: [u8; 32],
}
impl OriginalOwnerNamespace<'_, '_> {
    /// No further locks/I/O/allocations/SDK/account operations. Physical old-directory/name
    /// observation is a separate entry after lock waits; Root repeats its own real files.
    fn validate_source_only(&self) -> Result<(), AppError> {
        let store = self.admission.store();
        self.admission.validate_task(self.task)?;
        self.association.require_original_data()?;
        if !self.pins.managed
            || self.pins.dirty
            || self
                .pins
                .table
                .as_ref()
                .ok_or_else(refused)?
                .contains_key(self.task.group())
            || self.pool.heads.contains_key(&self.identifier)
            || self.identifier != key(self.task.group(), self.task.signer())
        {
            return Err(refused());
        }
        self.task.retains_original(store, self.admission.source())
    }
    fn validate_old_files(&self) -> Result<(), AppError> {
        self.validate_source_only()?;
        require_old_owner_files_absent(self.admission.store(), &self.association, self.identifier)?;
        self.validate_source_only()
    }
    fn require_registered_or_saved_task(&self) -> Result<(), AppError> {
        if self.pins.pending_groups.excludes_group(self.task.group()) {
            self.pins
                .pending_groups
                .require_same_task(self.admission.store(), self.task)
        } else {
            // The task can leave the unresolved registry only after its real directory/root
            // barrier receipt. The concrete Root guard still validates those exact files.
            self.task.saved_directory().map(|_| ())
        }
    }
}

/// Shared NEGATIVE physical predicate for warm owned and cold unowned concrete roles.
/// The caller already holds admission, pins and canonical and pays the bounded selectors.
/// It neither creates/adopts an owner directory nor returns a scope/storage permission.
fn require_old_owner_files_absent(
    store: &ServerStore,
    association: &CurrentFilenameAssociation<'_>,
    identifier: [u8; 32],
) -> Result<(), AppError> {
    association.validate_original_directory()?;
    let parent = fixed_owner_child(&store.dir, "owner-state")?;
    match fs::symlink_metadata(&parent) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return association.validate_original_directory();
        }
        Ok(metadata) if old_directory_regular(&metadata) => {}
        _ => return Err(refused()),
    }
    let base = hex::encode(identifier);
    for suffix in [".bin", ".replace.tmp"] {
        let mut name = String::with_capacity(base.len() + suffix.len());
        name.push_str(&base);
        name.push_str(suffix);
        if base.capacity() > 64 || name.capacity() > 128 {
            return Err(refused());
        }
        let path = fixed_owner_child(&parent, &name)?;
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(refused()),
        }
    }
    association.validate_original_directory()
}

/// Closed child selectors only. Preallocate checked final capacity rather than clone+join,
/// whose amortized growth could double the paid four-path requested-buffer inventory.
fn fixed_owner_child(parent: &Path, child: &str) -> Result<PathBuf, AppError> {
    let ceiling = crate::store::critical_namespaces::current_layout::MAX_OPEN_BASE_PATH + 256;
    let capacity = parent
        .as_os_str()
        .len()
        .checked_add(child.len() + 1)
        .filter(|length| *length <= ceiling)
        .ok_or_else(refused)?;
    let mut path = PathBuf::with_capacity(capacity);
    path.push(parent);
    path.push(child);
    if path.as_os_str().len() > capacity || path.capacity() > ceiling {
        return Err(refused());
    }
    Ok(path)
}

fn old_directory_regular(metadata: &fs::Metadata) -> bool {
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

/// Pre-mkdir concrete owner locks. It cannot be handed to Root's activation Seed/Head APIs.
pub(in crate::store) struct CreatingV2OwnerNamespace<'guard, 'writer> {
    inner: OriginalOwnerNamespace<'guard, 'writer>,
    admission: &'guard VacantV2GenesisAdmission<'writer>,
}
/// Original unregistered task data under the actual absent-row namespace. It can install
/// negative exclusion only; it supplies neither directory I/O nor a Root Seed/Head role.
pub(in crate::store) struct RegisteringV2OwnerNamespace<'guard, 'writer> {
    inner: OriginalOwnerNamespace<'guard, 'writer>,
    admission: &'guard VacantV2GenesisAdmission<'writer>,
}
/// Root's pre-seed role, AFTER the original task has an actual SavedDirectory receipt.
pub(in crate::store) struct VacantV2OwnerNamespace<'guard, 'writer> {
    inner: OriginalOwnerNamespace<'guard, 'writer>,
    admission: &'guard VacantV2GenesisAdmission<'writer>,
}
/// Actual Init1 owner custody before Root; only this exact type supports writer installation.
pub(in crate::store) struct InitializingV2OwnerNamespace<'guard, 'writer> {
    inner: OriginalOwnerNamespace<'guard, 'writer>,
    admission: &'guard V2InitializingAdmission<'writer>,
}
/// Current2 read/proof/resync custody. No Init1 permit or Head writer can be obtained from it.
pub(in crate::store) struct CurrentV2OwnerNamespace<'guard, 'writer> {
    inner: OriginalOwnerNamespace<'guard, 'writer>,
    admission: &'guard V2CurrentAdmission<'writer>,
}

macro_rules! negative_owner_accessors {
    ($name:ident) => {
        impl<'guard, 'writer> $name<'guard, 'writer> {
            pub(in crate::store) fn store(&self) -> &ServerStore {
                self.admission.store()
            }
            pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
                self.admission.source()
            }
            pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
                self.inner.task
            }
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.inner.validate_source_only()?;
                self.inner.require_registered_or_saved_task()
            }
            pub(in crate::store) fn validate_old_files(&self) -> Result<(), AppError> {
                self.validate_source_only()?;
                self.inner.validate_old_files()
            }
        }
    };
}
negative_owner_accessors!(CreatingV2OwnerNamespace);
negative_owner_accessors!(VacantV2OwnerNamespace);
negative_owner_accessors!(InitializingV2OwnerNamespace);
negative_owner_accessors!(CurrentV2OwnerNamespace);

impl<'guard, 'writer> InitializingV2OwnerNamespace<'guard, 'writer> {
    pub(in crate::store) fn admission(&self) -> &'guard V2InitializingAdmission<'writer> {
        self.admission
    }
    // ONLY the canonical family sees its own actual registry through this same held guard.
    // No namespace module, callback or raw Store caller can obtain the mutable Runtime.
    pub(super) fn registration_parts(
        &mut self,
    ) -> (
        &V2InitializingAdmission<'writer>,
        &mut super::v2_registration::Runtime,
    ) {
        (self.admission, &mut self.inner.pool.v2_writers)
    }
}

impl<'guard, 'writer> RegisteringV2OwnerNamespace<'guard, 'writer> {
    /// Caller ALREADY holds the same original Root after these owner locks. No recursive
    /// admission/pins/canonical/Root acquisition and no source label/status can install it.
    pub(in crate::store) fn install_under_archive_root(
        &mut self,
        prepared: &mut PreparedGroupRegistration,
        root: &crate::store::critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        let store = self.admission.store();
        let task = self.inner.task;
        self.inner.validate_old_files()?;
        self.inner
            .pool
            .v2_writers
            .require_no_live_writer(&self.inner.identifier)?;
        if self.inner.pins.pending_groups.excludes_group(task.group()) {
            return Err(refused());
        }
        store.validate_mounted_current_layout_under_archive_root(root)?;
        self.admission
            .source()
            .source()
            .validate_under_archive_root(store, root)?;
        if root.descriptor.vault != task.layout_data().vault
            || root.descriptor.root != task.layout_data().root
            || !store.archive_group_absent_under_original_root(root, task.group())?
        {
            return Err(refused());
        }
        // The exact original task/source is repeated after both old-directory and Root/file
        // observations. These are the last fallible checks before atomic negative custody.
        self.inner.validate_source_only()?;
        self.inner
            .pins
            .pending_groups
            .install(store, task, prepared)
    }
}

impl VacantV2OwnerNamespace<'_, '_> {
    /// Permanent actual directory/parent custody takes over negative fencing. Caller owns
    /// Root and the opaque saved receipt; no weak-expiry/Drop/status-based reclamation exists.
    /// Returned inert owners MUST drain outside every host writer.
    pub(in crate::store) fn retire_saved_directory(
        &mut self,
        directory: &crate::store::critical_namespaces::current_layout::SavedV2GroupDirectory,
        root: &crate::store::critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<Option<RetiredGroupEntry>, AppError> {
        self.validate_source_only()?;
        let store = self.admission.store();
        let task = self.inner.task;
        directory.validate_under_archive_root(store, self.source(), task, root)?;
        let position = self
            .inner
            .pins
            .pending_groups
            .buffer
            .as_ref()
            .and_then(|buffer| {
                buffer.slots.iter().position(|entry| {
                    entry
                        .as_ref()
                        .is_some_and(|entry| entry.group() == task.group())
                })
            });
        let Some(position) = position else {
            return Ok(None);
        };
        self.inner
            .pins
            .pending_groups
            .require_same_task(store, task)?;
        // Repeat the original exact source after all fixed-name/file observations. The final
        // move has no allocation, callback or fee destruction; permanent dir is not removed.
        self.inner.validate_source_only()?;
        let entry = self
            .inner
            .pins
            .pending_groups
            .buffer
            .as_mut()
            .and_then(|buffer| buffer.slots[position].take());
        Ok(entry.map(|entry| RetiredGroupEntry { _entry: entry }))
    }
}

fn lock_vacant_namespace<'guard, 'writer>(
    admission: &'guard VacantV2GenesisAdmission<'writer>,
    task: &'guard PreparedV2GenesisGroupNamespace,
) -> Result<OriginalOwnerNamespace<'guard, 'writer>, AppError> {
    admission.require_vacant_lineage_for_own_group_task(task)?;
    let store = admission.store();
    let pins = store.owner_evidence.pins.lock().map_err(|_| refused())?;
    let pool = store
        .owner_evidence
        .canonical
        .lock()
        .map_err(|_| refused())?;
    let association = admission.current_filename_association()?;
    let inner = OriginalOwnerNamespace {
        admission: OriginalAdmission::Vacant(admission),
        task,
        pins,
        pool,
        association,
        identifier: key(task.group(), task.signer()),
    };
    inner.validate_old_files()?;
    inner
        .pool
        .v2_writers
        .require_no_live_writer(&inner.identifier)?;
    Ok(inner)
}

impl ServerStore {
    pub(in crate::store) fn lock_registering_v2_group_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard VacantV2GenesisAdmission<'writer>,
        task: &'guard PreparedV2GenesisGroupNamespace,
    ) -> Result<RegisteringV2OwnerNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        let inner = lock_vacant_namespace(admission, task)?;
        if inner.pins.pending_groups.excludes_group(task.group()) {
            return Err(refused());
        }
        Ok(RegisteringV2OwnerNamespace { inner, admission })
    }
    pub(in crate::store) fn lock_creating_v2_group_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard VacantV2GenesisAdmission<'writer>,
        task: &'guard PreparedV2GenesisGroupNamespace,
    ) -> Result<CreatingV2OwnerNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        let inner = lock_vacant_namespace(admission, task)?;
        inner.pins.pending_groups.require_same_task(self, task)?;
        Ok(CreatingV2OwnerNamespace { inner, admission })
    }
    pub(in crate::store) fn lock_vacant_v2_group_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard VacantV2GenesisAdmission<'writer>,
        task: &'guard PreparedV2GenesisGroupNamespace,
    ) -> Result<VacantV2OwnerNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        task.saved_directory()?;
        let inner = lock_vacant_namespace(admission, task)?;
        inner.require_registered_or_saved_task()?;
        Ok(VacantV2OwnerNamespace { inner, admission })
    }
    pub(in crate::store) fn lock_initializing_v2_group_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard V2InitializingAdmission<'writer>,
        task: &'guard PreparedV2GenesisGroupNamespace,
    ) -> Result<InitializingV2OwnerNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        admission.require_initializing_lineage_for_own_group_task(task)?;
        task.saved_directory()?;
        let pins = self.owner_evidence.pins.lock().map_err(|_| refused())?;
        let pool = self
            .owner_evidence
            .canonical
            .lock()
            .map_err(|_| refused())?;
        let association = admission.current_filename_association()?;
        let inner = OriginalOwnerNamespace {
            admission: OriginalAdmission::Initializing(admission),
            task,
            pins,
            pool,
            association,
            identifier: key(task.group(), task.signer()),
        };
        inner.validate_old_files()?;
        inner.require_registered_or_saved_task()?;
        Ok(InitializingV2OwnerNamespace { inner, admission })
    }
    pub(in crate::store) fn lock_current_v2_group_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard V2CurrentAdmission<'writer>,
        task: &'guard PreparedV2GenesisGroupNamespace,
    ) -> Result<CurrentV2OwnerNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        admission.require_current_lineage_for_own_group_task(task)?;
        task.saved_directory()?;
        let pins = self.owner_evidence.pins.lock().map_err(|_| refused())?;
        let pool = self
            .owner_evidence
            .canonical
            .lock()
            .map_err(|_| refused())?;
        let association = admission.current_filename_association()?;
        let inner = OriginalOwnerNamespace {
            admission: OriginalAdmission::Current(admission),
            task,
            pins,
            pool,
            association,
            identifier: key(task.group(), task.signer()),
        };
        inner.validate_old_files()?;
        inner.require_registered_or_saved_task()?;
        Ok(CurrentV2OwnerNamespace { inner, admission })
    }
}
