//! The per-server saved-files list (PR #56, `docs/design-saved-files-list.md`).
//!
//! A finished save releases the private encrypted copy it was made from, so before this list
//! nothing remembered where a saved file went. The list records each finished save of a shared
//! file: its plaintext CID, where it was saved (folder and name), the saved file's identity when
//! the save was journaled, its size and when it was saved. Native seals the encoded list into
//! `servers/<id>.saved` inside the vault (`catcoms_app::ServerStore`), so this module is the codec,
//! the bounded list operations, the read-only checks of a recorded file and the sequence of the
//! opt-in "Replace my older saved copy".
//!
//! Invariants this module keeps:
//!
//! - **Records, not copies.** The list never keeps a copy. Dropping an entry (the cap, Clear, a
//!   newer save at the same place) only forgets it. The only code here that changes an external
//!   file is [`replace_older_copy`] and [`drop_first_name`], and both act only through the
//!   verified-object primitives in `external.rs`.
//! - **Bounded.** At most [`MAX_SAVED_FILES`] entries and [`MAX_SAVED_LIST_BYTES`] of plaintext;
//!   a decode refuses anything larger, so a damaged or hostile record cannot make a reader
//!   allocate without bound.
//! - **One entry per saved copy.** Two saves of one file are two copies and two entries. No two
//!   entries name the same place (the same file identity, or the same folder and name), because a
//!   name holds one file: a newer entry at a place replaces the older one there.
//! - **Versioned and non-destructive.** The plaintext starts with a domain string and a version
//!   byte. Anything this reader does not understand decodes as an error, and the caller must then
//!   leave the record exactly as it is (only leaving the server removes it), so a record written
//!   by a newer build survives a round trip through an older one.
//! - **Paths stay native.** Folder paths and identities are redacted from `Debug` output and never
//!   leave native; the renderer gets the entry ID, the CID, the file name and display facts.
use super::external::{self, FileIdentity, HeldName, Unchanged, Verified};
use super::*;

/// Most entries one server's list keeps. It matches the file index's own 256-row cap; older
/// entries are dropped first.
pub const MAX_SAVED_FILES: usize = 256;
/// Largest encoded plaintext of one list. 256 entries of maximal folder paths would be about
/// 4.5 MiB, so for very long paths this bound, not the count, caps the list (typical entries are
/// about 200 bytes). Readers read at most this plus the seal overhead.
pub const MAX_SAVED_LIST_BYTES: usize = 1024 * 1024;
/// Exact domain string at the start of the plaintext. A different sealed record copied over this
/// one (all vault records share the `db_key`) decodes as unreadable instead of being misread.
const DOMAIN: &[u8] = b"mewtual/saved-files";
/// The only version this build reads and writes.
pub const SAVED_LIST_VERSION: u8 = 1;
/// The owner binding is the group id (at most 256 bytes, as in the cache setting record) followed
/// by this device's member id in that group.
const MAX_OWNER_BYTES: usize = 512;

/// Random handle of one entry, the renderer's only way to name it for an action. It is not a
/// capability by itself: every command re-reads the sealed list and acts only on an entry that is
/// there now.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SavedFileId([u8; 16]);
impl std::fmt::Debug for SavedFileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SavedFileId({})", self.to_hex())
    }
}
impl SavedFileId {
    /// A fresh random ID from the sanctioned RNG seam.
    pub fn fresh(rng: &mut dyn CryptoRngCore) -> Self {
        let mut bytes = [0u8; 16];
        rng.fill_bytes(&mut bytes);
        Self(bytes)
    }
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
    /// 32 lowercase hex digits, the renderer's form.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
    /// Parse exactly 32 lowercase hex digits; anything else is `None`.
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 32
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        let mut bytes = [0u8; 16];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
        }
        Some(Self(bytes))
    }
}

/// Where a saved file is: the platform that wrote it, the encoded folder path, the file name and,
/// for a journaled save, the folder's and the file's handle identities. Native-only.
#[derive(Clone, PartialEq, Eq)]
pub struct SavedLocation {
    pub(super) platform: u8,
    pub(super) folder: Vec<u8>,
    pub(super) leaf: String,
    /// `(folder identity, file identity)`. Present only when the save was journaled: the journal
    /// is started only where the filesystem's identities are trusted (birth time, local volume),
    /// and only an identity lets a later check, reveal or replace prove the file is the one saved.
    pub(super) identity: Option<(FileIdentity, FileIdentity)>,
}
impl std::fmt::Debug for SavedLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedLocation")
            .field("identified", &self.identity.is_some())
            .finish_non_exhaustive()
    }
}
impl SavedLocation {
    /// The location of a journaled save from its Published journal: the journal's folder, its
    /// final (published) name and both identities.
    pub fn from_journal(journal: &ExportJournal) -> Self {
        Self {
            platform: journal.folder_id.platform(),
            folder: journal.folder.clone(),
            leaf: journal.final_leaf.clone(),
            identity: Some((journal.folder_id, journal.file_id)),
        }
    }
    /// The location of a save published by the non-resumable writer: its folder and name only,
    /// no identity (that writer runs exactly where no trusted identity exists). `None` when the
    /// path has no parent, no valid name, or a folder over the bound.
    pub fn unidentified(path: &Path) -> Option<Self> {
        let folder = external::encode_os(path.parent()?.as_os_str());
        let leaf = path.file_name()?.to_str()?.to_owned();
        let location = Self {
            platform: external::CURRENT_PLATFORM,
            folder,
            leaf,
            identity: None,
        };
        location.valid().then_some(location)
    }
    /// The saved file's name, for display.
    pub fn leaf(&self) -> &str {
        &self.leaf
    }
    /// Whether this location carries identities a check can compare.
    pub fn identified(&self) -> bool {
        self.identity.is_some()
    }
    /// Whether this platform can check it: identities written on this platform. An entry from a
    /// vault moved to another operating system is never reinterpreted.
    pub fn checkable(&self) -> bool {
        self.identified() && self.platform == external::CURRENT_PLATFORM
    }
    fn folder_path(&self) -> Option<PathBuf> {
        external::decode_os(self.platform, &self.folder).map(PathBuf::from)
    }
    /// Whether the saved folder is exactly `folder`, compared as recorded (no filesystem access),
    /// the same rule as [`ExportJournal::is_in_folder`]: a moved Downloads folder, other casing or
    /// a link answers "no". A location from another platform answers "no".
    pub fn is_in_folder(&self, folder: &Path) -> bool {
        self.folder_path().is_some_and(|own| own == folder)
    }
    /// Whether two locations name the same place: the same file identity, or the same folder and
    /// the same name (case-insensitively for Windows, whose names are). A name holds one file, so
    /// a newer entry at a place means the older entry's file is not there any more.
    fn same_place(&self, other: &Self) -> bool {
        if let (Some((_, mine)), Some((_, theirs))) = (self.identity, other.identity) {
            if mine == theirs {
                return true;
            }
        }
        self.platform == other.platform
            && self.folder == other.folder
            && same_leaf(self.platform, &self.leaf, &other.leaf)
    }
    fn valid(&self) -> bool {
        matches!(
            self.platform,
            external::PLATFORM_WINDOWS | external::PLATFORM_UNIX
        ) && !self.folder.is_empty()
            && self.folder.len() <= external::MAX_FOLDER_BYTES
            && external::valid_leaf(&self.leaf)
            && self.identity.is_none_or(|(folder, file)| {
                folder.platform() == self.platform && file.platform() == self.platform
            })
    }
}

/// Whether two names are the same name on `platform`. Windows compares names without regard to
/// case (this approximates NTFS's own upcase table with Unicode lowercasing, which is enough for a
/// list rule that only ever forgets an entry or decides not to rename).
fn same_leaf(platform: u8, a: &str, b: &str) -> bool {
    if platform == external::PLATFORM_WINDOWS {
        a == b || a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

/// One finished save.
#[derive(Clone, PartialEq, Eq)]
pub struct SavedFile {
    pub id: SavedFileId,
    /// The file's plaintext CID (its address in the server's file index).
    pub cid: Cid,
    /// Plaintext bytes, as published.
    pub size: u64,
    /// Wall-clock milliseconds (catcoms-rt `Clock::now_ms`) when the save published.
    pub saved_at_ms: u64,
    pub location: SavedLocation,
}
impl std::fmt::Debug for SavedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedFile")
            .field("id", &self.id)
            .field("size", &self.size)
            .field("saved_at_ms", &self.saved_at_ms)
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}
impl SavedFile {
    fn valid(&self) -> bool {
        self.size <= super::export::EXPORT_MAX_BYTES && self.location.valid()
    }
    fn encode(&self, e: &mut Encoder) {
        e.put_bytes(self.id.as_bytes()).expect("fixed id");
        e.put_bytes(self.cid.as_bytes()).expect("fixed cid");
        e.put_u64(self.size);
        e.put_u64(self.saved_at_ms);
        e.put_u8(self.location.platform);
        e.put_bytes(&self.location.folder).expect("bounded folder");
        e.put_bytes(self.location.leaf.as_bytes())
            .expect("bounded leaf");
        match self.location.identity {
            None => {
                e.put_u8(0);
            }
            Some((folder, file)) => {
                e.put_u8(1);
                e.put_bytes(&folder.encode()).expect("fixed identity");
                e.put_bytes(&file.encode()).expect("fixed identity");
            }
        }
    }
    fn decode(d: &mut Decoder<'_>) -> Option<Self> {
        let id = SavedFileId(d.get_bytes().ok()?.try_into().ok()?);
        let cid = Cid::from_bytes(d.get_bytes().ok()?.try_into().ok()?);
        let size = d.get_u64().ok()?;
        let saved_at_ms = d.get_u64().ok()?;
        let platform = d.get_u8().ok()?;
        let folder = d.get_bytes().ok()?.to_vec();
        let leaf = std::str::from_utf8(d.get_bytes().ok()?).ok()?.to_owned();
        let identity = match d.get_u8().ok()? {
            0 => None,
            1 => Some((
                FileIdentity::decode(d.get_bytes().ok()?)?,
                FileIdentity::decode(d.get_bytes().ok()?)?,
            )),
            _ => return None,
        };
        let entry = Self {
            id,
            cid,
            size,
            saved_at_ms,
            location: SavedLocation {
                platform,
                folder,
                leaf,
                identity,
            },
        };
        entry.valid().then_some(entry)
    }
}

/// Why a record could not be read. Either way the caller must leave it untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedListError {
    /// A version this build does not know (a newer Mewtual wrote it).
    Version(u8),
    /// Not a saved-files record, or damaged.
    Malformed,
}

/// One server's saved-files list, oldest entry first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFiles {
    owner: Vec<u8>,
    entries: Vec<SavedFile>,
}
impl SavedFiles {
    /// An empty list for `owner` (the group id followed by this device's member id in it).
    pub fn new(owner: &[u8]) -> Self {
        Self {
            owner: owner.to_vec(),
            entries: Vec::new(),
        }
    }
    /// The membership this list belongs to. A list naming another owner must never be applied to
    /// this server (another group, or an earlier membership of the same group).
    pub fn owner(&self) -> &[u8] {
        &self.owner
    }
    /// Entries, oldest first.
    pub fn entries(&self) -> &[SavedFile] {
        &self.entries
    }
    pub fn get(&self, id: SavedFileId) -> Option<&SavedFile> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Record a finished save as the newest entry. An older entry at the same place (see
    /// [`SavedLocation::same_place`]) is removed, then the oldest entries are dropped until the
    /// list fits [`MAX_SAVED_FILES`] and [`MAX_SAVED_LIST_BYTES`]. A single entry always fits: its
    /// fields are bounded far below the byte limit. Returns `false` for an invalid entry or an ID
    /// already in use (nothing changes then).
    pub fn record(&mut self, entry: SavedFile) -> bool {
        if !entry.valid() || self.entries.iter().any(|other| other.id == entry.id) {
            return false;
        }
        self.entries
            .retain(|other| !other.location.same_place(&entry.location));
        self.entries.push(entry);
        while self.entries.len() > MAX_SAVED_FILES || self.encoded_len() > MAX_SAVED_LIST_BYTES {
            self.entries.remove(0);
        }
        true
    }

    /// The copy "Replace my older saved copy" may act on when `new` is recorded: the newest older
    /// entry for the same file, checkable here, in the folder `new` went to (same folder
    /// identity), and not at `new`'s own place. Read before [`Self::record`] under the same lock,
    /// so it is exactly what the list held when this save finished. `Err(true)` when there is no
    /// such entry but the same file has a checkable copy in another folder (the note says so);
    /// `Err(false)` when there is nothing to replace at all.
    pub fn replace_candidate(&self, new: &SavedFile) -> Result<SavedFile, bool> {
        let Some((new_folder, _)) = new.location.identity else {
            return Err(false);
        };
        let mut elsewhere = false;
        for entry in self.entries.iter().rev() {
            if entry.cid != new.cid
                || entry.id == new.id
                || !entry.location.checkable()
                || entry.location.same_place(&new.location)
            {
                continue;
            }
            match entry.location.identity {
                Some((folder, _)) if folder == new_folder => return Ok(entry.clone()),
                _ => elsewhere = true,
            }
        }
        Err(elsewhere)
    }

    /// Forget one entry (Clear, or an old copy a replace removed). Returns whether it was there.
    /// The file itself is never touched.
    pub fn forget(&mut self, id: SavedFileId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        self.entries.len() != before
    }

    /// The entry whose saved file is exactly the one `journal` published (same folder identity
    /// and file identity): the copy a Saved private row stands for.
    pub fn for_journal(&self, journal: &ExportJournal) -> Option<&SavedFile> {
        self.entries
            .iter()
            .find(|entry| entry.location.identity == Some((journal.folder_id, journal.file_id)))
    }

    /// Forget the entry of exactly the file `journal` published, for Clear of a Saved private row.
    /// Other copies of the same file stay listed. Returns whether one was removed.
    pub fn forget_journal(&mut self, journal: &ExportJournal) -> bool {
        match self.for_journal(journal).map(|entry| entry.id) {
            Some(id) => self.forget(id),
            None => false,
        }
    }

    /// Compare-and-swap of an entry's file name, after "Replace my older saved copy" gave the new
    /// copy its old name: only while entry `id` still names `from`. Returns whether it changed.
    pub fn rename(&mut self, id: SavedFileId, from: &str, to: &str) -> bool {
        if !external::valid_leaf(to) {
            return false;
        }
        match self
            .entries
            .iter_mut()
            .find(|entry| entry.id == id && entry.location.leaf == from)
        {
            Some(entry) => {
                entry.location.leaf = to.to_owned();
                true
            }
            None => false,
        }
    }

    fn encoded_len(&self) -> usize {
        self.encode().len()
    }

    /// The version 1 plaintext (see the design document for the layout).
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(DOMAIN).expect("fixed domain");
        e.put_u8(SAVED_LIST_VERSION);
        e.put_bytes(&self.owner).expect("bounded owner");
        e.put_u32(self.entries.len() as u32);
        for entry in &self.entries {
            entry.encode(&mut e);
        }
        e.finish()
    }

    /// Strict decode. Anything other than a canonical version 1 record within every bound is an
    /// error: an unknown version is [`SavedListError::Version`], everything else
    /// [`SavedListError::Malformed`].
    pub fn decode(bytes: &[u8]) -> Result<Self, SavedListError> {
        let bad = |_| SavedListError::Malformed;
        if bytes.len() > MAX_SAVED_LIST_BYTES {
            return Err(SavedListError::Malformed);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes().map_err(bad)? != DOMAIN {
            return Err(SavedListError::Malformed);
        }
        let version = d.get_u8().map_err(bad)?;
        if version != SAVED_LIST_VERSION {
            return Err(SavedListError::Version(version));
        }
        let owner = d.get_bytes().map_err(bad)?.to_vec();
        if owner.is_empty() || owner.len() > MAX_OWNER_BYTES {
            return Err(SavedListError::Malformed);
        }
        let count = d.get_u32().map_err(bad)? as usize;
        if count > MAX_SAVED_FILES {
            return Err(SavedListError::Malformed);
        }
        let mut entries: Vec<SavedFile> = Vec::with_capacity(count);
        for _ in 0..count {
            let entry = SavedFile::decode(&mut d).ok_or(SavedListError::Malformed)?;
            if entries
                .iter()
                .any(|other| other.id == entry.id || other.location.same_place(&entry.location))
            {
                return Err(SavedListError::Malformed);
            }
            entries.push(entry);
        }
        d.finish().map_err(bad)?;
        Ok(Self { owner, entries })
    }
}

/// Read-only check of a recorded file. The folder is held by one handle and must still be the
/// identified one, then the saved name is opened relative to that handle with attributes-only
/// access (no read access, so antivirus scans and other programs' share modes do not delay it)
/// and compared by identity. Nothing is opened for writing, created, renamed or deleted. An entry
/// without identity, from another platform, or in a folder that moved or cannot be reached is
/// `Unknown`.
pub fn saved_file_presence(entry: &SavedFile) -> SavedPresence {
    match locate_saved(entry) {
        Ok(_) => SavedPresence::Ours,
        Err(presence) => presence,
    }
}

/// [`saved_file_presence`] for "Show in folder": the path only while the same read-only check
/// proves the saved name still holds the saved file, else the presence that says why not.
/// Native-only: the caller hands the path to the system file manager, never to the renderer. The
/// check and the reveal are separate steps, so a file swapped in between is shown in its folder
/// but never opened or run (as for PR #51's Saved rows).
pub fn saved_file_location(entry: &SavedFile) -> Result<PathBuf, SavedPresence> {
    locate_saved(entry)
}

fn locate_saved(entry: &SavedFile) -> Result<PathBuf, SavedPresence> {
    if !entry.location.checkable() {
        return Err(SavedPresence::Unknown);
    }
    let Some((folder_id, file_id)) = entry.location.identity else {
        return Err(SavedPresence::Unknown);
    };
    let Some(folder) = entry.location.folder_path() else {
        return Err(SavedPresence::Unknown);
    };
    // Moved, swapped or unreachable: where the file went cannot be told from here.
    let Ok(Some(held)) = external::hold_folder(&folder, Some(folder_id)) else {
        return Err(SavedPresence::Unknown);
    };
    match held.name_state(&entry.location.leaf, file_id) {
        HeldName::Ours => Ok(folder.join(&entry.location.leaf)),
        HeldName::Absent => Err(SavedPresence::Missing),
        HeldName::Other => Err(SavedPresence::Replaced),
        HeldName::Unknown => Err(SavedPresence::Unknown),
    }
}

/// What "Replace my older saved copy" is asked to do: the older copy (an entry read from the list
/// when the new save was recorded), the new copy's location (it must have identities), and the
/// name a one-click save asked for (native's sanitized Downloads name), which decides whether the
/// new copy takes the old name. `None` for a name the user chose in Save as: that name is never
/// renamed (design review M2). The export's own journal cannot tell these apart, because creation
/// numbers the name before the staging file exists.
#[derive(Debug, Clone)]
pub struct ReplaceRequest {
    pub old: SavedFile,
    pub new: SavedLocation,
    pub started_from: Option<String>,
}

/// Why nothing was deleted. Each maps to one note in native.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceRefusal {
    /// One of the copies has no identity (or is from another platform): nothing can be proven.
    NoIdentity,
    /// The old copy is in another folder than the new one.
    OtherFolder,
    /// Something else is at the old name, or the old copy's content is not exactly what Mewtual
    /// saved, or it changed while it was being checked.
    Changed,
    /// The old copy has another name too (a hard link someone made, or Mewtual's own temporary
    /// name of an unfinished tidy-up), so deleting one name would not remove it.
    MoreNames,
    /// The old copy is read-only.
    ReadOnly,
    /// Windows: the old copy is a reparse point of some kind (a cloud placeholder, a deduplicated
    /// or compressed file). Mewtual does not read those through the handle it would delete with.
    Reparse,
    /// Unix: other accounts can (or may) change the folder or the file, so nothing is deleted
    /// automatically there. Reported only for a copy seen to be ours.
    Shared,
    /// The folder at that path is not the identified one any more.
    FolderChanged,
    /// It could not be checked right now (held open by another program, an I/O error).
    Unchecked,
    /// The invocation ended or the deadline passed (`keep_going` answered false).
    Stopped,
}

/// What [`replace_older_copy`] found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceOutcome {
    /// Nothing is at the old name: the old copy is gone. The caller forgets the old entry.
    Missing,
    /// The old name already holds the new copy (the new save took the free name, perhaps in other
    /// letter case on Windows). The caller forgets the old entry.
    AlreadyNew,
    /// The old copy was deleted and the new copy keeps its own name (it started from another
    /// name, for example one the user typed in Save as). The caller forgets the old entry.
    Removed,
    /// The old copy was deleted and the new copy now also has the old name. The caller forgets
    /// the old entry, renames the new entry from its first name to `old_leaf`, and only if that
    /// succeeded calls [`drop_first_name`].
    Linked { old_leaf: String },
    /// The old copy was deleted, but the new copy could not take its name (taken meanwhile, or
    /// the link failed). The caller forgets the old entry and tells the user the new copy's name.
    RemovedNotMoved,
    /// Nothing was deleted.
    Refused(ReplaceRefusal),
    /// Unix: the old copy was moved aside for its last check and could not be put back. It may be
    /// in its folder under a `mewtual-removing-` name. The old entry is kept.
    Stranded,
}

/// "Replace my older saved copy", steps 1 to 4 of the design: hold the folder, prove the old copy
/// (identity, one name, writable, no reparse point, exact content), delete it through the verified
/// object, and when the new save started from the old copy's name give the new copy that name
/// with a no-replace link. Never truncates or overwrites anything, never deletes anything but the
/// proven old copy, and never renames a name the user chose. `keep_going` (the invocation, the
/// server incarnation and a deadline) is asked before every effect and every 1 MiB of hashing.
/// Runs on a blocking worker.
pub fn replace_older_copy(
    request: &ReplaceRequest,
    keep_going: &dyn Fn() -> bool,
    rng: &mut dyn CryptoRngCore,
) -> ReplaceOutcome {
    use ReplaceOutcome::Refused;
    let (Some((old_folder, old_id)), Some((new_folder, new_id))) =
        (request.old.location.identity, request.new.identity)
    else {
        return Refused(ReplaceRefusal::NoIdentity);
    };
    if !request.old.location.checkable() || !request.new.checkable() {
        return Refused(ReplaceRefusal::NoIdentity);
    }
    if old_folder != new_folder {
        return Refused(ReplaceRefusal::OtherFolder);
    }
    // Both names live in the folder with this identity; the new copy's path to it is the one this
    // save just used, so it is the one opened (an older entry may have reached it by another path).
    let Some(folder) = request.new.folder_path() else {
        return Refused(ReplaceRefusal::NoIdentity);
    };
    if !keep_going() {
        return Refused(ReplaceRefusal::Stopped);
    }
    let old_leaf = &request.old.location.leaf;
    let removed = external::remove_unchanged(
        &folder,
        new_folder,
        old_leaf,
        external::UnchangedCopy {
            identity: old_id,
            size: request.old.size,
            cid: request.old.cid,
            newer: new_id,
        },
        keep_going,
        rng,
    );
    match removed {
        Ok(Unchanged::Removed) => {}
        Ok(Unchanged::Missing) => return ReplaceOutcome::Missing,
        Ok(Unchanged::AlreadyNew) => return ReplaceOutcome::AlreadyNew,
        Ok(Unchanged::NotOurs | Unchanged::Changed) => return Refused(ReplaceRefusal::Changed),
        Ok(Unchanged::MoreNames) => return Refused(ReplaceRefusal::MoreNames),
        Ok(Unchanged::ReadOnly) => return Refused(ReplaceRefusal::ReadOnly),
        Ok(Unchanged::Reparse) => return Refused(ReplaceRefusal::Reparse),
        Ok(Unchanged::Shared) => return Refused(ReplaceRefusal::Shared),
        Ok(Unchanged::FolderChanged) => return Refused(ReplaceRefusal::FolderChanged),
        Ok(Unchanged::Stopped) => return Refused(ReplaceRefusal::Stopped),
        Ok(Unchanged::Stranded) => return ReplaceOutcome::Stranded,
        Err(_) => return Refused(ReplaceRefusal::Unchecked),
    }
    // The new copy takes the old name only when this one-click save asked for exactly that name
    // and was numbered because the old copy held it (review M2: a name the user typed is never
    // renamed, and another listed name keeps its own).
    let asked_for_old = request
        .started_from
        .as_deref()
        .is_some_and(|asked| same_leaf(request.new.platform, asked, old_leaf));
    if !asked_for_old || same_leaf(request.new.platform, &request.new.leaf, old_leaf) {
        return ReplaceOutcome::Removed;
    }
    if !keep_going() {
        return ReplaceOutcome::RemovedNotMoved;
    }
    match external::link_name(&folder, new_folder, &request.new.leaf, old_leaf, new_id) {
        Ok(true) => ReplaceOutcome::Linked {
            old_leaf: old_leaf.clone(),
        },
        Ok(false) | Err(_) => ReplaceOutcome::RemovedNotMoved,
    }
}

/// Step 6: remove the new copy's first name `first_leaf` after [`ReplaceOutcome::Linked`] and the
/// list rename, only while it holds the new copy **and `old_leaf` still does too** (review H1):
/// the old name may have been deleted or replaced since the link, and then the first name is the
/// only copy. Returns whether the name was removed; `false` leaves both names (the same file) or
/// the only one.
pub fn drop_first_name(
    new: &SavedLocation,
    first_leaf: &str,
    old_leaf: &str,
    rng: &mut dyn CryptoRngCore,
) -> bool {
    let (Some((folder_id, file_id)), Some(folder)) = (new.identity, new.folder_path()) else {
        return false;
    };
    matches!(
        external::remove_second_name(&folder, folder_id, first_leaf, old_leaf, file_id, rng),
        Ok(Verified::Removed { .. })
    )
}

#[cfg(test)]
#[path = "saved_tests.rs"]
mod tests;
