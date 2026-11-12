//! Bounded readonly native captures for the actual accepted-history query.
//! Files and buffers cache before later fallible work; no mutable owner or raw File
//! escapes this private IO family. Its caller retains the SAME admitted lease/IO end.
use super::refused;
use crate::AppError;
use catcoms_replication::chat::cold_index::MAX_INDEX_FRAME_BYTES;
use catcoms_storage::kept::{session_at, session_id, SessionAt, SessionId};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use zeroize::Zeroizing;

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

/// Cached in the active Source before the first fallible metadata/name/content operation.
pub(super) struct HeldAcceptedFrame {
    file: Option<File>,
    id: Option<SessionId>,
    raw: Option<Zeroizing<Vec<u8>>>,
    used: usize,
    complete: bool,
}
impl HeldAcceptedFrame {
    pub(super) fn empty() -> Self {
        Self {
            file: None,
            id: None,
            raw: None,
            used: 0,
            complete: false,
        }
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
    pub(super) fn require_name(&self, path: &Path) -> Result<(), AppError> {
        let id = self.id.ok_or_else(refused)?;
        if session_id(self.file()?).map_err(io)? != id
            || !matches!(session_at(path, id), SessionAt::Ours)
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn capture(&mut self, path: &Path, maximum: usize) -> Result<(), AppError> {
        if maximum == 0 || maximum > MAX_INDEX_FRAME_BYTES {
            return Err(refused());
        }
        if self.complete {
            return self.require_unchanged(path);
        }
        if self.file.is_none() {
            let metadata = fs::symlink_metadata(path).map_err(io)?;
            if !regular(&metadata) || metadata.len() == 0 || metadata.len() > maximum as u64 {
                return Err(refused());
            }
            self.file = Some(File::open(path).map_err(io)?);
        }
        let metadata = self.file()?.metadata().map_err(io)?;
        if !regular(&metadata) || metadata.len() == 0 || metadata.len() > maximum as u64 {
            return Err(refused());
        }
        if self.id.is_none() {
            self.id = Some(session_id(self.file()?).map_err(io)?);
        }
        self.require_name(path)?;
        if self.raw.is_none() {
            self.raw = Some(Zeroizing::new(vec![0; maximum + 1]));
        }
        let file = self.file.as_mut().ok_or_else(refused)?;
        file.seek(SeekFrom::Start(0)).map_err(io)?;
        let raw = self.raw.as_mut().ok_or_else(refused)?;
        let mut used = 0;
        while used < raw.len() {
            match file.read(&mut raw[used..]) {
                Ok(0) => break,
                Ok(n) => used += n,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        if used != metadata.len() as usize {
            return Err(refused());
        }
        self.used = used;
        self.complete = true;
        self.require_name(path)
    }
    pub(super) fn require_unchanged(&self, path: &Path) -> Result<(), AppError> {
        self.require_name(path)?;
        let expected = self.raw()?;
        let file = self.file()?;
        let metadata = file.metadata().map_err(io)?;
        if !regular(&metadata) || metadata.len() != expected.len() as u64 {
            return Err(refused());
        }
        let mut offset = 0;
        let mut buffer = [0u8; 4096];
        while offset < expected.len() {
            let count = buffer.len().min(expected.len() - offset);
            match positioned_read(file, &mut buffer[..count], offset as u64) {
                Ok(0) => return Err(refused()),
                Ok(n) => {
                    if buffer[..n] != expected[offset..offset + n] {
                        return Err(refused());
                    }
                    offset += n;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        loop {
            match positioned_read(file, &mut buffer[..1], offset as u64) {
                Ok(0) => break,
                Ok(_) => return Err(refused()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        self.require_name(path)
    }
}

pub(super) struct HeldAcceptedDirectory {
    file: Option<File>,
    id: Option<SessionId>,
}
impl HeldAcceptedDirectory {
    pub(super) fn empty() -> Self {
        Self {
            file: None,
            id: None,
        }
    }
    pub(super) fn capture(&mut self, path: &Path) -> Result<(), AppError> {
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
        let file = self.file.as_ref().ok_or_else(refused)?;
        let metadata = file.metadata().map_err(io)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(refused());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(refused());
            }
        }
        if self.id.is_none() {
            self.id = Some(session_id(file).map_err(io)?);
        }
        self.require_name(path, self.id.ok_or_else(refused)?)
    }
    pub(super) fn require_name(&self, path: &Path, expected: SessionId) -> Result<(), AppError> {
        let file = self.file.as_ref().ok_or_else(refused)?;
        if self.id != Some(expected) {
            return Err(refused());
        }
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            path, file, expected,
        )
    }
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
