//! Real SDK build/stage/merge regressions for the isolated Add lifetime seam.
//! These test fixtures use unrestricted newly generated devices, not a grant
//! to mutate the production closed restored graph or publish a governance Add.

use super::*;
use crate::config::{capabilities, CIPHERSUITE};
use crate::serialize_key_package;
use openmls::treesync::errors::{LeafNodeValidationError, LifetimeError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn time(seconds: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .unwrap()
}

fn group(owner: &MlsDevice) -> ServerGroup {
    // Fixed founder lifetime avoids MlsGroup creation's default lifetime clock.
    ServerGroup::create_with_lifetime(owner, Lifetime::init(0, u64::MAX)).unwrap()
}

fn signed_package(
    member: &MlsDevice,
    builder: KeyPackageBuilder,
    suite: Ciphersuite,
    not_before: u64,
) -> KeyPackage {
    let package = builder
        .key_package_lifetime(Lifetime::init(not_before, 20_000))
        .build(
            suite,
            member.provider(),
            member.signer(),
            member.credential(),
        )
        .unwrap()
        .key_package()
        .clone();
    let wire = serialize_key_package(&package).unwrap();
    let mut remaining = wire.as_slice();
    let message = MlsMessageIn::tls_deserialize(&mut remaining).unwrap();
    assert!(remaining.is_empty());
    let MlsMessageBodyIn::KeyPackage(incoming) = message.extract() else {
        panic!("fixture must be a complete genuine signed KeyPackage");
    };
    // Even deliberately insufficient group capabilities must retain real valid
    // signatures. No unchecked conversion or synthetic verified input is used.
    incoming
        .validate_with_time(
            member.provider().crypto(),
            ProtocolVersion::Mls10,
            time(not_before),
        )
        .unwrap()
}

fn package(member: &MlsDevice) -> KeyPackage {
    signed_package(
        member,
        KeyPackage::builder().leaf_node_capabilities(capabilities()),
        CIPHERSUITE,
        10_000,
    )
}

fn loaded<'g>(
    group: &'g mut ServerGroup,
    owner: &'g MlsDevice,
    packages: Vec<KeyPackage>,
) -> CommitBuilder<'g, LoadedPsks> {
    group
        .group
        .commit_builder()
        .consume_proposal_store(false)
        .force_self_update(true)
        .propose_adds(packages)
        .load_psks(owner.provider().storage())
        .unwrap()
}

fn assert_unchanged(group: &ServerGroup, epoch: u64, authenticator: &[u8; 32]) {
    assert_eq!(group.epoch(), epoch);
    assert_eq!(&group.epoch_authenticator_id(), authenticator);
    assert_eq!(group.member_count(), 1);
    assert!(group.group.pending_commit().is_none());
}

fn lifetime_error(error: LifetimeError) -> CreateCommitError {
    CreateCommitError::ProposalValidationError(ProposalValidationError::LeafNodeValidation(
        LeafNodeValidationError::Lifetime(error),
    ))
}

#[test]
fn explicit_add_checks_boundaries_then_really_stages_and_merges_selected_members() {
    for now in [9_999, 10_000, 19_999, 20_000, 20_001] {
        let owner = MlsDevice::generate().unwrap();
        let member = MlsDevice::generate().unwrap();
        let mut group = group(&owner);
        let original_epoch = group.epoch();
        let original_authenticator = group.epoch_authenticator_id();
        let result = loaded(&mut group, &owner, vec![package(&member)]).build_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(now),
            |_| true,
        );
        if (10_000..20_000).contains(&now) {
            let bundle = result.unwrap().stage_commit(owner.provider()).unwrap();
            assert!(bundle.welcome().is_some());
            assert!(bundle.group_info().is_some());
            assert!(!bundle.commit().tls_serialize_detached().unwrap().is_empty());
            assert_eq!(group.epoch(), original_epoch);
            assert!(group.group.pending_commit().is_some());
            group.group.merge_pending_commit(owner.provider()).unwrap();
            assert_eq!(group.epoch(), original_epoch + 1);
            assert_eq!(group.member_count(), 2);
            assert!(group
                .group
                .members()
                .any(|actual| actual.signature_key == member.public_key_ref()));
            assert_ne!(group.epoch_authenticator_id(), original_authenticator);
            assert!(group.group.pending_commit().is_none());
        } else {
            let expected = if now < 10_000 {
                LifetimeError::NotValidYet {
                    not_before: 10_000,
                    now,
                }
            } else {
                LifetimeError::Expired {
                    not_after: 20_000,
                    now,
                }
            };
            assert_eq!(result.err().unwrap(), lifetime_error(expected));
            assert_unchanged(&group, original_epoch, &original_authenticator);
        }
    }
}

#[test]
fn legacy_add_expiry_does_not_change_after_explicit_package_validation() {
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut group = group(&owner);
    let package = package(&member);
    let epoch = group.epoch();
    let authenticator = group.epoch_authenticator_id();
    let error = loaded(&mut group, &owner, vec![package.clone()])
        .build(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            |_| true,
        )
        .err()
        .unwrap();
    assert!(matches!(
        error,
        CreateCommitError::ProposalValidationError(ProposalValidationError::LeafNodeValidation(
            LeafNodeValidationError::Lifetime(LifetimeError::Expired {
                not_after: 20_000,
                ..
            })
        ))
    ));
    assert_unchanged(&group, epoch, &authenticator);
    let bundle = loaded(&mut group, &owner, vec![package])
        .build_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(10_000),
            |_| true,
        )
        .unwrap()
        .stage_commit(owner.provider())
        .unwrap();
    assert!(bundle.welcome().is_some());
    group.group.merge_pending_commit(owner.provider()).unwrap();
    assert_eq!(group.member_count(), 2);
}

#[test]
fn explicit_add_preserves_real_group_suite_and_leaf_capability_rejections() {
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut group = group(&owner);
    let epoch = group.epoch();
    let authenticator = group.epoch_authenticator_id();
    let other_suite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;
    let other = signed_package(
        &member,
        KeyPackage::builder().leaf_node_capabilities(Capabilities::new(
            None,
            Some(&[other_suite]),
            None,
            None,
            Some(&[CredentialType::Basic]),
        )),
        other_suite,
        10_000,
    );
    let error = loaded(&mut group, &owner, vec![other])
        .build_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(0),
            |_| true,
        )
        .err()
        .unwrap();
    assert_eq!(
        error,
        CreateCommitError::ProposalValidationError(
            ProposalValidationError::InvalidAddProposalCiphersuiteOrVersion,
        )
    );
    let insufficient = signed_package(
        &member,
        KeyPackage::builder().leaf_node_capabilities(Capabilities::new(
            None,
            Some(&[]),
            None,
            None,
            Some(&[CredentialType::Basic]),
        )),
        CIPHERSUITE,
        10_000,
    );
    let error = loaded(&mut group, &owner, vec![insufficient])
        .build_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(0),
            |_| true,
        )
        .err()
        .unwrap();
    assert_eq!(
        error,
        CreateCommitError::ProposalValidationError(ProposalValidationError::LeafNodeValidation(
            LeafNodeValidationError::CiphersuiteNotInCapabilities
        ),)
    );
    assert_unchanged(&group, epoch, &authenticator);
}

#[test]
fn explicit_add_keeps_original_key_uniqueness_and_other_proposal_checks() {
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut group = group(&owner);
    let epoch = group.epoch();
    let authenticator = group.epoch_authenticator_id();
    // Distinct genuine packages from the same full signer cannot create two lives.
    let error = loaded(&mut group, &owner, vec![package(&member), package(&member)])
        .build_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(0),
            |_| true,
        )
        .err()
        .unwrap();
    assert_eq!(
        error,
        CreateCommitError::ProposalValidationError(ProposalValidationError::DuplicateSignatureKey)
    );
    let error = group
        .group
        .commit_builder()
        .consume_proposal_store(false)
        .propose_adds([package(&member)])
        .propose_removals([LeafNodeIndex::new(123)])
        .load_psks(owner.provider().storage())
        .unwrap()
        .build_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(10_000),
            |_| true,
        )
        .err()
        .unwrap();
    assert_eq!(
        error,
        CreateCommitError::ProposalValidationError(ProposalValidationError::UnknownMemberRemoval)
    );
    assert_unchanged(&group, epoch, &authenticator);
}

#[test]
fn explicit_add_uses_one_time_for_every_added_package_and_preserves_pre_unix_error() {
    let owner = MlsDevice::generate().unwrap();
    let first = MlsDevice::generate().unwrap();
    let second = MlsDevice::generate().unwrap();
    let mut group = group(&owner);
    let epoch = group.epoch();
    let authenticator = group.epoch_authenticator_id();
    let first = package(&first);
    let second = signed_package(
        &second,
        KeyPackage::builder().leaf_node_capabilities(capabilities()),
        CIPHERSUITE,
        10_001,
    );
    let error = loaded(&mut group, &owner, vec![first.clone(), second.clone()])
        .build_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(10_000),
            |_| true,
        )
        .err()
        .unwrap();
    assert_eq!(
        error,
        lifetime_error(LifetimeError::NotValidYet {
            not_before: 10_001,
            now: 10_000
        })
    );
    let before_unix = UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap();
    let error = loaded(&mut group, &owner, vec![first.clone()])
        .build_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            before_unix,
            |_| true,
        )
        .err()
        .unwrap();
    assert_eq!(
        error,
        lifetime_error(LifetimeError::SystemTimeBeforeUnixEpoch)
    );
    assert_unchanged(&group, epoch, &authenticator);
    let bundle = loaded(&mut group, &owner, vec![first, second])
        .build_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            time(10_001),
            |_| true,
        )
        .unwrap()
        .stage_commit(owner.provider())
        .unwrap();
    assert!(bundle.welcome().is_some());
    group.group.merge_pending_commit(owner.provider()).unwrap();
    assert_eq!(group.member_count(), 3);
    assert_eq!(group.epoch(), epoch + 1);
}

#[test]
fn original_proposal_filter_does_not_turn_explicit_time_into_all_sdk_clock_override() {
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut group = group(&owner);
    let epoch = group.epoch();
    let mut seen = 0;
    let bundle = loaded(&mut group, &owner, vec![package(&member)])
        .build_with_time(
            owner.provider().rand(),
            owner.provider().crypto(),
            owner.signer(),
            UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap(),
            |_| {
                seen += 1;
                false
            },
        )
        .unwrap()
        .stage_commit(owner.provider())
        .unwrap();
    assert_eq!(seen, 1);
    assert!(bundle.welcome().is_none());
    group.group.merge_pending_commit(owner.provider()).unwrap();
    assert_eq!(group.epoch(), epoch + 1);
    assert_eq!(group.member_count(), 1);
}
