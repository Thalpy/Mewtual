//! External destination primitives for a journaled private-download export.
//!
//! This module is the only code in the retained engine that touches a path outside the vault.
//! Everything else in `kept` reads and writes the private, sealed transfer directories. The
//! export journal (see `export.rs`) uses these primitives to prove that a partly written file in
//! the user's Downloads folder is still the exact object Mewtual created before it truncates,
//! appends to, publishes or deletes it. A saved path alone is never that proof.
//!
//! Proof of ownership is the identity of an *open handle*, never a path comparison:
//!
//! - Windows: `GetFileInformationByHandle` through `winapi-util`'s safe wrapper (the workspace
//!   denies `unsafe_code`), giving the volume serial number, the 64-bit file index (which carries
//!   an NTFS reuse sequence number), the link count and the attributes of the object the handle
//!   refers to. A leaf is opened with `FILE_FLAG_OPEN_REPARSE_POINT`, so a symbolic link or
//!   junction placed at our name is opened as itself (with its own identity) instead of being
//!   followed to some other file.
//! - Unix: `dev`, `ino` and `nlink` from `fstat` on the opened handle, cross-checked against
//!   `lstat` of the path so that a symbolic link at our name is refused rather than followed.
//!   **This branch is compile-unverified on the Windows development host**; Linux CI is its first
//!   compiler. It uses only stable `std`.
//! - Both: the object's **birth time** (`Metadata::created`, `statx` on Linux) is part of the
//!   identity. ext4 and xfs reuse a deleted file's inode number quickly; without birth time a
//!   later file from another program could inherit our partial's `(dev, ino)` and be truncated
//!   or deleted as ours. A filesystem that reports no birth time, or a zero volume or object id
//!   (some SMB and FUSE mounts), has no usable identity: the caller then keeps today's
//!   non-resumable export instead of journaling. NTFS creation-time tunneling can give a new file
//!   the old creation time, but not the old file index, so the combined identity still differs.
//!
//! The destination *folder* is deliberately opened with link following: a Downloads folder that is
//! itself a junction or symlink is the user's configured location, and its target's identity is
//! what the journal records and later compares.
//!
//! Residual limit (documented in THREAT-MODEL): identity checks and the path-based `hard_link` /
//! `remove_file` that follow them are separate system calls. A same-user process that swaps files
//! at exactly that moment can win the race. Such a process already has equivalent authority over
//! the user's Downloads folder.
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Component, Path};

/// Platform tags stored in every identity. The tag also selects how the saved folder path is
/// decoded, so a vault copied to a different operating system can never reinterpret a Windows
/// UTF-16 path as Unix bytes (or the reverse). A mismatch is reported as an unreachable
/// destination by the caller, never as a match.
pub(super) const PLATFORM_WINDOWS: u8 = 1;
pub(super) const PLATFORM_UNIX: u8 = 2;
#[cfg(windows)]
pub(super) const CURRENT_PLATFORM: u8 = PLATFORM_WINDOWS;
#[cfg(unix)]
pub(super) const CURRENT_PLATFORM: u8 = PLATFORM_UNIX;
/// No identity source: a fresh export reports Unsupported and native keeps today's
/// non-resumable writer. Existing journals cannot be resumed on such a platform.
#[cfg(not(any(windows, unix)))]
pub(super) const CURRENT_PLATFORM: u8 = 0;

/// Fixed encoded identity: platform tag, 64-bit volume, 128-bit object identifier. The identifier
/// is the 64-bit file index or inode followed by the 64-bit birth time in nanoseconds since the
/// Unix epoch.
pub(super) const IDENTITY_BYTES: usize = 25;
/// Encoded folder path bound (8,192 UTF-16 units on Windows). Together with the two leaves this
/// keeps the worst-case journal near 18 KiB, inside the existing 400 KiB Download record slot.
pub(super) const MAX_FOLDER_BYTES: usize = 16 * 1024;
/// Encoded leaf-name bound. Native names are at most 120 scalars plus a collision suffix.
pub(super) const MAX_LEAF_BYTES: usize = 1024;
/// Same collision-candidate ceiling as the native non-resumable Downloads writer.
pub(super) const MAX_CANDIDATES: usize = 1000;
/// Staging suffix for a journaled partial. Deliberately NOT the browsers' `.part` (Firefox) or the
/// native fallback writer's `.part`: another program's in-progress download should never sit at
/// our name merely by convention. Identity, not the suffix, remains the ownership proof.
pub(super) const STAGING_SUFFIX: &str = ".mewtual-part";

#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_DIRECTORY: u64 = 0x10;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u64 = 0x400;

/// Identity of one filesystem object as reported by an open handle. It is a local, stable
/// tracking identifier, so its Debug output is redacted like the paths it accompanies.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct FileIdentity {
    platform: u8,
    volume: u64,
    id: [u8; 16],
}
impl std::fmt::Debug for FileIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileIdentity { .. }")
    }
}
impl FileIdentity {
    pub(super) fn platform(&self) -> u8 {
        self.platform
    }
    pub(super) fn encode(&self) -> [u8; IDENTITY_BYTES] {
        let mut out = [0; IDENTITY_BYTES];
        out[0] = self.platform;
        out[1..9].copy_from_slice(&self.volume.to_le_bytes());
        out[9..].copy_from_slice(&self.id);
        out
    }
    /// Structural decode only. Whether the platform matches the running one is a separate,
    /// non-destructive question answered where the journal is used.
    pub(super) fn decode(bytes: &[u8]) -> Option<Self> {
        let bytes: &[u8; IDENTITY_BYTES] = bytes.try_into().ok()?;
        if !matches!(bytes[0], PLATFORM_WINDOWS | PLATFORM_UNIX) {
            return None;
        }
        Some(Self {
            platform: bytes[0],
            volume: u64::from_le_bytes(bytes[1..9].try_into().ok()?),
            id: bytes[9..].try_into().ok()?,
        })
    }
    #[cfg(test)]
    pub(in crate::kept) fn for_test(platform: u8, volume: u64, id: u8) -> Self {
        Self {
            platform,
            volume,
            id: [id; 16],
        }
    }
}

/// What an open handle reports about the object it actually refers to.
#[derive(Debug, Clone, Copy)]
pub(super) struct Observed {
    pub identity: FileIdentity,
    pub links: u64,
    /// A plain file: not a directory and (on Windows) not a name-surrogate reparse point (a
    /// symbolic link, junction or mount point that would redirect to another object).
    pub regular: bool,
    pub directory: bool,
    /// Windows only: the object carries a reparse point that is NOT a name surrogate (a cloud
    /// files placeholder, WOF compression, deduplication). It is still the same plain file, but
    /// its data must be read through a normal handle so the filter can present it.
    pub reparse: bool,
}

/// Windows classification of an opened object. Only name surrogates make it foreign: OneDrive
/// Files On-Demand, `compact /exe` and data deduplication tag ordinary files with reparse points,
/// and treating those as foreign would report our own partial as "changed" and let Remove leave
/// its plaintext behind.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn regular_from(attributes: u64, name_surrogate: bool) -> bool {
    const DIRECTORY: u64 = 0x10;
    const REPARSE_POINT: u64 = 0x400;
    attributes & DIRECTORY == 0 && !(attributes & REPARSE_POINT != 0 && name_surrogate)
}

/// The usable identity of an object, or `Unsupported` when the filesystem cannot provide one that
/// survives restart without collisions (no birth time, birth time before the epoch, or a zero
/// volume/object number). Callers treat `Unsupported` as "do not journal", never as a match.
fn identity_of(
    platform: u8,
    volume: u64,
    index: u64,
    born: io::Result<std::time::SystemTime>,
) -> io::Result<FileIdentity> {
    let unsupported = || {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "this filesystem gives no stable file identity",
        )
    };
    let born = born
        .map_err(|_| unsupported())?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| unsupported())?;
    let born = u64::try_from(born.as_nanos()).map_err(|_| unsupported())?;
    if volume == 0 || index == 0 || born == 0 {
        return Err(unsupported());
    }
    let mut id = [0; 16];
    id[..8].copy_from_slice(&index.to_le_bytes());
    id[8..].copy_from_slice(&born.to_le_bytes());
    Ok(FileIdentity {
        platform,
        volume,
        id,
    })
}

/// Identity of the object behind `file`, from the handle itself.
pub(super) fn observe(file: &File) -> io::Result<Observed> {
    inject(Effect::Identity)?;
    #[cfg(windows)]
    {
        let info = winapi_util::file::information(file)?;
        let attributes = info.file_attributes();
        // std reads the creation time and, for a reparse point, its tag through this same handle
        // (GetFileInformationByHandle, then FileAttributeTagInfo); `is_symlink` is std's
        // name-surrogate test. Metadata that cannot be read counts as a surrogate (foreign).
        let metadata = file.metadata();
        let surrogate = metadata
            .as_ref()
            .map_or(true, |metadata| metadata.file_type().is_symlink());
        let born = metadata.and_then(|metadata| metadata.created());
        let reparse = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        Ok(Observed {
            identity: identity_of(
                PLATFORM_WINDOWS,
                info.volume_serial_number(),
                info.file_index(),
                born,
            )?,
            links: info.number_of_links(),
            regular: regular_from(attributes, surrogate),
            directory: attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
            reparse: reparse && !surrogate,
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok(Observed {
            identity: identity_of(
                PLATFORM_UNIX,
                metadata.dev(),
                metadata.ino(),
                metadata.created(),
            )?,
            links: metadata.nlink(),
            regular: metadata.file_type().is_file(),
            directory: metadata.file_type().is_dir(),
            reparse: false,
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = file;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "file identity is unavailable on this platform",
        ))
    }
}

/// Identity of the destination folder, following a folder-level junction or symlink (the
/// user's configured Downloads location). Refuses anything that is not a directory.
pub(super) fn folder_identity(folder: &Path) -> io::Result<FileIdentity> {
    #[cfg(windows)]
    let handle = {
        use std::os::windows::fs::OpenOptionsExt;
        // A directory handle needs backup semantics; read access suffices for the query and the
        // default share mode keeps this short probe from excluding the user's own file actions.
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(folder)?
    };
    #[cfg(not(windows))]
    let handle = File::open(folder)?;
    let observed = observe(&handle)?;
    if !observed.directory {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "the destination is not a folder",
        ));
    }
    Ok(observed.identity)
}

/// Result of opening a leaf inside the destination folder without following a link.
pub(super) enum Leaf {
    Opened(File, Observed),
    /// Nothing exists at this name.
    Missing,
    /// Something exists, but it is a link, a directory or another non-regular object. It can
    /// never be ours, so it is never written, truncated or deleted.
    Foreign,
}
// `File`'s own Debug output includes its path; name only the variant.
impl std::fmt::Debug for Leaf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Opened(..) => "Leaf::Opened(..)",
            Self::Missing => "Leaf::Missing",
            Self::Foreign => "Leaf::Foreign",
        })
    }
}

/// Open `path` as a regular file without following a link at the final component. `write`
/// requests write access; cleanup probes use read access so a read-only attribute set by the
/// user cannot make a proven partial look foreign.
pub(super) fn open_leaf(path: &Path, write: bool) -> io::Result<Leaf> {
    let linked = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Leaf::Missing),
        Err(error) => return Err(error),
    };
    if linked.file_type().is_symlink() || !linked.is_file() {
        return Ok(Leaf::Foreign);
    }
    let mut options = OpenOptions::new();
    options.read(true).write(write);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open a reparse point as itself. If a link replaced our file after the metadata probe
        // above, the handle names the link object, whose identity cannot match the journal.
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Leaf::Missing),
        Err(error) => return Err(error),
    };
    let observed = observe(&file)?;
    if !observed.regular {
        return Ok(Leaf::Foreign);
    }
    #[cfg(windows)]
    if observed.reparse {
        // A plain file behind a non-surrogate reparse point (cloud placeholder, WOF, dedup). The
        // flagged handle bypasses the filter that presents its data, so do I/O through a normal
        // handle, and require that handle to name the same object.
        let plain = match OpenOptions::new().read(true).write(write).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Leaf::Missing),
            Err(error) => return Err(error),
        };
        let again = observe(&plain)?;
        if !again.regular || again.identity != observed.identity {
            return Ok(Leaf::Foreign);
        }
        return Ok(Leaf::Opened(plain, again));
    }
    #[cfg(unix)]
    {
        // Unix opens follow links. Require that the opened object is the same one `lstat`
        // described; a link swapped in between the two calls fails this comparison. The
        // regular-file check above already refused FIFOs and devices before any open.
        use std::os::unix::fs::MetadataExt;
        if observed.identity.volume != linked.dev()
            || observed.identity.id[..8] != linked.ino().to_le_bytes()
        {
            return Ok(Leaf::Foreign);
        }
    }
    Ok(Leaf::Opened(file, observed))
}

/// Exclusively create a new leaf. `create_new` refuses any existing entry at the name, including
/// a dangling link, so an existing user file can never be adopted or truncated here.
pub(super) fn create_leaf(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
}

/// Whether any entry (file, folder or link, dangling or not) occupies `path`.
pub(super) fn occupied(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Folder-level conditions that mean the destination cannot be reached right now (missing
/// folder or volume, revoked permission). Other errors are treated as transient local failures.
pub(super) fn unreachable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied | io::ErrorKind::NotADirectory
    )
}

/// Platform encoding of a folder path for the sealed journal.
pub(super) fn encode_os(value: &OsStr) -> Vec<u8> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        value.encode_wide().flat_map(u16::to_le_bytes).collect()
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        value.as_bytes().to_vec()
    }
    #[cfg(not(any(windows, unix)))]
    {
        value.to_string_lossy().as_bytes().to_vec()
    }
}

/// Decode a saved folder path only for the platform that wrote it.
pub(super) fn decode_os(platform: u8, bytes: &[u8]) -> Option<OsString> {
    if platform != CURRENT_PLATFORM || bytes.is_empty() {
        return None;
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        if bytes.len() % 2 != 0 {
            return None;
        }
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        Some(OsString::from_wide(&units))
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(OsStr::from_bytes(bytes).to_os_string())
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

/// A leaf the journal may name: valid UTF-8, one normal path component, within the bound, and
/// free of the characters native's name sanitizer already removes (including `:`, which on
/// Windows would address an alternate data stream of some other file).
pub(super) fn valid_leaf(leaf: &str) -> bool {
    let mut components = Path::new(leaf).components();
    !leaf.is_empty()
        && leaf.len() <= MAX_LEAF_BYTES
        && leaf != "."
        && leaf != ".."
        && !leaf.ends_with(['.', ' '])
        && !leaf.chars().any(|c| {
            c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
        && matches!(components.next(), Some(Component::Normal(name)) if name == OsStr::new(leaf))
        && components.next().is_none()
        && !windows_device_name(leaf)
}

/// Windows reserves these names (with any extension) for devices: opening `NUL.txt` or `COM1.log`
/// can address the device instead of a file. Native's name sanitizer already prefixes such peer
/// names, but a "Save as" name comes from the user's dialog and must be refused here instead.
/// Applied on every platform so a journal written on one cannot name a device on another.
fn windows_device_name(leaf: &str) -> bool {
    let stem = leaf
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let numbered = (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && matches!(stem.as_bytes()[3], b'1'..=b'9');
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered
}

/// Collision candidate `number` for a sanitized name, matching the native writer's convention:
/// `name`, `name (1).ext`, `name (2).ext`, ...
pub(super) fn numbered(name: &str, number: usize) -> String {
    if number == 0 {
        return name.to_string();
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("mewtual-download");
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
    {
        Some(extension) => format!("{stem} ({number}).{extension}"),
        None => format!("{stem} ({number})"),
    }
}

/// Staging name for a final leaf.
pub(super) fn staging_for(final_leaf: &str) -> String {
    format!("{final_leaf}{STAGING_SUFFIX}")
}

/// External effects that tests can make fail deterministically (disk full, unsupported links,
/// a locked file). Production builds compile the seam to nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Effect {
    Link,
    Write,
    Remove,
    /// The identity check of the final name right after a successful link.
    Check,
    /// Any handle identity query (`observe`), to model a filesystem without stable identities
    /// (some network shares and FUSE mounts) on a test host that has them.
    Identity,
    /// The sealed record write in `persistence.rs` (not an external path); shares the seam so a
    /// test can model a full vault volume.
    RecordWrite,
}
#[cfg(test)]
thread_local! {
    /// Per-test-thread injected failure: (effect, error kind, remaining count).
    pub(in crate::kept) static INJECT: std::cell::Cell<Option<(Effect, io::ErrorKind, u32)>> =
        const { std::cell::Cell::new(None) };
}
pub(super) fn inject(effect: Effect) -> io::Result<()> {
    #[cfg(test)]
    {
        if let Some((target, kind, remaining)) = INJECT.with(std::cell::Cell::get) {
            if target == effect && remaining > 0 {
                INJECT.with(|slot| slot.set(Some((target, kind, remaining - 1))));
                return Err(io::Error::from(kind));
            }
        }
    }
    let _ = effect;
    Ok(())
}
/// No-replace publication primitive: refuses an existing destination on every platform.
pub(super) fn link(from: &Path, to: &Path) -> io::Result<()> {
    inject(Effect::Link)?;
    fs::hard_link(from, to)
}
pub(super) fn remove(path: &Path) -> io::Result<()> {
    inject(Effect::Remove)?;
    fs::remove_file(path)
}

#[cfg(test)]
#[path = "external_tests.rs"]
pub(super) mod tests;
