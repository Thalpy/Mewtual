//! Native helpers used ONLY inside an already-admitted finite A worker.
//!
//! These functions do not acquire permission or retain a session themselves. Their caller
//! holds the SAME consumed Stage/Commit marker until every returned/local File is closed.
//! A cached native ID is only a refusal filter: an existing file must contain the complete
//! exact frame. Partial or unknown files are preserved, never appended, adopted or removed.
use super::super::super::artifacts::TextArtifactRoleData;
use super::super::super::common::refused;
use super::super::super::io::{read_control, require_named, OpenedTextControl};
use crate::AppError;
use catcoms_storage::kept::{session_at, session_id, SessionAt, SessionId};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAX_FRAME: usize = 2 * 1024 * 1024 + 40;

fn io(error: impl std::fmt::Display) -> AppError {
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

pub(in crate::store::persistence::v2_text::transaction) fn require_directory(
    path: &Path,
    id: SessionId,
) -> Result<File, AppError> {
    let directory = crate::store::persistence::open_archive_directory(path)?;
    crate::store::persistence::v2_genesis_activation::require_directory_name(path, &directory, id)?;
    Ok(directory)
}

pub(in crate::store::persistence::v2_text::transaction) fn sync_parent(
    path: &Path,
    id: SessionId,
) -> Result<(), AppError> {
    let directory = require_directory(path, id)?;
    crate::store::persistence::sync_directory(path).map_err(io)?;
    crate::store::persistence::v2_genesis_activation::require_directory_name(path, &directory, id)
}

pub(in crate::store::persistence::v2_text::transaction) fn require_absent(
    path: &Path,
) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(refused()),
    }
}

pub(in crate::store::persistence::v2_text::transaction) fn require_exact(
    path: &Path,
    id: Option<SessionId>,
    expected: &[u8],
) -> Result<OpenedTextControl, AppError> {
    if expected.is_empty() || expected.len() > MAX_FRAME {
        return Err(refused());
    }
    let control = read_control(path, expected.len())?;
    if id.is_some_and(|id| id != control.id) || control.frame.as_slice() != expected {
        return Err(refused());
    }
    Ok(control)
}

/// Flush a COMPLETE already-verified object through a writable SAME-object handle.
/// Windows requires write access for FlushFileBuffers. No bytes are changed or truncated.
pub(in crate::store::persistence::v2_text::transaction) fn sync_exact(
    path: &Path,
    id: Option<SessionId>,
    expected: &[u8],
) -> Result<OpenedTextControl, AppError> {
    let control = require_exact(path, id, expected)?;
    let mut writable = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io)?;
    if session_id(&writable).map_err(io)? != control.id {
        return Err(refused());
    }
    require_named(path, &control)?;
    writable.sync_all().map_err(io)?;
    require_file_bytes(&mut writable, expected)?;
    require_named(path, &control)?;
    Ok(control)
}

/// The only content-writing helper: create_new, cache native identity, then write the
/// original complete bytes. The callback runs with the actual newly created File open,
/// after identity caching and before ANY content write. Its refusal keeps the file intact.
/// A retry with cached identity may sync only an already complete exact object.
pub(in crate::store::persistence::v2_text::transaction) fn write_complete_only(
    path: &Path,
    expected: &[u8],
    recorded: &mut Option<SessionId>,
    created: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), AppError> {
    if expected.is_empty() || expected.len() > MAX_FRAME {
        return Err(refused());
    }
    if let Some(id) = *recorded {
        // This also rejects a same-ID partial. Closed-ID reuse cannot renew a write grant.
        drop(sync_exact(path, Some(id), expected)?);
        return Ok(());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io)?;
    let id = session_id(&file).map_err(io)?;
    *recorded = Some(id);
    created()?;
    let metadata = file.metadata().map_err(io)?;
    if !regular(&metadata)
        || metadata.len() != 0
        || !matches!(session_at(path, id), SessionAt::Ours)
    {
        return Err(refused());
    }
    file.write_all(expected).map_err(io)?;
    file.sync_all().map_err(io)?;
    require_file_bytes(&mut file, expected)?;
    if !matches!(session_at(path, id), SessionAt::Ours) {
        return Err(refused());
    }
    Ok(())
}

/// Streaming verification avoids another full WAL-sized allocation after a barrier.
/// Exact length plus an EOF probe also catches append/truncation during the comparison.
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
    let metadata = file.metadata().map_err(io)?;
    if !regular(&metadata) || metadata.len() != expected.len() as u64 {
        return Err(refused());
    }
    Ok(())
}

/// First-A closed names only. Future control roles need their own frozen namespace and
/// consumer; accepting a generic caller filename or guessed control prefix is forbidden.
pub(in crate::store::persistence::v2_text::transaction) fn artifact_child(
    base: &Path,
    role: TextArtifactRoleData,
    id: &[u8; 32],
) -> Result<PathBuf, AppError> {
    let prefix = match role {
        TextArtifactRoleData::ActionChunk
        | TextArtifactRoleData::CipherChunk
        | TextArtifactRoleData::Manifest
        | TextArtifactRoleData::TokenIndex => "p-",
        TextArtifactRoleData::Archive => "h-",
        TextArtifactRoleData::Retain => "a-",
        _ => return Err(refused()),
    };
    let capacity = base
        .as_os_str()
        .len()
        .checked_add(71)
        .filter(|n| *n <= super::super::MAX_PATH)
        .ok_or_else(refused)?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(70);
    name.push_str(prefix);
    for byte in id {
        name.push(HEX[(byte >> 4) as usize] as char);
        name.push(HEX[(byte & 15) as usize] as char);
    }
    name.push_str(".bin");
    let mut result = PathBuf::with_capacity(capacity);
    result.push(base);
    result.push(name);
    if result.capacity() > super::super::MAX_PATH {
        return Err(refused());
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
