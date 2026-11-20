//! Actual read-only filesystem boundaries, without any fabricated Current or Saved role.
use super::*;

#[test]
fn bounded_control_read_rejects_missing_empty_oversize_and_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let path = fixed_child(temp.path(), "head.bin").unwrap();
    assert!(read_control(&path, 64).is_err());
    assert!(!path.exists());
    fs::write(&path, []).unwrap();
    assert!(read_control(&path, 64).is_err());
    fs::write(&path, [7; 65]).unwrap();
    assert!(read_control(&path, 64).is_err());
    fs::write(&path, [8; 64]).unwrap();
    let original = read_control(&path, 64).unwrap();
    assert_eq!(original.frame.as_slice(), &[8; 64]);
    assert_eq!(original.frame.capacity(), 65);
    let renamed = temp.path().join("retained-original");
    fs::rename(&path, renamed).unwrap();
    fs::write(&path, [8; 64]).unwrap();
    assert!(require_named(&path, &original).is_err());
    assert_eq!(read_control(&path, 64).unwrap().frame.as_slice(), &[8; 64]);
}

#[test]
fn only_closed_literal_and_complete_archive_names_are_constructed() {
    let base = Path::new("original-target");
    assert!(fixed_child(base, "../head.bin").is_err());
    assert!(fixed_child(base, "unknown.seed").is_err());
    let path = archive_child(base, &[0xab; 32]).unwrap();
    let leaf = path.file_name().unwrap().to_str().unwrap();
    assert_eq!(leaf.len(), 70);
    assert_eq!(leaf, format!("h-{}.bin", "ab".repeat(32)));
    assert!(archive_child(&PathBuf::from("a".repeat(MAX_TEXT_PATH)), &[0; 32]).is_err());
}

#[cfg(unix)]
#[test]
fn link_is_preserved_and_never_treated_as_an_original_regular_control() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    fs::write(&original, [9; 64]).unwrap();
    let path = fixed_child(temp.path(), "head.bin").unwrap();
    std::os::unix::fs::symlink(&original, &path).unwrap();
    assert!(read_control(&path, 64).is_err());
    assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    assert_eq!(fs::read(original).unwrap(), [9; 64]);
}
