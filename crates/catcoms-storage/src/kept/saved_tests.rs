//! The saved-files list: codec, bounds, version handling, the read-only checks of a recorded file,
//! and the "Replace my older saved copy" sequence at every interleaving, against real files
//! (PR #56, `docs/design-saved-files-list.md`).
#[cfg(unix)]
use super::super::external::AFTER_ASIDE;
#[cfg(windows)]
use super::super::external::BEFORE_REMOVE;
use super::super::external::{BEFORE_LINK, BEFORE_REPLACE_DELETE};
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::cell::Cell;
use std::fs;
use std::rc::Rc;

const OWNER: [u8; 64] = [3; 64];
const CONTENT: [&[u8]; 2] = [b"saved file, first part", b" and its second part"];

/// Recording at the default setting, which most rules here are about.
trait RecordDefault {
    fn record_default(&mut self, entry: SavedFile) -> bool;
}
impl RecordDefault for SavedFiles {
    fn record_default(&mut self, entry: SavedFile) -> bool {
        self.record(entry, SavedLimit::DEFAULT)
    }
}

/// Entry `n` (its own ID, CID, time and name), optionally with a folder of `folder_bytes` bytes.
fn numbered(n: u32, folder_bytes: Option<usize>) -> SavedFile {
    let mut location = plain_location(&format!("f{n}.txt"));
    if let Some(bytes) = folder_bytes {
        location.folder = vec![b'a'; bytes];
    }
    let mut id = [0u8; 16];
    id[..4].copy_from_slice(&n.to_be_bytes());
    SavedFile {
        id: SavedFileId::from_bytes(id),
        cid: Cid::of(&n.to_be_bytes()),
        size: 1,
        saved_at_ms: u64::from(n),
        location,
    }
}

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(56)
}

/// A real journaled save of `chunks` as `name` in `dir`: the export's own create, write, link and
/// staging removal, without a store (those are the external effects the list records). A taken
/// `name` is numbered, exactly as a second save of the same file is.
pub(in crate::kept) fn published(
    dir: &Path,
    name: &str,
    chunks: &[&[u8]],
) -> (PathBuf, ExportJournal) {
    let plan = ExportPlan::for_test(chunks);
    let mut file = ExportFile::create(dir, name, plan, &mut rng()).unwrap();
    for (index, chunk) in chunks.iter().enumerate() {
        file.accept(index, chunk).unwrap();
    }
    let Publication::Ready(journal) = file.prepare_publication().unwrap() else {
        panic!("not yet published");
    };
    file.checkpointed(journal);
    let path = file.link().unwrap();
    let journal = file.published_journal();
    assert_eq!(file.finish(&mut rng()).unwrap(), PartialRemoval::KeptSaved);
    (path, journal)
}

fn entry(id: u8, cid: &[u8], location: SavedLocation) -> SavedFile {
    SavedFile {
        id: SavedFileId::from_bytes([id; 16]),
        cid: Cid::of(cid),
        size: 5,
        saved_at_ms: 1_700_000_000_000 + u64::from(id),
        location,
    }
}

fn plain_location(leaf: &str) -> SavedLocation {
    SavedLocation::unidentified(&std::env::temp_dir().join(leaf)).unwrap()
}

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn downloads(root: &tempfile::TempDir) -> PathBuf {
    let dir = root.path().join("Downloads");
    // 0755 whatever the umask, as the export tests do (Unix shared-folder rule).
    create_folder(&dir).unwrap();
    dir
}

/// The identity of the object at `path`, read through a handle.
fn identity_at(path: &Path) -> FileIdentity {
    let file = fs::File::open(path).unwrap();
    external::observe(&file).unwrap().identity
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// ---- codec and list rules ----

#[test]
fn a_list_round_trips_with_and_without_identities() {
    let root = tempdir();
    let dir = downloads(&root);
    let (_, journal) = published(&dir, "report.pdf", &[b"hello"]);
    let mut list = SavedFiles::new(&OWNER);
    assert!(list.record_default(entry(1, b"a", SavedLocation::from_journal(&journal))));
    assert!(list.record_default(entry(2, b"b", plain_location("plain.txt"))));
    let bytes = list.encode();
    let decoded = SavedFiles::decode(&bytes).unwrap();
    assert_eq!(decoded, list);
    assert_eq!(decoded.owner(), &OWNER);
    assert!(decoded.entries()[0].location.identified());
    assert!(decoded.entries()[0].location.checkable());
    assert!(!decoded.entries()[1].location.identified());
    assert_eq!(decoded.entries()[0].location.leaf(), "report.pdf");
    assert_eq!(decoded.encode(), bytes, "canonical");
    let empty = SavedFiles::new(&OWNER);
    assert_eq!(SavedFiles::decode(&empty.encode()).unwrap(), empty);
}

#[test]
fn debug_output_never_shows_a_path_or_name() {
    let location = plain_location("secret-name.txt");
    let file = entry(1, b"a", location.clone());
    for text in [format!("{location:?}"), format!("{file:?}")] {
        assert!(!text.contains("secret-name"), "{text}");
        let temp = std::env::temp_dir();
        assert!(!text.contains(temp.to_string_lossy().as_ref()), "{text}");
    }
}

#[test]
fn each_saved_copy_has_its_own_entry_and_a_place_holds_one() {
    // Design review M1: two saves of one file are two copies, so both stay listed.
    let mut list = SavedFiles::new(&OWNER);
    assert!(list.record_default(entry(1, b"a", plain_location("a.txt"))));
    assert!(list.record_default(entry(2, b"a", plain_location("a (1).txt"))));
    assert_eq!(list.entries().len(), 2);
    // A newer save at the same place (the user deleted a.txt, a later save took the name) means
    // the older entry's file is not there: it is replaced, never listed twice.
    assert!(list.record_default(entry(3, b"a", plain_location("a.txt"))));
    let ids: Vec<_> = list.entries().iter().map(|e| e.id.as_bytes()[0]).collect();
    assert_eq!(ids, [2, 3]);
    // Windows names ignore case, so a Windows entry at "A.TXT" is the same place.
    if external::CURRENT_PLATFORM == external::PLATFORM_WINDOWS {
        assert!(list.record_default(entry(4, b"b", plain_location("A.TXT"))));
        let ids: Vec<_> = list.entries().iter().map(|e| e.id.as_bytes()[0]).collect();
        assert_eq!(ids, [2, 4]);
    }
    // An ID already in use is refused, changing nothing.
    let before = list.clone();
    assert!(!list.record_default(entry(2, b"c", plain_location("c.txt"))));
    assert_eq!(list, before);
}

#[test]
fn the_same_file_identity_is_one_place_even_under_another_name() {
    let root = tempdir();
    let dir = downloads(&root);
    let (_, journal) = published(&dir, "report.pdf", &[b"hello"]);
    let mut list = SavedFiles::new(&OWNER);
    assert!(list.record_default(entry(1, b"a", SavedLocation::from_journal(&journal))));
    // "Replace my older saved copy" renames entries; a second entry for the same object (under
    // another name) can never exist.
    let mut renamed = SavedLocation::from_journal(&journal);
    renamed.leaf = "other.pdf".into();
    assert!(list.record_default(entry(2, b"a", renamed)));
    assert_eq!(list.entries().len(), 1);
    assert_eq!(list.entries()[0].id, SavedFileId::from_bytes([2; 16]));
}

#[test]
fn the_default_list_keeps_256_entries_dropping_the_oldest() {
    assert_eq!(SavedLimit::DEFAULT.get(), 256);
    let mut list = SavedFiles::new(&OWNER);
    for n in 0..266 {
        assert!(list.record_default(numbered(n, None)));
    }
    assert_eq!(list.entries().len(), 256);
    assert_eq!(list.entries()[0].saved_at_ms, 10, "the ten oldest went");
    assert_eq!(list.entries().last().unwrap().saved_at_ms, 265);
    let bytes = list.encode();
    assert_eq!(
        bytes[4 + b"mewtual/saved-files".len()],
        1,
        "a default list stays version 1"
    );
    assert!(SavedFiles::decode(&bytes).is_ok());
}

#[test]
fn the_setting_is_clamped_into_16_to_4096() {
    for (asked, kept) in [
        (0, 16),
        (15, 16),
        (16, 16),
        (300, 300),
        (4096, 4096),
        (4097, 4096),
        (u64::MAX, 4096),
    ] {
        assert_eq!(SavedLimit::clamped(asked).get(), kept, "{asked}");
    }
    assert_eq!(SavedLimit::LARGEST.get(), SavedLimit::MAX);
    // The byte bound scales with the setting at 4 KiB per entry, and a reader never reads less
    // than version 1's 1 MiB or more than 16 MiB.
    assert_eq!(SavedLimit::DEFAULT.max_bytes(), 1024 * 1024);
    assert_eq!(SavedLimit::clamped(16).max_bytes(), 64 * 1024);
    assert_eq!(SavedLimit::clamped(16).read_bound(), 1024 * 1024);
    assert_eq!(SavedLimit::LARGEST.read_bound(), 16 * 1024 * 1024);
}

#[test]
fn lowering_the_setting_forgets_the_oldest_entries_and_raising_keeps_all() {
    let mut list = SavedFiles::new(&OWNER);
    for n in 0..100 {
        assert!(list.record_default(numbered(n, None)));
    }
    // Raising keeps every entry, and a raised list grows past the default.
    let larger = SavedLimit::clamped(4096);
    assert_eq!(list.trimmed_by(larger), 0);
    for n in 100..300 {
        assert!(list.record(numbered(n, None), larger));
    }
    assert_eq!(list.entries().len(), 300);
    assert_eq!(list.entries()[0].saved_at_ms, 0, "nothing was forgotten");
    // Lowering forgets the oldest: the preview and the trim agree, and the newest stay.
    let smaller = SavedLimit::clamped(16);
    assert_eq!(list.trimmed_by(smaller), 284);
    let before = list.clone();
    assert_eq!(list.trim(smaller), 284);
    assert_eq!(list.entries(), &before.entries()[284..]);
    assert_eq!(list.entries()[0].saved_at_ms, 284);
    // Already within the setting: nothing more goes.
    assert_eq!(list.trim(smaller), 0);
}

#[test]
fn very_long_folders_are_capped_by_bytes_not_count() {
    let long = |n| numbered(n, Some(super::external::MAX_FOLDER_BYTES));
    let mut list = SavedFiles::new(&OWNER);
    for n in 0..200 {
        assert!(list.record_default(long(n)));
    }
    assert!(list.entries().len() < 100, "{}", list.entries().len());
    assert!(list.encode().len() <= SavedLimit::DEFAULT.max_bytes());
    assert_eq!(list.entries().last().unwrap().saved_at_ms, 199);
    assert!(SavedFiles::decode(&list.encode()).is_ok());
    // Even the smallest setting keeps one maximal entry (and the newest).
    let mut list = SavedFiles::new(&OWNER);
    for n in 0..20 {
        assert!(list.record(long(n), SavedLimit::clamped(16)));
    }
    assert!(!list.entries().is_empty());
    assert_eq!(list.entries().last().unwrap().saved_at_ms, 19);
    assert!(list.encode().len() <= SavedLimit::clamped(16).max_bytes());
}

#[test]
fn the_largest_setting_stays_within_16_mib_and_writes_version_2() {
    // Long paths at the largest setting: the byte bound, not the count, caps the list, and the
    // record never passes 16 MiB, the most any reader reads.
    // (Built directly and trimmed once: recording thousands of saves one by one re-measures the
    // whole list each time, which is right for one save but slow for a test.)
    let largest = SavedLimit::LARGEST;
    let mut list = SavedFiles::new(&OWNER);
    list.entries = (0..1099)
        .map(|n| numbered(n, Some(super::external::MAX_FOLDER_BYTES)))
        .collect();
    list.trim(largest);
    assert!(list.record(
        numbered(1099, Some(super::external::MAX_FOLDER_BYTES)),
        largest
    ));
    let bytes = list.encode();
    assert!(bytes.len() <= largest.max_bytes(), "{}", bytes.len());
    assert!(bytes.len() > 1024 * 1024, "past version 1's bound");
    assert!(list.entries().len() < 1100);
    assert_eq!(list.entries().last().unwrap().saved_at_ms, 1099);
    assert_eq!(bytes[4 + b"mewtual/saved-files".len()], SAVED_LIST_VERSION);
    assert_eq!(SavedFiles::decode(&bytes).unwrap(), list);
    // Many short entries: the count caps it at exactly 4096.
    let mut list = SavedFiles::new(&OWNER);
    list.entries = (0..4099).map(|n| numbered(n, None)).collect();
    assert!(list.record(numbered(4099, None), largest));
    assert_eq!(list.entries().len(), 4096);
    assert_eq!(list.entries()[0].saved_at_ms, 4, "the four oldest went");
    let bytes = list.encode();
    assert_eq!(bytes[4 + b"mewtual/saved-files".len()], SAVED_LIST_VERSION);
    assert_eq!(SavedFiles::decode(&bytes).unwrap(), list);
}

#[test]
fn a_full_list_in_one_windows_folder_decodes_in_one_pass() {
    // Adversarial review of the setting, HIGH-1: one-click saves all go to Downloads, so a full
    // list is 4096 entries in one folder, and on Windows every pairwise comparison lowercased both
    // names. Comparing every pair (about 8.4 million at 4096) ran for seconds under the store and
    // registry locks on every save; the one-pass check is linear.
    use catcoms_rt::Clock;
    let mut list = SavedFiles::new(&OWNER);
    list.entries = (0..4096)
        .map(|n| {
            let mut entry = numbered(n, None);
            entry.location.platform = external::PLATFORM_WINDOWS;
            entry
        })
        .collect();
    let bytes = list.encode();
    let clock = catcoms_rt::SystemClock;
    let started = clock.monotonic_ms();
    assert_eq!(SavedFiles::decode(&bytes).unwrap(), list);
    let took = clock.monotonic_ms() - started;
    // About 10 ms here in a debug build; the pairwise check took about 1.6 s.
    assert!(took < 500, "decoding a full list took {took} ms");
    // The same rule as before: a name differing only in case is the same place on Windows (not
    // on Unix), and the same file identity under another name is too.
    let mut twice = list.clone();
    let identity = Some((
        FileIdentity::for_test(external::PLATFORM_WINDOWS, 1, 1),
        FileIdentity::for_test(external::PLATFORM_WINDOWS, 1, 2),
    ));
    twice.entries[2].location.identity = identity;
    assert!(SavedFiles::decode(&twice.encode()).is_ok());
    twice.entries[3].location.identity = identity;
    assert_eq!(
        SavedFiles::decode(&twice.encode()),
        Err(SavedListError::Malformed)
    );
    let mut cased = list.clone();
    cased.entries[1].location.leaf = cased.entries[0].location.leaf.to_uppercase();
    assert_eq!(
        SavedFiles::decode(&cased.encode()),
        Err(SavedListError::Malformed)
    );
    for entry in &mut cased.entries {
        entry.location.platform = external::PLATFORM_UNIX;
    }
    assert!(SavedFiles::decode(&cased.encode()).is_ok());
}

#[test]
fn version_1_is_written_within_its_bounds_and_neither_version_reads_past_its_own() {
    let mut list = SavedFiles::new(&OWNER);
    for n in 0..257 {
        assert!(list.record(numbered(n, None), SavedLimit::LARGEST));
    }
    let version_at = 4 + b"mewtual/saved-files".len();
    let bytes = list.encode();
    assert_eq!(bytes[version_at], 2, "257 entries need version 2");
    // The same 257 entries labelled version 1 exceed what version 1 may hold.
    let mut as_v1 = bytes.clone();
    as_v1[version_at] = 1;
    assert_eq!(SavedFiles::decode(&as_v1), Err(SavedListError::Malformed));
    // Version 2 holds at most 4096 entries.
    let count_at = version_at + 1 + 4 + OWNER.len();
    let mut over = bytes.clone();
    over[count_at..count_at + 4].copy_from_slice(&4097u32.to_be_bytes());
    assert_eq!(SavedFiles::decode(&over), Err(SavedListError::Malformed));
    // One fewer entry fits version 1 again.
    list.trim(SavedLimit::DEFAULT);
    assert_eq!(list.encode()[version_at], 1);
    // The preview's length arithmetic matches the encoding exactly.
    assert_eq!(
        list.header_len()
            + list
                .entries()
                .iter()
                .map(SavedFile::encoded_len)
                .sum::<usize>(),
        list.encode().len()
    );
}

#[test]
fn the_setting_record_round_trips_and_refuses_anything_else() {
    let limit = SavedLimit::clamped(1024);
    let bytes = limit.encode_for(&OWNER);
    assert_eq!(SavedLimit::decode(&bytes), Ok((OWNER.to_vec(), limit)));
    let version_at = 4 + b"mewtual/saved-files-limit".len();
    let mut newer = bytes.clone();
    newer[version_at] = 2;
    assert_eq!(SavedLimit::decode(&newer), Err(SavedListError::Version(2)));
    // Out of range (a record is never clamped on read), trailing bytes, a list's plaintext.
    let limit_at = bytes.len() - 4;
    for value in [0u32, 15, 4097] {
        let mut out = bytes.clone();
        out[limit_at..].copy_from_slice(&value.to_be_bytes());
        assert_eq!(
            SavedLimit::decode(&out),
            Err(SavedListError::Malformed),
            "{value}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        SavedLimit::decode(&trailing),
        Err(SavedListError::Malformed)
    );
    assert_eq!(
        SavedLimit::decode(&SavedFiles::new(&OWNER).encode()),
        Err(SavedListError::Malformed)
    );
    assert_eq!(
        SavedFiles::decode(&bytes),
        Err(SavedListError::Malformed),
        "and the other way round"
    );
}

#[test]
fn an_unknown_version_or_a_foreign_record_is_refused_not_misread() {
    let mut list = SavedFiles::new(&OWNER);
    list.record_default(entry(1, b"a", plain_location("a.txt")));
    let bytes = list.encode();
    let version_at = 4 + b"mewtual/saved-files".len();
    assert_eq!(bytes[version_at], 1);
    let mut newer = bytes.clone();
    newer[version_at] = 3;
    assert_eq!(SavedFiles::decode(&newer), Err(SavedListError::Version(3)));
    // Another sealed record's plaintext (the cache setting's shape) is not a list.
    let mut other = catcoms_wire::Encoder::new();
    other.put_u8(1);
    other.put_bytes(&OWNER).unwrap();
    other.put_u8(0).put_u8(1);
    assert_eq!(
        SavedFiles::decode(&other.finish()),
        Err(SavedListError::Malformed)
    );
    let mut domain = bytes.clone();
    domain[4] ^= 1;
    assert_eq!(SavedFiles::decode(&domain), Err(SavedListError::Malformed));
}

#[test]
fn every_malformed_shape_is_refused() {
    let mut list = SavedFiles::new(&OWNER);
    list.record_default(entry(1, b"a", plain_location("a.txt")));
    list.record_default(entry(2, b"b", plain_location("b.txt")));
    let good = list.encode();
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(
        SavedFiles::decode(&trailing),
        Err(SavedListError::Malformed)
    );
    for cut in [1, good.len() / 2, good.len() - 1] {
        assert_eq!(
            SavedFiles::decode(&good[..cut]),
            Err(SavedListError::Malformed),
            "cut at {cut}"
        );
    }
    // A duplicate ID, and two entries at one place.
    for (id, leaf) in [(1u8, "c.txt"), (3, "a.txt")] {
        let mut duplicate = SavedFiles::new(&OWNER);
        duplicate.entries = vec![
            entry(1, b"a", plain_location("a.txt")),
            entry(id, b"c", plain_location(leaf)),
        ];
        assert_eq!(
            SavedFiles::decode(&duplicate.encode()),
            Err(SavedListError::Malformed)
        );
    }
    let encode = |edit: &dyn Fn(&mut SavedFiles)| {
        let mut list = SavedFiles::new(&OWNER);
        list.entries = vec![entry(1, b"a", plain_location("a.txt"))];
        edit(&mut list);
        list.encode()
    };
    let mut over = good.clone();
    let count_at = 4 + b"mewtual/saved-files".len() + 1 + 4 + OWNER.len();
    over[count_at..count_at + 4].copy_from_slice(&257u32.to_be_bytes());
    assert_eq!(SavedFiles::decode(&over), Err(SavedListError::Malformed));
    for bytes in [
        encode(&|list| list.owner.clear()),
        encode(&|list| list.owner = vec![1; 513]),
        encode(&|list| list.entries[0].location.leaf = "../escape".into()),
        encode(&|list| list.entries[0].location.leaf = "NUL.txt".into()),
        encode(&|list| list.entries[0].size = super::super::export::EXPORT_MAX_BYTES + 1),
        encode(&|list| list.entries[0].location.platform = 9),
        encode(&|list| list.entries[0].location.folder.clear()),
        encode(&|list| {
            list.entries[0].location.identity = Some((
                FileIdentity::for_test(external::PLATFORM_WINDOWS, 1, 1),
                FileIdentity::for_test(external::PLATFORM_UNIX, 1, 2),
            ));
        }),
    ] {
        assert_eq!(SavedFiles::decode(&bytes), Err(SavedListError::Malformed));
    }
    assert_eq!(
        SavedFiles::decode(&vec![0; 16 * 1024 * 1024 + 1]),
        Err(SavedListError::Malformed)
    );
}

#[test]
fn forget_rename_and_journal_lookups_touch_only_the_named_entry() {
    let root = tempdir();
    let dir = downloads(&root);
    let (_, first) = published(&dir, "first.txt", &[b"one"]);
    let (_, second) = published(&dir, "second.txt", &[b"two"]);
    let mut list = SavedFiles::new(&OWNER);
    list.record_default(entry(1, b"a", SavedLocation::from_journal(&first)));
    list.record_default(entry(2, b"b", SavedLocation::from_journal(&second)));
    let id = SavedFileId::from_bytes([1; 16]);
    assert!(!list.rename(id, "wrong.txt", "first (2).txt"));
    assert!(!list.rename(id, "first.txt", "bad/name"));
    assert!(list.rename(id, "first.txt", "first (2).txt"));
    assert_eq!(list.get(id).unwrap().location.leaf(), "first (2).txt");
    assert_eq!(
        list.for_journal(&second).map(|e| e.id),
        Some(SavedFileId::from_bytes([2; 16]))
    );
    assert!(list.forget_journal(&second));
    assert!(!list.forget_journal(&second));
    assert!(list.forget(id));
    assert!(!list.forget(id));
    assert!(list.entries().is_empty());
    assert_eq!(fs::read(dir.join("first.txt")).unwrap(), b"one");
    assert_eq!(fs::read(dir.join("second.txt")).unwrap(), b"two");
}

#[test]
fn the_replace_candidate_is_the_newest_older_copy_in_the_same_folder() {
    let root = tempdir();
    let dir = downloads(&root);
    let elsewhere = root.path().join("Elsewhere");
    create_folder(&elsewhere).unwrap();
    let (_, older) = published(&dir, "report.pdf", &CONTENT);
    let (_, newer_here) = published(&dir, "report.pdf", &CONTENT);
    let (_, there) = published(&elsewhere, "report.pdf", &CONTENT);
    let (_, new) = published(&dir, "report.pdf", &CONTENT);
    let new_entry = entry(9, b"file", SavedLocation::from_journal(&new));
    let mut list = SavedFiles::new(&OWNER);
    list.record_default(entry(1, b"file", SavedLocation::from_journal(&older)));
    list.record_default(entry(2, b"file", SavedLocation::from_journal(&newer_here)));
    list.record_default(entry(3, b"file", SavedLocation::from_journal(&there)));
    list.record_default(entry(4, b"other", plain_location("x.txt")));
    assert_eq!(
        list.replace_candidate(&new_entry).map(|e| e.id),
        Ok(SavedFileId::from_bytes([2; 16])),
        "the newest copy in the same folder, never one elsewhere"
    );
    // Only a copy elsewhere: no candidate, and the note can say why.
    let mut only_elsewhere = SavedFiles::new(&OWNER);
    only_elsewhere.record_default(entry(3, b"file", SavedLocation::from_journal(&there)));
    assert_eq!(
        only_elsewhere.replace_candidate(&new_entry),
        Err(NoCandidate::Elsewhere)
    );
    // An older copy that cannot be proven (no identity on either side).
    let mut plain = SavedFiles::new(&OWNER);
    plain.record_default(entry(5, b"file", plain_location("report.pdf")));
    assert_eq!(
        plain.replace_candidate(&new_entry),
        Err(NoCandidate::Unprovable)
    );
    let unidentified_new = entry(9, b"file", plain_location("report (3).pdf"));
    assert_eq!(
        list.replace_candidate(&unidentified_new),
        Err(NoCandidate::Unprovable)
    );
    // Nothing of this file listed (another file only, or nothing at all): nothing to say.
    let mut others = SavedFiles::new(&OWNER);
    others.record_default(entry(4, b"other", plain_location("x.txt")));
    assert_eq!(
        others.replace_candidate(&new_entry),
        Err(NoCandidate::NoneSaved)
    );
    // The new entry's own place is never its own candidate (the name already holds it).
    let mut same = SavedFiles::new(&OWNER);
    same.record_default(entry(6, b"file", SavedLocation::from_journal(&new)));
    assert_eq!(
        same.replace_candidate(&new_entry),
        Err(NoCandidate::NoneSaved)
    );
}

#[test]
fn entry_ids_parse_only_in_their_own_form() {
    let id = SavedFileId::from_bytes([0xab; 16]);
    assert_eq!(SavedFileId::from_hex(&id.to_hex()), Some(id));
    for bad in [
        "",
        "ab",
        &"AB".repeat(16),
        &"ab".repeat(17),
        &format!("{}zz", "ab".repeat(15)),
    ] {
        assert_eq!(SavedFileId::from_hex(bad), None, "{bad}");
    }
    let mut rng = rng();
    assert_ne!(SavedFileId::fresh(&mut rng), SavedFileId::fresh(&mut rng));
}

// ---- read-only checks ----

#[test]
fn a_recorded_file_is_checked_read_only_by_identity() {
    let root = tempdir();
    let dir = downloads(&root);
    let (path, journal) = published(&dir, "report.pdf", &[b"hello"]);
    let saved = entry(1, b"hello", SavedLocation::from_journal(&journal));
    let before = listing(&dir);
    assert_eq!(saved_file_presence(&saved), SavedPresence::Ours);
    assert_eq!(saved_file_location(&saved), Ok(path.clone()));
    assert_eq!(listing(&dir), before, "nothing changed");
    assert!(saved.location.is_in_folder(&dir));
    // Replaced by an editor's atomic save.
    fs::rename(&path, dir.join("original")).unwrap();
    fs::write(&path, b"edited").unwrap();
    assert_eq!(saved_file_presence(&saved), SavedPresence::Replaced);
    assert_eq!(saved_file_location(&saved), Err(SavedPresence::Replaced));
    assert_eq!(fs::read(&path).unwrap(), b"edited");
    // Missing.
    fs::remove_file(&path).unwrap();
    assert_eq!(saved_file_presence(&saved), SavedPresence::Missing);
    assert!(!path.exists(), "nothing was created");
    // The folder moved: unknown, never "missing".
    let moved = dir.with_file_name("Moved");
    fs::rename(&dir, &moved).unwrap();
    assert_eq!(saved_file_presence(&saved), SavedPresence::Unknown);
    assert!(!saved.location.is_in_folder(&moved));
    fs::rename(&moved, &dir).unwrap();
    // A folder put at the saved name is never ours.
    fs::create_dir(&path).unwrap();
    assert_eq!(saved_file_presence(&saved), SavedPresence::Replaced);
    fs::remove_dir(&path).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.join("original"), &path).unwrap();
        assert_eq!(saved_file_presence(&saved), SavedPresence::Replaced);
    }
}

#[cfg(windows)]
#[test]
fn a_check_is_not_delayed_by_another_programs_share_mode() {
    // Design review L6: the check opens with attributes-only access, which takes part in no
    // share-mode check, so a program holding the saved file without sharing cannot make it read
    // as "could not check".
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempdir();
    let dir = downloads(&root);
    let (path, journal) = published(&dir, "report.pdf", &[b"hello"]);
    let _held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let saved = entry(1, b"hello", SavedLocation::from_journal(&journal));
    assert_eq!(saved_file_presence(&saved), SavedPresence::Ours);
}

#[test]
fn an_entry_without_identity_cannot_be_checked() {
    let root = tempdir();
    let dir = downloads(&root);
    let path = dir.join("plain.txt");
    fs::write(&path, b"saved by the writer without a journal").unwrap();
    let saved = entry(1, b"x", SavedLocation::unidentified(&path).unwrap());
    assert!(!saved.location.identified() && !saved.location.checkable());
    assert!(saved.location.is_in_folder(&dir));
    assert_eq!(saved_file_presence(&saved), SavedPresence::Unknown);
    assert_eq!(saved_file_location(&saved), Err(SavedPresence::Unknown));
    assert!(SavedLocation::unidentified(Path::new("plain.txt")).is_none());
    assert!(SavedLocation::unidentified(Path::new("/")).is_none());
    assert!(SavedLocation::unidentified(&dir.join("bad:name")).is_none());
}

#[test]
fn an_entry_from_another_platform_is_unknown_not_reinterpreted() {
    let root = tempdir();
    let dir = downloads(&root);
    let (_, journal) = published(&dir, "report.pdf", &[b"hello"]);
    let mut location = SavedLocation::from_journal(&journal);
    let other = if external::CURRENT_PLATFORM == external::PLATFORM_WINDOWS {
        external::PLATFORM_UNIX
    } else {
        external::PLATFORM_WINDOWS
    };
    location.platform = other;
    location.identity = Some((
        FileIdentity::for_test(other, 1, 1),
        FileIdentity::for_test(other, 1, 2),
    ));
    assert!(location.identified() && !location.checkable());
    let saved = entry(1, b"hello", location);
    assert_eq!(saved_file_presence(&saved), SavedPresence::Unknown);
    assert!(!saved.location.is_in_folder(&dir));
}

// ---- "Replace my older saved copy" ----

/// The old save ("report.pdf") and a second save of the same file, numbered "report (1).pdf"
/// because the old copy holds the name, as a replace request.
struct Replace {
    _root: tempfile::TempDir,
    dir: PathBuf,
    old_path: PathBuf,
    new_path: PathBuf,
    request: ReplaceRequest,
}
fn second_save(name: &str) -> Replace {
    let root = tempdir();
    let dir = downloads(&root);
    let (old_path, old) = published(&dir, "report.pdf", &CONTENT);
    let (new_path, new) = published(&dir, name, &CONTENT);
    let content = CONTENT.concat();
    let request = ReplaceRequest {
        old: SavedFile {
            id: SavedFileId::from_bytes([1; 16]),
            cid: Cid::of(&content),
            size: content.len() as u64,
            saved_at_ms: 1,
            location: SavedLocation::from_journal(&old),
        },
        new: SavedLocation::from_journal(&new),
        // A one-click save asks for the listed name; creation numbers it when it is taken.
        started_from: Some(name.to_owned()),
    };
    Replace {
        _root: root,
        dir,
        old_path,
        new_path,
        request,
    }
}
fn go() -> bool {
    true
}

#[test]
fn a_second_save_takes_the_old_name_and_drops_its_first_name() {
    let fx = second_save("report.pdf");
    assert_eq!(fx.new_path.file_name().unwrap(), "report (1).pdf");
    let new_id = identity_at(&fx.new_path);
    let outcome = replace_older_copy(&fx.request, &go, &mut rng());
    assert_eq!(
        outcome,
        ReplaceOutcome::Linked {
            old_leaf: "report.pdf".into()
        }
    );
    // The old name now holds the new copy, and the new copy still has its first name.
    assert_eq!(identity_at(&fx.old_path), new_id);
    assert_eq!(identity_at(&fx.new_path), new_id);
    assert_eq!(
        drop_first_name(&fx.request.new, "report (1).pdf", "report.pdf", &mut rng()),
        FirstNameDrop::Removed
    );
    assert_eq!(listing(&fx.dir), ["report.pdf"]);
    assert_eq!(fs::read(&fx.old_path).unwrap(), CONTENT.concat());
}

#[test]
fn a_name_the_user_chose_is_never_renamed() {
    // Design review M2: a different name (typed in Save as, or another listed name) keeps its
    // name; only the proven old copy goes.
    let fx = second_save("my-final.pdf");
    let outcome = replace_older_copy(&fx.request, &go, &mut rng());
    assert_eq!(outcome, ReplaceOutcome::Removed);
    assert_eq!(listing(&fx.dir), ["my-final.pdf"]);
    assert_eq!(fs::read(&fx.new_path).unwrap(), CONTENT.concat());
    // Save as of a name that merely looks numbered ("report (1).pdf" typed by the user) is the
    // user's name too: native passes no asked-for name, so it is never renamed.
    let mut fx = second_save("report.pdf");
    fx.request.started_from = None;
    assert_eq!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Removed
    );
    assert_eq!(listing(&fx.dir), ["report (1).pdf"]);
}

#[test]
fn a_changed_old_copy_is_never_deleted() {
    for edit in [&b"saved file, FIRST part and its second part"[..], b"short"] {
        let fx = second_save("report.pdf");
        fs::write(&fx.old_path, edit).unwrap(); // same identity, other bytes
        let outcome = replace_older_copy(&fx.request, &go, &mut rng());
        assert_eq!(outcome, ReplaceOutcome::Refused(ReplaceRefusal::Changed));
        assert_eq!(fs::read(&fx.old_path).unwrap(), edit, "untouched");
        assert_eq!(listing(&fx.dir), ["report (1).pdf", "report.pdf"]);
    }
}

#[test]
fn another_file_at_the_old_name_is_never_deleted() {
    let fx = second_save("report.pdf");
    // An editor's atomic save: the old copy moved away and a new file took its name.
    fs::rename(&fx.old_path, fx.dir.join("backup")).unwrap();
    fs::write(&fx.old_path, CONTENT.concat()).unwrap(); // same bytes, another file
    let outcome = replace_older_copy(&fx.request, &go, &mut rng());
    assert_eq!(outcome, ReplaceOutcome::Refused(ReplaceRefusal::Changed));
    assert_eq!(listing(&fx.dir), ["backup", "report (1).pdf", "report.pdf"]);
    // A folder at the old name.
    fs::remove_file(&fx.old_path).unwrap();
    fs::create_dir(&fx.old_path).unwrap();
    let outcome = replace_older_copy(&fx.request, &go, &mut rng());
    assert_eq!(outcome, ReplaceOutcome::Refused(ReplaceRefusal::Changed));
    assert!(fx.old_path.is_dir());
}

#[test]
fn a_missing_old_copy_or_its_name_holding_the_new_copy_needs_nothing() {
    let fx = second_save("report.pdf");
    fs::remove_file(&fx.old_path).unwrap();
    assert_eq!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Missing
    );
    assert_eq!(
        listing(&fx.dir),
        ["report (1).pdf"],
        "the new copy is untouched"
    );

    // The old copy was gone before the second save, which therefore took the name itself.
    let root = tempdir();
    let dir = downloads(&root);
    let (old_path, old) = published(&dir, "report.pdf", &CONTENT);
    fs::remove_file(&old_path).unwrap();
    let (new_path, new) = published(&dir, "report.pdf", &CONTENT);
    assert_eq!(new_path, old_path);
    let content = CONTENT.concat();
    let mut request = ReplaceRequest {
        old: SavedFile {
            id: SavedFileId::from_bytes([1; 16]),
            cid: Cid::of(&content),
            size: content.len() as u64,
            saved_at_ms: 1,
            location: SavedLocation::from_journal(&old),
        },
        new: SavedLocation::from_journal(&new),
        started_from: Some("report.pdf".into()),
    };
    assert_eq!(
        replace_older_copy(&request, &go, &mut rng()),
        ReplaceOutcome::AlreadyNew
    );
    // Design review M3(d): on Windows the old entry may differ only in letter case.
    if external::CURRENT_PLATFORM == external::PLATFORM_WINDOWS {
        request.old.location.leaf = "Report.PDF".into();
        assert_eq!(
            replace_older_copy(&request, &go, &mut rng()),
            ReplaceOutcome::AlreadyNew
        );
    }
    assert_eq!(listing(&dir), ["report.pdf"]);
}

#[test]
fn a_read_only_old_copy_or_one_with_another_name_is_kept() {
    let fx = second_save("report.pdf");
    let mut permissions = fs::metadata(&fx.old_path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&fx.old_path, permissions.clone()).unwrap();
    assert_eq!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Refused(ReplaceRefusal::ReadOnly)
    );
    assert!(fx.old_path.exists());
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&fx.old_path, permissions).unwrap();

    fs::hard_link(&fx.old_path, fx.dir.join("a second name")).unwrap();
    assert_eq!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Refused(ReplaceRefusal::MoreNames)
    );
    assert_eq!(
        listing(&fx.dir),
        ["a second name", "report (1).pdf", "report.pdf"]
    );
}

#[test]
fn copies_without_identity_or_in_other_folders_are_refused() {
    let fx = second_save("report.pdf");
    let mut request = fx.request.clone();
    request.old.location = SavedLocation::unidentified(&fx.old_path).unwrap();
    assert_eq!(
        replace_older_copy(&request, &go, &mut rng()),
        ReplaceOutcome::Refused(ReplaceRefusal::NoIdentity)
    );
    let elsewhere = fx._root.path().join("Elsewhere");
    create_folder(&elsewhere).unwrap();
    let (_, there) = published(&elsewhere, "report.pdf", &CONTENT);
    let mut request = fx.request.clone();
    request.old.location = SavedLocation::from_journal(&there);
    assert_eq!(
        replace_older_copy(&request, &go, &mut rng()),
        ReplaceOutcome::Refused(ReplaceRefusal::OtherFolder)
    );
    assert_eq!(listing(&fx.dir), ["report (1).pdf", "report.pdf"]);
    assert_eq!(listing(&elsewhere), ["report.pdf"]);
}

#[test]
fn a_stop_before_any_effect_or_while_hashing_deletes_nothing() {
    for allowed in [0, 1, 2] {
        let fx = second_save("report.pdf");
        let calls = Cell::new(0);
        let keep_going = || {
            calls.set(calls.get() + 1);
            calls.get() <= allowed
        };
        assert_eq!(
            replace_older_copy(&fx.request, &keep_going, &mut rng()),
            ReplaceOutcome::Refused(ReplaceRefusal::Stopped),
            "stopped after {allowed} answers"
        );
        assert_eq!(listing(&fx.dir), ["report (1).pdf", "report.pdf"]);
    }
}

#[cfg(windows)]
#[test]
fn an_old_copy_held_open_for_writing_cannot_be_checked_and_stays() {
    use std::os::windows::fs::OpenOptionsExt;
    let fx = second_save("report.pdf");
    let held = fs::OpenOptions::new()
        .write(true)
        .share_mode(0x1 | 0x2 | 0x4)
        .open(&fx.old_path)
        .unwrap();
    assert_eq!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Refused(ReplaceRefusal::Unchecked)
    );
    drop(held);
    assert_eq!(listing(&fx.dir), ["report (1).pdf", "report.pdf"]);
}

#[cfg(windows)]
#[test]
fn windows_nobody_can_change_the_old_copy_between_its_check_and_its_delete() {
    // The hashed handle shares only reading: a write, a rename, a delete or a replacement of the
    // old copy at that moment all fail, so the object deleted is exactly the one checked.
    let fx = second_save("report.pdf");
    let attempts = Rc::new(Cell::new(0));
    let seen = attempts.clone();
    let dir = fx.dir.clone();
    BEFORE_REPLACE_DELETE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |path: &Path| {
            assert!(fs::OpenOptions::new().write(true).open(path).is_err());
            assert!(fs::rename(path, dir.join("moved")).is_err());
            assert!(fs::remove_file(path).is_err());
            assert!(fs::rename(&dir, dir.with_file_name("Moved folder")).is_err());
            seen.set(seen.get() + 1);
        }));
    });
    let outcome = replace_older_copy(&fx.request, &go, &mut rng());
    BEFORE_REPLACE_DELETE.with(|hook| hook.borrow_mut().take());
    assert_eq!(attempts.get(), 1);
    assert!(
        matches!(outcome, ReplaceOutcome::Linked { .. }),
        "{outcome:?}"
    );
}

#[cfg(unix)]
#[test]
fn unix_a_change_after_the_hash_is_caught_and_put_back() {
    // The old copy is edited (other length) or replaced right after it was hashed: the check at
    // the aside name refuses, and the user's file is put back at its name untouched.
    for swap in [false, true] {
        let fx = second_save("report.pdf");
        let dir = fx.dir.clone();
        BEFORE_REPLACE_DELETE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |path: &Path| {
                if swap {
                    fs::rename(path, dir.join("moved by the user")).unwrap();
                    fs::write(path, b"another file").unwrap();
                } else {
                    fs::write(path, b"edited after the check").unwrap();
                }
            }));
        });
        let outcome = replace_older_copy(&fx.request, &go, &mut rng());
        BEFORE_REPLACE_DELETE.with(|hook| hook.borrow_mut().take());
        assert_eq!(outcome, ReplaceOutcome::Refused(ReplaceRefusal::Changed));
        let expected: &[u8] = if swap {
            b"another file"
        } else {
            b"edited after the check"
        };
        assert_eq!(fs::read(&fx.old_path).unwrap(), expected);
        assert!(listing(&fx.dir)
            .iter()
            .all(|name| !name.starts_with("mewtual-removing-")));
    }
}

#[cfg(unix)]
#[test]
fn unix_a_shared_folder_or_file_refuses_only_after_seeing_the_copy() {
    use std::os::unix::fs::PermissionsExt;
    let fx = second_save("report.pdf");
    // A file other accounts may write.
    fs::set_permissions(&fx.old_path, fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Refused(ReplaceRefusal::Shared)
    );
    fs::set_permissions(&fx.old_path, fs::Permissions::from_mode(0o644)).unwrap();
    // A folder other accounts may change (no sticky bit).
    fs::set_permissions(&fx.dir, fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Refused(ReplaceRefusal::Shared)
    );
    // Design review M3(a): a copy that is not there is "missing", never "shared".
    fs::remove_file(&fx.old_path).unwrap();
    assert_eq!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Missing
    );
    fs::set_permissions(&fx.dir, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_name_taken_before_the_link_keeps_the_new_copy_under_its_own_name() {
    let fx = second_save("report.pdf");
    let old_path = fx.old_path.clone();
    BEFORE_LINK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_: &Path| {
            fs::write(&old_path, b"saved by another program meanwhile").unwrap();
        }));
    });
    let outcome = replace_older_copy(&fx.request, &go, &mut rng());
    BEFORE_LINK.with(|hook| hook.borrow_mut().take());
    assert_eq!(
        outcome,
        ReplaceOutcome::RemovedNotMoved {
            still_named: Some(true)
        }
    );
    assert_eq!(
        fs::read(&fx.old_path).unwrap(),
        b"saved by another program meanwhile",
        "never replaced"
    );
    assert_eq!(fs::read(&fx.new_path).unwrap(), CONTENT.concat());
}

#[test]
fn the_old_copy_is_kept_when_the_new_copy_is_gone_before_the_delete() {
    // Adversarial review H1: the user deletes the apparent duplicate (or antivirus quarantines
    // it) while the old copy is being checked. The old copy must then stay: never no copy at all.
    let fx = second_save("report.pdf");
    fs::remove_file(&fx.new_path).unwrap();
    assert_eq!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Refused(ReplaceRefusal::NewGone),
        "gone before the replace starts"
    );
    assert_eq!(fs::read(&fx.old_path).unwrap(), CONTENT.concat());

    // Gone (or attempted) right before the delete.
    let fx = second_save("report.pdf");
    let new_path = fx.new_path.clone();
    let removed = Rc::new(Cell::new(false));
    let seen = removed.clone();
    BEFORE_REPLACE_DELETE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_: &Path| {
            seen.set(fs::remove_file(&new_path).is_ok());
        }));
    });
    let outcome = replace_older_copy(&fx.request, &go, &mut rng());
    BEFORE_REPLACE_DELETE.with(|hook| hook.borrow_mut().take());
    if cfg!(windows) {
        // Windows holds the new copy under a delete-denying pin, so the delete attempt fails and
        // the replace goes ahead beside a copy that is still there.
        assert!(!removed.get(), "the pinned new copy could not be deleted");
        assert!(
            matches!(outcome, ReplaceOutcome::Linked { .. }),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&fx.old_path).unwrap(), CONTENT.concat());
    } else {
        // Unix has no share modes: the new copy went, so the old one is kept.
        assert!(removed.get());
        assert_eq!(outcome, ReplaceOutcome::Refused(ReplaceRefusal::NewGone));
        assert_eq!(fs::read(&fx.old_path).unwrap(), CONTENT.concat());
    }
}

#[test]
fn renaming_onto_an_old_entrys_place_keeps_the_list_readable() {
    // Adversarial review M1: forgetting the old entry failed, then the rename succeeded. The list
    // must keep one entry per place, or it would decode as damaged afterwards.
    let fx = second_save("report.pdf");
    let mut list = SavedFiles::new(&OWNER);
    list.record_default(fx.request.old.clone());
    let new = SavedFile {
        id: SavedFileId::from_bytes([2; 16]),
        cid: fx.request.old.cid,
        size: fx.request.old.size,
        saved_at_ms: 2,
        location: fx.request.new.clone(),
    };
    list.record_default(new);
    assert_eq!(list.entries().len(), 2);
    assert!(list.rename(
        SavedFileId::from_bytes([2; 16]),
        "report (1).pdf",
        "report.pdf"
    ));
    assert_eq!(list.entries().len(), 1);
    assert_eq!(list.entries()[0].location.leaf(), "report.pdf");
    assert_eq!(SavedFiles::decode(&list.encode()), Ok(list));
}

#[test]
fn the_first_name_stays_when_the_old_name_went_after_the_link() {
    // Design review H1: between the link and the removal of the first name, the old name is
    // deleted (or another file is renamed over it). Then the first name is the only copy, and
    // it must stay.
    for replaced in [false, true] {
        let fx = second_save("report.pdf");
        assert!(matches!(
            replace_older_copy(&fx.request, &go, &mut rng()),
            ReplaceOutcome::Linked { .. }
        ));
        fs::remove_file(&fx.old_path).unwrap();
        if replaced {
            fs::write(&fx.old_path, b"another file took the name").unwrap();
        }
        // Re-review MEDIUM: the answer says the copy is only at its first name, so native never
        // tells the user both names are the same file.
        assert_eq!(
            drop_first_name(&fx.request.new, "report (1).pdf", "report.pdf", &mut rng()),
            FirstNameDrop::OnlyFirst
        );
        assert_eq!(fs::read(&fx.new_path).unwrap(), CONTENT.concat());
    }
    // Neither name holds it any more: nothing is claimed.
    let fx = second_save("report.pdf");
    assert!(matches!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Linked { .. }
    ));
    fs::remove_file(&fx.old_path).unwrap();
    fs::remove_file(&fx.new_path).unwrap();
    assert_eq!(
        drop_first_name(&fx.request.new, "report (1).pdf", "report.pdf", &mut rng()),
        FirstNameDrop::Unknown
    );
}

#[test]
fn the_new_copys_own_name_is_never_claimed_once_it_went_before_the_link() {
    // Re-review LOW 4: the new copy is deleted right before it would take the old name.
    let fx = second_save("report.pdf");
    let removed = Rc::new(Cell::new(false));
    let seen = removed.clone();
    BEFORE_LINK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |new_copy: &Path| {
            seen.set(fs::remove_file(new_copy).is_ok());
        }));
    });
    let outcome = replace_older_copy(&fx.request, &go, &mut rng());
    BEFORE_LINK.with(|hook| hook.borrow_mut().take());
    if cfg!(windows) {
        // The pin keeps the new copy's name, so the link goes ahead.
        assert!(!removed.get(), "the pinned new copy could not be deleted");
        assert!(
            matches!(outcome, ReplaceOutcome::Linked { .. }),
            "{outcome:?}"
        );
    } else {
        // The old copy was already removed, and the new copy went: the note must not name it.
        assert!(removed.get());
        assert_eq!(
            outcome,
            ReplaceOutcome::RemovedNotMoved {
                still_named: Some(false)
            }
        );
    }
}

#[cfg(unix)]
#[test]
fn unix_the_new_copy_gone_after_the_rename_aside_puts_the_old_copy_back() {
    // Re-review LOW 4: the second check, right before the unlink. The new copy goes after the old
    // one was moved aside and verified there: the old copy is put back at its name, unchanged.
    let fx = second_save("report.pdf");
    let new_path = fx.new_path.clone();
    AFTER_ASIDE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_: &Path, _: &Path| {
            fs::remove_file(&new_path).unwrap();
        }));
    });
    let outcome = replace_older_copy(&fx.request, &go, &mut rng());
    AFTER_ASIDE.with(|hook| hook.borrow_mut().take());
    assert_eq!(outcome, ReplaceOutcome::Refused(ReplaceRefusal::NewGone));
    assert_eq!(fs::read(&fx.old_path).unwrap(), CONTENT.concat());
    assert_eq!(listing(&fx.dir), ["report.pdf"]);
}

#[cfg(windows)]
#[test]
fn windows_the_other_name_cannot_go_while_the_first_one_is_removed() {
    // A delete-denying handle on one name does not protect another hard link of the same file on
    // Windows (seen here: share access is checked per name), so the removal pins the old name
    // itself while it deletes the first one. Deleting or replacing the old name at that moment
    // fails, and the copy is never left with no name at all.
    let fx = second_save("report.pdf");
    assert!(matches!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Linked { .. }
    ));
    let other = fx.old_path.clone();
    let dir = fx.dir.clone();
    let refused = Rc::new(Cell::new(0));
    let seen = refused.clone();
    BEFORE_REMOVE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_: &Path| {
            let removed = fs::remove_file(&other);
            fs::write(dir.join("intruder"), b"x").unwrap();
            let replaced = fs::rename(dir.join("intruder"), &other);
            seen.set(u8::from(removed.is_err()) + u8::from(replaced.is_err()));
        }));
    });
    let dropped = drop_first_name(&fx.request.new, "report (1).pdf", "report.pdf", &mut rng());
    BEFORE_REMOVE.with(|hook| hook.borrow_mut().take());
    assert_eq!(
        refused.get(),
        2,
        "the old name could be neither deleted nor replaced"
    );
    assert_eq!(dropped, FirstNameDrop::Removed);
    assert_eq!(listing(&fx.dir), ["intruder", "report.pdf"]);
    assert_eq!(fs::read(&fx.old_path).unwrap(), CONTENT.concat());
}

#[test]
fn dropping_a_first_name_that_is_not_the_new_copy_does_nothing() {
    let fx = second_save("report.pdf");
    assert!(matches!(
        replace_older_copy(&fx.request, &go, &mut rng()),
        ReplaceOutcome::Linked { .. }
    ));
    fs::remove_file(&fx.new_path).unwrap();
    fs::write(&fx.new_path, b"someone else's file").unwrap();
    // The copy is at the old name only.
    assert_eq!(
        drop_first_name(&fx.request.new, "report (1).pdf", "report.pdf", &mut rng()),
        FirstNameDrop::OnlyOld
    );
    assert_eq!(fs::read(&fx.new_path).unwrap(), b"someone else's file");
}
