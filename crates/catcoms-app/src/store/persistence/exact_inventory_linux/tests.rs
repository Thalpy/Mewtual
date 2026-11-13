//! Actual Linux directory descriptors and names, including hostile local debris.
use super::*;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;

const NAMES: [&str; 10] = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];

struct Fixture {
    path: std::path::PathBuf,
    file: File,
    id: SessionId,
    // Keep the original directory open through every proof; close before cleanup.
    _temp: tempfile::TempDir,
}
impl Fixture {
    fn new(names: &[&str]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fixed");
        fs::create_dir(&path).unwrap();
        for name in names {
            fs::write(path.join(name), b"retained original").unwrap();
        }
        let file = super::super::open_archive_directory(&path).unwrap();
        let id = catcoms_storage::kept::session_id(&file).unwrap();
        Self {
            path,
            file,
            id,
            _temp: temp,
        }
    }
    fn check(&self, names: &[&str], pages: bool) -> Result<(), AppError> {
        require_exact_names((&self.path, &self.file, self.id), names, pages)
    }
}

#[test]
fn all_ten_slots_repeat_through_the_same_held_directory_without_shared_offset() {
    let f = Fixture::new(&NAMES);
    for _ in 0..4 {
        f.check(&NAMES, false).unwrap();
    }
    assert_eq!(fs::read(f.path.join("j")).unwrap(), b"retained original");
}

#[test]
fn empty_missing_and_unexpected_slots_have_distinct_exact_results() {
    let empty = Fixture::new(&[]);
    empty.check(&[], false).unwrap();
    assert!(empty.check(&["missing"], false).is_err());
    let f = Fixture::new(&["a"]);
    assert!(f.check(&[], false).is_err());
    assert!(f.check(&["a", "missing"], false).is_err());
    f.check(&["a"], false).unwrap();
}

#[test]
fn invalid_duplicate_and_oversized_specifications_never_become_a_profile() {
    let f = Fixture::new(&["a"]);
    for names in [
        &["a", "a"][..],
        &[""][..],
        &["."][..],
        &[".."][..],
        &["a/b"][..],
        &["a\0b"][..],
        &["a"; 11][..],
    ] {
        assert!(f.check(names, false).is_err());
    }
    let long = "x".repeat(256);
    assert!(f.check(&[&long], false).is_err());
    f.check(&["a"], false).unwrap();
}

#[test]
fn unknown_non_unicode_bytes_are_preserved_and_never_normalized() {
    let f = Fixture::new(&["a"]);
    let unknown = f.path.join(OsStr::from_bytes(b"private-\xff"));
    fs::write(&unknown, b"private bytes").unwrap();
    assert!(f.check(&["a"], false).is_err());
    assert_eq!(fs::read(unknown).unwrap(), b"private bytes");
    assert_eq!(fs::read(f.path.join("a")).unwrap(), b"retained original");
}

#[test]
fn known_symlink_fifo_and_directory_are_not_regular_files() {
    for kind in 0..3 {
        let f = Fixture::new(&[]);
        let path = f.path.join("a");
        match kind {
            0 => {
                let outside = f._temp.path().join("outside");
                fs::write(&outside, b"do not follow").unwrap();
                symlink(outside, &path).unwrap();
            }
            1 => rustix::fs::mkfifoat(&f.file, c"a", Mode::RUSR | Mode::WUSR).unwrap(),
            _ => fs::create_dir(&path).unwrap(),
        }
        assert!(f.check(&["a"], false).is_err());
        assert!(fs::symlink_metadata(path).is_ok());
    }
}

#[test]
fn pages_directory_requires_its_explicit_name_and_kind_exception() {
    let f = Fixture::new(&["a"]);
    fs::create_dir(f.path.join("text.pages")).unwrap();
    assert!(f.check(&["a", "text.pages"], false).is_err());
    f.check(&["a", "text.pages"], true).unwrap();
    assert!(f.check(&["a"], true).is_err());
    fs::remove_dir(f.path.join("text.pages")).unwrap();
    fs::write(f.path.join("text.pages"), b"wrong kind").unwrap();
    assert!(f.check(&["a", "text.pages"], true).is_err());
}

#[test]
fn unknown_subtrees_are_refused_without_traversal_or_cleanup() {
    let f = Fixture::new(&["a"]);
    let subtree = f.path.join("private");
    fs::create_dir(&subtree).unwrap();
    for index in 0..64 {
        fs::write(subtree.join(format!("held-{index}")), b"keep").unwrap();
    }
    assert!(f.check(&["a"], false).is_err());
    assert_eq!(fs::read(subtree.join("held-63")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(subtree).unwrap().count(), 64);
}

#[test]
fn replaced_directory_name_before_scan_cannot_adopt_the_new_directory() {
    let f = Fixture::new(&["a"]);
    let old = f._temp.path().join("original");
    fs::rename(&f.path, &old).unwrap();
    fs::create_dir(&f.path).unwrap();
    fs::write(f.path.join("a"), b"replacement").unwrap();
    assert!(f.check(&["a"], false).is_err());
    assert_eq!(fs::read(old.join("a")).unwrap(), b"retained original");
    assert_eq!(fs::read(f.path.join("a")).unwrap(), b"replacement");
}

#[test]
fn replaced_directory_after_complete_enumeration_fails_the_last_named_proof() {
    let f = Fixture::new(&["a"]);
    let path = f.path.clone();
    let old = f._temp.path().join("original");
    let old_for_hook = old.clone();
    AFTER_ENUMERATION.with(|slot| {
        assert!(slot
            .borrow_mut()
            .replace(Box::new(move || {
                fs::rename(&path, old_for_hook).unwrap();
                fs::create_dir(&path).unwrap();
                fs::write(path.join("a"), b"replacement").unwrap();
            }))
            .is_none());
    });
    assert!(f.check(&["a"], false).is_err());
    assert!(AFTER_ENUMERATION.with(|slot| slot.borrow().is_none()));
    assert_eq!(fs::read(old.join("a")).unwrap(), b"retained original");
    assert_eq!(fs::read(f.path.join("a")).unwrap(), b"replacement");
}

#[test]
fn many_unknown_slots_do_not_trigger_collection_or_remove_any_file() {
    let f = Fixture::new(&NAMES);
    for index in 0..128 {
        fs::write(f.path.join(format!("private-{index}")), b"preserve").unwrap();
    }
    assert!(f.check(&NAMES, false).is_err());
    assert_eq!(fs::read_dir(&f.path).unwrap().count(), 138);
    assert_eq!(fs::read(f.path.join("private-127")).unwrap(), b"preserve");
}

#[test]
fn symlinked_directory_name_is_not_the_original_directory_even_with_same_bytes() {
    let f = Fixture::new(&["a"]);
    let old = f._temp.path().join("original");
    fs::rename(&f.path, &old).unwrap();
    symlink(&old, &f.path).unwrap();
    assert!(f.check(&["a"], false).is_err());
    assert_eq!(fs::read(old.join("a")).unwrap(), b"retained original");
}

#[test]
fn application_scratch_invoice_includes_the_actual_target_layouts() {
    assert!(FIXED_INVENTORY_BYTES >= BUFFER_BYTES + 2 * size_of::<Stat>());
    assert!(FIXED_INVENTORY_BYTES <= 16 * 1024);
}
