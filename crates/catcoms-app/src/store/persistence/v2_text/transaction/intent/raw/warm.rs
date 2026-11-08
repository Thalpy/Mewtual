//! Narrow SAME-original first-S Stage continuation, not a generic repair grant.
//!
//! The caller must hold the genuine S Stage marker and exact original SDK/A custody,
//! and prove full original W_I before a pre-MAIN WS stage, or full own W_S before S/
//! head outputs. This helper alone grants no admission, WAL adoption or selector CAS.
//! A recorded native label only rejects substitution; it is NOT incarnation proof.
//! Fresh authority comes from that original current Stage and its frozen intended
//! bytes. Only that previously cached-created slot may reconstruct a proper prefix.
//! Cold/missing/unknown objects remain separate; the general A helper stays complete-only.
use super::super::super::work::io as native;
use super::*;
use catcoms_storage::kept::{session_at, session_id, SessionAt};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

// This is the actual bounded first-S WAL bound (two Heads + one signature + framing),
// not a permission to read the general 2MiB WORK cap while holding Current.
// Frozen WORK fixed fields3686 (two max1725 Heads) +bytes(carry of3 roots,
// zero entries) +one42-byte artifact wrapper +405-byte S +40-byte WAL AEAD frame.
// FORMAT2130 is S artifact+selector, NOT a WAL bound.
const MAX_FIRST_S_FRAME: usize = 3686 + 4 + (1 + 3 * 36 + 2 + 4) + 42 + 405 + 40;
const _: () = assert!(MAX_FIRST_S_FRAME == 4292);

/// Finite effect ONLY inside the genuine SAME-S Stage worker after its full WAL proof.
/// Missing recorded output, wrong native/type, excess length or differing prefix is
/// preserved/refused. No truncation, unlink, new nonce, replacement SDK plan or signing.
/// Files/bytes/sync/name checks finish before the caller releases its actual lease/End.
pub(super) fn write_original(
    path: &Path,
    expected: &[u8],
    recorded: &mut Option<SessionId>,
    created: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), AppError> {
    if expected.is_empty() || expected.len() > MAX_FIRST_S_FRAME {
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
