//! Fixed-folder NEGATIVE inventory, never source, cleanup or write authority.
//!
//! Linux std::read_dir hides libc's buffer. RawDir instead borrows this fixed
//! paid buffer and returns names borrowed from it. No growing buffer, path
//! construction, Unicode conversion, recursion or fallback scan is permitted.
use crate::store::critical_namespaces::refused;
use crate::AppError;
use catcoms_storage::kept::SessionId;
use rustix::fd::OwnedFd;
use rustix::fs::{fstat, openat, statat, AtFlags, FileType, Mode, OFlags, RawDir, Stat};
use std::fs::File;
use std::mem::{size_of, MaybeUninit};
use std::path::Path;

const BUFFER_BYTES: usize = 8192;
const MAX_NAMES: usize = 10;

/// Application overlap, not kernel memory/RSS: fixed getdents scratch, lending
/// iterator and entry, independent FD, three stat records, masks, borrowed names,
/// and conservative syscall/error wrappers. Callers retain their OWN paid fee.
pub(super) const FIXED_INVENTORY_BYTES: usize = BUFFER_BYTES
    + size_of::<RawDir<'static, OwnedFd>>()
    + size_of::<rustix::fs::RawDirEntry<'static>>()
    + size_of::<OwnedFd>()
    + 3 * size_of::<Stat>()
    + MAX_NAMES * size_of::<&str>()
    + 1024;
const _: () = assert!(FIXED_INVENTORY_BYTES <= 16 * 1024);

pub(super) fn require_exact_names(
    directory: (&Path, &File, SessionId),
    expected: &[&str],
    permits_pages_directory: bool,
) -> Result<(), AppError> {
    // Validate the complete fixed specification before opening anything. A
    // duplicate spec must never make a missing slot appear complete.
    if expected.len() > MAX_NAMES
        || expected.iter().enumerate().any(|(index, name)| {
            name.is_empty()
                || name.len() > 255
                || matches!(*name, "." | "..")
                || name.as_bytes().iter().any(|b| matches!(b, 0 | b'/'))
                || expected[..index].contains(name)
        })
    {
        return Err(refused());
    }
    let require_name = || {
        super::v2_genesis_activation::require_directory_name(directory.0, directory.1, directory.2)
    };
    require_name()?;

    // dup/try_clone share a directory offset and would let a prior scan change
    // the next scan's result. openat('.') creates an independent description of
    // this SAME held directory, not a candidate selected by its mutable path.
    let fd = openat(
        directory.1,
        c".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io)?;
    let held = fstat(directory.1).map_err(io)?;
    let opened = fstat(&fd).map_err(io)?;
    if held.st_dev != opened.st_dev || held.st_ino != opened.st_ino {
        return Err(refused());
    }
    let mut buffer = [MaybeUninit::uninit(); BUFFER_BYTES];
    let mut entries = RawDir::new(&fd, &mut buffer);
    let mut seen = 0u16;
    let mut dots = 0u8;
    // Include at most two exact dot entries AND one extra-entry/EOF observation.
    // An oversized entry/INVAL is a refusal, never permission to resize/restart.
    for _ in 0..expected.len() + 3 {
        let Some(entry) = entries.next() else {
            if seen != (1u16 << expected.len()) - 1 {
                return Err(refused());
            }
            #[cfg(test)]
            run_after_enumeration();
            return require_name();
        };
        let entry = entry.map_err(io)?;
        let name = entry.file_name();
        let bytes = name.to_bytes();
        let dot = match bytes {
            b"." => 1,
            b".." => 2,
            _ => 0,
        };
        if dot != 0 {
            if dots & dot != 0 {
                return Err(refused());
            }
            dots |= dot;
            continue;
        }
        let index = expected
            .iter()
            .position(|expected| expected.as_bytes() == bytes)
            .ok_or_else(refused)?;
        let bit = 1u16 << index;
        if seen & bit != 0 {
            return Err(refused());
        }
        // Always stat without following, even when getdents supplies DT_UNKNOWN.
        // Entry names stay borrowed CStr bytes; no path or name allocation occurs.
        let metadata = statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io)?;
        let kind = FileType::from_raw_mode(metadata.st_mode);
        let expected_kind = if permits_pages_directory && expected[index] == "text.pages" {
            FileType::Directory
        } else {
            FileType::RegularFile
        };
        if kind != expected_kind {
            return Err(refused());
        }
        seen |= bit;
    }
    Err(refused())
}

fn io(error: rustix::io::Errno) -> AppError {
    AppError::Io(error.to_string())
}

#[cfg(test)]
thread_local! {
    static AFTER_ENUMERATION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn run_after_enumeration() {
    if let Some(hook) = AFTER_ENUMERATION.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(test)]
mod tests;
