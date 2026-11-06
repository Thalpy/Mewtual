use super::*;

#[test]
fn named_directory_proof_accepts_original_and_refuses_identical_empty_replacement() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("original-directory");
    fs::create_dir(&path).unwrap();
    let held = open_archive_directory(&path).unwrap();
    let id = catcoms_storage::kept::session_id(&held).unwrap();
    assert!(current_directory_at(&path, id).unwrap());
    // Contents/absence of contents prove nothing: retain the original handle so its
    // object number cannot be reused, then create another equally empty directory.
    fs::rename(&path, fixture.path().join("displaced-original")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(!current_directory_at(&path, id).unwrap());
    assert_eq!(catcoms_storage::kept::session_id(&held).unwrap(), id);
}

#[test]
fn missing_or_regular_file_name_never_proves_a_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("original-directory");
    fs::create_dir(&path).unwrap();
    let held = open_archive_directory(&path).unwrap();
    let id = catcoms_storage::kept::session_id(&held).unwrap();
    fs::rename(&path, fixture.path().join("displaced-original")).unwrap();
    assert!(current_directory_at(&path, id).is_err());
    fs::write(&path, b"not a directory").unwrap();
    assert!(current_directory_at(&path, id).is_err());
}
