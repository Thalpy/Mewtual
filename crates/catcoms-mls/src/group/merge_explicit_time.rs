//! Real SDK storage evidence for the isolated pending-merge timestamp seam.
//! These unrestricted test graphs are not production private-Add admissions.
use super::*;
use crate::config::{capabilities, CIPHERSUITE};
use crate::persist::snapshot_server;
use openmls_memory_storage::owner_restore::OwnerCheckpointRestoreLimits;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn time() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::new(12_345, 789_000_000))
        .unwrap()
}

fn new_group(owner: &MlsDevice) -> ServerGroup {
    ServerGroup::create_with_lifetime(owner, Lifetime::init(0, u64::MAX)).unwrap()
}

fn stage_add(
    owner: &MlsDevice,
    group: &mut ServerGroup,
    member: &MlsDevice,
) -> (MlsMessageOut, Option<MlsMessageOut>, Option<MlsMessageOut>) {
    let package = KeyPackage::builder()
        .leaf_node_capabilities(capabilities())
        .key_package_lifetime(Lifetime::init(10_000, 20_000))
        .build(
            CIPHERSUITE,
            member.provider(),
            member.signer(),
            member.credential(),
        )
        .unwrap()
        .key_package()
        .clone();
    let output = group
        .group
        .commit_builder()
        .consume_proposal_store(false)
        .force_self_update(true)
        .propose_adds(vec![package])
        .load_psks(owner.provider().storage())
        .unwrap()
        .build_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(),
            |_| true,
        )
        .unwrap()
        .stage_commit(owner.provider())
        .unwrap();
    assert!(output.welcome().is_some());
    assert!(group.group.pending_commit().is_some());
    assert_eq!(group.epoch(), 0);
    output.into_messages()
}

fn stored_message_secrets(owner: &MlsDevice) -> serde_json::Value {
    let rows = owner.provider().storage().values.read().unwrap();
    let mut matches = rows
        .iter()
        .filter(|(key, _)| key.starts_with(b"MessageSecrets"));
    let (_, raw) = matches.next().expect("actual SDK message-secrets row");
    assert!(matches.next().is_none(), "fixture owns exactly one group");
    // Inspect only the timestamp and public shape below. Never print secret rows.
    serde_json::from_slice(raw).unwrap()
}

#[derive(PartialEq, Eq)]
struct SnapshotData<'a> {
    signer: &'a [u8],
    group: &'a [u8],
    rows: BTreeMap<&'a [u8], &'a [u8]>,
}

fn snapshot_data(bytes: &[u8]) -> SnapshotData<'_> {
    let mut outer = catcoms_wire::Decoder::new(bytes);
    let mut storage = catcoms_wire::Decoder::new(outer.get_bytes().unwrap());
    let mut rows = BTreeMap::new();
    for _ in 0..storage.get_u32().unwrap() {
        let key = storage.get_bytes().unwrap();
        let value = storage.get_bytes().unwrap();
        assert!(rows.insert(key, value).is_none(), "duplicate fixture row");
    }
    storage.finish().unwrap();
    let signer = outer.get_bytes().unwrap();
    let group = outer.get_bytes().unwrap();
    outer.finish().unwrap();
    SnapshotData {
        signer,
        group,
        rows,
    }
}

#[test]
fn explicit_merge_persists_exact_injected_timestamp_with_actual_new_epoch() {
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut group = new_group(&owner);
    let (commit, welcome, group_info) = stage_add(&owner, &mut group, &member);
    group
        .group
        .merge_pending_commit_with_time(owner.provider(), time())
        .unwrap();
    assert_eq!(group.epoch(), 1);
    assert_eq!(group.member_count(), 2);
    assert!(group.group.pending_commit().is_none());
    assert!(group
        .group
        .members()
        .any(|actual| actual.signature_key == member.public_key_ref()));
    assert!(group_info.is_some());
    assert!(!commit.tls_serialize_detached().unwrap().is_empty());
    let joined =
        ServerGroup::join(&member, &welcome.unwrap().tls_serialize_detached().unwrap()).unwrap();
    assert!(
        group.epoch_authenticator_id() == joined.epoch_authenticator_id(),
        "genuine Welcome must select the same merged epoch"
    );
    assert!(
        group
            .channel_secret(&owner, catcoms_wire::DocType::Channel, 19)
            .unwrap()
            == joined
                .channel_secret(&member, catcoms_wire::DocType::Channel, 19)
                .unwrap(),
        "actual owner and joiner must derive the same channel secret"
    );
    let stored = stored_message_secrets(&owner);
    assert_eq!(
        stored["message_secrets"]["added_at"],
        serde_json::to_value(time()).unwrap(),
        "persisted timestamp must use the trusted original operation time"
    );
    assert_eq!(stored["past_epoch_trees"].as_array().unwrap().len(), 0);
    assert!(MlsGroup::text_add_merge_time_layout_bytes() >= std::mem::size_of::<SystemTime>());
    // A later retry on an already operational graph cannot retimestamp it.
    let before = snapshot_server(&owner, &group).unwrap();
    let (cold_owner, cold_group) = crate::restore_server(&before).unwrap();
    assert_eq!(
        cold_group.epoch_authenticator_id(),
        group.epoch_authenticator_id()
    );
    let cold_snapshot = snapshot_server(&cold_owner, &cold_group).unwrap();
    // A restored map has a fresh hash seed. Compare complete exact key/value
    // bytes and outer bindings rather than an incidental iteration order.
    // Boolean assertions keep private row contents out of failure diagnostics.
    assert!(
        snapshot_data(&cold_snapshot) == snapshot_data(&before),
        "cold round-trip must preserve every original row and outer binding"
    );
    group
        .group
        .merge_pending_commit_with_time(owner.provider(), UNIX_EPOCH)
        .unwrap();
    assert!(
        snapshot_server(&owner, &group).unwrap().as_slice() == before.as_slice(),
        "already merged retry must preserve the complete snapshot"
    );
}

#[test]
fn explicit_merge_operational_no_pending_is_exact_legacy_noop() {
    let owner = MlsDevice::generate().unwrap();
    let mut group = new_group(&owner);
    let before = snapshot_server(&owner, &group).unwrap();
    // No pending operation means no timestamp is encoded or validated. Preserve
    // this legacy behavior even for a representable time before the Unix epoch.
    let before_unix = UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap();
    group
        .group
        .merge_pending_commit_with_time(owner.provider(), before_unix)
        .unwrap();
    group.group.merge_pending_commit(owner.provider()).unwrap();
    assert!(
        snapshot_server(&owner, &group).unwrap().as_slice() == before.as_slice(),
        "operational no-op must preserve the complete snapshot"
    );
    assert_eq!(group.epoch(), 0);
    assert_eq!(group.member_count(), 1);
}

#[test]
fn explicit_merge_keeps_legacy_partial_failure_and_noop_retry_semantics() {
    for explicit in [false, true] {
        let owner = MlsDevice::generate().unwrap();
        let member = MlsDevice::generate().unwrap();
        let mut group = new_group(&owner);
        let _outputs = stage_add(&owner, &mut group, &member);
        let before = snapshot_server(&owner, &group).unwrap();
        // Install the existing permanent negative storage fence after staging.
        // Its first write refusal exercises the actual SDK failure edge, not a
        // mocked success flag or replacement storage implementation.
        let closed = owner.provider().storage().begin_owner_checkpoint_restore(
            group.group_id_ref(),
            owner.public_key_ref(),
            OwnerCheckpointRestoreLimits {
                max_typed_bytes: 8 * 1024 * 1024,
                max_work: 8 * 1024 * 1024,
            },
        );
        // Staged state and initialized old ratchets are outside the original
        // restore grammar. Installation still permanently fences this provider
        // before that strict scan refuses; no fake successful guard is needed.
        assert!(closed.is_err());
        let result = if explicit {
            group
                .group
                .merge_pending_commit_with_time(owner.provider(), time())
        } else {
            group.group.merge_pending_commit(owner.provider())
        };
        assert!(matches!(
            result,
            Err(MergePendingCommitError::MergeCommitError(
                MergeCommitError::StorageError(_)
            ))
        ));
        assert!(
            snapshot_server(&owner, &group).unwrap().as_slice() == before.as_slice(),
            "refused first storage write must preserve original rows"
        );
        assert_eq!(group.epoch(), 0);
        assert_eq!(group.member_count(), 1);
        // The SDK consumes Pending BEFORE that fallible write. A later Ok(noop)
        // therefore is not evidence that the original Add merged. An operation
        // owner must retain its sticky failed/uncertain phase and cached outputs;
        // this timestamp API deliberately does not provide recovery authority.
        assert!(group.group.pending_commit().is_none());
        group
            .group
            .merge_pending_commit_with_time(owner.provider(), time())
            .unwrap();
        assert_eq!(group.epoch(), 0);
        assert!(
            snapshot_server(&owner, &group).unwrap().as_slice() == before.as_slice(),
            "no-op after failed merge cannot establish a new saved epoch"
        );
    }
}
