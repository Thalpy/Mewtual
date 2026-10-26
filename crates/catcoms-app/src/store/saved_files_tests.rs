//! The sealed saved-files list record, `servers/<id>.saved` (PR #56,
//! `docs/design-saved-files-list.md`): round trip across a store reopen, group binding, the
//! non-destructive handling of a record this build cannot read, and removal on leave.
use super::*;
use catcoms_storage::kept::{SavedFile, SavedFileId, SavedFiles, SavedLocation};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const PASS: &[u8] = b"correct horse";

fn list(group: &[u8], leaf: &str) -> SavedFiles {
    let mut list = SavedFiles::new(group);
    list.record(SavedFile {
        id: SavedFileId::from_bytes([1; 16]),
        cid: catcoms_storage::Cid::of(b"file"),
        size: 4,
        saved_at_ms: 1_700_000_000_000,
        location: SavedLocation::unidentified(&std::env::temp_dir().join(leaf)).unwrap(),
    })
    .unwrap();
    list
}

#[test]
fn the_saved_files_list_survives_a_restart_sealed() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(56);
    let saved = list(b"group", "report-secret-name.pdf");
    {
        let store = ServerStore::open(dir.path(), PASS, &mut rng).unwrap();
        assert_eq!(store.load_saved_files(7, b"group"), SavedFilesLoad::Missing);
        store
            .save_saved_files(7, b"group", &saved, &mut rng)
            .unwrap();
    }
    // Sealed at rest: neither the name nor the folder is readable in the file.
    let raw = std::fs::read(dir.path().join("servers").join("7.saved")).unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(!text.contains("report-secret-name"));
    assert!(!raw
        .windows(b"report-secret-name".len())
        .any(|w| w == b"report-secret-name"));
    // A process restart (new mount) reads it back.
    let store = ServerStore::open(dir.path(), PASS, &mut rng).unwrap();
    assert_eq!(
        store.load_saved_files(7, b"group"),
        SavedFilesLoad::Found(saved)
    );
    // Another server number has none.
    assert_eq!(store.load_saved_files(8, b"group"), SavedFilesLoad::Missing);
}

#[test]
fn a_list_for_another_group_never_applies_and_is_never_written_there() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(56);
    let store = ServerStore::open(dir.path(), PASS, &mut rng).unwrap();
    store
        .save_saved_files(7, b"group", &list(b"group", "a.txt"), &mut rng)
        .unwrap();
    // A stale record under a reused server number reads as no list at all.
    assert_eq!(
        store.load_saved_files(7, b"other group"),
        SavedFilesLoad::Missing
    );
    // And a list naming another group is never written under this server.
    assert!(store
        .save_saved_files(7, b"group", &list(b"other group", "b.txt"), &mut rng)
        .is_err());
    assert_eq!(
        store.load_saved_files(7, b"group"),
        SavedFilesLoad::Found(list(b"group", "a.txt"))
    );
}

#[test]
fn an_unreadable_list_reads_as_unreadable_and_leave_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(56);
    let store = ServerStore::open(dir.path(), PASS, &mut rng).unwrap();
    let path = dir.path().join("servers").join("7.saved");
    // Damaged bytes, another record's sealed bytes (the cache setting), a newer version, and an
    // oversized file all read as unreadable, and reading changes nothing.
    std::fs::write(&path, b"damaged beyond the seal").unwrap();
    assert_eq!(
        store.load_saved_files(7, b"group"),
        SavedFilesLoad::Unreadable
    );
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
    assert_eq!(
        store.load_saved_files(7, b"group"),
        SavedFilesLoad::Unreadable
    );
    // A version 2 record (as a newer build would write it), sealed with this vault's key.
    let mut newer = list(b"group", "a.txt").encode();
    let version_at = 4 + b"mewtual/saved-files".len();
    newer[version_at] = 2;
    let sealed = seal(&store.keys.db_key().unwrap(), &newer, &mut rng).unwrap();
    store.write_saved_files_record(7, &frame(&sealed)).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        store.load_saved_files(7, b"group"),
        SavedFilesLoad::Unreadable
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "reading is read-only"
    );
    std::fs::write(
        &path,
        vec![0u8; catcoms_storage::kept::MAX_SAVED_LIST_BYTES + 41],
    )
    .unwrap();
    assert_eq!(
        store.load_saved_files(7, b"group"),
        SavedFilesLoad::Unreadable
    );
    // Leaving the server removes it, whatever it holds, with the other per-server records.
    store.remove_server(7).unwrap();
    assert!(!path.exists());
    assert_eq!(store.load_saved_files(7, b"group"), SavedFilesLoad::Missing);
}

#[test]
fn leaving_removes_a_readable_list_too() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(56);
    let store = ServerStore::open(dir.path(), PASS, &mut rng).unwrap();
    store
        .save_saved_files(7, b"group", &list(b"group", "a.txt"), &mut rng)
        .unwrap();
    store
        .save_saved_files(8, b"group", &list(b"group", "b.txt"), &mut rng)
        .unwrap();
    store.remove_server(7).unwrap();
    assert_eq!(store.load_saved_files(7, b"group"), SavedFilesLoad::Missing);
    // Only that server's list goes.
    assert!(matches!(
        store.load_saved_files(8, b"group"),
        SavedFilesLoad::Found(_)
    ));
}
