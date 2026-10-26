//! The saved-files list: codec, bounds, version handling and the read-only checks of a recorded
//! file against real files (PR #56, `docs/design-saved-files-list.md`).
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::fs;

const GROUP: [u8; 32] = [3; 32];

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(56)
}

/// A real journaled save of `chunks` as `name` in `dir`: the export's own create, write, link and
/// staging removal, without a store (those are the external effects the list records).
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
    let dir = std::env::temp_dir();
    SavedLocation::unidentified(&dir.join(leaf)).unwrap()
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

#[test]
fn a_list_round_trips_with_and_without_identities() {
    let root = tempdir();
    let dir = downloads(&root);
    let (_, journal) = published(&dir, "report.pdf", &[b"hello"]);
    let mut list = SavedFiles::new(&GROUP);
    assert_eq!(
        list.record(entry(1, b"a", SavedLocation::from_journal(&journal))),
        Some(None)
    );
    assert_eq!(
        list.record(entry(2, b"b", plain_location("plain.txt"))),
        Some(None)
    );
    let bytes = list.encode();
    let decoded = SavedFiles::decode(&bytes).unwrap();
    assert_eq!(decoded, list);
    assert_eq!(decoded.group(), &GROUP);
    assert_eq!(decoded.entries().len(), 2);
    assert!(decoded.entries()[0].location.identified());
    assert!(!decoded.entries()[1].location.identified());
    assert_eq!(decoded.entries()[0].location.leaf(), "report.pdf");
    // Canonical: encoding the decoded list gives the same bytes.
    assert_eq!(decoded.encode(), bytes);
    // An empty list round-trips too.
    let empty = SavedFiles::new(&GROUP);
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
fn one_entry_per_file_and_the_newest_save_replaces_it() {
    let mut list = SavedFiles::new(&GROUP);
    list.record(entry(1, b"a", plain_location("a.txt")))
        .unwrap();
    list.record(entry(2, b"b", plain_location("b.txt")))
        .unwrap();
    // A second save of file "a" returns the earlier entry and becomes the newest.
    let previous = list
        .record(entry(3, b"a", plain_location("a (1).txt")))
        .unwrap();
    assert_eq!(
        previous.map(|old| old.id),
        Some(SavedFileId::from_bytes([1; 16]))
    );
    let ids: Vec<_> = list.entries().iter().map(|e| e.id.as_bytes()[0]).collect();
    assert_eq!(ids, [2, 3]);
    assert_eq!(
        list.by_cid(&Cid::of(b"a")).unwrap().location.leaf(),
        "a (1).txt"
    );
    // An ID already used by another file is refused, changing nothing.
    let before = list.clone();
    assert_eq!(list.record(entry(2, b"c", plain_location("c.txt"))), None);
    assert_eq!(list, before);
}

#[test]
fn the_list_keeps_at_most_256_entries_dropping_the_oldest() {
    let mut list = SavedFiles::new(&GROUP);
    for n in 0..(MAX_SAVED_FILES as u32 + 10) {
        let mut id = [0u8; 16];
        id[..4].copy_from_slice(&n.to_be_bytes());
        list.record(SavedFile {
            id: SavedFileId::from_bytes(id),
            cid: Cid::of(&n.to_be_bytes()),
            size: 1,
            saved_at_ms: u64::from(n),
            location: plain_location("f.txt"),
        })
        .unwrap();
    }
    assert_eq!(list.entries().len(), MAX_SAVED_FILES);
    // The ten oldest went; the newest stayed.
    assert_eq!(list.entries()[0].saved_at_ms, 10);
    assert_eq!(
        list.entries().last().unwrap().saved_at_ms,
        u64::from(MAX_SAVED_FILES as u32 + 9)
    );
    assert!(SavedFiles::decode(&list.encode()).is_ok());
}

#[test]
fn very_long_folders_are_capped_by_bytes_not_count() {
    // A maximal folder path: about 16 KiB per entry, so 1 MiB holds about 60 of them.
    let dir = std::env::temp_dir().join("x".repeat(4000));
    let long = |n: u32| {
        let mut location = SavedLocation::unidentified(&dir.join("f.txt")).unwrap();
        location.folder = vec![b'a'; super::external::MAX_FOLDER_BYTES];
        let mut id = [0u8; 16];
        id[..4].copy_from_slice(&n.to_be_bytes());
        SavedFile {
            id: SavedFileId::from_bytes(id),
            cid: Cid::of(&n.to_be_bytes()),
            size: 1,
            saved_at_ms: u64::from(n),
            location,
        }
    };
    let mut list = SavedFiles::new(&GROUP);
    for n in 0..200 {
        list.record(long(n)).unwrap();
    }
    assert!(list.entries().len() < 100, "{}", list.entries().len());
    assert!(list.encode().len() <= MAX_SAVED_LIST_BYTES);
    assert_eq!(list.entries().last().unwrap().saved_at_ms, 199);
    assert!(SavedFiles::decode(&list.encode()).is_ok());
}

#[test]
fn an_unknown_version_or_a_foreign_record_is_refused_not_misread() {
    let mut list = SavedFiles::new(&GROUP);
    list.record(entry(1, b"a", plain_location("a.txt")))
        .unwrap();
    let bytes = list.encode();
    // The version byte follows the length-prefixed domain.
    let version_at = 4 + b"mewtual/saved-files".len();
    assert_eq!(bytes[version_at], SAVED_LIST_VERSION);
    let mut newer = bytes.clone();
    newer[version_at] = 2;
    assert_eq!(SavedFiles::decode(&newer), Err(SavedListError::Version(2)));
    // Another sealed record's plaintext (here: the cache setting's shape) is not a list.
    let mut other = catcoms_wire::Encoder::new();
    other.put_u8(1);
    other.put_bytes(&GROUP).unwrap();
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
    let mut list = SavedFiles::new(&GROUP);
    list.record(entry(1, b"a", plain_location("a.txt")))
        .unwrap();
    list.record(entry(2, b"b", plain_location("b.txt")))
        .unwrap();
    let good = list.encode();
    // Trailing bytes.
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(
        SavedFiles::decode(&trailing),
        Err(SavedListError::Malformed)
    );
    // Truncated anywhere.
    for cut in [1, good.len() / 2, good.len() - 1] {
        assert_eq!(
            SavedFiles::decode(&good[..cut]),
            Err(SavedListError::Malformed),
            "cut at {cut}"
        );
    }
    // Duplicate IDs and duplicate files.
    for (id, cid) in [(1u8, &b"c"[..]), (3, &b"a"[..])] {
        let mut duplicate = SavedFiles::new(&GROUP);
        duplicate.entries = vec![
            entry(1, b"a", plain_location("a.txt")),
            entry(id, cid, plain_location("c.txt")),
        ];
        assert_eq!(
            SavedFiles::decode(&duplicate.encode()),
            Err(SavedListError::Malformed)
        );
    }
    // A count over the cap, an empty group, an invalid leaf, an oversized size, a bad identity
    // flag, and a platform that does not match the identities.
    let encode = |edit: &dyn Fn(&mut SavedFiles)| {
        let mut list = SavedFiles::new(&GROUP);
        list.entries = vec![entry(1, b"a", plain_location("a.txt"))];
        edit(&mut list);
        list.encode()
    };
    let mut over = good.clone();
    let count_at = 4 + b"mewtual/saved-files".len() + 1 + 4 + GROUP.len();
    over[count_at..count_at + 4].copy_from_slice(&(MAX_SAVED_FILES as u32 + 1).to_be_bytes());
    assert_eq!(SavedFiles::decode(&over), Err(SavedListError::Malformed));
    for bytes in [
        encode(&|list| list.group.clear()),
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
    // An oversized record is refused before decoding.
    assert_eq!(
        SavedFiles::decode(&vec![0; MAX_SAVED_LIST_BYTES + 1]),
        Err(SavedListError::Malformed)
    );
}

#[test]
fn forget_and_rename_touch_only_the_named_entry() {
    let root = tempdir();
    let dir = downloads(&root);
    let (_, first) = published(&dir, "first.txt", &[b"one"]);
    let (_, second) = published(&dir, "second.txt", &[b"two"]);
    let mut list = SavedFiles::new(&GROUP);
    list.record(entry(1, b"a", SavedLocation::from_journal(&first)))
        .unwrap();
    list.record(entry(2, b"b", SavedLocation::from_journal(&second)))
        .unwrap();
    // Rename is a compare-and-swap on the current name.
    let id = SavedFileId::from_bytes([1; 16]);
    assert!(!list.rename(id, "wrong.txt", "first (2).txt"));
    assert!(!list.rename(id, "first.txt", "bad/name"));
    assert!(list.rename(id, "first.txt", "first (2).txt"));
    assert_eq!(list.get(id).unwrap().location.leaf(), "first (2).txt");
    // Forgetting by the published journal removes only the entry of that exact file.
    assert!(list.forget_journal(&second));
    assert!(!list.forget_journal(&second));
    assert_eq!(list.entries().len(), 1);
    assert!(list.forget(id));
    assert!(!list.forget(id));
    assert!(list.entries().is_empty());
    // The files themselves were never touched.
    assert_eq!(fs::read(dir.join("first.txt")).unwrap(), b"one");
    assert_eq!(fs::read(dir.join("second.txt")).unwrap(), b"two");
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

#[test]
fn a_recorded_file_is_checked_read_only_by_identity() {
    let root = tempdir();
    let dir = downloads(&root);
    let (path, journal) = published(&dir, "report.pdf", &[b"hello"]);
    let saved = entry(1, b"hello", SavedLocation::from_journal(&journal));
    let listing = || {
        let mut names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    let before = listing();
    assert_eq!(saved_file_presence(&saved), SavedPresence::Ours);
    assert_eq!(saved_file_location(&saved), Ok(path.clone()));
    assert_eq!(listing(), before, "nothing changed");
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
    // A link put at the saved name is never ours.
    fs::rename(&moved, &dir).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.join("original"), &path).unwrap();
        assert_eq!(saved_file_presence(&saved), SavedPresence::Replaced);
    }
}

#[test]
fn an_entry_without_identity_cannot_be_checked() {
    let root = tempdir();
    let dir = downloads(&root);
    let path = dir.join("plain.txt");
    fs::write(&path, b"saved by the writer without a journal").unwrap();
    let saved = entry(1, b"x", SavedLocation::unidentified(&path).unwrap());
    assert!(!saved.location.identified());
    assert!(saved.location.is_in_folder(&dir));
    assert_eq!(saved_file_presence(&saved), SavedPresence::Unknown);
    assert_eq!(saved_file_location(&saved), Err(SavedPresence::Unknown));
    // No parent or no valid name: no location at all.
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
    let saved = entry(1, b"hello", location);
    assert_eq!(saved_file_presence(&saved), SavedPresence::Unknown);
    assert!(!saved.location.is_in_folder(&dir));
}
