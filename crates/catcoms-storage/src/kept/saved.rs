//! The per-server saved-files list (PR #56, `docs/design-saved-files-list.md`).
//!
//! A finished save releases the private encrypted copy it was made from, so before this list
//! nothing remembered where a saved file went. The list records each finished save of a shared
//! file: its plaintext CID, where it was saved (folder and name), the saved file's identity when
//! the save was journaled, its size and when it was saved. Native seals the encoded list into
//! `servers/<id>.saved` inside the vault (`catcoms_app::ServerStore`), so this module is only the
//! codec, the bounded list operations and the read-only checks of a recorded file.
//!
//! Invariants this module keeps:
//!
//! - **Records, not copies.** Nothing here creates, changes or deletes an external file. Dropping
//!   an entry (the cap, Clear, a newer save of the same file) only forgets it.
//! - **Bounded.** At most [`MAX_SAVED_FILES`] entries and [`MAX_SAVED_LIST_BYTES`] of plaintext;
//!   a decode refuses anything larger, so a damaged or hostile record cannot make a reader
//!   allocate without bound.
//! - **One entry per file.** CIDs are unique; a newer save of the same file replaces its entry.
//! - **Versioned and non-destructive.** The plaintext starts with a domain string and a version
//!   byte. Anything this reader does not understand decodes as an error, and the caller must then
//!   leave the record exactly as it is (only leaving the server removes it), so a record written
//!   by a newer build survives a round trip through an older one.
//! - **Paths stay native.** Folder paths and identities are redacted from `Debug` output and never
//!   leave native; the renderer gets the entry ID, the CID, the file name and display facts.
use super::export::{name_state, NameState};
use super::external::{self, FileIdentity};
use super::*;

/// Most entries one server's list keeps. It matches the file index's own 256-row cap, so a list
/// can hold one entry for every file a server can list at once; older entries are dropped first.
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
/// Group ids are at most this long, as in the cache setting record.
const MAX_GROUP_BYTES: usize = 256;

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
    fn folder_path(&self) -> Option<PathBuf> {
        external::decode_os(self.platform, &self.folder).map(PathBuf::from)
    }
    /// Whether the saved folder is exactly `folder`, compared as recorded (no filesystem access),
    /// the same rule as [`ExportJournal::is_in_folder`]: a moved Downloads folder, other casing or
    /// a link answers "no". A location from another platform answers "no".
    pub fn is_in_folder(&self, folder: &Path) -> bool {
        self.folder_path().is_some_and(|own| own == folder)
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
    group: Vec<u8>,
    entries: Vec<SavedFile>,
}
impl SavedFiles {
    /// An empty list for `group`.
    pub fn new(group: &[u8]) -> Self {
        Self {
            group: group.to_vec(),
            entries: Vec::new(),
        }
    }
    /// The group this list belongs to. A list naming another group must never be applied to this
    /// server (a stale record under a reused server number).
    pub fn group(&self) -> &[u8] {
        &self.group
    }
    /// Entries, oldest first.
    pub fn entries(&self) -> &[SavedFile] {
        &self.entries
    }
    pub fn get(&self, id: SavedFileId) -> Option<&SavedFile> {
        self.entries.iter().find(|entry| entry.id == id)
    }
    pub fn by_cid(&self, cid: &Cid) -> Option<&SavedFile> {
        self.entries.iter().find(|entry| entry.cid == *cid)
    }

    /// Record a finished save. The existing entry for the same file (if any) is removed and
    /// returned, the new entry is appended as the newest, and the oldest entries are dropped until
    /// the list fits [`MAX_SAVED_FILES`] and [`MAX_SAVED_LIST_BYTES`]. A single entry always fits:
    /// its fields are bounded far below the byte limit. Returns `None` for an invalid entry or an
    /// ID already in use (nothing changes then).
    pub fn record(&mut self, entry: SavedFile) -> Option<Option<SavedFile>> {
        if !entry.valid()
            || self
                .entries
                .iter()
                .any(|other| other.id == entry.id && other.cid != entry.cid)
        {
            return None;
        }
        let previous = self
            .entries
            .iter()
            .position(|other| other.cid == entry.cid)
            .map(|at| self.entries.remove(at));
        self.entries.push(entry);
        while self.entries.len() > MAX_SAVED_FILES || self.encoded_len() > MAX_SAVED_LIST_BYTES {
            self.entries.remove(0);
        }
        Some(previous)
    }

    /// Forget one entry (Clear). Returns whether it was there. The file itself is never touched.
    pub fn forget(&mut self, id: SavedFileId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        self.entries.len() != before
    }

    /// Forget the entry whose saved file is exactly the one `journal` published (same folder
    /// identity and file identity), for Clear of a Saved private row. A different copy of the same
    /// file stays listed. Returns whether one was removed.
    pub fn forget_journal(&mut self, journal: &ExportJournal) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|entry| entry.location.identity != Some((journal.folder_id, journal.file_id)));
        self.entries.len() != before
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
        e.put_bytes(&self.group).expect("bounded group");
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
        let group = d.get_bytes().map_err(bad)?.to_vec();
        if group.is_empty() || group.len() > MAX_GROUP_BYTES {
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
                .any(|other| other.id == entry.id || other.cid == entry.cid)
            {
                return Err(SavedListError::Malformed);
            }
            entries.push(entry);
        }
        d.finish().map_err(bad)?;
        Ok(Self { group, entries })
    }
}

/// Read-only check of a recorded file: the folder must still be the identified one, then the
/// saved name is opened without following a link and compared by identity. Nothing is opened for
/// writing, created, renamed or deleted. An entry without identity, from another platform, or in a
/// folder that moved or cannot be reached is `Unknown`.
pub fn saved_file_presence(entry: &SavedFile) -> SavedPresence {
    match locate_saved(entry) {
        Ok(_) => SavedPresence::Ours,
        Err(presence) => presence,
    }
}

/// [`saved_file_presence`] for "Show in folder": the path only while the same read-only check
/// proves the saved name still holds the saved file, else the presence that says why not.
/// Native-only: the caller hands the path to the system file manager, never to the renderer.
pub fn saved_file_location(entry: &SavedFile) -> Result<PathBuf, SavedPresence> {
    locate_saved(entry)
}

fn locate_saved(entry: &SavedFile) -> Result<PathBuf, SavedPresence> {
    let Some((folder_id, file_id)) = entry.location.identity else {
        return Err(SavedPresence::Unknown);
    };
    let Some(folder) = entry.location.folder_path() else {
        return Err(SavedPresence::Unknown);
    };
    match external::folder_identity(&folder) {
        Ok(id) if id == folder_id => {}
        // Moved, swapped or unreachable: where the file went cannot be told from here.
        _ => return Err(SavedPresence::Unknown),
    }
    let path = folder.join(&entry.location.leaf);
    match name_state(&path, file_id) {
        NameState::Ours => Ok(path),
        NameState::Absent => Err(SavedPresence::Missing),
        NameState::Other => Err(SavedPresence::Replaced),
        NameState::Unknown => Err(SavedPresence::Unknown),
    }
}

#[cfg(test)]
#[path = "saved_tests.rs"]
mod tests;
