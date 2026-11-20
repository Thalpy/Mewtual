//! Held, bounded warm-recovery reads. No creation, sync, rename, unlink or scratch permission.
//!
//! The actual Source is cached in its same-vault lease bundle before calling these methods.
//! Metadata captures run under Current; potentially large WAL reads run only in the finite
//! outside-writer worker. Every error preserves Present/Absent rather than guessing absence.
use super::*;
use catcoms_storage::kept::{session_at, session_id, SessionAt, SessionId};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Presence {
    Unseen,
    Absent,
    Present,
}

/// Installed empty in the active Source BEFORE any File/buffer can survive an error.
pub(super) struct HeldRetainRecoveryControl {
    file: Option<File>,
    id: Option<SessionId>,
    length: Option<usize>,
    raw: Option<Zeroizing<Vec<u8>>>,
    used: usize,
    complete: bool,
    presence: Presence,
}
impl HeldRetainRecoveryControl {
    pub(super) fn empty() -> Self {
        Self {
            file: None,
            id: None,
            length: None,
            raw: None,
            used: 0,
            complete: false,
            presence: Presence::Unseen,
        }
    }
    pub(super) fn native_data(&self) -> Result<SessionId, AppError> {
        self.id.ok_or_else(refused)
    }
    pub(super) fn length_data(&self) -> Result<usize, AppError> {
        self.length.ok_or_else(refused)
    }
    pub(super) fn is_absent_data(&self) -> bool {
        self.presence == Presence::Absent
    }
    pub(super) fn is_present_data(&self) -> bool {
        self.presence == Presence::Present
    }
    pub(super) fn require_absent(&self, path: &Path) -> Result<(), AppError> {
        if !self.is_absent_data() {
            return Err(refused());
        }
        require_absent(path)
    }
    /// Genuine NotFound is a typed observation, not a catch-all error conversion. A
    /// Present object remains Present even after a later open/read/name failure.
    pub(super) fn capture_metadata(&mut self, path: &Path, maximum: usize) -> Result<(), AppError> {
        if maximum == 0 || maximum > MAX_RECOVERY_WAL_FRAME {
            return Err(refused());
        }
        if self.presence == Presence::Absent {
            return self.require_absent(path);
        }
        if self.file.is_none() {
            let metadata = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && self.presence == Presence::Unseen =>
                {
                    self.presence = Presence::Absent;
                    return require_absent(path);
                }
                Err(error) => return Err(io(error)),
            };
            self.presence = Presence::Present;
            if !regular(&metadata) || metadata.len() > maximum as u64 {
                return Err(refused());
            }
            // File is in the outside Source slot BEFORE held metadata/native ID/read.
            self.file = Some(File::open(path).map_err(io)?);
        }
        let metadata = self.file()?.metadata().map_err(io)?;
        if !regular(&metadata) || metadata.len() > maximum as u64 {
            return Err(refused());
        }
        let length = usize::try_from(metadata.len()).map_err(|_| refused())?;
        if let Some(original) = self.length {
            if original != length {
                return Err(refused());
            }
        } else {
            self.length = Some(length);
        }
        if self.id.is_none() {
            self.id = Some(session_id(self.file()?).map_err(io)?);
        }
        self.require_name(path)
    }
    fn file(&self) -> Result<&File, AppError> {
        self.file.as_ref().ok_or_else(refused)
    }
    pub(super) fn raw(&self) -> Result<&[u8], AppError> {
        if !self.complete {
            return Err(refused());
        }
        Ok(&self.raw.as_ref().ok_or_else(refused)?[..self.used])
    }
    /// Actual large read, called only by the admitted finite worker OUTSIDE writers. The
    /// original raw allocation is cached before seek/read; retries cannot replace the FD.
    pub(super) fn read_complete(&mut self, path: &Path) -> Result<(), AppError> {
        if self.complete {
            return self.require_unchanged(path);
        }
        if !self.is_present_data() {
            return Err(refused());
        }
        self.require_metadata(path)?;
        let length = self.length.ok_or_else(refused)?;
        let capacity = length.checked_add(1).ok_or_else(refused)?;
        if self.raw.is_none() {
            self.raw = Some(Zeroizing::new(vec![0; capacity]));
        }
        let file = self.file.as_mut().ok_or_else(refused)?;
        file.seek(SeekFrom::Start(0)).map_err(io)?;
        let raw = self.raw.as_mut().ok_or_else(refused)?;
        let mut used = 0;
        while used < raw.len() {
            match file.read(&mut raw[used..]) {
                Ok(0) => break,
                Ok(count) => used += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        if used != length {
            return Err(refused());
        }
        self.used = used;
        self.complete = true;
        self.require_unchanged(path)
    }
    pub(super) fn require_name(&self, path: &Path) -> Result<(), AppError> {
        if !self.is_present_data() {
            return Err(refused());
        }
        let id = self.id.ok_or_else(refused)?;
        if session_id(self.file()?).map_err(io)? != id
            || !matches!(session_at(path, id), SessionAt::Ours)
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn require_metadata(&self, path: &Path) -> Result<(), AppError> {
        match self.presence {
            Presence::Absent => self.require_absent(path),
            Presence::Unseen => Err(refused()),
            Presence::Present => {
                let metadata = self.file()?.metadata().map_err(io)?;
                if !regular(&metadata) || metadata.len() != self.length.ok_or_else(refused)? as u64
                {
                    return Err(refused());
                }
                self.require_name(path)
            }
        }
    }
    pub(super) fn require_unchanged(&self, path: &Path) -> Result<(), AppError> {
        self.require_metadata(path)?;
        let expected = self.raw()?;
        let mut at = 0;
        let mut bytes = [0u8; 4096];
        while at < expected.len() {
            let limit = bytes.len().min(expected.len() - at);
            match positioned_read(self.file()?, &mut bytes[..limit], at as u64) {
                Ok(0) => return Err(refused()),
                Ok(count) => {
                    if bytes[..count] != expected[at..at + count] {
                        return Err(refused());
                    }
                    at += count;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        loop {
            match positioned_read(self.file()?, &mut bytes[..1], at as u64) {
                Ok(0) => break,
                Ok(_) => return Err(refused()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        self.require_name(path)
    }
}

pub(super) struct HeldRetainRecoveryDirectory {
    file: Option<File>,
    id: Option<SessionId>,
}
impl HeldRetainRecoveryDirectory {
    pub(super) fn empty() -> Self {
        Self {
            file: None,
            id: None,
        }
    }
    pub(super) fn capture(&mut self, path: &Path, expected: SessionId) -> Result<(), AppError> {
        if self.file.is_none() {
            #[cfg(windows)]
            let file = {
                use std::os::windows::fs::OpenOptionsExt;
                use windows_sys::Win32::Storage::FileSystem::{
                    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
                    FILE_SHARE_READ, FILE_SHARE_WRITE,
                };
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(path)
                    .map_err(io)?
            };
            #[cfg(not(windows))]
            let file = File::open(path).map_err(io)?;
            self.file = Some(file);
        }
        if self.id.is_none() {
            self.id = Some(session_id(self.file.as_ref().ok_or_else(refused)?).map_err(io)?);
        }
        self.require_name(path, expected)
    }
    pub(super) fn require_name(&self, path: &Path, expected: SessionId) -> Result<(), AppError> {
        if self.id != Some(expected) {
            return Err(refused());
        }
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            path,
            self.file.as_ref().ok_or_else(refused)?,
            expected,
        )
    }
}

pub(super) fn require_absent(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(refused()),
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
pub(super) fn io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}
fn positioned_read(file: &File, bytes: &mut [u8], offset: u64) -> std::io::Result<usize> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        file.seek_read(bytes, offset)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_at(bytes, offset)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (file, bytes, offset);
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}
