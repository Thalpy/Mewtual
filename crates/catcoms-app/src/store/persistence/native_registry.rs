//! Read-only, fixed-name captures for the native-registry migration preparation.
//! No path supplied by a row or caller reaches these operations. The declaration writer
//! below accepts only the original private mount and its cached fixed-name outputs.
use super::*;
use crate::store::critical_namespaces::{self as critical, native_registry::preparation as native};
use catcoms_storage::kept::{session_at, session_id, SessionAt, SessionId};
use critical::read_pool::CatalogReadReservation;
pub(in crate::store) mod operational;

fn denied() -> AppError {
    critical::refused()
}
fn io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

fn real_file(metadata: &fs::Metadata) -> bool {
    let good = metadata.is_file() && !metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        good && metadata.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    {
        good
    }
}
fn absent(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(denied()),
    }
}

/// Every opener without the private native-registry physical handler reserves its
/// fixed artifacts fail-closed BEFORE older cleanup; any kind of presence or
/// access error is unsupported, never permission to discard future data.
pub(in crate::store) fn refuse_unsupported_storage(dir: &Path) -> Result<(), AppError> {
    if dir.as_os_str().len() > native::MAX_BASE_PATH {
        return Err(denied());
    }
    absent(&dir.join(native::SEED_NAME))?;
    absent(&dir.join(operational::UPGRADE_SEED_NAME))?;
    let root = dir.join(critical::ARCHIVE_BOOTSTRAP_ROOT);
    for name in [
        native::MARKER_NAME,
        native::INDEX_NAME,
        native::INDEX_STAGE_NAME,
        operational::UPGRADE_INDEX_NAME,
        operational::UPGRADE_MARKER_NAME,
    ] {
        absent(&root.join(name))?;
    }
    Ok(())
}
/// Fixed, bounded names only, before even allocating a path buffer.
fn path(store: &ServerStore, root: bool, name: &str) -> Result<PathBuf, AppError> {
    if store.dir.as_os_str().len() > native::MAX_BASE_PATH {
        return Err(denied());
    }
    let mut result = PathBuf::with_capacity(native::MAX_BASE_PATH + 256);
    result.push(&store.dir);
    if root {
        result.push(critical::ARCHIVE_BOOTSTRAP_ROOT);
    }
    if !name.is_empty() {
        result.push(name);
    }
    if result.as_os_str().len() > native::MAX_BASE_PATH + 256
        || result.capacity() > native::MAX_BASE_PATH + 256
    {
        return Err(denied());
    }
    Ok(result)
}

struct HeldDirectory {
    file: File,
    identity: SessionId,
}
impl HeldDirectory {
    fn capture(path: &Path) -> Result<Self, AppError> {
        if !archive_real_directory(&fs::symlink_metadata(path).map_err(io)?) {
            return Err(denied());
        }
        let file = open_archive_directory(path)?;
        let identity = session_id(&file).map_err(io)?;
        let result = Self { file, identity };
        result.require(path)?;
        Ok(result)
    }
    fn require(&self, path: &Path) -> Result<(), AppError> {
        if !archive_real_directory(&self.file.metadata().map_err(io)?)
            || !archive_real_directory(&fs::symlink_metadata(path).map_err(io)?)
            || session_id(&self.file).map_err(io)? != self.identity
        {
            return Err(denied());
        }
        // session_at is a REGULAR FILE association check. Directories must
        // instead reopen the named real directory and compare its OS identity
        // with the still-held original, as the existing Current handlers do.
        let named = open_archive_directory(path)?;
        if !archive_real_directory(&named.metadata().map_err(io)?)
            || session_id(&named).map_err(io)? != self.identity
        {
            return Err(denied());
        }
        Ok(())
    }
}

/// Present is always the original opened object plus its exact bounded frame.
/// Absent is a strict name association, rechecked at every use; an IO error is neither.
pub(in crate::store) struct HeldNativeInput {
    file: File,
    identity: SessionId,
    raw: Zeroizing<Vec<u8>>,
}
impl HeldNativeInput {
    pub(in crate::store) fn identity(&self) -> SessionId {
        self.identity
    }
    fn capture(path: &Path, cap: usize) -> Result<Option<Self>, AppError> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(metadata) if real_file(&metadata) && metadata.len() <= cap as u64 => {}
            _ => return Err(denied()),
        }
        let (file, raw) = read_archive_object(path, cap)?.ok_or_else(denied)?;
        let identity = session_id(&file).map_err(io)?;
        let result = Self {
            file,
            identity,
            raw,
        };
        result.require(path)?;
        Ok(Some(result))
    }
    fn require(&self, path: &Path) -> Result<(), AppError> {
        if !real_file(&self.file.metadata().map_err(io)?)
            || session_id(&self.file).map_err(io)? != self.identity
            || !matches!(session_at(path, self.identity), SessionAt::Ours)
        {
            return Err(denied());
        }
        let (named, bytes) = read_archive_object(path, self.raw.len())?.ok_or_else(denied)?;
        if !real_file(&named.metadata().map_err(io)?)
            || session_id(&named).map_err(io)? != self.identity
            || bytes != self.raw
            || !matches!(session_at(path, self.identity), SessionAt::Ours)
        {
            return Err(denied());
        }
        Ok(())
    }
    pub(in crate::store) fn raw(&self) -> &[u8] {
        &self.raw
    }
}

/// Original pre-cleaner objects held continuously from classification through role issuance.
/// A second pathname read, even of equal bytes, must not replace any of these observations.
pub(in crate::store) struct NativeDeclarationInventory {
    pub(in crate::store) seed: Option<HeldNativeInput>,
    pub(in crate::store) stage: Option<HeldNativeInput>,
    pub(in crate::store) marker: Option<HeldNativeInput>,
    pub(in crate::store) index: Option<HeldNativeInput>,
}
impl NativeDeclarationInventory {
    pub(in crate::store) fn capture(store: &ServerStore) -> Result<Self, AppError> {
        let value = Self {
            seed: HeldNativeInput::capture(
                &path(store, false, native::SEED_NAME)?,
                native::MAX_SEED_FRAME,
            )?,
            stage: HeldNativeInput::capture(
                &path(store, false, critical::Slot::Stage.name())?,
                native::MAX_CATALOG_FRAME,
            )?,
            marker: HeldNativeInput::capture(
                &path(store, true, native::MARKER_NAME)?,
                native::MAX_MARKER_FRAME,
            )?,
            index: HeldNativeInput::capture(
                &path(store, true, native::INDEX_NAME)?,
                native::MAX_INDEX_FRAME,
            )?,
        };
        value.require(store)?;
        Ok(value)
    }
    pub(in crate::store) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        for (root, name, value) in [
            (false, native::SEED_NAME, &self.seed),
            (false, critical::Slot::Stage.name(), &self.stage),
            (true, native::MARKER_NAME, &self.marker),
            (true, native::INDEX_NAME, &self.index),
        ] {
            let name = path(store, root, name)?;
            match value {
                Some(value) => value.require(&name)?,
                None => absent(&name)?,
            }
        }
        Ok(())
    }
}

/// The startup-private owner retains both directories and every input across preparation.
/// All buffers are paid before capture; no mutable actor/Host is exposed by its owner.
pub(in crate::store) struct NativeMigrationInputs {
    catalog: HeldNativeInput,
    legacy: Option<HeldNativeInput>,
    root: HeldDirectory,
    vault: HeldDirectory,
    // Negative first-torn compatibility role only. Seeing actual governance progress
    // means its Stage belongs to that writer; this role can remove ONLY our own seed.
    governance_compat: bool,
}
impl NativeMigrationInputs {
    pub(in crate::store) fn require_selected(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_writing(store)?;
        self.catalog
            .require(&path(store, false, critical::Slot::Current.name())?)?;
        self.require_writing(store)
    }
    pub(in crate::store) fn capture(
        store: &ServerStore,
        fee: &CatalogReadReservation,
        pool: &CriticalCatalogReadPool,
    ) -> Result<Self, AppError> {
        fee.validate(pool, native::PREPARATION_FEE)?;
        let vault = HeldDirectory::capture(&path(store, false, "")?)?;
        let root = HeldDirectory::capture(&path(store, true, "")?)?;
        require_undeclared_names(store)?;
        let catalog = HeldNativeInput::capture(
            &path(store, false, critical::Slot::Current.name())?,
            native::MAX_CATALOG_FRAME,
        )?
        .ok_or_else(denied)?;
        let legacy = HeldNativeInput::capture(&store.registry_path(), native::MAX_LEGACY_FRAME)?;
        let result = Self {
            catalog,
            legacy,
            root,
            vault,
            governance_compat: false,
        };
        result.require(store, fee, pool)?;
        Ok(result)
    }
    pub(in crate::store) fn catalog(&self) -> &[u8] {
        self.catalog.raw()
    }
    pub(in crate::store) fn legacy(&self) -> Option<&[u8]> {
        self.legacy.as_ref().map(HeldNativeInput::raw)
    }
    pub(in crate::store) fn require(
        &self,
        store: &ServerStore,
        fee: &CatalogReadReservation,
        pool: &CriticalCatalogReadPool,
    ) -> Result<(), AppError> {
        fee.validate(pool, native::PREPARATION_FEE)?;
        self.vault.require(&path(store, false, "")?)?;
        self.root.require(&path(store, true, "")?)?;
        require_undeclared_names(store)?;
        self.catalog
            .require(&path(store, false, critical::Slot::Current.name())?)?;
        match &self.legacy {
            Some(input) => input.require(&store.registry_path())?,
            None => absent(&store.registry_path())?,
        }
        // Parent associations bracket all leaf observations, including a concurrent rename.
        self.root.require(&path(store, true, "")?)?;
        self.vault.require(&path(store, false, "")?)
    }

    /// Cold handler capture: the selected catalog may already be the seeded candidate.
    /// It still holds actual directories, catalog and legacy input, never decoded labels.
    pub(in crate::store) fn capture_recovery(store: &ServerStore) -> Result<Self, AppError> {
        Self::capture_recovery_role(store, false)
    }
    pub(in crate::store) fn capture_governance_abandonment(
        store: &ServerStore,
    ) -> Result<Self, AppError> {
        Self::capture_recovery_role(store, true)
    }
    fn capture_recovery_role(
        store: &ServerStore,
        governance_compat: bool,
    ) -> Result<Self, AppError> {
        let vault = HeldDirectory::capture(&path(store, false, "")?)?;
        let root = HeldDirectory::capture(&path(store, true, "")?)?;
        let catalog = HeldNativeInput::capture(
            &path(store, false, critical::Slot::Current.name())?,
            native::MAX_CATALOG_FRAME,
        )?
        .ok_or_else(denied)?;
        let legacy = HeldNativeInput::capture(&store.registry_path(), native::MAX_LEGACY_FRAME)?;
        let value = Self {
            catalog,
            legacy,
            root,
            vault,
            governance_compat,
        };
        value.require_writing(store)?;
        Ok(value)
    }
    fn require_writing(&self, store: &ServerStore) -> Result<(), AppError> {
        self.vault.require(&path(store, false, "")?)?;
        self.root.require(&path(store, true, "")?)?;
        if self.governance_compat {
            require_governance_abandonment_debt(store, self.catalog.raw())?;
        } else {
            require_no_other_declaration(store)?;
        }
        match &self.legacy {
            Some(input) => input.require(&store.registry_path())?,
            None => absent(&store.registry_path())?,
        }
        self.root.require(&path(store, true, "")?)?;
        self.vault.require(&path(store, false, "")?)?;
        if store.chat_mount.observer().is_cancelled() {
            return Err(denied());
        }
        Ok(())
    }
}

pub(in crate::store) fn native_governance_seed_present(
    store: &ServerStore,
) -> Result<bool, AppError> {
    Ok(!matches!(fs::symlink_metadata(path(
            store,
            false,
            critical::governance_layout::GOVERNANCE_SEED,
        )?), Err(error) if error.kind() == std::io::ErrorKind::NotFound))
}
/// Recognize only complete known governance DATA, not a configured namespace4 owner. This
/// negative compatibility role leaves ALL governance files/Stage untouched and retires only
/// an unauthenticated ID7 claim. Public governance opening can then resume its own protocol.
fn require_governance_abandonment_debt(
    store: &ServerStore,
    current: &[u8],
) -> Result<(), AppError> {
    for name in [
        critical::ARCHIVE_DECLARATION_SEED,
        critical::current_group::DECLARATION_SEED,
        critical::member_outbox_layout::MEMBER_OUTBOX_SEED,
    ] {
        absent(&path(store, false, name)?)?;
    }
    for name in [
        native::MARKER_NAME,
        native::INDEX_NAME,
        native::INDEX_STAGE_NAME,
    ] {
        absent(&path(store, true, name)?)?;
    }
    let key = Zeroizing::new(store.keys.db_key()?);
    let governance = store.native_governance_abandonment_current_data(current)?;
    let seed_path = path(store, false, critical::governance_layout::GOVERNANCE_SEED)?;
    if let Some(seed) = HeldNativeInput::capture(
        &seed_path,
        critical::governance_layout::codec::MAX_SEED_FRAME,
    )? {
        let opened = critical::governance_layout::codec::open_seed(&key, seed.raw())?;
        let saved = critical::governance_layout::codec::parse_seed(
            &key,
            &store.chat_vault_domain()?,
            &opened,
        )?;
        if current != saved.original && current != saved.candidate {
            return Err(denied());
        }
        seed.require(&seed_path)?;
    } else {
        let cap = governance.ok_or_else(denied)?;
        let marker_path = path(store, true, critical::governance_layout::GOVERNANCE_MARKER)?;
        let marker = HeldNativeInput::capture(
            &marker_path,
            critical::governance_layout::codec::MAX_MARKER_FRAME,
        )?
        .ok_or_else(denied)?;
        critical::governance_layout::codec::require_marker(&key, cap, marker.raw())?;
        marker.require(&marker_path)?;
    }
    Ok(())
}

fn require_no_other_declaration(store: &ServerStore) -> Result<(), AppError> {
    for name in [
        critical::ARCHIVE_DECLARATION_SEED,
        critical::current_group::DECLARATION_SEED,
        critical::governance_layout::GOVERNANCE_SEED,
        critical::member_outbox_layout::MEMBER_OUTBOX_SEED,
    ] {
        absent(&path(store, false, name)?)?;
    }
    absent(&path(store, true, native::INDEX_STAGE_NAME)?)
}

/// Borrowed cached ciphertexts, not a publication capability. The private caller authenticates
/// their complete relation before every call; no path or generated replacement is accepted.
pub(in crate::store) struct NativeDeclarationBytes<'a> {
    pub(in crate::store) original: &'a [u8],
    pub(in crate::store) candidate: &'a [u8],
    pub(in crate::store) seed: &'a [u8],
    pub(in crate::store) marker: &'a [u8],
    pub(in crate::store) index: &'a [u8],
}

/// Warm handles are cached BEFORE a fallible sync or write. Cold construction separately
/// classifies every artifact before issuing this same concrete completion role. No handle
/// is adopted on a warm retry merely because a replacement has identical ciphertext.
#[derive(Default)]
pub(in crate::store) struct NativeDeclarationIo {
    seed: Option<critical::pending_join::CreatedStage>,
    stage: Option<critical::pending_join::CreatedStage>,
    marker: Option<critical::pending_join::CreatedStage>,
    index: Option<critical::pending_join::CreatedStage>,
    promoted: bool,
    seed_removed: bool,
    duplicate_removed: bool,
}

fn cut(point: u8) -> Result<(), AppError> {
    #[cfg(test)]
    if INSTALL_FAULT.with(|value| value.get()) == point {
        INSTALL_FAULT.with(|value| value.set(0));
        INSTALL_FIRED.with(|value| value.set(true));
        return Err(AppError::Io(
            "injected native registry declaration cut".into(),
        ));
    }
    let _ = point;
    Ok(())
}
#[cfg(test)]
thread_local! {
    static INSTALL_FAULT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    static INSTALL_FIRED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static BEFORE_SEED_REMOVAL: std::cell::RefCell<Option<NativeRemovalHook>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
type NativeRemovalHook = Box<dyn FnOnce(&Path)>;
#[cfg(test)]
pub(in crate::store) fn before_native_seed_removal_for_test(hook: NativeRemovalHook) {
    BEFORE_SEED_REMOVAL.with(|value| *value.borrow_mut() = Some(hook));
}
#[cfg(test)]
pub(in crate::store) fn arm_native_install_fault(point: u8) {
    INSTALL_FAULT.with(|value| value.set(point));
    INSTALL_FIRED.with(|value| value.set(false));
}
#[cfg(test)]
pub(in crate::store) fn take_native_install_fault() -> bool {
    INSTALL_FIRED.with(|value| value.replace(false))
}

fn held(path: &Path, value: &critical::pending_join::CreatedStage) -> Result<(), AppError> {
    let identity = value.identity.ok_or_else(denied)?;
    if !real_file(&value.file.metadata().map_err(io)?)
        || session_id(&value.file).map_err(io)? != identity
        || !matches!(session_at(path, identity), SessionAt::Ours)
    {
        return Err(denied());
    }
    Ok(())
}
fn exact(
    path: &Path,
    value: &critical::pending_join::CreatedStage,
    expected: &[u8],
) -> Result<(), AppError> {
    held(path, value)?;
    let named = HeldNativeInput::capture(path, expected.len())?.ok_or_else(denied)?;
    if Some(named.identity) != value.identity || named.raw() != expected {
        return Err(denied());
    }
    held(path, value)
}
fn capture_writable(
    path: &Path,
    expected: &[u8],
    exact_only: bool,
) -> Result<Option<critical::pending_join::CreatedStage>, AppError> {
    let Some(named) = HeldNativeInput::capture(path, expected.len())? else {
        return Ok(None);
    };
    if named.raw() != &expected[..named.raw().len()] || (exact_only && named.raw() != expected) {
        return Err(denied());
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io)?;
    let identity = session_id(&file).map_err(io)?;
    if identity != named.identity {
        return Err(denied());
    }
    let value = critical::pending_join::CreatedStage {
        file,
        identity: Some(identity),
    };
    held(path, &value)?;
    Ok(Some(value))
}
fn claim(
    path: &Path,
    slot: &mut Option<critical::pending_join::CreatedStage>,
) -> Result<(), AppError> {
    if slot.is_none() {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(io)?;
        *slot = Some(critical::pending_join::CreatedStage {
            file,
            identity: None,
        });
    }
    let value = slot.as_mut().ok_or_else(denied)?;
    if value.identity.is_none() {
        value.identity = Some(session_id(&value.file).map_err(io)?);
    }
    held(path, value)?;
    value.file.sync_all().map_err(io)?;
    held(path, value)
}
fn complete(
    path: &Path,
    slot: &mut Option<critical::pending_join::CreatedStage>,
    expected: &[u8],
    short: u8,
) -> Result<(), AppError> {
    use std::io::{Seek, SeekFrom};
    claim(path, slot)?;
    let value = slot.as_mut().ok_or_else(denied)?;
    let named = HeldNativeInput::capture(path, expected.len())?.ok_or_else(denied)?;
    if Some(named.identity) != value.identity || named.raw() != &expected[..named.raw().len()] {
        return Err(denied());
    }
    let offset = named.raw().len();
    drop(named);
    value
        .file
        .seek(SeekFrom::Start(offset as u64))
        .map_err(io)?;
    #[cfg(test)]
    if INSTALL_FAULT.with(|value| value.get()) == short && short != 0 {
        let end = offset + (expected.len() - offset) / 2;
        value.file.write_all(&expected[offset..end]).map_err(io)?;
        cut(short)?;
    }
    let _ = short;
    value.file.write_all(&expected[offset..]).map_err(io)?;
    value.file.sync_all().map_err(io)?;
    exact(path, value, expected)
}

impl NativeDeclarationIo {
    pub(in crate::store) fn capture_recovery(
        store: &ServerStore,
        bytes: &NativeDeclarationBytes<'_>,
        selected: &[u8],
        inventory: &NativeDeclarationInventory,
    ) -> Result<Self, AppError> {
        inventory.require(store)?;
        let promoted = selected == bytes.candidate;
        if !promoted && selected != bytes.original {
            return Err(denied());
        }
        // Classify ALL names before any write. After promotion, missing/partial index or
        // marker cannot be reconstructed even from a valid seed: commit implied both.
        let value = Self {
            seed: capture_writable(&path(store, false, native::SEED_NAME)?, bytes.seed, true)?,
            stage: capture_writable(
                &path(store, false, critical::Slot::Stage.name())?,
                bytes.candidate,
                promoted,
            )?,
            marker: capture_writable(
                &path(store, true, native::MARKER_NAME)?,
                bytes.marker,
                promoted,
            )?,
            index: capture_writable(
                &path(store, true, native::INDEX_NAME)?,
                bytes.index,
                promoted,
            )?,
            promoted,
            seed_removed: false,
            duplicate_removed: false,
        };
        if value.seed.is_none() || (promoted && (value.marker.is_none() || value.index.is_none())) {
            return Err(denied());
        }
        for (issued, original) in [
            (&value.seed, &inventory.seed),
            (&value.stage, &inventory.stage),
            (&value.marker, &inventory.marker),
            (&value.index, &inventory.index),
        ] {
            if issued.as_ref().and_then(|file| file.identity)
                != original.as_ref().map(HeldNativeInput::identity)
            {
                return Err(denied());
            }
        }
        inventory.require(store)?;
        Ok(value)
    }

    pub(in crate::store) fn write(
        &mut self,
        store: &ServerStore,
        inputs: &NativeMigrationInputs,
        bytes: &NativeDeclarationBytes<'_>,
    ) -> Result<(), AppError> {
        inputs.require_writing(store)?;
        let current = path(store, false, critical::Slot::Current.name())?;
        let stage = path(store, false, critical::Slot::Stage.name())?;
        let seed = path(store, false, native::SEED_NAME)?;
        let marker = path(store, true, native::MARKER_NAME)?;
        let index = path(store, true, native::INDEX_NAME)?;
        if !self.promoted {
            inputs.catalog.require(&current)?;
            if inputs.catalog.raw() != bytes.original {
                return Err(denied());
            }
            claim(&seed, &mut self.seed)?;
            inputs.vault.file.sync_all().map_err(io)?;
            cut(1)?;
            claim(&stage, &mut self.stage)?;
            inputs.vault.file.sync_all().map_err(io)?;
            cut(2)?;
            complete(&seed, &mut self.seed, bytes.seed, 3)?;
            inputs.vault.file.sync_all().map_err(io)?;
            cut(4)?;
            complete(&index, &mut self.index, bytes.index, 5)?;
            inputs.root.file.sync_all().map_err(io)?;
            cut(6)?;
            complete(&marker, &mut self.marker, bytes.marker, 7)?;
            inputs.root.file.sync_all().map_err(io)?;
            cut(8)?;
            complete(&stage, &mut self.stage, bytes.candidate, 9)?;
            inputs.vault.file.sync_all().map_err(io)?;
            cut(10)?;
            inputs.require_writing(store)?;
            inputs.catalog.require(&current)?;
            exact(&seed, self.seed.as_ref().ok_or_else(denied)?, bytes.seed)?;
            exact(
                &marker,
                self.marker.as_ref().ok_or_else(denied)?,
                bytes.marker,
            )?;
            exact(&index, self.index.as_ref().ok_or_else(denied)?, bytes.index)?;
            exact(
                &stage,
                self.stage.as_ref().ok_or_else(denied)?,
                bytes.candidate,
            )?;
            // Last original-mount/name proof immediately precedes selection. There is no
            // actor, exposed alias or online writer in this startup-private transaction.
            inputs.require_writing(store)?;
            fs::rename(&stage, &current).map_err(io)?;
            self.promoted = true; // Cache selection BEFORE the first fallible covering barrier.
            cut(11).map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
        }
        let selected = if inputs.catalog.raw() == bytes.candidate {
            inputs.catalog.require(&current)?;
            &inputs.catalog.file
        } else {
            let original = self.stage.as_ref().ok_or_else(denied)?;
            exact(&current, original, bytes.candidate)?;
            &original.file
        };
        selected.sync_all().map_err(io)?;
        inputs.vault.file.sync_all().map_err(io)?;
        for (name, slot, expected) in [
            (&marker, &self.marker, bytes.marker),
            (&index, &self.index, bytes.index),
        ] {
            let value = slot.as_ref().ok_or_else(denied)?;
            exact(name, value, expected)?;
            value.file.sync_all().map_err(io)?;
        }
        inputs.root.file.sync_all().map_err(io)?;
        inputs.require_writing(store)?;
        if inputs.catalog.raw() == bytes.candidate && !self.duplicate_removed {
            if let Some(value) = self.stage.as_ref() {
                exact(&stage, value, bytes.candidate)?;
                fs::remove_file(&stage).map_err(io)?;
                drop(self.stage.take());
            } else {
                absent(&stage)?;
            }
            self.duplicate_removed = true;
        } else {
            absent(&stage)?;
        }
        inputs.vault.file.sync_all().map_err(io)?;
        absent(&stage)?;
        if !self.seed_removed {
            exact(&seed, self.seed.as_ref().ok_or_else(denied)?, bytes.seed)?;
            #[cfg(test)]
            if let Some(hook) = BEFORE_SEED_REMOVAL.with(|value| value.borrow_mut().take()) {
                hook(&store.dir);
            }
            // Seed verification may be a large read. Re-prove the selected name AFTER it,
            // and after the legacy/input waits, so cleanup cannot strand a rolled-back
            // catalog with orphaned index/marker and no remaining declaration evidence.
            inputs.require_writing(store)?;
            exact(&index, self.index.as_ref().ok_or_else(denied)?, bytes.index)?;
            exact(
                &marker,
                self.marker.as_ref().ok_or_else(denied)?,
                bytes.marker,
            )?;
            if inputs.catalog.raw() == bytes.candidate {
                inputs.catalog.require(&current)?;
            } else {
                exact(
                    &current,
                    self.stage.as_ref().ok_or_else(denied)?,
                    bytes.candidate,
                )?;
            }
            held(&seed, self.seed.as_ref().ok_or_else(denied)?)?;
            if store.chat_mount.observer().is_cancelled() {
                return Err(denied());
            }
            fs::remove_file(&seed).map_err(io)?;
            self.seed_removed = true;
            // Windows delete-pending: release OUR last handle before any fallible barrier.
            drop(self.seed.take());
            cut(12)?;
        }
        absent(&seed)?;
        inputs.vault.file.sync_all().map_err(io)?;
        cut(13)?;
        absent(&seed)?;
        inputs.require_writing(store)
    }
}

/// Closed fixed-name reads for the pre-cleaner handler. Callers authenticate content before
/// acquiring a completion role. Regular-file and no-reparse checks precede allocation.
pub(in crate::store) fn read_native_current(
    store: &ServerStore,
) -> Result<Option<Zeroizing<Vec<u8>>>, AppError> {
    Ok(HeldNativeInput::capture(
        &path(store, false, critical::Slot::Current.name())?,
        native::MAX_CATALOG_FRAME,
    )?
    .map(|value| value.raw))
}

pub(in crate::store) fn native_artifacts_present(store: &ServerStore) -> Result<bool, AppError> {
    for (root, name) in [
        (false, native::SEED_NAME),
        (true, native::MARKER_NAME),
        (true, native::INDEX_NAME),
        (true, native::INDEX_STAGE_NAME),
    ] {
        match fs::symlink_metadata(path(store, root, name)?) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Ok(true),
        }
    }
    Ok(false)
}

/// Dispose only a first seed that never authenticated, with an authenticated pre-ID7
/// catalog and no output. The caller classifies both catalog slots before calling here.
pub(in crate::store) fn dispose_native_first_torn(
    store: &ServerStore,
    inputs: &NativeMigrationInputs,
    inventory: NativeDeclarationInventory,
) -> Result<(), AppError> {
    inputs.require_writing(store)?;
    inventory.require(store)?;
    inputs
        .catalog
        .require(&path(store, false, critical::Slot::Current.name())?)?;
    for name in [native::MARKER_NAME, native::INDEX_NAME] {
        absent(&path(store, true, name)?)?;
    }
    let seed_path = path(store, false, native::SEED_NAME)?;
    let NativeDeclarationInventory {
        seed,
        stage,
        marker,
        index,
    } = inventory;
    if marker.is_some() || index.is_some() {
        return Err(denied());
    }
    let seed = seed.ok_or_else(denied)?;
    let stage_path = path(store, false, critical::Slot::Stage.name())?;
    if let Some(stage) = stage.filter(|_| !inputs.governance_compat) {
        if stage.raw().is_empty() {
            stage.require(&stage_path)?;
            fs::remove_file(&stage_path).map_err(io)?;
            drop(stage);
            cut(14)?;
            absent(&stage_path)?;
            cut(15)?;
            inputs.vault.file.sync_all().map_err(io)?;
            absent(&stage_path)?;
        }
    }
    inputs.require_writing(store)?;
    seed.require(&seed_path)?;
    fs::remove_file(&seed_path).map_err(io)?;
    drop(seed);
    cut(16)?;
    absent(&seed_path)?;
    inputs.vault.file.sync_all().map_err(io)?;
    cut(17)?;
    absent(&seed_path)
}

fn require_undeclared_names(store: &ServerStore) -> Result<(), AppError> {
    absent(&path(store, false, critical::Slot::Stage.name())?)?;
    absent(&path(store, false, native::SEED_NAME)?)?;
    for name in [
        critical::ARCHIVE_DECLARATION_SEED,
        critical::current_group::DECLARATION_SEED,
        critical::governance_layout::GOVERNANCE_SEED,
        critical::member_outbox_layout::MEMBER_OUTBOX_SEED,
    ] {
        absent(&path(store, false, name)?)?;
    }
    for name in [
        native::MARKER_NAME,
        native::INDEX_NAME,
        native::INDEX_STAGE_NAME,
    ] {
        absent(&path(store, true, name)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_registry_directory_capture_refuses_an_equal_empty_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("original");
        fs::create_dir(&path).unwrap();
        let capture = HeldDirectory::capture(&path).unwrap();
        capture.require(&path).unwrap();
        fs::rename(&path, dir.path().join("retained-original")).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(capture.require(&path).is_err());
    }
}
