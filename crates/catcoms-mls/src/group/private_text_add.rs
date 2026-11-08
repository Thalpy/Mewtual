//! Genuine SDK coverage for the narrow serial builder, not a private paid
//! operation or a grant to mutate Mewtual's closed production graph.
use super::*;
use crate::config::{capabilities, CIPHERSUITE};
use crate::persist::snapshot_server;
use openmls::schedule::PreSharedKeyId;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn time() -> SystemTime {
    UNIX_EPOCH.checked_add(Duration::from_secs(12_345)).unwrap()
}

fn group(owner: &MlsDevice) -> ServerGroup {
    ServerGroup::create_with_lifetime(owner, Lifetime::init(0, u64::MAX)).unwrap()
}

fn package(member: &MlsDevice) -> KeyPackage {
    KeyPackage::builder()
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
        .clone()
}

fn serial_add(owner: &MlsDevice, group: &mut ServerGroup, member: &MlsDevice) {
    let epoch = group.epoch();
    let count = group.member_count();
    let bundle = group
        .group
        .commit_builder()
        .consume_proposal_store(true)
        .force_self_update(true)
        .propose_adds([package(member)])
        .load_psks(owner.provider().storage())
        .unwrap()
        .build_private_text_add_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(),
        )
        .unwrap()
        .stage_commit(owner.provider())
        .unwrap();
    let (commit, welcome, group_info) = bundle.into_messages();
    assert_eq!(group.epoch(), epoch);
    assert!(group.group.pending_commit().is_some());
    assert!(group_info.is_some());
    assert!(!commit.tls_serialize_detached().unwrap().is_empty());
    group
        .group
        .merge_pending_commit_with_time(owner.provider(), time())
        .unwrap();
    assert_eq!(group.epoch(), epoch + 1);
    assert_eq!(group.member_count(), count + 1);
    let joined =
        ServerGroup::join(member, &welcome.unwrap().tls_serialize_detached().unwrap()).unwrap();
    assert!(
        joined.epoch_authenticator_id() == group.epoch_authenticator_id(),
        "serial Commit and real Welcome must select the same epoch"
    );
    assert!(
        group
            .channel_secret(owner, catcoms_wire::DocType::Channel, 73)
            .unwrap()
            == joined
                .channel_secret(member, catcoms_wire::DocType::Channel, 73)
                .unwrap(),
        "actual serial owner and joiner must agree on the channel secret"
    );
}

#[test]
fn serial_one_add_really_stages_merges_and_preserves_ordinary_builder_behavior() {
    let owner = MlsDevice::generate().unwrap();
    let first = MlsDevice::generate().unwrap();
    let mut selected = group(&owner);
    serial_add(&owner, &mut selected, &first);

    // The separately named strategy is not a global mode. Ordinary explicit-
    // clock building still accepts multiple Adds and uses its original runtime.
    let others: Vec<_> = (0..2).map(|_| MlsDevice::generate().unwrap()).collect();
    let bundle = selected
        .group
        .commit_builder()
        .force_self_update(true)
        .propose_adds(others.iter().map(package))
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
    assert!(bundle.welcome().is_some());
    selected
        .group
        .merge_pending_commit_with_time(owner.provider(), time())
        .unwrap();
    assert_eq!(selected.member_count(), 4);
    assert_eq!(selected.epoch(), 2);
}

#[test]
fn private_shape_refusals_preserve_original_sdk_rows_and_pending_state() {
    // zero/two Adds, missing forced update, nonconsuming queue policy, Remove
    // instead of Add, and even same-valued nondefault leaf parameters refuse.
    for mode in 0..6 {
        let owner = MlsDevice::generate().unwrap();
        let member = MlsDevice::generate().unwrap();
        let extra = MlsDevice::generate().unwrap();
        let mut selected = group(&owner);
        let before = snapshot_server(&owner, &selected).unwrap();
        let packages = match mode {
            0 | 4 => Vec::new(),
            1 => vec![package(&member), package(&extra)],
            _ => vec![package(&member)],
        };
        let mut builder = selected
            .group
            .commit_builder()
            .consume_proposal_store(mode != 3)
            .force_self_update(mode != 2)
            .propose_adds(packages);
        if mode == 4 {
            builder = builder.propose_removals([LeafNodeIndex::new(0)]);
        }
        if mode == 5 {
            builder = builder.leaf_node_parameters(
                LeafNodeParameters::builder()
                    .with_capabilities(capabilities())
                    .build(),
            );
        }
        let result = builder
            .load_psks(owner.provider().storage())
            .unwrap()
            .build_private_text_add_with_time(
                owner.provider().rand(),
                owner.provider().crypto(),
                owner.signer(),
                time(),
            );
        assert!(matches!(
            result,
            Err(CreateCommitError::UnsupportedPrivateTextAdd)
        ));
        drop(result);
        assert!(selected.group.pending_commit().is_none());
        assert_eq!(selected.epoch(), 0);
        assert!(
            snapshot_server(&owner, &selected).unwrap().as_slice() == before.as_slice(),
            "refused private shape must preserve all original rows"
        );
    }
}

#[test]
fn private_serial_add_refuses_a_genuine_full_sixty_four_slot_tree() {
    let owner = MlsDevice::generate().unwrap();
    let members: Vec<_> = (0..63).map(|_| MlsDevice::generate().unwrap()).collect();
    let mut selected = group(&owner);
    let bundle = selected
        .group
        .commit_builder()
        .force_self_update(true)
        .propose_adds(members.iter().map(package))
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
    assert!(bundle.welcome().is_some());
    selected
        .group
        .merge_pending_commit_with_time(owner.provider(), time())
        .unwrap();
    assert_eq!(selected.member_count(), 64);
    assert_eq!(selected.group.treesync().blank_leaves().count(), 0);
    let candidate = MlsDevice::generate().unwrap();
    let before = snapshot_server(&owner, &selected).unwrap();
    let result = selected
        .group
        .commit_builder()
        .consume_proposal_store(true)
        .force_self_update(true)
        .propose_adds([package(&candidate)])
        .load_psks(owner.provider().storage())
        .unwrap()
        .build_private_text_add_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(),
        );
    assert!(matches!(
        result,
        Err(CreateCommitError::UnsupportedPrivateTextAdd)
    ));
    drop(result);
    assert!(
        snapshot_server(&owner, &selected).unwrap().as_slice() == before.as_slice(),
        "full-tree refusal cannot publish a sixty-fifth member"
    );
    assert_eq!(selected.member_count(), 64);
}

#[test]
fn private_serial_add_reuses_an_actual_blank_after_real_member_removal() {
    let owner = MlsDevice::generate().unwrap();
    let mut selected = group(&owner);
    for _ in 0..3 {
        serial_add(&owner, &mut selected, &MlsDevice::generate().unwrap());
    }
    selected
        .group
        .remove_members(owner.provider(), owner.signer(), &[LeafNodeIndex::new(1)])
        .unwrap();
    selected
        .group
        .merge_pending_commit_with_time(owner.provider(), time())
        .unwrap();
    assert!(selected
        .group
        .treesync()
        .blank_leaves()
        .any(|index| index == LeafNodeIndex::new(1)));
    let replacement = MlsDevice::generate().unwrap();
    serial_add(&owner, &mut selected, &replacement);
    assert_eq!(selected.member_count(), 4);
    assert!(selected.group.members().any(|member| {
        member.index == LeafNodeIndex::new(1)
            && member.signature_key == replacement.public_key_ref()
    }));
}

#[test]
fn private_serial_add_refuses_an_actual_signed_queued_proposal() {
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut selected = group(&owner);
    selected
        .group
        .propose_remove_member(owner.provider(), owner.signer(), LeafNodeIndex::new(0))
        .unwrap();
    assert!(selected.group.pending_proposals().next().is_some());
    let before = snapshot_server(&owner, &selected).unwrap();
    let result = selected
        .group
        .commit_builder()
        .consume_proposal_store(true)
        .force_self_update(true)
        .propose_adds([package(&member)])
        .load_psks(owner.provider().storage())
        .unwrap()
        .build_private_text_add_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(),
        );
    assert!(matches!(
        result,
        Err(CreateCommitError::UnsupportedPrivateTextAdd)
    ));
    drop(result);
    assert!(selected.group.pending_proposals().next().is_some());
    assert!(selected.group.pending_commit().is_none());
    assert!(
        snapshot_server(&owner, &selected).unwrap().as_slice() == before.as_slice(),
        "unsupported queued input must not consume or rewrite the real proposal"
    );
}

#[test]
fn private_serial_add_refuses_real_loaded_psk_without_consuming_its_proposal() {
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut selected = group(&owner);
    let psk = PreSharedKeyId::external(vec![19; 16], vec![23; 32]);
    psk.store(owner.provider(), &[29; 32]).unwrap();
    selected
        .group
        .propose_pre_shared_key(owner.provider(), owner.signer(), psk)
        .unwrap();
    let before = snapshot_server(&owner, &selected).unwrap();
    // Use the real stored secret and signed proposal, so load_psks succeeds.
    // This guard cannot retroactively pay that earlier load; the actual private
    // candidate must exclude queued/PSK input before reaching it as documented.
    let result = selected
        .group
        .commit_builder()
        .consume_proposal_store(true)
        .force_self_update(true)
        .propose_adds([package(&member)])
        .load_psks(owner.provider().storage())
        .unwrap()
        .build_private_text_add_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(),
        );
    assert!(matches!(
        result,
        Err(CreateCommitError::UnsupportedPrivateTextAdd)
    ));
    drop(result);
    assert!(selected.group.pending_proposals().next().is_some());
    assert!(selected.group.pending_commit().is_none());
    assert!(
        snapshot_server(&owner, &selected).unwrap().as_slice() == before.as_slice(),
        "private PSK refusal must preserve the real secret and signed proposal"
    );
}

#[test]
fn private_serial_add_refuses_blank_or_out_of_subtree_unmerged_targets() {
    for blank_target in [false, true] {
        let owner = MlsDevice::generate().unwrap();
        let mut selected = group(&owner);
        for _ in 0..3 {
            serial_add(&owner, &mut selected, &MlsDevice::generate().unwrap());
        }
        if blank_target {
            selected
                .group
                .remove_members(owner.provider(), owner.signer(), &[LeafNodeIndex::new(1)])
                .unwrap();
            selected
                .group
                .merge_pending_commit_with_time(owner.provider(), time())
                .unwrap();
        }
        {
            // Deliberately tamper a fixture's public tree row, then use the
            // ordinary untrusted snapshot loader. This is no authenticated
            // Source, paid closed restore or production mutation permission.
            let mut rows = owner.provider().storage().values.write().unwrap();
            let (_, raw) = rows
                .iter_mut()
                .find(|(key, _)| key.starts_with(b"Tree"))
                .unwrap();
            let mut tree: serde_json::Value = serde_json::from_slice(raw).unwrap();
            let (parent, target) = if blank_target { (1, 1) } else { (0, 2) };
            let node = &mut tree["tree"]["parent_nodes"][parent]["node"];
            assert!(node.is_object(), "fixture needs its actual full parent");
            node["unmerged_leaves"]["list"] = serde_json::json!([target]);
            *raw = serde_json::to_vec(&tree).unwrap();
        }
        let tainted = snapshot_server(&owner, &selected).unwrap();
        let (restored_owner, mut restored) = crate::restore_server(&tainted).unwrap();
        let before = snapshot_server(&restored_owner, &restored).unwrap();
        let member = MlsDevice::generate().unwrap();
        let result = restored
            .group
            .commit_builder()
            .consume_proposal_store(true)
            .force_self_update(true)
            .propose_adds([package(&member)])
            .load_psks(restored_owner.provider().storage())
            .unwrap()
            .build_private_text_add_with_time(
                restored_owner.provider().rand(),
                restored_owner.provider().crypto(),
                restored_owner.signer(),
                time(),
            );
        // Without the new guard, resolution can assert on the blank target or
        // repeat a present target from an unrelated subtree. Both must refuse
        // before the shared commit algorithm, never panic or publish a result.
        assert!(matches!(
            result,
            Err(CreateCommitError::UnsupportedPrivateTextAdd)
        ));
        drop(result);
        assert!(restored.group.pending_commit().is_none());
        assert!(
            snapshot_server(&restored_owner, &restored)
                .unwrap()
                .as_slice()
                == before.as_slice(),
            "malformed unmerged metadata must remain unchanged on refusal"
        );
    }
}

/// Separate compile-feature matrix: ordinary full-suite builds must keep the
/// seven supported-profile regressions strict. Run this test explicitly with
/// each of openmls/backtrace, openmls/crypto-debug and openmls/content-debug.
/// No environment/runtime flag changes the production support predicate.
#[test]
#[ignore = "requires an OpenMLS diagnostic feature; run its focused feature-matrix command"]
fn diagnostic_profile_refuses_valid_private_add_before_mutation() {
    assert!(
        !MlsGroup::private_text_add_profile_supported_data(),
        "this opt-in test requires an unsupported SDK diagnostic build"
    );
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut selected = group(&owner);
    let before = snapshot_server(&owner, &selected).unwrap();
    // This is the complete valid one-Add shape used by the successful ordinary
    // tests. Only the SDK's compile-time diagnostic profile causes refusal.
    let result = selected
        .group
        .commit_builder()
        .consume_proposal_store(true)
        .force_self_update(true)
        .propose_adds([package(&member)])
        .load_psks(owner.provider().storage())
        .unwrap()
        .build_private_text_add_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(),
        );
    assert!(matches!(
        result,
        Err(CreateCommitError::UnsupportedPrivateTextAdd)
    ));
    drop(result);
    assert!(selected.group.pending_commit().is_none());
    assert!(
        snapshot_server(&owner, &selected).unwrap().as_slice() == before.as_slice(),
        "unsupported diagnostic profile must preserve every original SDK row"
    );
}
