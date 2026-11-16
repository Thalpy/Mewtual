//! Real bounded readonly IO only. No Source/Current/lane/workspace-clear authority is faked.
use super::*;

#[test]
fn absent_is_only_not_found_and_never_a_present_empty_or_failed_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("known-control");
    let mut absent = HeldWorkspaceControl::empty();
    absent.capture_metadata(&path, 64).unwrap();
    assert!(absent.is_absent_data());
    fs::write(&path, []).unwrap();
    assert!(absent.require_absent(&path).is_err());
    let mut empty = HeldWorkspaceControl::empty();
    empty.capture_metadata(&path, 64).unwrap();
    assert!(empty.is_present_data());
    assert!(!empty.is_absent_data());
    empty.read_complete(&path).unwrap();
    assert!(empty.raw().unwrap().is_empty());
    assert!(empty.require_absent(&path).is_err());
    let mut oversized = HeldWorkspaceControl::empty();
    fs::write(&path, [1; 65]).unwrap();
    assert!(oversized.capture_metadata(&path, 64).is_err());
    assert!(oversized.is_present_data());
    assert!(!oversized.is_absent_data());
    assert_eq!(fs::read(path).unwrap(), [1; 65]);
}

#[test]
fn metadata_capture_defers_raw_allocation_and_keeps_original_fd_after_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("wal");
    let retained = temp.path().join("held-original");
    fs::write(&path, [3; 300]).unwrap();
    let mut control = HeldWorkspaceControl::empty();
    control.capture_metadata(&path, 300).unwrap();
    assert!(control.raw.is_none());
    assert!(!control.complete);
    let original = control.id.unwrap();
    fs::rename(&path, &retained).unwrap();
    fs::write(&path, [3; 300]).unwrap();
    assert!(control.read_complete(&path).is_err());
    assert_eq!(session_id(control.file().unwrap()).unwrap(), original);
    assert!(control.is_present_data());
    assert!(control.raw.is_none());
    assert_eq!(fs::read(retained).unwrap(), [3; 300]);
}

#[test]
fn raw_read_is_bounded_and_same_size_content_changes_are_not_a_clear_result() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("wal");
    fs::write(&path, [4; 300]).unwrap();
    let mut control = HeldWorkspaceControl::empty();
    control.capture_metadata(&path, 300).unwrap();
    control.read_complete(&path).unwrap();
    assert_eq!(control.raw().unwrap(), [4; 300]);
    assert_eq!(control.raw.as_ref().unwrap().capacity(), 301);
    fs::write(&path, [5; 300]).unwrap();
    assert!(control.require_unchanged(&path).is_err());
    assert_eq!(control.raw().unwrap(), [4; 300]);
    assert_eq!(fs::read(path).unwrap(), [5; 300]);
}

#[cfg(unix)]
#[test]
fn a_link_is_preserved_as_present_refusal() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real-control");
    let path = temp.path().join("wal");
    fs::write(&real, [6; 32]).unwrap();
    std::os::unix::fs::symlink(&real, &path).unwrap();
    let mut control = HeldWorkspaceControl::empty();
    assert!(control.capture_metadata(&path, 64).is_err());
    assert!(control.is_present_data());
    assert!(control.file.is_none());
    assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    assert_eq!(fs::read(real).unwrap(), [6; 32]);
}
