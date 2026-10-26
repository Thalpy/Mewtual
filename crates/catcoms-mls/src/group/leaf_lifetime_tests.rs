//! Welcome joins must survive expired KeyPackage leaves; admitting an expired KeyPackage must not.
//!
//! A leaf added from a KeyPackage keeps that KeyPackage's lifetime (openmls default: 84 days) until
//! its owner replaces it. Only the designated committer ever commits, so every other member's leaf
//! keeps its join-time lifetime forever, and openmls checks every such leaf against the wall clock
//! when a Welcome is joined. Without [`ServerGroup::join`]'s lifetime policy, every join to a group
//! with one such member fails 84 days after that member joined. See
//! `docs/design-mls-leaf-lifetime.md`.
//!
//! openmls reads `SystemTime::now()` itself, so these fixtures use lifetimes that are already over
//! (1970) or not yet valid (the far future) instead of waiting. Nothing here reads a clock.

use super::*;
use crate::config::{capabilities, CIPHERSUITE};
use crate::invite::InviteError;
use catcoms_wire::DocType;

/// A lifetime that ended in 1970: expired whenever the test runs.
fn expired() -> Lifetime {
    Lifetime::init(1, 2)
}

/// A lifetime that starts at the end of time: not yet valid whenever the test runs.
fn not_yet_valid() -> Lifetime {
    Lifetime::init(u64::MAX - 1, u64::MAX)
}

/// Found a group whose founder leaf carries `lifetime`.
///
/// The production create config, rebuilt here only so the founder's leaf lifetime can be chosen.
/// Kept local to this test so production config has no test-only surface.
fn found_with_lifetime(device: &MlsDevice, lifetime: Lifetime) -> ServerGroup {
    let config = MlsGroupCreateConfig::builder()
        .ciphersuite(CIPHERSUITE)
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .use_ratchet_tree_extension(true)
        .capabilities(capabilities())
        .lifetime(lifetime)
        .build();
    let group = MlsGroup::new(
        device.provider(),
        device.signer(),
        &config,
        device.credential(),
    )
    .expect("openmls does not validate the founder's own lifetime");
    ServerGroup {
        group,
        catalog: Default::default(),
    }
}

/// Add `joiner` without replacing the committer's leaf, so the committer's own KeyPackage-sourced
/// leaf stays in the tree the Welcome carries, exactly like a non-committing member's leaf does in
/// production. Returns the serialized Welcome.
fn add_keeping_committer_leaf(
    group: &mut ServerGroup,
    committer: &MlsDevice,
    key_package: KeyPackage,
) -> Vec<u8> {
    let (_commit, welcome, _info) = group
        .group
        .add_members_without_update(committer.provider(), committer.signer(), &[key_package])
        .expect("the added KeyPackage itself is fresh, so the Add is valid");
    group
        .group
        .merge_pending_commit(committer.provider())
        .unwrap();
    welcome.tls_serialize_detached().unwrap()
}

/// The lifetime this member's own leaf still carries, if it is KeyPackage-sourced. (`own_leaf` is
/// public on openmls 0.8 and 0.9; the whole-tree accessor is not public on 0.8.)
fn own_key_package_lifetime(group: &ServerGroup) -> Option<Lifetime> {
    match group.group.own_leaf()?.leaf_node_source() {
        openmls::treesync::LeafNodeSource::KeyPackage(lifetime) => Some(*lifetime),
        _ => None,
    }
}

/// An invite-shaped KeyPackage for `device` with an explicit lifetime.
fn key_package_with_lifetime(
    device: &MlsDevice,
    group_id: &[u8],
    lifetime: Lifetime,
) -> KeyPackage {
    let membership = crate::invite::MembershipCredential {
        device_id: device.device_id(),
        group_id: group_id.to_vec(),
        invite_nonce: [9; 16],
    };
    let credential = CredentialWithKey {
        credential: BasicCredential::new(membership.encode()).into(),
        signature_key: device.signer().public().into(),
    };
    KeyPackage::builder()
        .key_package_lifetime(lifetime)
        .leaf_node_capabilities(capabilities())
        .build(CIPHERSUITE, device.provider(), device.signer(), credential)
        .unwrap()
        .key_package()
        .clone()
}

/// The regression: a Welcome whose tree holds an expired KeyPackage leaf must still join, and the
/// joiner must land in the same group state as the member that admitted it.
///
/// The same Welcome is also offered to openmls's default join (through a duplicate of the joiner's
/// device, because a join attempt consumes the KeyPackage's private material), which must refuse it
/// on the lifetime. That half proves the fixture really reproduces the production failure, so the
/// first half cannot pass for an unrelated reason.
#[test]
fn a_welcome_with_an_expired_key_package_leaf_still_joins() {
    let alice = MlsDevice::generate().unwrap();
    let mut alice_group = found_with_lifetime(&alice, expired());
    let carol = MlsDevice::generate().unwrap();
    let carol_kp = carol
        .key_package_for_invite(&alice_group.group_id(), [1; 16])
        .unwrap();
    let welcome = add_keeping_committer_leaf(&mut alice_group, &alice, carol_kp);
    assert_eq!(
        own_key_package_lifetime(&alice_group),
        Some(expired()),
        "fixture: the committer's leaf, in the Welcome's tree, must still be the expired \
         KeyPackage leaf"
    );

    // openmls's default join refuses this tree on the lifetime alone.
    let carol_default = carol.duplicate().unwrap();
    let msg = MlsMessageIn::tls_deserialize(&mut &welcome[..]).unwrap();
    let MlsMessageBodyIn::Welcome(raw) = msg.extract() else {
        panic!("a Welcome");
    };
    let refused =
        StagedWelcome::new_from_welcome(carol_default.provider(), &join_config(), raw, None)
            .map(|_| ());
    let refused = format!("{refused:?}");
    assert!(
        refused.contains("Lifetime"),
        "fixture: openmls's default join must refuse the expired leaf, got {refused}"
    );

    // The production join accepts the same Welcome and agrees with the group.
    let mut carol_group =
        ServerGroup::join(&carol, &welcome).expect("an expired leaf must not block a Welcome join");
    assert_eq!(carol_group.epoch(), alice_group.epoch());
    assert_eq!(
        carol_group.member_device_ids(),
        alice_group.member_device_ids()
    );
    assert_eq!(
        carol_group
            .channel_secret(&carol, DocType::Channel, 1)
            .unwrap(),
        alice_group
            .channel_secret(&alice, DocType::Channel, 1)
            .unwrap(),
        "the joiner derives the group's exporter"
    );
    let sealed = alice_group
        .create_application_message(&alice, b"welcome, carol")
        .unwrap();
    match carol_group.process_incoming(&carol, &sealed).unwrap() {
        Incoming::Application(plain) => assert_eq!(plain, b"welcome, carol"),
        other => panic!("expected the application message, got {other:?}"),
    }
}

/// The production shape, with three parties: the expired leaf belongs to a member who is neither
/// the joiner nor the Welcome's signer, and the Welcome comes from an ordinary path-carrying Add.
///
/// X founds with an expired leaf and admits C without replacing its own leaf. C then admits D
/// through the production `add_member` (a commit with a path, so C's own leaf is refreshed) and X
/// follows that commit. D's Welcome is signed by C, and its tree still holds X's expired leaf,
/// exactly as a long-quiet member's leaf sits in a real group.
#[test]
fn a_welcome_signed_by_another_member_joins_past_a_third_members_expired_leaf() {
    let x = MlsDevice::generate().unwrap();
    let mut x_group = found_with_lifetime(&x, expired());
    let c = MlsDevice::generate().unwrap();
    let c_kp = c
        .key_package_for_invite(&x_group.group_id(), [1; 16])
        .unwrap();
    let c_welcome = add_keeping_committer_leaf(&mut x_group, &x, c_kp);
    let mut c_group = ServerGroup::join(&c, &c_welcome).unwrap();

    let d = MlsDevice::generate().unwrap();
    let d_kp = d
        .key_package_for_invite(&c_group.group_id(), [2; 16])
        .unwrap();
    let added = c_group.add_member(&c, d_kp).unwrap();
    match x_group.process_incoming(&x, &added.commit).unwrap() {
        Incoming::CommitApplied { removed } => assert!(!removed),
        other => panic!("X must follow C's commit, got {other:?}"),
    }
    assert_eq!(
        own_key_package_lifetime(&x_group),
        Some(expired()),
        "fixture: X's leaf is still the expired KeyPackage leaf in D's tree"
    );
    assert_eq!(
        own_key_package_lifetime(&c_group),
        None,
        "fixture: C's own commit replaced C's leaf, so C (the signer) is not the expired one"
    );

    let mut d_group = ServerGroup::join(&d, &added.welcome)
        .expect("a third member's expired leaf must not block a Welcome join");
    let secret = |group: &ServerGroup, device: &MlsDevice| {
        group.channel_secret(device, DocType::Channel, 1).unwrap()
    };
    assert_eq!(secret(&d_group, &d), secret(&c_group, &c));
    assert_eq!(secret(&d_group, &d), secret(&x_group, &x));
    let sealed = x_group
        .create_application_message(&x, b"from the quiet member")
        .unwrap();
    match d_group.process_incoming(&d, &sealed).unwrap() {
        Incoming::Application(plain) => assert_eq!(plain, b"from the quiet member"),
        other => panic!("expected the application message, got {other:?}"),
    }
}

/// The protection that stays: the committer refuses to admit a KeyPackage outside its lifetime, in
/// either direction, its group does not move, and a valid Add still goes through afterwards.
///
/// This covers the committer. Every other member runs the same openmls check when it processes
/// the Add (`validate_add_proposals`), which openmls's own commit builder will not let a test reach
/// with an out-of-lifetime KeyPackage. openmls reports these as strings through `MlsError::Protocol`,
/// so the match is on the text both 0.8 and 0.9 put in it.
#[test]
fn admitting_a_key_package_outside_its_lifetime_is_still_refused() {
    let alice = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&alice).unwrap();
    let epoch = group.epoch();
    let members = group.member_device_ids();
    for (label, lifetime) in [("expired", expired()), ("not yet valid", not_yet_valid())] {
        let joiner = MlsDevice::generate().unwrap();
        let kp = key_package_with_lifetime(&joiner, &group.group_id(), lifetime);
        let refused = group.add_member(&alice, kp);
        assert!(
            matches!(&refused, Err(MlsError::Protocol(e)) if e.contains("Lifetime")),
            "an {label} KeyPackage must be refused on its lifetime, got {refused:?}"
        );
        assert_eq!(group.epoch(), epoch, "the refused Add did not advance");
        assert_eq!(group.member_device_ids(), members);
    }
    let joiner = MlsDevice::generate().unwrap();
    let kp = joiner
        .key_package_for_invite(&group.group_id(), [3; 16])
        .unwrap();
    group
        .add_member(&alice, kp)
        .expect("a fresh KeyPackage is still admitted after the refusals");
    assert_eq!(group.epoch(), epoch + 1);
}

/// The time an honest KeyPackage was minted, in ms: openmls back-dates `not_before` by one hour.
/// Taken from the KeyPackage so no test reads a clock.
fn minted_at_ms(key_package: &KeyPackage) -> u64 {
    (key_package.life_time().not_before() + 3_600) * 1_000
}

fn wire(key_package: &KeyPackage) -> Vec<u8> {
    crate::device::serialize_key_package(key_package).unwrap()
}

/// A joiner chooses its own KeyPackage lifetime. One that is valid now but ends soon would be
/// admitted by the committer and then refused, permanently, by every member that processes the
/// Add later (see `MIN_ADMITTED_KEY_PACKAGE_LIFETIME_MS`). The admission parse refuses it.
///
/// openmls still checks these KeyPackages against the real clock, so their windows are placed
/// around an honest KeyPackage's mint time with days of margin; the policy itself is checked
/// against an explicit `now_ms`.
#[test]
fn admission_refuses_a_key_package_that_expires_too_soon() {
    use crate::device::MIN_ADMITTED_KEY_PACKAGE_LIFETIME_MS as FLOOR;
    let alice = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&alice).unwrap();
    let joiner = MlsDevice::generate().unwrap();

    // An honest KeyPackage clears the floor at its mint time and for weeks after.
    let honest = joiner
        .key_package_for_invite(&group.group_id(), [1; 16])
        .unwrap();
    let now = minted_at_ms(&honest);
    alice.parse_key_package(&wire(&honest), now).unwrap();
    alice
        .parse_key_package(&wire(&honest), now + 20 * 24 * 3_600 * 1_000)
        .expect("an honest KeyPackage admitted three weeks late still clears the floor");

    // One second short of the floor is refused, exactly at it is admitted.
    let not_before = honest.life_time().not_before();
    let floor_secs = FLOOR / 1_000;
    for (not_after, admitted) in [
        (now / 1_000 + floor_secs - 1, false),
        (now / 1_000 + floor_secs, true),
    ] {
        let kp = key_package_with_lifetime(
            &joiner,
            &group.group_id(),
            Lifetime::init(not_before, not_after),
        );
        let parsed = alice.parse_key_package(&wire(&kp), now).map(|_| ());
        if admitted {
            parsed.expect("a KeyPackage valid for exactly the floor is admitted");
        } else {
            assert!(
                matches!(
                    parsed,
                    Err(MlsError::Invite(InviteError::KeyPackageExpiresTooSoon))
                ),
                "a KeyPackage one second short of the floor must be refused, got {parsed:?}"
            );
        }
    }

    // Already expired: openmls's own wire validation refuses it before the floor is consulted.
    let expired_kp = key_package_with_lifetime(&joiner, &group.group_id(), expired());
    let parsed = alice.parse_key_package(&wire(&expired_kp), now).map(|_| ());
    assert!(
        matches!(&parsed, Err(MlsError::Protocol(e)) if e.contains("ifetime")),
        "an expired KeyPackage is refused on the wire, got {parsed:?}"
    );
}
