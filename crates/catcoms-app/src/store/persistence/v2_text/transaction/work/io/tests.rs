//! Bounded native-helper regressions. These do not manufacture an admitted transaction;
//! the paired genuine Stage/Commit tests exercise the permission/lease/gate lifecycle.
use super::*;

#[test]
fn create_caches_identity_before_callback_and_complete_retry_never_calls_it_again() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("frame");
    let mut recorded = None;
    let mut called = false;
    write_complete_only(&path, b"the exact original", &mut recorded, || {
        called = true;
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        Ok(())
    })
    .unwrap();
    assert!(called && recorded.is_some());
    let actual = require_exact(&path, recorded, b"the exact original").unwrap();
    assert_eq!(Some(actual.id), recorded);
    drop(actual);
    write_complete_only(&path, b"the exact original", &mut recorded, || {
        panic!("not a new creation")
    })
    .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"the exact original");
    assert!(
        write_complete_only(&path, b"a changed original", &mut recorded, || panic!(
            "not a new creation"
        ))
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"the exact original");
}

#[test]
fn callback_refusal_keeps_created_file_and_same_id_partial_is_never_resumed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("frame");
    let mut recorded = None;
    assert!(write_complete_only(&path, b"complete", &mut recorded, || Err(refused())).is_err());
    assert!(recorded.is_some());
    assert_eq!(fs::read(&path).unwrap(), b"");
    // Simulate a failed native write on the SAME still-present object. Identity alone
    // cannot authorize appending the missing suffix after that creation File has closed.
    fs::write(&path, b"com").unwrap();
    assert!(matches!(
        session_at(&path, recorded.unwrap()),
        SessionAt::Ours
    ));
    assert!(
        write_complete_only(&path, b"complete", &mut recorded, || panic!("not fresh")).is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"com");
    let mut unknown = None;
    assert!(
        write_complete_only(&path, b"complete", &mut unknown, || panic!("not created")).is_err()
    );
    assert!(unknown.is_none());
    assert_eq!(fs::read(&path).unwrap(), b"com");
}

#[test]
fn equal_bytes_at_a_replacement_native_object_are_not_the_original_incarnation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("frame");
    let mut recorded = None;
    write_complete_only(&path, b"original", &mut recorded, || Ok(())).unwrap();
    let held = require_exact(&path, recorded, b"original").unwrap();
    fs::rename(&path, temp.path().join("old-frame")).unwrap();
    fs::write(&path, b"original").unwrap();
    // Holding the old native object prevents an inode/file-ID reuse coincidence.
    assert!(!matches!(session_at(&path, held.id), SessionAt::Ours));
    assert!(require_exact(&path, recorded, b"original").is_err());
    assert!(sync_exact(&path, recorded, b"original").is_err());
    assert!(
        write_complete_only(&path, b"original", &mut recorded, || panic!("not fresh")).is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"original");
}

#[test]
fn absence_directory_identity_and_exact_frame_bounds_are_not_interchangeable() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let other = temp.path().join("other");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&other).unwrap();
    let held = crate::store::persistence::open_archive_directory(&first).unwrap();
    let id = session_id(&held).unwrap();
    sync_parent(&first, id).unwrap();
    assert!(require_directory(&other, id).is_err());
    assert!(sync_parent(&other, id).is_err());
    assert!(require_absent(&first).is_err());
    let path = temp.path().join("missing");
    require_absent(&path).unwrap();
    let mut recorded = None;
    assert!(write_complete_only(&path, b"", &mut recorded, || panic!("must not open")).is_err());
    assert!(!path.exists() && recorded.is_none());
    fs::write(&path, b"x").unwrap();
    assert!(require_exact(&path, None, b"").is_err());
    assert!(require_exact(&path, None, b"xx").is_err());
    assert!(require_exact(&path, None, b"y").is_err());
    assert!(require_absent(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"x");
}

#[test]
fn artifact_paths_use_only_first_a_closed_roles_and_complete_ids() {
    let base = Path::new("pages");
    for (role, prefix) in [
        (TextArtifactRoleData::ActionChunk, "p-"),
        (TextArtifactRoleData::CipherChunk, "p-"),
        (TextArtifactRoleData::Manifest, "p-"),
        (TextArtifactRoleData::TokenIndex, "p-"),
        (TextArtifactRoleData::Archive, "h-"),
        (TextArtifactRoleData::Retain, "a-"),
    ] {
        assert_eq!(
            artifact_child(base, role, &[0xab; 32])
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap(),
            format!("{prefix}{}.bin", "ab".repeat(32))
        );
    }
    for role in [
        TextArtifactRoleData::Intent,
        TextArtifactRoleData::Signature,
        TextArtifactRoleData::Publication,
    ] {
        assert!(artifact_child(base, role, &[0; 32]).is_err());
    }
    assert!(artifact_child(
        Path::new(&"x".repeat(super::super::super::MAX_PATH)),
        TextArtifactRoleData::Retain,
        &[0; 32]
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_is_neither_absent_nor_an_exact_frame_or_fresh_creation() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    let link = temp.path().join("link");
    fs::write(&target, b"original").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(require_absent(&link).is_err());
    assert!(require_exact(&link, None, b"original").is_err());
    let mut recorded = None;
    assert!(
        write_complete_only(&link, b"original", &mut recorded, || panic!("not created")).is_err()
    );
    assert!(recorded.is_none());
    assert_eq!(fs::read(target).unwrap(), b"original");
}
