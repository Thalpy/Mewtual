//! Bounded ephemeral Current read proof. Capture paths/Files BEFORE the final original
//! source/ID3 proof; afterwards recheck only these held/named objects and named absences.
//! This private utility grants no Current/work/signing permission and cannot outlive the
//! actual Current guard. It never reads a large WAL/body or retains an obsolete FD.
use super::*;
use catcoms_storage::kept::{session_at, session_id, SessionAt};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::marker::PhantomData;

const MAX_FILES: usize = 64;
const MAX_ABSENCES: usize = 4;
/// Vector backing/held-File layouts only. Callers additionally invoice their actual
/// bounded number of owned path buffers; reserving unused slots creates no path data.
pub(super) const READY_NAMED_FILES_LAYOUT_BYTES: usize =
    2 * std::mem::size_of::<ReadyNamedFiles<'static>>()
        + MAX_FILES * std::mem::size_of::<HeldNamedFile<'static>>()
        + MAX_ABSENCES * std::mem::size_of::<PathBuf>()
        + 8192; // ONE sequential comparison buffer, never a fresh full-body copy.
struct HeldNamedFile<'expected> {
    path: PathBuf,
    file: File,
    id: SessionId,
    len: usize,
    // Local borrowed original only: never copied into Saved or returned permission.
    exact: Option<&'expected [u8]>,
}
struct HeldNamedDirectory {
    path: PathBuf,
    file: File,
    id: SessionId,
}
pub(super) struct ReadyNamedFiles<'guard> {
    target: HeldNamedDirectory,
    pages: HeldNamedDirectory,
    files: Vec<HeldNamedFile<'guard>>,
    absences: Vec<PathBuf>,
    // No owning/escaping getter exists. All callers keep this local to validation.
    _current: PhantomData<&'guard ()>,
}
impl<'guard> ReadyNamedFiles<'guard> {
    pub(super) fn capture(
        _guard: &'guard V2TextCurrentGroupGuard<'_, '_>,
        target: &Path,
        target_id: SessionId,
        pages: &Path,
        pages_id: SessionId,
    ) -> Result<Self, AppError> {
        Ok(Self {
            target: directory(target, target_id)?,
            pages: directory(pages, pages_id)?,
            files: Vec::with_capacity(MAX_FILES),
            absences: Vec::with_capacity(MAX_ABSENCES),
            _current: PhantomData,
        })
    }
    pub(super) fn add_file(
        &mut self,
        path: PathBuf,
        id: SessionId,
        len: usize,
    ) -> Result<(), AppError> {
        if self.files.len() >= MAX_FILES || path.as_os_str().len() > MAX_PATH {
            return Err(refused());
        }
        let metadata = fs::symlink_metadata(&path).map_err(failed)?;
        if !regular(&metadata) || metadata.len() != len as u64 {
            return Err(refused());
        }
        let file = File::open(&path).map_err(failed)?;
        let held = HeldNamedFile {
            path,
            file,
            id,
            len,
            exact: None,
        };
        held.require_unchanged()?;
        self.files.push(held);
        Ok(())
    }
    pub(super) fn add_absence(&mut self, path: PathBuf) -> Result<(), AppError> {
        if self.absences.len() >= MAX_ABSENCES || path.as_os_str().len() > MAX_PATH {
            return Err(refused());
        }
        absent(&path)?;
        self.absences.push(path);
        Ok(())
    }
    /// Bounded exact body comparison belongs BEFORE the final source/ID3 proof. Keep
    /// this same File for the final named tail; do not retain a new full WAL buffer.
    pub(super) fn add_exact_file(
        &mut self,
        path: PathBuf,
        id: SessionId,
        expected: &'guard [u8],
    ) -> Result<(), AppError> {
        self.add_file(path, id, expected.len())?;
        let held = self.files.last_mut().ok_or_else(refused)?;
        held.exact = Some(expected);
        held.require_unchanged()
    }
    /// Both allowed head names are precomputed/captured before source/ID3. Only a real
    /// NotFound chooses the main-name lost-rename continuation; errors never mean absent.
    pub(super) fn add_staged_head(
        &mut self,
        target: &Path,
        main: &Path,
        id: SessionId,
        len: usize,
    ) -> Result<(), AppError> {
        let stage = fixed_child(target, "head.stage")?;
        match fs::symlink_metadata(&stage) {
            Ok(_) => self.add_file(stage, id, len),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.add_absence(stage)?;
                self.add_file(owned_path(main)?, id, len)
            }
            Err(error) => Err(failed(error)),
        }
    }
    /// FINAL named/held tail only (including any bounded SAME-File exact comparison):
    /// no path construction, file open, account, codec,
    /// source adoption, RNG or SDK operation is introduced after the caller's ID3 proof.
    pub(super) fn require_unchanged(&self) -> Result<(), AppError> {
        self.target.require_name()?;
        self.pages.require_name()?;
        for file in &self.files {
            file.require_unchanged()?;
        }
        for path in &self.absences {
            absent(path)?;
        }
        Ok(())
    }
}
impl HeldNamedFile<'_> {
    fn require_unchanged(&self) -> Result<(), AppError> {
        // Signing's SAME small WI must still match after the final source seam;
        // session/length alone would accept in-place equal-length corruption. Read
        // through this SAME held File using only the original borrowed bytes.
        if let Some(expected) = self.exact {
            require_exact_held(&self.file, expected)?;
        }
        let metadata = self.file.metadata().map_err(failed)?;
        if !regular(&metadata)
            || metadata.len() != self.len as u64
            || session_id(&self.file).map_err(failed)? != self.id
            || !matches!(session_at(&self.path, self.id), SessionAt::Ours)
        {
            return Err(refused());
        }
        Ok(())
    }
}
impl HeldNamedDirectory {
    fn require_name(&self) -> Result<(), AppError> {
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &self.path, &self.file, self.id,
        )
    }
}
fn directory(path: &Path, id: SessionId) -> Result<HeldNamedDirectory, AppError> {
    let owned = owned_path(path)?;
    let file = crate::store::persistence::open_archive_directory(path)?;
    let held = HeldNamedDirectory {
        path: owned,
        file,
        id,
    };
    held.require_name()?;
    Ok(held)
}
fn absent(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
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
    {
        plain
    }
}
fn failed(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

/// No new handle, path or output allocation. The bounded exact original is borrowed
/// only by this local proof. All actual external/cooperative write exclusion remains
/// the caller's held Current/canonical responsibility, not a claim of atomic reads.
fn require_exact_held(file: &File, expected: &[u8]) -> Result<(), AppError> {
    let mut stream = file;
    stream.seek(SeekFrom::Start(0)).map_err(failed)?;
    let mut at = 0;
    let mut buffer = [0; 8192];
    while at < expected.len() {
        let cap = (expected.len() - at).min(buffer.len());
        match stream.read(&mut buffer[..cap]) {
            Ok(0) => return Err(refused()),
            Ok(n) if buffer[..n] == expected[at..at + n] => at += n,
            Ok(_) => return Err(refused()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(failed(error)),
        }
    }
    loop {
        match stream.read(&mut buffer[..1]) {
            Ok(0) => return Ok(()),
            Ok(_) => return Err(refused()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(failed(error)),
        }
    }
}
