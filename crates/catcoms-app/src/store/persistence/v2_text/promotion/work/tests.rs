//! Low-level file-writer controls only. Actual source/Current/lease admissions are exercised
//! by the paired genuine three-family tests, not fabricated by this helper-level unit test.
use super::*;

fn write(path: &Path, bytes: &[u8], recorded: &mut Option<SessionId>) -> Result<(), AppError> {
    write_same(
        path,
        bytes,
        recorded,
        V2TextPromotionTestCut::SeedCreated,
        &mut PromotionTestHooks::default(),
    )
}

#[test]
fn reopening_matching_native_id_never_renews_partial_content_write_permission() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("created-control");
    let original = [41; 96];
    let mut recorded = None;
    write(&path, &original, &mut recorded).unwrap();
    let id = recorded.unwrap();
    // Deterministic same-inode truncation represents a prior short write. Even this exact
    // native ID and matching prefix cannot make warm creation authority cover new writes.
    fs::write(&path, &original[..37]).unwrap();
    assert!(matches!(session_at(&path, id), SessionAt::Ours));
    assert!(write(&path, &original, &mut recorded).is_err());
    assert_eq!(fs::read(&path).unwrap(), original[..37]);
    assert_eq!(recorded, Some(id));
}

#[test]
fn unowned_equal_complete_frame_is_not_adopted_by_fresh_creation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("foreign-control");
    let bytes = [42; 96];
    fs::write(&path, bytes).unwrap();
    let mut recorded = None;
    assert!(write(&path, &bytes, &mut recorded).is_err());
    assert!(recorded.is_none());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn complete_original_candidate_can_be_resynced_without_new_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("complete-control");
    let bytes = [43; 96];
    let mut recorded = None;
    write(&path, &bytes, &mut recorded).unwrap();
    let original = recorded;
    write(&path, &bytes, &mut recorded).unwrap();
    assert_eq!(recorded, original);
    assert_eq!(fs::read(path).unwrap(), bytes);
}
