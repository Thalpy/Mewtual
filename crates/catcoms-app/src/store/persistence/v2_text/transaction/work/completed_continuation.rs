//! SAME original completed-source A continuation only. Full original WC MAIN
//! and real nextA Stage grant precede suffix work. Native ID is a refusal filter;
//! only a present regular bounded whole matching prefix receives original bytes.
//! No missing adoption, truncation, new nonce, sign, quota or SDK reconstruction.
use super::io as native;
use super::*;
use catcoms_storage::kept::{session_at, session_id, SessionAt};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

// Actual retained-A complete WAL ceiling; the expected bytes are already paid
// in the same16MiB work class. Only one8KiB comparison buffer is used here.
const MAX_COMPLETED_A_FRAME: usize = WAL_PLAIN_ALLOCATION_CAP + 40;

/// Finite SAME original NEXT-A worker only, AFTER full exact prior WC proof.
/// Missing/replaced/nonregular/excess/nonprefix output is preserved/refused.
pub(super) fn write_original(
    path: &Path,
    expected: &[u8],
    recorded: &mut Option<SessionId>,
    created: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), AppError> {
    if expected.is_empty() || expected.len() > MAX_COMPLETED_A_FRAME {
        return Err(refused());
    }
    let Some(id) = *recorded else {
        return native::write_complete_only(path, expected, recorded, created);
    };
    // This path NEVER creates or replaces. A saved closed-ID witness cannot make an
    // absent object ours. Nonmatching bytes/prefix are preserved, not parsed as a
    // replacement plan. This is no arbitrary authenticated-format classifier; the
    // independently authorized intended ciphertext, not a parsed file tag, fixes work.
    let named = fs::symlink_metadata(path).map_err(failed)?;
    if !regular(&named)
        || named.len() > expected.len() as u64
        || !matches!(session_at(path, id), SessionAt::Ours)
    {
        return Err(refused());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(failed)?;
    let metadata = file.metadata().map_err(failed)?;
    if !regular(&metadata)
        || metadata.len() != named.len()
        || session_id(&file).map_err(failed)? != id
    {
        return Err(refused());
    }
    let prefix = usize::try_from(metadata.len()).map_err(|_| refused())?;
    compare(&mut file, &expected[..prefix])?;
    require_named_file(path, &file, id, prefix)?;
    if prefix < expected.len() {
        // Whole prefix was verified before the first byte write. Prefix zero is
        // permitted only by this actual same-Cell cached-created Stage continuation;
        // a generic existing empty file or an absent slot never enters this branch.
        file.seek(SeekFrom::Start(prefix as u64)).map_err(failed)?;
        file.write_all(&expected[prefix..]).map_err(failed)?;
    }
    file.sync_all().map_err(failed)?;
    compare(&mut file, expected)?;
    require_named_file(path, &file, id, expected.len())
}
fn require_named_file(path: &Path, file: &File, id: SessionId, len: usize) -> Result<(), AppError> {
    let held = file.metadata().map_err(failed)?;
    if !regular(&held)
        || held.len() != len as u64
        || session_id(file).map_err(failed)? != id
        || !matches!(session_at(path, id), SessionAt::Ours)
    {
        return Err(refused());
    }
    Ok(())
}
fn compare(file: &mut File, expected: &[u8]) -> Result<(), AppError> {
    file.seek(SeekFrom::Start(0)).map_err(failed)?;
    let mut at = 0;
    let mut buffer = [0; 8192];
    while at < expected.len() {
        let cap = (expected.len() - at).min(buffer.len());
        match file.read(&mut buffer[..cap]) {
            Ok(0) => return Err(refused()),
            Ok(n) if buffer[..n] == expected[at..at + n] => at += n,
            Ok(_) => return Err(refused()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(failed(e)),
        }
    }
    loop {
        match file.read(&mut buffer[..1]) {
            Ok(0) => return Ok(()),
            Ok(_) => return Err(refused()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(failed(e)),
        }
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
    {
        plain
    }
}
fn failed(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}
