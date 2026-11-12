//! Syntactic closed-path allocation tests require no filesystem or fake source.
use super::*;
#[test]
fn closed_fixed_child_preallocates_complete_near_cap_path_and_refuses_excess() {
    let leaf = "text.wal";
    let base = PathBuf::from("x".repeat(MAX_WORKSPACE_PATH - 1 - leaf.len()));
    let path = fixed_child(&base, leaf).unwrap();
    assert_eq!(path.as_os_str().len(), MAX_WORKSPACE_PATH);
    assert!(path.capacity() <= MAX_WORKSPACE_PATH);
    let excess = PathBuf::from("x".repeat(MAX_WORKSPACE_PATH - leaf.len()));
    assert!(fixed_child(&excess, leaf).is_err());
    let oversized = PathBuf::from("x".repeat(MAX_WORKSPACE_PATH + 1));
    assert!(fixed_child(&oversized, leaf).is_err());
    assert!(fixed_child(Path::new("known"), "unsupported/control").is_err());
}
#[test]
fn retired_closed_hex_child_preallocates_complete_cap_and_preserves_kind_names() {
    let base = PathBuf::from("x".repeat(MAX_WORKSPACE_PATH - 71));
    for kind in [
        RetirementKindData::AcceptedIndex,
        RetirementKindData::TokenIndex,
        RetirementKindData::PublicationIndex,
        RetirementKindData::Intent,
        RetirementKindData::Signature,
    ] {
        let path = super::read::retired_path(&base, kind, &[0x7a; 32]).unwrap();
        assert_eq!(path.as_os_str().len(), MAX_WORKSPACE_PATH);
        assert!(path.capacity() <= MAX_WORKSPACE_PATH);
    }
    let excess = PathBuf::from("x".repeat(MAX_WORKSPACE_PATH - 70));
    assert!(super::read::retired_path(&excess, RetirementKindData::Intent, &[1; 32]).is_err());
    assert!(super::read::retired_path(
        &PathBuf::from("x".repeat(MAX_WORKSPACE_PATH + 1)),
        RetirementKindData::Signature,
        &[1; 32]
    )
    .is_err());
}

#[test]
fn complete_path_capacity_arithmetic_refuses_overflow_before_allocation() {
    assert!(closed_child_capacity_data(usize::MAX, 0).is_err());
    assert!(closed_child_capacity_data(0, usize::MAX).is_err());
    assert!(closed_child_capacity_data(usize::MAX - 1, 4).is_err());
    assert_eq!(
        closed_child_capacity_data(MAX_WORKSPACE_PATH - 71, 70).unwrap(),
        MAX_WORKSPACE_PATH
    );
}
