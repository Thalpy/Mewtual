//! Closed-name, bounded READS for the operational text capture.
//!
//! These objects never create/sync/rename/unlink anything. In particular a missing object
//! is not a completed transaction, a cleared pending intent, or scratch-disposal authority.
//! The caller holds the actual Current/Root guard and an independent concrete read invoice.
use super::common::refused;
use crate::AppError;
use catcoms_storage::kept::{session_at, session_id, SessionAt, SessionId};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub(super) const MAX_TEXT_PATH: usize = 32 * 1024 + 256;

fn io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

/// An ephemeral held original. It must drain before the next operational control
/// replacement; saved/negative/query metadata must retain frozen facts rather than this FD.
pub(super) struct OpenedTextControl {
    pub(super) file: File,
    pub(super) id: SessionId,
    pub(super) frame: Zeroizing<Vec<u8>>,
}
impl std::fmt::Debug for OpenedTextControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedTextControl").finish_non_exhaustive()
    }
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

/// The only names accepted by the physical reader are fixed literals selected in this
/// module or the complete hex encoding of an authenticated immutable control ID.
pub(super) fn fixed_child(base: &Path, leaf: &'static str) -> Result<PathBuf, AppError> {
    if !matches!(
        leaf,
        "head.bin"
            | "head.stage"
            | "text.seed"
            | "text.seed.stage.v1"
            | "text.wal"
            | "text.wal.stage.v1"
            | "text.pages"
    ) {
        return Err(refused());
    }
    child(base, leaf)
}

fn child(base: &Path, leaf: &str) -> Result<PathBuf, AppError> {
    let capacity = base
        .as_os_str()
        .len()
        .checked_add(1 + leaf.len())
        .filter(|n| *n <= MAX_TEXT_PATH)
        .ok_or_else(refused)?;
    let mut path = PathBuf::with_capacity(capacity);
    path.push(base);
    path.push(leaf);
    if path.capacity() > MAX_TEXT_PATH {
        return Err(refused());
    }
    Ok(path)
}

/// Archive H has a closed `h-<complete lowercase hex ID>.bin` name, distinct from private
/// page names. No caller-provided filename, shortened display hash, or directory scan.
pub(super) fn archive_child(base: &Path, id: &[u8; 32]) -> Result<PathBuf, AppError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(70);
    name.push_str("h-");
    for byte in id {
        name.push(HEX[(byte >> 4) as usize] as char);
        name.push(HEX[(byte & 15) as usize] as char);
    }
    name.push_str(".bin");
    child(base, &name)
}

pub(super) fn require_named(path: &Path, object: &OpenedTextControl) -> Result<(), AppError> {
    if session_id(&object.file).map_err(io)? != object.id
        || !matches!(session_at(path, object.id), SessionAt::Ours)
    {
        return Err(refused());
    }
    Ok(())
}

/// A single bounded buffer includes its EOF probe. Pre-open, held metadata, native identity
/// and post-read name checks are independent: an exact-sized link/replacement is not accepted.
/// No `read_to_end` growth is hidden behind the cap and no partial read is interpreted as EOF.
pub(super) fn read_control(path: &Path, maximum: usize) -> Result<OpenedTextControl, AppError> {
    if maximum == 0 || maximum > 2 * 1024 * 1024 + 40 {
        return Err(refused());
    }
    let metadata = fs::symlink_metadata(path).map_err(io)?;
    if !regular(&metadata) || metadata.len() == 0 || metadata.len() > maximum as u64 {
        return Err(refused());
    }
    let mut file = File::open(path).map_err(io)?;
    let metadata = file.metadata().map_err(io)?;
    let length = usize::try_from(metadata.len()).map_err(|_| refused())?;
    if !regular(&metadata) || length == 0 || length > maximum {
        return Err(refused());
    }
    let id = session_id(&file).map_err(io)?;
    if !matches!(session_at(path, id), SessionAt::Ours) {
        return Err(refused());
    }
    let capacity = length.checked_add(1).ok_or_else(refused)?;
    let mut frame = Zeroizing::new(vec![0; capacity]);
    let mut used = 0;
    while used < capacity {
        match file.read(&mut frame[used..]) {
            Ok(0) => break,
            Ok(n) => used += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(io(error)),
        }
    }
    if used != length {
        return Err(refused());
    }
    frame.truncate(used);
    let object = OpenedTextControl { file, id, frame };
    require_named(path, &object)?;
    Ok(object)
}

#[cfg(test)]
mod tests;
