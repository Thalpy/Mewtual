use super::*;
use crate::{restore_server, snapshot_server, MlsDevice};
use openmls_traits::OpenMlsProvider;
use std::cell::Cell;
use std::collections::BTreeSet;

/// One genuine batched MLS commit keeps the large-catalog test focused on enumeration instead
/// of rebuilding the epoch hundreds of times. No network/admission/bridge property is claimed.
fn roster(added: usize) -> (MlsDevice, ServerGroup, Vec<DeviceId>) {
    let owner = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    let gid = group.group_id();
    let mut ids = Vec::with_capacity(added);
    let packages: Vec<_> = (0..added)
        .map(|index| {
            let device = MlsDevice::generate().unwrap();
            ids.push(device.device_id());
            device
                .key_package_for_invite(&gid, (index as u128 + 1).to_be_bytes())
                .unwrap()
        })
        .collect();
    if !packages.is_empty() {
        let messages = group
            .group
            .add_members(owner.provider(), owner.signer(), &packages)
            .unwrap();
        group.group.merge_pending_commit(owner.provider()).unwrap();
        drop(messages);
    }
    assert_eq!(group.member_count(), added + 1);
    (owner, group, ids)
}

/// Alter the real persisted leaf representation, then restore through the production loader.
/// Authentication of a local snapshot does not by itself constrain every restored key shape.
fn tamper_stored_leaf(owner: &MlsDevice, leaf: u32, field: &str, length: usize) {
    let mut stored = owner.provider().storage().values.write().unwrap();
    let mut trees = stored
        .iter_mut()
        .filter(|(key, _)| key.starts_with(b"Tree"));
    let (_, bytes) = trees.next().expect("one actual stored MLS tree");
    assert!(trees.next().is_none());
    let mut tree: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let pointer = format!("/tree/leaf_nodes/{leaf}/node/payload/{field}/vec");
    let value = tree.pointer_mut(&pointer).expect("exact stored leaf shape");
    *value = serde_json::json!(vec![3u8; length]);
    *bytes = serde_json::to_vec(&tree).unwrap();
}

#[test]
fn member_catalog_native_count_reuses_observation_until_a_commit_changes_epoch() {
    let owner = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    assert!(group.current_member_catalog().is_none());
    assert_eq!(group.member_count(), 1);
    let original = group.current_member_catalog().unwrap();
    assert_eq!(group.member_count(), 1);
    assert!(Arc::ptr_eq(
        &original.0,
        &group.current_member_catalog().unwrap().0
    ));

    let joiner = MlsDevice::generate().unwrap();
    group
        .stage_add(&owner, joiner.key_package().unwrap())
        .unwrap();
    assert_eq!(group.member_count(), 1);
    assert!(Arc::ptr_eq(
        &original.0,
        &group.current_member_catalog().unwrap().0
    ));
    group.abort_staged(&owner).unwrap();
    assert_eq!(group.member_count(), 1);
    group
        .add_member(&owner, joiner.key_package().unwrap())
        .unwrap();
    assert!(group.current_member_catalog().is_none());
    assert_eq!(group.member_count(), 2);
    assert!(!Arc::ptr_eq(
        &original.0,
        &group.current_member_catalog().unwrap().0
    ));
}

#[test]
fn member_catalog_covers_real_mls_sources_beyond_both_transport_and_descriptor_limits() {
    let (owner, group, ids) = roster(513);
    let view = group.current_member_catalog().unwrap();
    let mut cursor = view.start();
    let mut seen = BTreeSet::new();
    let mut turns = 0;
    loop {
        let page = view
            .page(&group, &cursor, owner.device_id(), ids[0])
            .unwrap();
        assert!(page.entries.len() <= PAGE_MEMBERS);
        assert!(page.next.next - cursor.next <= MAX_MEMBER_CATALOG_PROBES as u64);
        for entry in &page.entries {
            assert!(
                seen.insert(entry.device_id()),
                "source repeated across pages"
            );
            assert_eq!(
                entry.device_id(),
                DeviceId::from_public_key_bytes(entry.signature_key())
            );
        }
        turns += 1;
        assert!(turns <= 514, "enumeration must make finite progress");
        cursor = page.next;
        if page.complete {
            break;
        }
    }
    let expected: BTreeSet<_> = ids.iter().skip(1).copied().collect();
    assert_eq!(seen, expected);
    assert!(seen.contains(ids.last().unwrap()));
    assert!(view.0.last_leaf.unwrap() > 512);
    assert_eq!(
        cursor.occupied, 514,
        "count requester/helper before exclusion"
    );
    let done = view
        .page(&group, &cursor, owner.device_id(), ids[0])
        .unwrap();
    assert!(done.complete && done.entries.is_empty());
}

#[test]
fn member_catalog_holes_yield_bounded_turns_and_original_cursor_can_retry_local_busy() {
    let (owner, mut group, ids) = roster(129);
    let removed: Vec<_> = (1..129).map(LeafNodeIndex::new).collect();
    let messages = group
        .group
        .remove_members(owner.provider(), owner.signer(), &removed)
        .unwrap();
    group.group.merge_pending_commit(owner.provider()).unwrap();
    drop(messages);
    assert!(group.current_member_catalog().is_none());
    assert_eq!(group.member_count(), 2);
    let view = group.current_member_catalog().unwrap();
    let start = view.start();
    let first = view
        .page(&group, &start, owner.device_id(), owner.device_id())
        .unwrap();
    assert!(!first.complete && first.entries.is_empty());
    assert_eq!(first.next.next, 64);
    // Proposing a page consumes no caller cursor. The same proposed work survives a proven
    // local capacity refusal; later helper/variant owners maintain their independent cursors.
    let retry = view
        .page(&group, &start, owner.device_id(), owner.device_id())
        .unwrap();
    assert_eq!(retry.next.next, first.next.next);
    assert_eq!(start.next, 0);
    let second = view
        .page(&group, &first.next, owner.device_id(), owner.device_id())
        .unwrap();
    assert!(!second.complete && second.entries.is_empty());
    assert_eq!(second.next.next, 128);
    let last = view
        .page(&group, &second.next, owner.device_id(), owner.device_id())
        .unwrap();
    assert!(last.complete);
    assert_eq!(last.entries.len(), 1);
    assert_eq!(last.entries[0].device_id(), ids[128]);
    assert_eq!(last.next.occupied, 2);
}

#[test]
fn member_catalog_epoch_and_same_epoch_restored_instance_each_refuse_old_entries_and_cursors() {
    let (owner, mut group, ids) = roster(2);
    let view = group.current_member_catalog().unwrap();
    let cursor = view.start();
    let page = view
        .page(&group, &cursor, owner.device_id(), ids[0])
        .unwrap();
    let entry = &page.entries[0];
    entry.validate(&group).unwrap();
    let snapshot = snapshot_server(&owner, &group).unwrap();
    let (_restored_owner, restored) = restore_server(&snapshot).unwrap();
    assert_eq!(restored.epoch(), group.epoch());
    assert_eq!(restored.group_id(), group.group_id());
    assert_eq!(entry.validate(&restored), Err(MemberCatalogError::Stale));
    assert!(matches!(
        view.page(&restored, &cursor, owner.device_id(), ids[0]),
        Err(MemberCatalogError::Stale)
    ));

    let new_device = MlsDevice::generate().unwrap();
    group
        .add_member(&owner, new_device.key_package().unwrap())
        .unwrap();
    assert!(group.current_member_catalog().is_none());
    assert_eq!(entry.validate(&group), Err(MemberCatalogError::Stale));
    assert!(matches!(
        view.page(&group, &cursor, owner.device_id(), ids[0]),
        Err(MemberCatalogError::Stale)
    ));
    assert_eq!(group.member_count(), 4);
    let current = group.current_member_catalog().unwrap();
    assert!(matches!(
        current.page(&group, &cursor, owner.device_id(), ids[0]),
        Err(MemberCatalogError::Stale)
    ));
}

#[test]
fn member_catalog_removed_receiver_retires_old_view_without_granting_active_membership() {
    let owner = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    let receiver = MlsDevice::generate().unwrap();
    let outcome = group
        .add_member(&owner, receiver.key_package().unwrap())
        .unwrap();
    let mut receiving = ServerGroup::join(&receiver, &outcome.welcome).unwrap();
    assert_eq!(receiving.member_count(), 2);
    let old = receiving.current_member_catalog().unwrap();
    let cursor = old.start();

    let staged = group.stage_remove(&owner, &receiver.device_id()).unwrap();
    group.merge_staged_self(&owner).unwrap();
    receiving
        .process_incoming(&receiver, &staged.commit)
        .unwrap();
    assert!(!receiving.is_active());
    assert!(receiving.current_member_catalog().is_none());
    assert!(matches!(
        old.page(&receiving, &cursor, receiver.device_id(), owner.device_id()),
        Err(MemberCatalogError::Stale)
    ));
    // Catalogs expose public tree metadata only. Even on an inactive removed receiver, a new
    // observation can describe that tree; it cannot restore file, route or membership authority.
    assert_eq!(receiving.member_count(), 1);
    let current = receiving.current_member_catalog().unwrap();
    let page = current
        .page(
            &receiving,
            &current.start(),
            receiver.device_id(),
            receiver.device_id(),
        )
        .unwrap();
    assert!(page.complete);
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].device_id(), owner.device_id());
    assert!(!receiving.is_active());
}

#[test]
fn member_catalog_restored_oversized_keys_refuse_before_indexed_member_copies() {
    for field in ["encryption_key/key", "signature_key/value"] {
        let (owner, group, ids) = roster(1);
        let leaf = group
            .group
            .members()
            .find(|member| DeviceId::from_public_key_bytes(&member.signature_key) == ids[0])
            .unwrap()
            .index
            .u32();
        tamper_stored_leaf(&owner, leaf, field, 4096);
        // Real serialization/restoration, not a forged catalog flag. Native observation can
        // traverse the restored roster; the bounded page must refuse before any indexed copy.
        let snapshot = snapshot_server(&owner, &group).unwrap();
        let (restored_owner, restored) = restore_server(&snapshot).unwrap();
        assert_eq!(restored.member_count(), 2);
        let view = restored.current_member_catalog().unwrap();
        let copies = Cell::new(0);
        let result = view.page_with(
            &restored,
            &view.start(),
            restored_owner.device_id(),
            restored_owner.device_id(),
            &mut || copies.set(copies.get() + 1),
        );
        assert!(matches!(result, Err(MemberCatalogError::Unsupported)));
        assert_eq!(
            copies.get(),
            0,
            "unsupported keys must refuse BEFORE copying"
        );
    }
}

#[test]
fn member_catalog_restored_credential_bound_is_checked_before_indexed_copy() {
    for length in [
        MAX_MEMBER_CATALOG_CREDENTIAL_BYTES,
        MAX_MEMBER_CATALOG_CREDENTIAL_BYTES + 1,
    ] {
        let (owner, group, ids) = roster(1);
        let leaf = group
            .group
            .members()
            .find(|member| DeviceId::from_public_key_bytes(&member.signature_key) == ids[0])
            .unwrap()
            .index
            .u32();
        tamper_stored_leaf(
            &owner,
            leaf,
            "credential/serialized_credential_content",
            length,
        );
        let snapshot = snapshot_server(&owner, &group).unwrap();
        let (_restored_owner, restored) = restore_server(&snapshot).unwrap();
        assert_eq!(restored.member_count(), 2);
        let view = restored.current_member_catalog().unwrap();
        let copies = Cell::new(0);
        let result = view.entry(&restored, leaf, &mut || copies.set(copies.get() + 1));
        if length == MAX_MEMBER_CATALOG_CREDENTIAL_BYTES {
            assert_eq!(result.unwrap().device_id(), ids[0]);
            assert_eq!(copies.get(), 1);
        } else {
            assert!(matches!(result, Err(MemberCatalogError::Unsupported)));
            assert_eq!(
                copies.get(),
                0,
                "oversized credential copied before refusal"
            );
        }
    }
}
