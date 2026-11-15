//! The ONLY raw filesystem mutation sites of the member outbox.
//!
//! `scripts/check-store-raw-fs.sh` lists each mutating line here exactly once. These helpers
//! acquire no permission themselves: their callers in `transaction.rs`/`engine.rs` hold the
//! exclusive per-context owner claim and the actual `VaultPhysicalWorkLease` for the whole
//! operation, and every `File` opened here is closed before that lease drops.
//!
//! Invariants every helper preserves:
//! * Nothing is truncated. An existing file is either exactly the expected bytes, or a strict
//!   prefix of them that is completed by appending the missing suffix; any other content is
//!   preserved untouched and refused.
//! * Native identity (`SessionId`) is rechecked around each write, so a name swapped to
//!   another object mid-operation is refused rather than written through.
//! * A non-regular object (directory, link, reparse point) at a fixed name is never ours.
use super::data::refused;
use crate::AppError;
use catcoms_storage::kept::{session_at, session_id, SessionAt, SessionId};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Largest file any member-outbox helper reads or writes (a complete WAL file).
pub(super) const MAX_FILE: usize = super::frames::WAL_FILE_CAP;

pub(super) fn io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

fn regular(metadata: &fs::Metadata) -> bool {
    let plain = metadata.file_type().is_file() && !metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        plain && metadata.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    plain
}

/// A directory held by native identity. Every barrier re-proves the name still refers to it.
pub(super) struct HeldDirectory {
    path: PathBuf,
    id: SessionId,
}
impl std::fmt::Debug for HeldDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldDirectory").finish_non_exhaustive()
    }
}
impl HeldDirectory {
    pub(super) fn open(path: PathBuf) -> Result<Self, AppError> {
        let directory = crate::store::persistence::open_archive_directory(&path)?;
        let id = session_id(&directory).map_err(io)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &path, &directory, id,
        )?;
        Ok(Self { path, id })
    }
    /// Hold an ALREADY captured directory by its original native identity (the namespace5
    /// mount's parent). Unlike [`Self::open`] this never adopts whatever the name refers to
    /// now: it refuses unless the name still refers to exactly that directory object.
    pub(super) fn adopt(path: PathBuf, id: SessionId) -> Result<Self, AppError> {
        let held = Self { path, id };
        held.require()?;
        Ok(held)
    }
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
    pub(super) fn require(&self) -> Result<(), AppError> {
        let directory = crate::store::persistence::open_archive_directory(&self.path)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &self.path, &directory, self.id,
        )
    }
    /// Directory durability barrier, bracketed by identity checks.
    pub(super) fn sync(&self) -> Result<(), AppError> {
        self.require()?;
        crate::store::persistence::sync_directory(&self.path).map_err(io)?;
        self.require()
    }
    pub(super) fn child(&self, leaf: &str) -> PathBuf {
        self.path.join(leaf)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Presence {
    Absent,
    Present,
}

/// Observe a fixed regular-file name. Only NotFound is absence; any other error stays one.
pub(super) fn observe(path: &Path) -> Result<Presence, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if regular(&metadata) => Ok(Presence::Present),
        Ok(_) => Err(refused()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Presence::Absent),
        Err(error) => Err(io(error)),
    }
}

/// Observe the namespace directory name itself.
pub(super) fn observe_directory(path: &Path) -> Result<Presence, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            Ok(Presence::Present)
        }
        Ok(_) => Err(refused()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Presence::Absent),
        Err(error) => Err(io(error)),
    }
}

/// One bounded read with native identity checks before and after. `allow_empty` exists only
/// for classifying a possibly torn fixed stage; every other caller refuses an empty file.
pub(super) fn read_bounded(
    path: &Path,
    maximum: usize,
    allow_empty: bool,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if maximum == 0 || maximum > MAX_FILE {
        return Err(refused());
    }
    let metadata = fs::symlink_metadata(path).map_err(io)?;
    if !regular(&metadata) || metadata.len() > maximum as u64 {
        return Err(refused());
    }
    let mut file = File::open(path).map_err(io)?;
    let id = session_id(&file).map_err(io)?;
    let metadata = file.metadata().map_err(io)?;
    let length = usize::try_from(metadata.len()).map_err(|_| refused())?;
    if !regular(&metadata) || length > maximum || (length == 0 && !allow_empty) {
        return Err(refused());
    }
    require_same_name(path, &file, id)?;
    // One extra byte detects growth after the length check; no partial read is EOF.
    let capacity = length.checked_add(1).ok_or_else(refused)?;
    let mut buffer = Zeroizing::new(vec![0; capacity]);
    let mut used = 0;
    while used < capacity {
        match file.read(&mut buffer[used..]) {
            Ok(0) => break,
            Ok(n) => used += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(io(error)),
        }
    }
    if used != length {
        return Err(refused());
    }
    buffer.truncate(used);
    require_same_name(path, &file, id)?;
    Ok(buffer)
}

/// Make `path` hold exactly `expected`, durably. The caller must have authenticated
/// `expected` (it is our own presealed output or an authenticated WAL's exact frame).
///
/// * Absent: create_new, write all, sync.
/// * Present and a prefix (possibly empty) of `expected`: append only the missing suffix,
///   sync. This completes a torn write of the SAME bytes; it can never adopt other content.
/// * Present with any other content or a longer length: refused, file untouched.
///
/// The parent-directory barrier is the caller's, after all outputs of a step.
pub(super) fn ensure_exact(path: &Path, expected: &[u8]) -> Result<(), AppError> {
    if expected.is_empty() || expected.len() > MAX_FILE {
        return Err(refused());
    }
    let mut file = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !regular(&metadata) || metadata.len() > expected.len() as u64 {
                return Err(refused());
            }
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(io)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(io)?,
        Err(error) => return Err(io(error)),
    };
    let native = session_id(&file).map_err(io)?;
    require_same_name(path, &file, native)?;
    let metadata = file.metadata().map_err(io)?;
    if !regular(&metadata) || metadata.len() > expected.len() as u64 {
        return Err(refused());
    }
    let used = usize::try_from(metadata.len()).map_err(|_| refused())?;
    require_file_bytes(&mut file, &expected[..used])?;
    if used < expected.len() {
        file.seek(SeekFrom::End(0)).map_err(io)?;
        file.write_all(&expected[used..]).map_err(io)?;
    }
    file.sync_all().map_err(io)?;
    require_file_bytes(&mut file, expected)?;
    require_same_name(path, &file, native)
}

/// Rename one fixed name onto another in the same held directory (the WAL MAIN promotion
/// or the selector CAS). The caller has verified the source's exact bytes immediately before.
pub(super) fn rename_fixed(from: &Path, to: &Path) -> Result<(), AppError> {
    fs::rename(from, to).map_err(io)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Removal {
    Removed,
    AlreadyAbsent,
}

/// Remove one regular file the caller has proven deletable (a WAL-listed retiree, the
/// retired WAL itself, or a demonstrably owned pre-commit scratch stage).
pub(super) fn remove_listed(path: &Path) -> Result<Removal, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if regular(&metadata) => {}
        Ok(_) => return Err(refused()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Removal::AlreadyAbsent)
        }
        Err(error) => return Err(io(error)),
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(Removal::Removed),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Removal::AlreadyAbsent),
        Err(error) => Err(io(error)),
    }
}

/// Create the namespace directory. It must not exist; an existing one is opened instead.
pub(super) fn create_namespace(path: &Path) -> Result<(), AppError> {
    fs::create_dir(path).map_err(io)
}

/// At most `maximum + 1` names in a held directory. Used ONLY when `head.bin` is missing, to
/// prove a namespace holds nothing but an interrupted creation before completing it.
pub(super) fn bounded_entries(
    directory: &HeldDirectory,
    maximum: usize,
) -> Result<Vec<OsString>, AppError> {
    directory.require()?;
    let mut names = Vec::with_capacity(maximum + 1);
    for entry in fs::read_dir(directory.path()).map_err(io)? {
        names.push(entry.map_err(io)?.file_name());
        if names.len() > maximum {
            break;
        }
    }
    directory.require()?;
    Ok(names)
}

fn require_same_name(path: &Path, file: &File, native: SessionId) -> Result<(), AppError> {
    if session_id(file).map_err(io)? != native
        || !matches!(session_at(path, native), SessionAt::Ours)
    {
        return Err(refused());
    }
    Ok(())
}

/// Stream-compare the whole file against `expected`, including an EOF probe.
fn require_file_bytes(file: &mut File, expected: &[u8]) -> Result<(), AppError> {
    let metadata = file.metadata().map_err(io)?;
    if !regular(&metadata) || metadata.len() != expected.len() as u64 {
        return Err(refused());
    }
    file.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut used = 0;
    let mut buffer = [0; 8192];
    while used < expected.len() {
        let wanted = (expected.len() - used).min(buffer.len());
        match file.read(&mut buffer[..wanted]) {
            Ok(0) => return Err(refused()),
            Ok(n) if buffer[..n] == expected[used..used + n] => used += n,
            Ok(_) => return Err(refused()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(io(error)),
        }
    }
    loop {
        match file.read(&mut buffer[..1]) {
            Ok(0) => break,
            Ok(_) => return Err(refused()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(io(error)),
        }
    }
    Ok(())
}
