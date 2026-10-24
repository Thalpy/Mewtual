//! Crash-safety, scan and leave-cleanup tests for the primary cache. Each test builds a real
//! directory: these properties are about what is on disk after an interruption, not about logic.

use super::*;
use crate::{BlobStore, SealingBlobStore};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn primary(dir: &Path) -> SealingBlobStore<ChaCha20Rng> {
    SealingBlobStore::open_cache(dir, [7; 32], ChaCha20Rng::seed_from_u64(1)).unwrap()
}

fn incoming_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir.join(INCOMING_DIR)) else {
        return Vec::new();
    };
    let mut names: Vec<_> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn temp_names_round_trip_and_reject_foreign_shapes() {
    let cid = Cid::of(b"chunk");
    let name = temp_name(&cid, 9, 3);
    assert_eq!(parse_temp(&name), Some((std::process::id(), 9)));
    for bad in [
        "staging",
        "abc.1.2.3",
        &format!("{}.1.2", cid.to_hex()),
        &format!("{}.1.2.3.4", cid.to_hex()),
        &format!("{}.x.2.3", cid.to_hex()),
    ] {
        assert_eq!(parse_temp(bad), None, "{bad}");
    }
}

#[test]
fn primary_put_publishes_complete_records_and_leaves_no_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = primary(dir.path());
    store.cache_scan(true).unwrap();
    let cid = store.put(b"a whole chunk").unwrap();
    assert_eq!(
        store.get(&cid).unwrap().as_deref(),
        Some(&b"a whole chunk"[..])
    );
    assert!(
        incoming_names(dir.path()).is_empty(),
        "rename consumed the temp file"
    );
    let index = store.cache_index().unwrap();
    let on_disk = fs::metadata(dir.path().join(cid.to_hex())).unwrap().len();
    assert_eq!(index.sizes().get(&cid), Some(&on_disk));
    assert_eq!(index.usage(), on_disk);
    assert!(
        !index.mount_set().contains(&cid),
        "written during this mount, so never unclaimed this mount"
    );
}

#[test]
fn keep_style_stores_never_create_an_incoming_directory() {
    // Keep reuses SealingBlobStore for its per-file directories and rejects unexpected entries.
    let dir = tempfile::tempdir().unwrap();
    let mut plain =
        SealingBlobStore::open(dir.path(), [7; 32], ChaCha20Rng::seed_from_u64(1)).unwrap();
    plain.put(b"kept chunk").unwrap();
    assert!(!dir.path().join(INCOMING_DIR).exists());
    assert!(plain.cache_index().is_none());
    assert!(
        plain.cache_scan(true).is_err(),
        "no size-limited cache to scan"
    );
}

#[test]
fn an_interrupted_write_never_appears_held_and_is_recovered_at_mount() {
    let dir = tempfile::tempdir().unwrap();
    let cid = Cid::of(b"never finished");
    fs::create_dir_all(dir.path().join(INCOMING_DIR)).unwrap();
    // A crash after a partial temp write: the CID name was never created.
    let dead = MountGeneration::register();
    let abandoned = dir
        .path()
        .join(INCOMING_DIR)
        .join(temp_name(&cid, dead.value(), 1));
    drop(dead); // the handle that wrote it is gone
    fs::write(&abandoned, b"partial").unwrap();
    // Another process's leftover and a name this module never produces.
    let foreign = dir.path().join(INCOMING_DIR).join(format!(
        "{}.{}.1.1",
        cid.to_hex(),
        std::process::id().wrapping_add(1)
    ));
    fs::write(&foreign, b"partial").unwrap();
    fs::write(dir.path().join(INCOMING_DIR).join("junk"), b"x").unwrap();
    // A live handle's in-flight file must survive another handle's recovery.
    let live = MountGeneration::register();
    let in_flight = dir
        .path()
        .join(INCOMING_DIR)
        .join(temp_name(&cid, live.value(), 1));
    fs::write(&in_flight, b"being written").unwrap();

    let mut store = primary(dir.path());
    assert!(!store.has(&cid), "a temp file is never held content");
    let report = store.cache_scan(true).unwrap();
    assert_eq!(report.recovered_temps, 3);
    assert!(!abandoned.exists() && !foreign.exists());
    assert!(
        in_flight.exists(),
        "a live generation's write is left alone"
    );
    drop(live);
    let mut next = primary(dir.path());
    assert_eq!(next.cache_scan(true).unwrap().recovered_temps, 1);
    assert!(
        !in_flight.exists(),
        "once its handle is gone it is abandoned"
    );
}

#[test]
fn mount_scan_removes_torn_records_and_fixes_the_mount_set() {
    let dir = tempfile::tempdir().unwrap();
    let healthy = {
        let mut legacy =
            SealingBlobStore::open(dir.path(), [7; 32], ChaCha20Rng::seed_from_u64(2)).unwrap();
        legacy.put(b"healthy legacy record").unwrap()
    };
    let torn = Cid::of(b"torn by a crash");
    fs::write(dir.path().join(torn.to_hex()), [0u8; 39]).unwrap();
    // A torn record long enough to look sealed stays: it fails its read check and `put` repairs it.
    let long_torn = Cid::of(b"long torn");
    fs::write(dir.path().join(long_torn.to_hex()), [0u8; 64]).unwrap();
    fs::write(dir.path().join(LEAVING_MARKER), b"not a cid").unwrap();

    let mut store = primary(dir.path());
    let report = store.cache_scan(true).unwrap();
    assert!(report.complete);
    assert_eq!(report.removed_torn, 1);
    assert!(!dir.path().join(torn.to_hex()).exists());
    let index = store.cache_index().unwrap();
    assert!(index.complete());
    assert_eq!(index.mount_set().iter().copied().collect::<Vec<_>>(), {
        let mut v = vec![healthy, long_torn];
        v.sort();
        v
    });
    assert!(store.get(&long_torn).is_err());
    assert_eq!(store.put(b"long torn").unwrap(), long_torn);
    assert_eq!(
        store.get(&long_torn).unwrap().as_deref(),
        Some(&b"long torn"[..])
    );
    assert!(
        !store
            .cache_index()
            .unwrap()
            .mount_set()
            .contains(&long_torn),
        "a repaired record was written this mount"
    );
}

#[test]
fn refresh_keeps_the_mount_set_closed_to_other_handles_writes() {
    let dir = tempfile::tempdir().unwrap();
    let old = {
        let mut legacy =
            SealingBlobStore::open(dir.path(), [7; 32], ChaCha20Rng::seed_from_u64(3)).unwrap();
        legacy.put(b"present at mount").unwrap()
    };
    let mut actor = primary(dir.path());
    actor.cache_scan(true).unwrap();
    // A Studio-style second handle writes and deletes behind the actor handle's back.
    let mut other = primary(dir.path());
    let fresh = other.put(b"written by another handle").unwrap();
    assert!(!actor.cache_index().unwrap().sizes().contains_key(&fresh));
    actor.cache_scan(false).unwrap();
    let index = actor.cache_index().unwrap();
    assert!(
        index.sizes().contains_key(&fresh),
        "usage counts it after a refresh"
    );
    assert!(
        !index.mount_set().contains(&fresh),
        "but it never becomes unclaimed"
    );
    assert!(index.mount_set().contains(&old));
    other.delete(&old).unwrap();
    actor.cache_scan(false).unwrap();
    let index = actor.cache_index().unwrap();
    assert!(!index.sizes().contains_key(&old) && !index.mount_set().contains(&old));
}

#[test]
fn deletes_and_promotions_keep_the_index_current() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = primary(dir.path());
    store.cache_scan(true).unwrap();
    let staged = store.put_staged(b"uploaded chunk").unwrap();
    assert!(!store.cache_index().unwrap().sizes().contains_key(&staged));
    assert!(store.promote_staged(&staged).unwrap());
    assert!(store.cache_index().unwrap().sizes().contains_key(&staged));
    assert!(store.delete(&staged).unwrap());
    assert!(store.cache_index().unwrap().sizes().is_empty());
    let bounded = store.put_staged(b"bounded").unwrap();
    assert!(store.promote_staged_bounded(&bounded, 7).unwrap());
    assert!(store.cache_index().unwrap().sizes().contains_key(&bounded));
}

#[test]
fn leave_marker_round_trips_and_only_an_empty_namespace_is_removed() {
    let root = tempfile::tempdir().unwrap();
    let namespace = root.path().join("ab12");
    fs::create_dir_all(&namespace).unwrap();
    assert!(!has_leaving_marker(&namespace));
    write_leaving_marker(&namespace).unwrap();
    assert!(has_leaving_marker(&namespace));
    assert_eq!(
        leaving_namespaces(root.path()),
        vec![("ab12".to_string(), namespace.clone())]
    );
    // A Studio-pinned blob keeps the namespace; the marker still goes.
    fs::write(namespace.join(Cid::of(b"pinned").to_hex()), [1u8; 64]).unwrap();
    assert!(!finish_leaving(&namespace).unwrap());
    assert!(namespace.exists() && !has_leaving_marker(&namespace));
    fs::remove_file(namespace.join(Cid::of(b"pinned").to_hex())).unwrap();
    write_leaving_marker(&namespace).unwrap();
    assert!(finish_leaving(&namespace).unwrap());
    assert!(!namespace.exists());
    // An empty or foreign file is not our marker, and non-hex directories are never offered.
    let other = root.path().join("cd34");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join(LEAVING_MARKER), b"").unwrap();
    let alias = root.path().join("not-hex");
    fs::create_dir_all(&alias).unwrap();
    write_leaving_marker(&alias).unwrap();
    assert!(leaving_namespaces(root.path()).is_empty());
}

#[test]
fn scratch_and_kept_trees_are_removed_with_freed_bytes_counted() {
    let root = tempfile::tempdir().unwrap();
    let namespace = root.path().join("ab12");
    fs::create_dir_all(namespace.join(STAGING_DIR)).unwrap();
    fs::create_dir_all(namespace.join(INCOMING_DIR)).unwrap();
    fs::write(namespace.join(STAGING_DIR).join("a"), [0u8; 10]).unwrap();
    fs::write(namespace.join(INCOMING_DIR).join("b"), [0u8; 5]).unwrap();
    let scratch = remove_scratch(&namespace);
    assert_eq!(
        scratch,
        Removal {
            freed_bytes: 15,
            failed: 0
        }
    );
    assert!(!namespace.join(STAGING_DIR).exists());

    let kept = namespace.join(KEPT_DIR);
    fs::create_dir_all(kept.join("transfers-v2").join("d-1")).unwrap();
    fs::write(
        kept.join("transfers-v2").join("d-1").join("current"),
        [0u8; 100],
    )
    .unwrap();
    fs::write(kept.join("format-v2"), [0u8; 4]).unwrap();
    // While a live Keep store holds the lease nothing is removed, and that is reported.
    let held = crate::vault::try_lock_file(&kept.join(KEPT_LEASE)).unwrap();
    assert_eq!(
        remove_kept_tree(&namespace),
        Removal {
            freed_bytes: 0,
            failed: 1
        }
    );
    assert!(kept.join("format-v2").exists());
    drop(held);
    let removal = remove_kept_tree(&namespace);
    assert_eq!(
        removal,
        Removal {
            freed_bytes: 104,
            failed: 0
        }
    );
    assert!(!kept.exists());
    assert_eq!(
        remove_kept_tree(&namespace),
        Removal::default(),
        "idempotent"
    );
}

#[cfg(unix)]
#[test]
fn tree_removal_never_follows_a_link_out_of_the_namespace() {
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("precious"), b"keep me").unwrap();
    let namespace = root.path().join("ab12");
    fs::create_dir_all(namespace.join(STAGING_DIR)).unwrap();
    std::os::unix::fs::symlink(&outside, namespace.join(STAGING_DIR).join("link")).unwrap();
    std::os::unix::fs::symlink(&outside, namespace.join(KEPT_DIR)).unwrap();
    remove_scratch(&namespace);
    remove_kept_tree(&namespace);
    assert!(outside.join("precious").exists());
    assert!(
        !namespace.join(KEPT_DIR).exists(),
        "the link itself is removed"
    );
}
