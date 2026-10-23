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
//!   **This branch never runs on the Windows development host** (it is compiled and Clippy-checked
//!   for Android targets there); Linux CI is its first runner and macOS is not built at all. It
//!   uses stable `std`, `libc` constants and, since the PR51 review, `rustix` for the
//!   directory-relative calls (`openat`, `renameat`, `linkat`, `unlinkat`, `fstatfs`).
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
//! Deletion and publication (PR51 review; THREAT-MODEL has the guarantees). No `unsafe` is used:
//! every automatic delete goes through [`remove_verified`], which on Windows deletes through a
//! delete-denying handle verified to be the object (`fs_at`), and on Unix renames the verified
//! name aside relative to a pinned folder descriptor, verifies it again and unlinks it (`rustix`),
//! never in a folder other accounts can (or may) modify; on Unix the guarantee therefore holds only
//! against other accounts. Publication names exactly the held object: Linux and Android link the
//! open descriptor through `/proc/self/fd` ([`link_open`]); Windows holds a delete-denying pin on
//! the staging name across the path-based link ([`open_leaf_pinned`], [`publish_session`]); macOS
//! links by path and relies on the post-link check alone.
//!
//! Creating a new partial is also path-based (picker review L4): the folder identity check and
//! the exclusive create are separate calls. Each creation with a captured folder identity
//! re-checks it after creating and removes (by proven handle identity) and refuses a partial that
//! landed in a swapped folder. A swap and swap-back that both fall between the two checks still
//! puts an empty partial in the other folder; the link probe that follows requires the probe name
//! to be our own handle's object (whenever the filesystem reports identities), so it fails and no
//! plaintext is ever written there, even if a file was planted at our name meanwhile.
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
    #[cfg_attr(not(windows), allow(dead_code))]
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
    // Review U-M6: a directory-only, non-blocking open. A FIFO (or anything else) put at the saved
    // folder path fails at once with "not a directory" instead of blocking forever, while a
    // folder-level symlink to a directory is still followed.
    #[cfg(unix)]
    let handle = {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NONBLOCK)
            .open(folder)?
    };
    #[cfg(not(any(windows, unix)))]
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
    open_leaf_as(path, write, false)
}

/// Like [`open_leaf`] (read access), but on Windows the handle denies delete sharing for as long
/// as it is open: nobody can rename, replace or delete this file, or rename the folder that
/// contains it, until it is dropped. Publication holds it across the hard link so the object it
/// just proved is ours is exactly the object that gains the final name. Our own `hard_link` still
/// works (it does not ask for delete access). Unix has no share modes; there Linux and Android
/// link the open descriptor instead ([`link_open`]) and macOS relies on the post-link check.
pub(super) fn open_leaf_pinned(path: &Path) -> io::Result<Leaf> {
    open_leaf_as(path, false, true)
}

#[cfg(windows)]
const PINNED_SHARE: u32 = 0x1 | 0x2; // FILE_SHARE_READ | FILE_SHARE_WRITE, never FILE_SHARE_DELETE

#[cfg_attr(not(windows), allow(unused_variables))]
fn open_leaf_as(path: &Path, write: bool, pinned: bool) -> io::Result<Leaf> {
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
        if pinned {
            options.share_mode(PINNED_SHARE);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Review L11: a symlink swapped to a FIFO between the lstat above and this open must not
        // block forever (with Lock waiting behind the commit guard). O_NOFOLLOW refuses a link
        // at the final component; O_NONBLOCK makes a FIFO open return at once. On a regular
        // file O_NONBLOCK has no effect on reads or writes.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Leaf::Missing),
        // A link (ELOOP under O_NOFOLLOW) or a FIFO without a reader (ENXIO under O_NONBLOCK)
        // swapped in after the lstat: never ours, and never followed or waited on.
        #[cfg(unix)]
        Err(error) if matches!(error.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENXIO)) => {
            return Ok(Leaf::Foreign)
        }
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
        let mut plain = OpenOptions::new();
        plain.read(true).write(write);
        if pinned {
            use std::os::windows::fs::OpenOptionsExt;
            plain.share_mode(PINNED_SHARE);
        }
        let plain = match plain.open(path) {
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
///
/// This is a path-based call made after the caller's folder identity check, so the folder can be
/// swapped in between. Every caller with a captured folder identity re-checks it afterwards
/// (picker review L4, `confirm_folder` in `export.rs`) and removes a partial that landed in a
/// swapped folder; every link probe that follows checks the probe names our own handle.
pub(super) fn create_leaf(path: &Path) -> io::Result<File> {
    #[cfg(test)]
    BEFORE_CREATE.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook(path);
        }
    });
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)?;
    #[cfg(test)]
    AFTER_CREATE.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook(path);
        }
    });
    Ok(file)
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
///
/// The list follows Microsoft's file naming rules: `CON`, `PRN`, `AUX`, `NUL`, `COM0`-`COM9` and
/// `LPT0`-`LPT9` including the superscript digits `¹ ² ³` (picker review residual), plus the console
/// names `CONIN$` and `CONOUT$`. Trailing spaces before the extension are ignored, as Windows does.
/// Native's `safe_download_name` must stay at least this strict (a test pins the two together).
fn windows_device_name(leaf: &str) -> bool {
    let stem = leaf
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let mut chars = stem.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let digit = (chars.next(), chars.next());
    let numbered = matches!(prefix.as_str(), "COM" | "LPT")
        && matches!(digit, (Some('0'..='9' | '¹' | '²' | '³'), None));
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered
}

/// Longest path component the destination accepts, in its own units: NTFS allows 255 UTF-16 code
/// units, and ext4, APFS and most Unix filesystems allow 255 bytes.
const MAX_COMPONENT_UNITS: usize = 255;
/// Room reserved for the longest suffixes Mewtual appends to a final name: a collision number
/// (" (999)"), the staging suffix and the link-probe suffix ("-probe3").
const SUFFIX_RESERVE: usize = " (999)".len() + STAGING_SUFFIX.len() + "-probe3".len();

/// Length of `leaf` in the running platform's component units.
pub(super) fn component_units(leaf: &str) -> usize {
    #[cfg(windows)]
    {
        leaf.encode_utf16().count()
    }
    #[cfg(not(windows))]
    {
        leaf.len()
    }
}

/// Whether every name Mewtual may derive from `name` (numbered, staging, probe) stays within the
/// component limit. Review U-L3: the old `.part` writer only needed five extra bytes, so a long
/// multibyte name that it could save must not fail (or look like a no-link folder) here.
pub(super) fn leaf_fits(name: &str) -> bool {
    component_units(name) + SUFFIX_RESERVE <= MAX_COMPONENT_UNITS
}

/// Shorten a Downloads name so that [`leaf_fits`] holds, keeping a short extension. A name chosen
/// in "Save as" is never shortened; it is refused instead (the user picked that exact name).
pub(super) fn fit_leaf(name: &str) -> String {
    if leaf_fits(name) {
        return name.to_string();
    }
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 && component_units(&name[dot..]) <= 16 => name.split_at(dot),
        _ => (name, ""),
    };
    let budget = MAX_COMPONENT_UNITS - SUFFIX_RESERVE - component_units(extension);
    let mut kept = String::new();
    for c in stem.chars() {
        let mut next = kept.clone();
        next.push(c);
        if component_units(&next) > budget {
            break;
        }
        kept = next;
    }
    let kept = kept.trim_end_matches(['.', ' ']);
    let kept = if kept.is_empty() {
        "mewtual-download"
    } else {
        kept
    };
    format!("{kept}{extension}")
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
    /// After a hard link that DID create the new name, report an error anyway (a network
    /// filesystem whose reply was lost), to model an uncertain link error that made a link.
    LinkReport,
    Write,
    Remove,
    /// The identity check of the final name right after a successful link.
    Check,
    /// Any handle identity query (`observe`), to model a filesystem without stable identities
    /// (some network shares and FUSE mounts) on a test host that has them.
    Identity,
    /// The check that the open download can be linked by descriptor (`/proc/self/fd` on Linux),
    /// to model a container without `/proc` on any test host (review L-7).
    Descriptor,
    /// Unix: classifying a pinned folder's filesystem, to model a network or FUSE mount (an
    /// `Unknown` class) on a test host whose temporary folder is local (review M-3).
    #[cfg_attr(not(unix), allow(dead_code))]
    FsClass,
    /// The sealed record write in `persistence.rs` (not an external path); shares the seam so a
    /// test can model a full vault volume.
    RecordWrite,
    /// Creation of a new transfer directory in the vault (admission), likewise.
    CreateDir,
    /// The exclusive creation of a record's `next` file.
    RecordCreate,
    /// The flush of a record's `next` file before the rename.
    RecordSync,
    /// Removal of a torn `next` after a disk-full refusal.
    RecordDiscard,
    /// The directory flush after that removal.
    RecordDiscardBarrier,
}
#[cfg(test)]
thread_local! {
    /// Per-test-thread injected failure: (effect, error kind, remaining count).
    pub(in crate::kept) static INJECT: std::cell::Cell<Option<(Effect, io::ErrorKind, u32)>> =
        const { std::cell::Cell::new(None) };
    /// A second, independent slot, for tests that combine two failures.
    pub(in crate::kept) static INJECT_ALSO: std::cell::Cell<Option<(Effect, io::ErrorKind, u32)>> =
        const { std::cell::Cell::new(None) };
}
pub(super) fn inject(effect: Effect) -> io::Result<()> {
    #[cfg(test)]
    for slot in [&INJECT, &INJECT_ALSO] {
        if let Some((target, kind, remaining)) = slot.with(std::cell::Cell::get) {
            if target == effect && remaining > 0 {
                slot.with(|slot| slot.set(Some((target, kind, remaining - 1))));
                return Err(io::Error::from(kind));
            }
        }
    }
    let _ = effect;
    Ok(())
}
#[cfg(test)]
pub(in crate::kept) type LinkHook = Option<Box<dyn FnMut(&Path)>>;
#[cfg(test)]
thread_local! {
    /// Test-only interleaving point just before the path-based hard link: a test models another
    /// program acting on the source path at exactly that moment.
    pub(in crate::kept) static BEFORE_LINK: std::cell::RefCell<LinkHook> =
        const { std::cell::RefCell::new(None) };
    /// Test-only interleaving point just before an exclusive creation (picker review L4): a test
    /// swaps the destination folder between the identity check and the create.
    pub(in crate::kept) static BEFORE_CREATE: std::cell::RefCell<LinkHook> =
        const { std::cell::RefCell::new(None) };
    /// Test-only interleaving point right after a successful exclusive creation (picker
    /// re-review LOW 2): a test swaps the folder back and plants a file at our name.
    pub(in crate::kept) static AFTER_CREATE: std::cell::RefCell<LinkHook> =
        const { std::cell::RefCell::new(None) };
    /// Test-only interleaving point in [`remove_verified`], after the identity was verified and
    /// before the delete (PR51 H1): a test replaces the name with someone else's file there. The
    /// hook gets the original name's path.
    pub(in crate::kept) static BEFORE_REMOVE: std::cell::RefCell<LinkHook> =
        const { std::cell::RefCell::new(None) };
    /// Test-only interleaving point in Unix [`remove_verified`], right after the name was renamed
    /// aside and before the object there is verified (review M-2). The hook gets the original
    /// name's path and the aside path.
    #[cfg(unix)]
    pub(in crate::kept) static AFTER_ASIDE: std::cell::RefCell<AsideHook> =
        const { std::cell::RefCell::new(None) };
}
#[cfg(all(test, unix))]
pub(in crate::kept) type AsideHook = Option<Box<dyn FnMut(&Path, &Path)>>;

#[cfg(test)]
fn before_remove(path: &Path) {
    BEFORE_REMOVE.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook(path);
        }
    });
}
#[cfg(not(test))]
fn before_remove(_path: &Path) {}

#[cfg(all(test, unix))]
fn after_aside(original: &Path, aside: &Path) {
    AFTER_ASIDE.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook(original, aside);
        }
    });
}
#[cfg(all(not(test), unix))]
fn after_aside(_original: &Path, _aside: &Path) {}

/// What a destination's filesystem is known to support (PR51 M3). Fail closed: only filesystems
/// whose identity and hard links Mewtual relies on get a journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FsClass {
    /// Local filesystem with stable identities and hard links: the journaled writer may run.
    Journal,
    /// FAT, vfat or exFAT: no hard links, so no writer can publish there without replacing.
    NoLinks,
    /// Network, FUSE or an unrecognized type: no journal; the non-resumable writer (whose
    /// cleanup is still verified) may run if the folder passes its own checks.
    Unknown,
}

/// Classify the filesystem holding `folder`.
///
/// - Linux and Android: `fstatfs` magic numbers. ext2/3/4, xfs, btrfs, tmpfs and f2fs journal;
///   msdos/vfat and exFAT are `NoLinks`; anything else (nfs, cifs/smb, fuse, overlay, 9p) is
///   `Unknown`.
/// - macOS (not built by CI; unverified): `MNT_LOCAL` with `apfs` or `hfs` journals; `msdos` and
///   `exfat` are `NoLinks`.
/// - Windows: no vetted safe crate reports the filesystem name or the remote flag, so the folder
///   is canonicalized with std (`GetFinalPathNameByHandle`), which names UNC paths, mapped network
///   drives, DFS, WebDAV and `\\wsl$` as `\\?\UNC\...`; those are `Unknown`. Everything else is
///   `Journal`, with FAT/exFAT caught by the link probe at creation. Local virtual filesystems
///   (WinFsp, Dokan) look local here and rely on the probes; ReFS is unverified (64-bit index).
pub(super) fn fs_class(folder: &Path) -> io::Result<FsClass> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(folder)?;
        fs_class_of_dir(&dir)
    }
    #[cfg(windows)]
    {
        let canonical = fs::canonicalize(folder)?;
        Ok(classify_windows_canonical(&canonical.to_string_lossy()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = folder;
        Ok(FsClass::Unknown)
    }
}

/// Unix: [`fs_class`] of a folder already held open, so a pinned folder is classified without
/// resolving its path again.
#[cfg(unix)]
fn fs_class_of_dir(dir: &File) -> io::Result<FsClass> {
    // Test seam: an injected `Unsupported` models FAT/exFAT, any other kind a network or FUSE
    // mount.
    if let Err(error) = inject(Effect::FsClass) {
        return Ok(if error.kind() == io::ErrorKind::Unsupported {
            FsClass::NoLinks
        } else {
            FsClass::Unknown
        });
    }
    {
        let stat = rustix::fs::fstatfs(dir).map_err(io::Error::from)?;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            // `f_type` is a signed long whose width varies; the magic numbers are 32-bit.
            #[allow(clippy::unnecessary_cast)]
            Ok(classify_linux_magic(stat.f_type as u32))
        }
        #[cfg(target_vendor = "apple")]
        {
            const MNT_LOCAL: u32 = 0x0000_1000;
            let name: Vec<u8> = stat
                .f_fstypename
                .iter()
                .take_while(|c| **c != 0)
                .map(|c| *c as u8)
                .collect();
            Ok(classify_apple(
                &String::from_utf8_lossy(&name),
                stat.f_flags & MNT_LOCAL != 0,
            ))
        }
        #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
        {
            let _ = stat;
            Ok(FsClass::Unknown)
        }
    }
}

/// Linux `statfs.f_type` to a class. Pure, tested on every platform.
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
pub(super) fn classify_linux_magic(magic: u32) -> FsClass {
    const EXT234: u32 = 0xEF53;
    const XFS: u32 = 0x5846_5342;
    const BTRFS: u32 = 0x9123_683E;
    const TMPFS: u32 = 0x0102_1994;
    const F2FS: u32 = 0xF2F5_2010;
    const MSDOS: u32 = 0x4D44;
    const EXFAT: u32 = 0x2011_BAB0;
    match magic {
        EXT234 | XFS | BTRFS | TMPFS | F2FS => FsClass::Journal,
        MSDOS | EXFAT => FsClass::NoLinks,
        _ => FsClass::Unknown,
    }
}

/// macOS `f_fstypename` and `MNT_LOCAL` to a class. Pure, tested on every platform.
#[cfg_attr(not(target_vendor = "apple"), allow(dead_code))]
pub(super) fn classify_apple(name: &str, local: bool) -> FsClass {
    match name {
        "msdos" | "exfat" => FsClass::NoLinks,
        "apfs" | "hfs" if local => FsClass::Journal,
        _ => FsClass::Unknown,
    }
}

/// A Windows canonical path (std `canonicalize`) to a class. Pure, tested on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn classify_windows_canonical(canonical: &str) -> FsClass {
    let lower = canonical.to_ascii_lowercase();
    if lower.starts_with(r"\\?\unc\") || (lower.starts_with(r"\\") && !lower.starts_with(r"\\?\")) {
        FsClass::Unknown
    } else {
        FsClass::Journal
    }
}

/// Identity of an object this process holds open, for in-session "same object" checks (PR51 H1,
/// design review H-D): volume serial and file index on Windows, device and inode on Unix. Unlike
/// [`FileIdentity`] it needs no birth time, because it is only compared while the object is kept
/// alive (by the held handle or by its names), so its number cannot be reused for another file
/// meanwhile. It is never persisted. Debug output is redacted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SessionId {
    volume: u64,
    index: u64,
}
impl std::fmt::Debug for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionId { .. }")
    }
}

/// What a handle reports for an in-session check.
struct SessionObserved {
    id: SessionId,
    links: u64,
    regular: bool,
}

fn session_observe(file: &File) -> io::Result<SessionObserved> {
    inject(Effect::Identity)?;
    let unsupported = || {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "this filesystem gives no file number to check against",
        )
    };
    #[cfg(windows)]
    let (volume, index, links, regular) = {
        let info = winapi_util::file::information(file)?;
        let surrogate = file
            .metadata()
            .map_or(true, |metadata| metadata.file_type().is_symlink());
        (
            info.volume_serial_number(),
            info.file_index(),
            info.number_of_links(),
            regular_from(info.file_attributes(), surrogate),
        )
    };
    #[cfg(unix)]
    let (volume, index, links, regular) = {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        (
            metadata.dev(),
            metadata.ino(),
            metadata.nlink(),
            metadata.file_type().is_file(),
        )
    };
    #[cfg(not(any(windows, unix)))]
    let (volume, index, links, regular): (u64, u64, u64, bool) = {
        let _ = file;
        return Err(unsupported());
    };
    if volume == 0 || index == 0 {
        return Err(unsupported());
    }
    Ok(SessionObserved {
        id: SessionId { volume, index },
        links,
        regular,
    })
}

/// The [`SessionId`] of an object this process holds open; `Unsupported` when the filesystem
/// reports no usable number (then nothing there can ever be proven ours, so callers refuse).
pub fn session_id(file: &File) -> io::Result<SessionId> {
    session_observe(file).map(|observed| observed.id)
}

/// What a removal must find at the name before it may delete it.
#[derive(Clone, Copy)]
pub(super) enum Expected {
    /// The persisted identity of a journaled partial (survives restart).
    Journal(FileIdentity),
    /// The in-session number of an object this process created and keeps alive.
    Session(SessionId),
}

/// Outcome of [`remove_verified`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verified {
    /// The verified object's name was deleted. `links` is how many names it had before.
    Removed { links: u64 },
    /// Nothing is at the name.
    Missing,
    /// Something else is at the name; it was left exactly where it was.
    NotOurs,
    /// Unix: the folder can be modified by other accounts, so nothing is deleted automatically.
    #[cfg_attr(not(unix), allow(dead_code))]
    Shared,
    /// Unix: the folder opened at this path is no longer the expected one.
    #[cfg_attr(not(unix), allow(dead_code))]
    FolderChanged,
}

/// Delete `folder/leaf` only if it is the expected object (PR51 H1). Never deletes or changes the
/// attributes of anything that was not verified, through the same handle or directory entry.
///
/// - Windows: the name is opened with `DELETE` access, without following a reparse point and
///   without `FILE_SHARE_DELETE`, so while the handle is open nobody can rename, replace or delete
///   that name (or rename its folders). The identity is checked on that handle and the delete is
///   made through it (`fs_at::os::windows::FileExt::delete_by_handle`: POSIX semantics where
///   available, which also ignores the read-only attribute; otherwise classic disposition). Only
///   the name the handle was opened through goes, so a second name (a published final name) stays.
///   Long-lived handles elsewhere keep std's default share mode, which allows this open.
/// - Unix (no delete-by-inode exists): the folder is pinned by a descriptor (following a configured
///   folder link, then checked against `folder_id` when known). If other accounts can modify it
///   (owner neither us nor root, or group- or other-writable without the sticky bit) or may (a
///   network, FUSE or unrecognized filesystem, whose permissions this kernel does not decide;
///   review M-3), nothing is deleted (`Shared`). Otherwise the name is verified where it is (an
///   object that is not ours is never renamed; review M-2), renamed relative to that descriptor to
///   a random unguessable name (no-replace where the platform has it), verified again there, and
///   unlinked only if it is still ours; otherwise it is put back without replacing anything and
///   reported `NotOurs`, or, if it cannot be put back, the call fails (the caller keeps tracking
///   the save). A process running as the same account that watches the folder could still swap
///   the random name in the window before the unlink; such a process already has the authority
///   to delete the file itself.
pub(super) fn remove_verified(
    folder: &Path,
    folder_id: Option<FileIdentity>,
    leaf: &str,
    expected: Expected,
    rng: &mut dyn catcoms_rt::CryptoRngCore,
) -> io::Result<Verified> {
    #[cfg(windows)]
    {
        let _ = (folder_id, &rng);
        let path = folder.join(leaf);
        let Some(handle) = open_for_delete(&path)? else {
            return Ok(Verified::Missing);
        };
        let Some(links) = verify(&handle, expected)? else {
            return Ok(Verified::NotOurs);
        };
        before_remove(&path);
        inject(Effect::Remove)?;
        use fs_at::os::windows::FileExt;
        handle.delete_by_handle().map_err(|(_, error)| error)?;
        Ok(Verified::Removed { links })
    }
    #[cfg(unix)]
    {
        let pinned = match pin_folder(folder, folder_id)? {
            Some(pinned) => pinned,
            None => return Ok(Verified::FolderChanged),
        };
        // Review M-2: verify at the original name first, so an object that is not ours (a
        // directory, a link, the user's own file) is never renamed at all. Only a swap between
        // this check and the rename below can still move a foreign object aside, and the second
        // check at the aside name catches that. Combined review M-A: this read-only check also
        // comes before the shared-folder answer, so `Shared` (which tells the user to delete the
        // leftover) is reported only for a name that holds our own file; a missing or foreign
        // name is `Missing` or `NotOurs` there too.
        match open_at_nofollow(&pinned.dir, leaf) {
            Ok(Some(file)) => {
                if verify(&file, expected)?.is_none() {
                    return Ok(Verified::NotOurs);
                }
            }
            Ok(None) => return Ok(Verified::NotOurs),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Verified::Missing),
            Err(error) => return Err(error),
        }
        if pinned.shared {
            return Ok(Verified::Shared);
        }
        let aside = aside_name(rng);
        match rename_no_replace(&pinned.dir, leaf, &aside) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Verified::Missing),
            Err(error) => return Err(error),
        }
        after_aside(&folder.join(leaf), &folder.join(&aside));
        let verdict = match open_at_nofollow(&pinned.dir, &aside) {
            Ok(Some(file)) => verify(&file, expected),
            Ok(None) => Ok(None),
            Err(error) => Err(error),
        };
        // Putting an object back must succeed before anything is reported about it: a failure
        // leaves it at the random name, so the outcome is an error (the caller keeps tracking the
        // save as Unchecked), never "not ours" or "left in place".
        let put_back = |pinned: &Pinned| {
            rename_back(&pinned.dir, &aside, leaf).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("a name moved aside for checking could not be put back: {error}"),
                )
            })
        };
        match verdict {
            Ok(Some(links)) => {
                before_remove(&folder.join(leaf));
                let removed = inject(Effect::Remove).and_then(|()| {
                    rustix::fs::unlinkat(&pinned.dir, aside.as_str(), rustix::fs::AtFlags::empty())
                        .map_err(io::Error::from)
                });
                if let Err(error) = removed {
                    // Never leave our plaintext at an unrecorded name: put it back. If that fails
                    // too, a journaled save's next Remove finds it by identity (`sweep_aside`).
                    let _ = put_back(&pinned);
                    return Err(error);
                }
                let _ = pinned.dir.sync_all();
                Ok(Verified::Removed { links })
            }
            Ok(None) => {
                put_back(&pinned)?;
                Ok(Verified::NotOurs)
            }
            Err(error) => {
                put_back(&pinned)?;
                Err(error)
            }
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (folder, folder_id, leaf, expected, rng);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "verified removal is unavailable on this platform",
        ))
    }
}

/// What [`sweep_aside`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Aside {
    /// No object of this save sits at a "removing" name.
    Nothing,
    /// One did, and it was deleted.
    #[cfg_attr(not(unix), allow(dead_code))]
    Removed,
    /// One does and stays: only looking, a folder other accounts can modify (`shared`), or the
    /// unlink failed.
    #[cfg_attr(not(unix), allow(dead_code))]
    Left { shared: bool },
}

/// Unix: find (and, with `remove`, delete) an object of this save that a crash left at a random
/// "removing" name, between the rename aside and the unlink of [`remove_verified`]. Matching is by
/// the journal's identity, never by name. Windows never renames aside, so it finds nothing.
pub(super) fn sweep_aside(
    folder: &Path,
    folder_id: FileIdentity,
    expected: FileIdentity,
    remove: bool,
) -> io::Result<Aside> {
    #[cfg(unix)]
    {
        let Some(pinned) = pin_folder(folder, Some(folder_id))? else {
            return Ok(Aside::Nothing);
        };
        let mut names = Vec::new();
        for entry in rustix::fs::Dir::read_from(&pinned.dir).map_err(io::Error::from)? {
            let entry = entry.map_err(io::Error::from)?;
            if let Ok(name) = entry.file_name().to_str() {
                if name.starts_with(ASIDE_PREFIX) {
                    names.push(name.to_owned());
                }
            }
        }
        for name in names {
            let Ok(Some(file)) = open_at_nofollow(&pinned.dir, &name) else {
                continue;
            };
            if verify(&file, Expected::Journal(expected))?.is_none() {
                continue;
            }
            if !remove || pinned.shared {
                return Ok(Aside::Left {
                    shared: pinned.shared,
                });
            }
            return Ok(
                match rustix::fs::unlinkat(&pinned.dir, name.as_str(), rustix::fs::AtFlags::empty())
                {
                    Ok(()) => Aside::Removed,
                    Err(_) => Aside::Left { shared: false },
                },
            );
        }
        Ok(Aside::Nothing)
    }
    #[cfg(not(unix))]
    {
        let _ = (folder, folder_id, expected, remove);
        Ok(Aside::Nothing)
    }
}

/// The links of the object behind `handle` if it is the expected regular file, else `None`.
fn verify(handle: &File, expected: Expected) -> io::Result<Option<u64>> {
    match expected {
        Expected::Journal(identity) => {
            let observed = match observe(handle) {
                Ok(observed) => observed,
                Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(None),
                Err(error) => return Err(error),
            };
            Ok((observed.regular && observed.identity == identity).then_some(observed.links))
        }
        Expected::Session(id) => {
            let observed = match session_observe(handle) {
                Ok(observed) => observed,
                Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(None),
                Err(error) => return Err(error),
            };
            Ok((observed.regular && observed.id == id).then_some(observed.links))
        }
    }
}

#[cfg(windows)]
fn open_for_delete(path: &Path) -> io::Result<Option<File>> {
    use std::os::windows::fs::OpenOptionsExt;
    const DELETE: u32 = 0x0001_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    // No FILE_WRITE_ATTRIBUTES: the classic-disposition fallback for a read-only file (FAT, SMB)
    // then fails instead of clearing the attribute, which a second name would share.
    // Backup semantics so a directory or junction put at the name opens too (and is then found
    // not to be a regular file) instead of failing as "access denied".
    match OpenOptions::new()
        .access_mode(DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(PINNED_SHARE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
    {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Unix: the destination folder, held open for directory-relative operations.
#[cfg(unix)]
struct Pinned {
    dir: File,
    /// Other accounts can (or may, on a filesystem whose permissions this kernel does not decide)
    /// modify entries in it, so nothing is deleted automatically.
    shared: bool,
}

/// Unix: open `folder` (following a configured folder link), check it against `expected` when
/// known, and classify whether other accounts can modify it. `None` means it is not the expected
/// folder any more.
#[cfg(unix)]
fn pin_folder(folder: &Path, expected: Option<FileIdentity>) -> io::Result<Option<Pinned>> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(folder)?;
    if let Some(expected) = expected {
        match observe(&dir) {
            Ok(observed) if observed.directory && observed.identity == expected => {}
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    let metadata = dir.metadata()?;
    // Review M-3: mode bits describe who may change a folder only on a local filesystem whose
    // permissions are enforced by this kernel. On network and FUSE mounts (CIFS shows the mount
    // owner and `dir_mode`, sshfs hides the remote host's accounts) and wherever the class is
    // unknown, other accounts may well change the folder, so it counts as shared (fail closed).
    // ACLs (NFSv4, macOS `delete_child`) are not read; see THREAT-MODEL.
    let local = matches!(fs_class_of_dir(&dir), Ok(FsClass::Journal));
    Ok(Some(Pinned {
        shared: !local
            || shared_folder(
                metadata.uid(),
                metadata.mode(),
                rustix::process::geteuid().as_raw(),
            ),
        dir,
    }))
}

/// Whether accounts other than ours (and root) can modify entries in a folder with this owner and
/// mode: owned by another account, or group- or other-writable without the sticky bit. A sticky
/// folder (like `/tmp`) lets others create entries but not rename or remove ours. Pure, so it is
/// tested on every platform. A user-private-group setup (0775, group of one) is classified as
/// shared: the mode bits cannot tell who is in the group, so it fails closed.
#[cfg_attr(not(unix), allow(dead_code))]
pub(super) fn shared_folder(owner: u32, mode: u32, me: u32) -> bool {
    const GROUP_OR_OTHER_WRITE: u32 = 0o022;
    const STICKY: u32 = 0o1000;
    let foreign_owner = owner != me && owner != 0;
    foreign_owner || (mode & GROUP_OR_OTHER_WRITE != 0 && mode & STICKY == 0)
}

#[cfg(unix)]
const ASIDE_PREFIX: &str = "mewtual-removing-";

#[cfg(unix)]
fn aside_name(rng: &mut dyn catcoms_rt::CryptoRngCore) -> String {
    format!(
        "{ASIDE_PREFIX}{:016x}{:016x}",
        rng.next_u64(),
        rng.next_u64()
    )
}

#[cfg(unix)]
fn open_at_nofollow(dir: &File, name: &str) -> io::Result<Option<File>> {
    use rustix::fs::{Mode, OFlags};
    match rustix::fs::openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => Ok(Some(File::from(fd))),
        Err(rustix::io::Errno::NOENT) => Err(io::Error::from(io::ErrorKind::NotFound)),
        // A link (refused by O_NOFOLLOW) or a FIFO without a reader: never ours.
        Err(rustix::io::Errno::LOOP) | Err(rustix::io::Errno::NXIO) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Rename within the pinned folder, refusing to replace an existing name where the platform can.
#[cfg(unix)]
fn rename_no_replace(dir: &File, from: &str, to: &str) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
    match rustix::fs::renameat_with(dir, from, dir, to, rustix::fs::RenameFlags::NOREPLACE) {
        Ok(()) => return Ok(()),
        // NFS, many FUSE filesystems and old kernels lack no-replace rename. The random target
        // name cannot be guessed, so a plain rename to it replaces nothing in practice.
        Err(rustix::io::Errno::INVAL) | Err(rustix::io::Errno::NOSYS) => {}
        Err(rustix::io::Errno::NOTSUP) => {}
        Err(error) => return Err(error.into()),
    }
    rustix::fs::renameat(dir, from, dir, to).map_err(io::Error::from)
}

/// Put an object renamed aside back at its original name, never replacing anything that appeared
/// there meanwhile (then it stays at the aside name and the call fails, so the caller reports an
/// error; a later Remove of a journaled save finds its own file there by identity).
///
/// Review M-2: where no-replace rename is missing (NFS, CIFS, many FUSE), the fallback is a hard
/// link back (which refuses an existing name atomically) and then the removal of the aside name,
/// never a "check, then plain rename" that could replace an object created in between. Where that
/// link fails too (a directory, a filesystem without links), the object stays aside and the call
/// fails.
#[cfg(unix)]
fn rename_back(dir: &File, aside: &str, leaf: &str) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
    match rustix::fs::renameat_with(dir, aside, dir, leaf, rustix::fs::RenameFlags::NOREPLACE) {
        Ok(()) => return Ok(()),
        Err(rustix::io::Errno::INVAL) | Err(rustix::io::Errno::NOSYS) => {}
        Err(rustix::io::Errno::NOTSUP) => {}
        Err(error) => return Err(error.into()),
    }
    rustix::fs::linkat(dir, aside, dir, leaf, rustix::fs::AtFlags::empty())
        .map_err(io::Error::from)?;
    rustix::fs::unlinkat(dir, aside, rustix::fs::AtFlags::empty()).map_err(io::Error::from)
}
/// No-replace publication primitive: refuses an existing destination on every platform. Linux and
/// Android publish through [`link_open`] instead and do not probe, so they never call it.
#[cfg_attr(any(target_os = "linux", target_os = "android"), allow(dead_code))]
pub(super) fn link(from: &Path, to: &Path) -> io::Result<()> {
    inject(Effect::Link)?;
    #[cfg(test)]
    BEFORE_LINK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook(from);
        }
    });
    fs::hard_link(from, to)?;
    inject(Effect::LinkReport)
}

/// PR51 M1: whether this platform can link the open session's own object to the final name. On
/// Linux and Android that goes through `/proc/self/fd`, which a container or chroot may lack; a
/// journal is never started where it could not finish that way. Elsewhere publication links by
/// path (Windows under the delete-denying pin; macOS without one), so the answer is yes.
pub(super) fn link_by_descriptor_available(file: &File) -> bool {
    if inject(Effect::Descriptor).is_err() {
        return false;
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::io::AsRawFd;
        let Ok(held) = file.metadata() else {
            return false;
        };
        // Read-only: following the magic link must reach the very object we hold.
        let magic = format!("/proc/self/fd/{}", file.as_raw_fd());
        matches!(
            fs::metadata(&magic),
            Ok(found) if found.dev() == held.dev() && found.ino() == held.ino()
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = file;
        true
    }
}

/// PR51 M1: give the object behind `file` (the verified, open session) the name `final_leaf` in
/// `folder`, never replacing anything.
///
/// - Linux and Android: `linkat` of `/proc/self/fd/N` with `AT_SYMLINK_FOLLOW` into the folder,
///   pinned by a descriptor checked against `folder_id` when the caller has one (a writer without a
///   journal has none). The new name is for exactly the object we hold open, whatever has happened
///   to the staging name meanwhile, so a swapped staging name can never become the final name.
///   `ENOENT` (no `/proc`, or the object has no names left) and `EXDEV` mean no link was made and
///   are reported as `Unsupported`.
/// - Elsewhere: the path-based no-replace hard link from `staging` (Windows callers hold the
///   delete-denying pin across it; macOS has no link by descriptor, so the post-link check is its
///   only guarantee).
pub(super) fn link_open(
    file: &File,
    folder: &Path,
    folder_id: Option<FileIdentity>,
    staging: &str,
    final_leaf: &str,
) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::io::AsRawFd;
        let _ = staging;
        inject(Effect::Link)?;
        #[cfg(test)]
        BEFORE_LINK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook(&folder.join(staging));
            }
        });
        let Some(pinned) = pin_folder(folder, folder_id)? else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the destination folder changed",
            ));
        };
        let magic = format!("/proc/self/fd/{}", file.as_raw_fd());
        match rustix::fs::linkat(
            rustix::fs::CWD,
            magic.as_str(),
            &pinned.dir,
            final_leaf,
            rustix::fs::AtFlags::SYMLINK_FOLLOW,
        ) {
            Ok(()) => {}
            Err(rustix::io::Errno::EXIST) => {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists))
            }
            // Review L-3: a definite "no link was made" (no /proc any more, the object has no
            // names left, or the folder is now on another device). `Unsupported` lets the
            // journaled session halt without treating the save as possibly published.
            Err(rustix::io::Errno::NOENT) | Err(rustix::io::Errno::XDEV) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "the saved file could not be linked from the open download",
                ))
            }
            Err(error) => return Err(error.into()),
        }
        // Review L-1: no folder flush here. This runs under the session commit guard; the
        // journaled sequence flushes the folder after releasing it (PR51 M2), and the writers
        // without a journal flush it when they settle their staging name.
        inject(Effect::LinkReport)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = (file, folder_id);
        link(&folder.join(staging), &folder.join(final_leaf))
    }
}

/// What a no-follow open found at a name.
enum AtName {
    /// Something openable is there (still to be verified).
    Object(File),
    /// Nothing is there.
    Missing,
    /// Unix: a symbolic link or a FIFO without a reader; never ours, never followed or waited on.
    #[cfg_attr(not(unix), allow(dead_code))]
    Foreign,
}

/// Open `path` for an in-session check without following a link at the final component. With
/// `pinned` (Windows), the handle denies delete sharing for as long as it is open, so nobody can
/// rename, replace or delete that name or rename its folders meanwhile; it asks for read access
/// because a handle without data access takes part in no sharing check. Unix has no share modes;
/// `pinned` changes nothing there.
fn open_nofollow(path: &Path, pinned: bool) -> io::Result<AtName> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Backup semantics so a directory or junction put at the name opens too (and is then
        // found not to be a regular file) instead of failing as "access denied".
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
        if pinned {
            options.share_mode(PINNED_SHARE);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = pinned;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    match options.open(path) {
        Ok(file) => Ok(AtName::Object(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AtName::Missing),
        #[cfg(unix)]
        Err(error) if matches!(error.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENXIO)) => {
            Ok(AtName::Foreign)
        }
        Err(error) => Err(error),
    }
}

/// Read-only: whether `path` holds the object with `id` (review M-4). `Ok(None)` when nothing is
/// there; `Ok(Some(false))` for anything else (another file, a link, a folder, no file number).
pub(super) fn holds_session(path: &Path, id: SessionId) -> io::Result<Option<bool>> {
    match open_nofollow(path, false)? {
        AtName::Object(found) => Ok(Some(verify(&found, Expected::Session(id))?.is_some())),
        AtName::Missing => Ok(None),
        AtName::Foreign => Ok(Some(false)),
    }
}

/// The error a writer without a journal reports when a name it relies on holds another object.
fn session_changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "a different file appeared at the save's name",
    )
}

/// PR51 M1 for writers without a journal (native's non-resumable writer and the flipnote GIF
/// export): give the object behind `file`, which this process created as `folder/staging` and
/// still holds open, the absent name `folder/final_leaf`, never replacing anything, then confirm
/// that the final name holds that very object.
///
/// - Linux and Android: [`link_open`] links from the open descriptor into the pinned folder, so a
///   staging name swapped after the caller verified the bytes can never become the final name.
/// - Windows: the staging name is first opened with the delete-denying pin and checked to be
///   `file`'s object. While the pin is held nobody can rename, replace or delete it, or rename its
///   folders, so the path-based link names exactly the object checked.
/// - macOS: no link by descriptor and no pin. Only the post-link check holds there: a foreign
///   object found at the final name is reported as changed, and both names are left alone.
///
/// On Linux, Android and Windows a post-link check that cannot be made (an indexer briefly holding
/// the new name) does not fail the save, because the link itself already named our object. On
/// macOS it does: without the check nothing proves the final name is ours.
///
/// Errors: `AlreadyExists` (the final name is taken; nothing was linked), `NotFound` (the staging
/// name is gone or the folder changed; nothing was linked), `Unsupported` (no file number to check
/// against, or on Linux and Android a definite no-link from the descriptor: no `/proc`, the object
/// has no names left, or another device), `InvalidData` (a name holds another object; nothing of
/// ours is reported published), or the link's own error.
pub(super) fn publish_session(
    file: &File,
    folder: &Path,
    staging: &str,
    final_leaf: &str,
) -> io::Result<()> {
    let id = session_id(file)?;
    #[cfg(windows)]
    let _pin = match open_nofollow(&folder.join(staging), true)? {
        AtName::Object(pin) if verify(&pin, Expected::Session(id))?.is_some() => pin,
        AtName::Missing => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the partly saved file is gone",
            ))
        }
        _ => return Err(session_changed()),
    };
    link_open(file, folder, None, staging, final_leaf)?;
    let checked =
        inject(Effect::Check).and_then(|()| open_nofollow(&folder.join(final_leaf), false));
    let verified = match checked {
        Ok(AtName::Object(found)) => verify(&found, Expected::Session(id)).map(|v| v.is_some()),
        Ok(_) => Ok(false),
        Err(error) => Err(error),
    };
    match verified {
        Ok(true) => Ok(()),
        // The final name holds something else: never claim it.
        Ok(false) => Err(session_changed()),
        #[cfg(any(windows, target_os = "linux", target_os = "android"))]
        Err(_) => Ok(()),
        #[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
        Err(error) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the saved name could not be confirmed: {error}"),
        )),
    }
}

#[cfg(test)]
#[path = "external_tests.rs"]
pub(super) mod tests;
