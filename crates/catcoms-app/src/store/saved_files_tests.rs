//! The sealed saved-files list record, `servers/<id>.saved` (PR #56,
//! `docs/design-saved-files-list.md`): round trip across a store reopen, owner binding, the
//! non-destructive handling of records this build cannot read, "Start a new list" for a damaged
//! one only, and removal on leave.
use super::*;
use catcoms_storage::kept::{SavedFile, SavedFileId, SavedFiles, SavedLocation};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const PASS: &[u8] = b"correct horse";
/// Group id followed by the member id, as native binds it.
const OWNER: &[u8] = b"group+member-1";

fn list(owner: &[u8], leaf: &str) -> SavedFiles {
    let mut list = SavedFiles::new(owner);
    assert!(list.record(SavedFile {
        id: SavedFileId::from_bytes([1; 16]),
        cid: catcoms_storage::Cid::of(b"file"),
        size: 4,
        saved_at_ms: 1_700_000_000_000,
        location: SavedLocation::unidentified(&std::env::temp_dir().join(leaf)).unwrap(),
    }));
    list
}

fn record_path(dir: &Path, id: u64) -> PathBuf {
    dir.join("servers").join(format!("{id}.saved"))
}

#[test]
fn the_saved_files_list_survives_a_restart_sealed() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(56);
    let saved = list(OWNER, "report-secret-name.pdf");
    {
        let store = ServerStore::open(dir.path(), PASS, &mut rng).unwrap();
        assert_eq!(store.load_saved_files(7, OWNER), SavedFilesLoad::Missing);
        store.save_saved_files(7, OWNER, &saved, &mut rng).unwrap();
    }
    // Sealed at rest: neither the name nor the folder is readable in the file.
    let raw = std::fs::read(record_path(dir.path(), 7)).unwrap();
    assert!(!raw
        .windows(b"report-secret-name".len())
        .any(|w| w == b"report-secret-name"));
    let temp = std::env::temp_dir();
    let folder = temp.to_string_lossy();
    assert!(!String::from_utf8_lossy(&raw).contains(folder.as_ref()));
    // A process restart (new mount) reads it back.
    let store = ServerStore::open(dir.path(), PASS, &mut rng).unwrap();
    assert_eq!(
        store.load_saved_files(7, OWNER),
        SavedFilesLoad::Found(saved)
    );
    assert_eq!(store.load_saved_files(8, OWNER), SavedFilesLoad::Missing);
}

#[test]
fn a_list_for_another_owner_never_applies_and_is_never_written_there() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(56);
    let store = ServerStore::open(dir.path(), PASS, &mut rng).unwrap();
    store
        .save_saved_files(7, OWNER, &list(OWNER, "a.txt"), &mut rng)
        .unwrap();
    // Another group, or the same group after leaving and joining again (a new member id): a
    // stale record reads as no list at all, and the next save writes over it (review L3).
    for other in [&b"other group+member-1"[..], b"group+member-2"] {
        assert_eq!(store.load_saved_files(7, other), SavedFilesLoad::Missing);
    }
    assert!(store
        .save_saved_files(7, OWNER, &list(b"group+member-2", "b.txt"), &mut rng)
        .is_err());
    assert_eq!(
        store.load_saved_files(7, OWNER),
        SavedFilesLoad::Found(list(OWNER, "a.txt"))
    );
}

#[test]
fn records_this_build_cannot_read_are_told_apart_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(56);
    let store = ServerStore::open(dir.path(), PASS, &mut rng).unwrap();
    let path = record_path(dir.path(), 7);
    // Damaged bytes, another record's sealed bytes (the cache setting) and an oversized file.
    std::fs::write(&path, b"damaged beyond the seal").unwrap();
    assert_eq!(store.load_saved_files(7, OWNER), SavedFilesLoad::Damaged);
    store
        .save_cache_setting(
            9,
            b"group",
            &crate::cache_housekeeping::CacheSetting {
                limit: None,
                origin: crate::cache_housekeeping::CacheSettingOrigin::User,
            },
            &mut rng,
        )
        .unwrap();
    std::fs::copy(dir.path().join("servers").join("9.storage"), &path).unwrap();
    assert_eq!(store.load_saved_files(7, OWNER), SavedFilesLoad::Damaged);
    std::fs::write(
        &path,
        vec![0u8; catcoms_storage::kept::MAX_SAVED_LIST_BYTES + 41],
    )
    .unwrap();
    assert_eq!(store.load_saved_files(7, OWNER), SavedFilesLoad::Damaged);
    // A version 2 record, as a newer build would write it, sealed with this vault's key.
    let mut newer = list(OWNER, "a.txt").encode();
    let version_at = 4 + b"mewtual/saved-files".len();
    newer[version_at] = 2;
    let sealed = seal(&store.keys.db_key().unwrap(), &newer, &mut rng).unwrap();
    store.write_saved_files_record(7, &frame(&sealed)).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(store.load_saved_files(7, OWNER), SavedFilesLoad::Newer);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "reading is read-only"
    );
    // "Start a new list" never replaces a newer version's record.
    assert!(!store.reset_saved_files(7, OWNER, &mut rng).unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    // Something that is not a file is damaged too, never read through.
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert_ne!(store.load_saved_files(7, OWNER), SavedFilesLoad::Missing);
    std::fs::remove_dir(&path).unwrap();
}

#[test]
fn start_a_new_list_replaces_only_a_damaged_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(56);
    let store = ServerStore::open(dir.path(), PASS, &mut rng).unwrap();
    // Nothing to reset: missing, or a readable list.
    assert!(!store.reset_saved_files(7, OWNER, &mut rng).unwrap());
    assert!(!record_path(dir.path(), 7).exists());
    store
        .save_saved_files(7, OWNER, &list(OWNER, "a.txt"), &mut rng)
        .unwrap();
    assert!(!store.reset_saved_files(7, OWNER, &mut rng).unwrap());
    assert!(matches!(
        store.load_saved_files(7, OWNER),
        SavedFilesLoad::Found(list) if list.entries().len() == 1
    ));
    // Damaged: replaced by an empty list for this owner.
    std::fs::write(record_path(dir.path(), 7), b"damaged").unwrap();
    assert!(store.reset_saved_files(7, OWNER, &mut rng).unwrap());
    assert_eq!(
        store.load_saved_files(7, OWNER),
        SavedFilesLoad::Found(SavedFiles::new(OWNER))
    );
}

#[test]
fn leaving_removes_the_list_whatever_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(56);
    let store = ServerStore::open(dir.path(), PASS, &mut rng).unwrap();
    store
        .save_saved_files(7, OWNER, &list(OWNER, "a.txt"), &mut rng)
        .unwrap();
    store
        .save_saved_files(8, OWNER, &list(OWNER, "b.txt"), &mut rng)
        .unwrap();
    std::fs::write(record_path(dir.path(), 9), b"a newer or damaged record").unwrap();
    for id in [7, 9] {
        store.remove_server(id).unwrap();
        assert!(!record_path(dir.path(), id).exists());
        assert_eq!(store.load_saved_files(id, OWNER), SavedFilesLoad::Missing);
    }
    // Only those servers' lists go.
    assert!(matches!(
        store.load_saved_files(8, OWNER),
        SavedFilesLoad::Found(_)
    ));
}
