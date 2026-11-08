//! A genuine saved founder source survives a fully drained offline metadata
//! migration. The result remains historical DATA, not a relabeled Current3 owner.
use super::*;
use crate::store::critical_namespaces::current_layout::cold_read::tests::{fixture, ColdFixture};

#[test]
fn genuine_saved_founder_data_remains_readable_after_old_mount_and_sdk_are_drained() {
    let ColdFixture {
        layout,
        source,
        pool,
        group,
        signer,
        mut rng,
        directory,
    } = fixture(0x44c0, false);
    let before = layout
        .read_saved_bootstrap_source_data(&group, signer)
        .unwrap();
    let original_source = before.data().source_id_data();
    let original_records = before.data().view_data().records_frame.to_vec();
    let original_manifest = before.data().view_data().manifest_frame.to_vec();
    drop(before);
    drop(source); // Original cold SDK/read owner, before physical remount.
    drop(layout); // No Scope/Current conversion or live3 policy flip.
    let mut opening = PreparedV2GovernanceLayoutStore::open(
        directory.path(),
        b"genuine task-free cold group reads",
        &mut rng,
        policy(&pool),
    )
    .unwrap();
    opening.retry(&mut rng).unwrap();
    let ready = opening.into_governance_layout_store().unwrap();
    let read = ready
        .read_saved_bootstrap_source_data(&group, signer)
        .unwrap();
    assert_eq!(read.data().source_id_data(), original_source);
    assert_eq!(read.data().view_data().records_frame, original_records);
    assert_eq!(read.data().view_data().manifest_frame, original_manifest);
    read.validate_original(&ready.store).unwrap();
    drop(read);
    // The independently retained4 marker is an original named object even for
    // DATA reads: equal bytes in a replacement object do not renew the mount.
    let marker = directory
        .path()
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(GOVERNANCE_MARKER);
    let raw = fs::read(&marker).unwrap();
    fs::rename(&marker, marker.with_extension("original")).unwrap();
    fs::write(&marker, raw).unwrap();
    assert!(ready
        .read_saved_bootstrap_source_data(&group, signer)
        .is_err());
}
