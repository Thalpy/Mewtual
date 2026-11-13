//! Read-only, fixed-name captures for the native-registry migration preparation.
//! No path supplied by a row or caller reaches these operations. No writer lives here yet.
use super::*;
use crate::store::critical_namespaces::{self as critical, native_registry::preparation as native};
use catcoms_storage::kept::{session_at, session_id, SessionAt, SessionId};
use critical::read_pool::CatalogReadReservation;

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

/// Every opener still lacks the native-registry physical handler. Reserve its
/// fixed artifacts fail-closed BEFORE older cleanup; any kind of presence or
/// access error is unsupported, never permission to discard future data.
pub(in crate::store) fn refuse_unsupported_storage(dir: &Path) -> Result<(), AppError> {
    if dir.as_os_str().len() > native::MAX_BASE_PATH {
        return Err(denied());
    }
    absent(&dir.join(native::SEED_NAME))?;
    let root = dir.join(critical::ARCHIVE_BOOTSTRAP_ROOT);
    for name in [
        native::MARKER_NAME,
        native::INDEX_NAME,
        native::INDEX_STAGE_NAME,
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

/// The startup-private owner retains both directories and every input across preparation.
/// All buffers are paid before capture; no mutable actor/Host is exposed by its owner.
pub(in crate::store) struct NativeMigrationInputs {
    catalog: HeldNativeInput,
    legacy: Option<HeldNativeInput>,
    root: HeldDirectory,
    vault: HeldDirectory,
}
impl NativeMigrationInputs {
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
