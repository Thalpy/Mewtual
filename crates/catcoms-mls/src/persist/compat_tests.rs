//! Stored-state compatibility fixtures: MLS state **written by openmls 0.8.1** (the stack shipped
//! in the v0.3.0-alpha builds) must keep loading and working under whatever openmls this crate
//! is built with now.
//!
//! The fixtures in `fixtures/openmls-0.8.1/` were produced by [`generate_openmls_0_8_1_fixtures`]
//! on the openmls 0.8.1 lockfile of commit `1be214fe` (`openmls_rust_crypto` 0.5.1,
//! `openmls_basic_credential` 0.5.0), through this crate's own production API (`snapshot_server`,
//! `stage_add`, `stage_remove`, invite-bound membership credentials). One deliberate deviation:
//! every KeyPackage and the founder's leaf use a never-expiring lifetime. openmls checks leaf
//! lifetimes against the wall clock when a Welcome is joined or an Add is processed, so the
//! production default (84 days) would make these frozen files fail on a calendar date. They are
//! **frozen**: never regenerate them with a newer openmls, or they stop testing anything. They hold
//! private keys of throwaway test identities and no real user data.
//!
//! What each test pins, and why it matters for an upgrade:
//! - a founder's and a member's sealed-snapshot payload restore (signer entry, group, epoch);
//! - the restored member accepts 0.8.1 wire traffic (application message, Add commit);
//! - a Welcome produced by 0.8.1 is joined using KeyPackage material 0.8.1 stored;
//! - the two production staged-commit shapes (`stage_add`, `stage_remove`) survive a snapshot as
//!   a PendingCommit and can be both merged and aborted (`clear_pending_commit`).
//!
//! The opposite direction (state or KeyPackages created by a newer openmls, read by 0.8.1) cannot
//! be tested in-tree, because both openmls lines cannot share one lockfile; it is checked out of
//! tree and recorded in `docs/design-crypto-bigint-upgrade.md`.

use std::collections::HashMap;

use catcoms_wire::{Decoder, DocType, Encoder};
use openmls::prelude::{KeyPackage, Lifetime};
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::OpenMlsProvider;

use super::{deserialize_map, restore_server, serialize_map, snapshot_server};
use crate::{Incoming, MembershipCredential, MlsDevice, ServerGroup};

const DOC: u128 = 7;

/// 2100-01-01T00:00:00Z. Every fixture leaf must stay valid well past any CI run.
const FAR_FUTURE: u64 = 4_102_444_800;

/// The fixtures' leaf lifetime: valid from the epoch to the end of time.
fn forever() -> Lifetime {
    Lifetime::init(0, u64::MAX)
}

/// [`MlsDevice::key_package_for_invite`] with [`forever`] as its lifetime. Same credential, same
/// capabilities, same storage of the private bundle; only the lifetime differs.
fn invite_key_package(device: &MlsDevice, group_id: &[u8], invite_nonce: [u8; 16]) -> KeyPackage {
    use openmls::prelude::{BasicCredential, CredentialWithKey};
    let membership = MembershipCredential {
        device_id: device.device_id(),
        group_id: group_id.to_vec(),
        invite_nonce,
    };
    let credential = CredentialWithKey {
        credential: BasicCredential::new(membership.encode()).into(),
        signature_key: device.public_key_bytes().into(),
    };
    KeyPackage::builder()
        .leaf_node_capabilities(crate::config::capabilities())
        .key_package_lifetime(forever())
        .build(
            crate::CIPHERSUITE,
            device.provider(),
            device.signer(),
            credential,
        )
        .unwrap()
        .key_package()
        .clone()
}

/// One fixture file, embedded at compile time.
macro_rules! fixture {
    ($name:literal) => {
        include_bytes!(concat!("fixtures/openmls-0.8.1/", $name)).as_slice()
    };
}

/// Decoded `*.expect` record. Every field is written in this order by the generator.
struct Expect {
    epoch: u64,
    members: u32,
    secret: Vec<u8>,
    after_epoch: u64,
    after_members: u32,
    after_secret: Vec<u8>,
    device: Vec<u8>,
}

fn encode_expect(e: &Expect) -> Vec<u8> {
    let mut enc = Encoder::new();
    enc.put_u64(e.epoch).put_u32(e.members);
    enc.put_bytes(&e.secret).unwrap();
    enc.put_u64(e.after_epoch).put_u32(e.after_members);
    enc.put_bytes(&e.after_secret).unwrap();
    enc.put_bytes(&e.device).unwrap();
    enc.finish()
}

fn decode_expect(bytes: &[u8]) -> Expect {
    let mut d = Decoder::new(bytes);
    let e = Expect {
        epoch: d.get_u64().unwrap(),
        members: d.get_u32().unwrap(),
        secret: d.get_bytes().unwrap().to_vec(),
        after_epoch: d.get_u64().unwrap(),
        after_members: d.get_u32().unwrap(),
        after_secret: d.get_bytes().unwrap().to_vec(),
        device: d.get_bytes().unwrap().to_vec(),
    };
    d.finish().unwrap();
    e
}

fn secret(group: &ServerGroup, device: &MlsDevice) -> Vec<u8> {
    group
        .channel_secret(device, DocType::Channel, DOC)
        .unwrap()
        .to_vec()
}

/// A device that has not joined any group yet: its storage map (KeyPackage private material and
/// signer) and its public key. Uses the same map codec as [`snapshot_server`].
fn encode_prejoin(device: &MlsDevice) -> Vec<u8> {
    let map = device.provider().storage().values.read().unwrap().clone();
    let mut enc = Encoder::new();
    enc.put_bytes(&serialize_map(&map).unwrap()).unwrap();
    enc.put_bytes(&device.public_key_bytes()).unwrap();
    enc.finish()
}

fn decode_prejoin(bytes: &[u8]) -> MlsDevice {
    let mut d = Decoder::new(bytes);
    let map: HashMap<Vec<u8>, Vec<u8>> = deserialize_map(d.get_bytes().unwrap()).unwrap();
    let public_key = d.get_bytes().unwrap().to_vec();
    d.finish().unwrap();
    let provider = OpenMlsRustCrypto::default();
    *provider.storage().values.write().unwrap() = map;
    MlsDevice::restore(provider, &public_key).unwrap()
}

#[test]
fn openmls_0_8_1_founder_snapshot_restores_identity_group_and_secrets() {
    let expect = decode_expect(fixture!("founder.expect"));
    let (alice, group) = restore_server(fixture!("founder.snapshot")).unwrap();
    assert_eq!(group.epoch(), expect.epoch);
    assert_eq!(group.member_count() as u32, expect.members);
    assert_eq!(secret(&group, &alice), expect.secret);
    assert_eq!(alice.device_id().as_bytes().as_slice(), expect.device);
    assert!(alice.sign(b"after restore").is_ok());
}

#[test]
fn frozen_fixture_leaf_lifetimes_cannot_expire_under_ci() {
    // openmls validates KeyPackage-leaf lifetimes against the wall clock on Welcome joins and on
    // processed Adds. If a fixture leaf could expire, these tests would start failing on a date.
    let snapshots = [
        fixture!("founder.snapshot"),
        fixture!("member.snapshot"),
        fixture!("staged_add.snapshot"),
        fixture!("staged_remove.snapshot"),
    ];
    let mut checked = 0;
    for snapshot in snapshots {
        let (_, group) = restore_server(snapshot).unwrap();
        for not_after in group.key_package_leaf_not_after() {
            assert!(
                not_after > FAR_FUTURE,
                "fixture leaf expires at {not_after}"
            );
            checked += 1;
        }
    }
    // The KeyPackage carol joins with, through the Welcome.
    let carol = decode_prejoin(fixture!("carol_prejoin.storage"));
    let joined = ServerGroup::join(&carol, fixture!("carol.welcome")).unwrap();
    for not_after in joined.key_package_leaf_not_after() {
        assert!(
            not_after > FAR_FUTURE,
            "fixture leaf expires at {not_after}"
        );
        checked += 1;
    }
    assert!(checked >= 8, "every member leaf was checked ({checked})");
}

#[test]
fn openmls_0_8_1_member_snapshot_accepts_0_8_1_application_and_commit_traffic() {
    let expect = decode_expect(fixture!("member.expect"));
    let (bob, mut group) = restore_server(fixture!("member.snapshot")).unwrap();
    assert_eq!(group.epoch(), expect.epoch);
    assert_eq!(group.member_count() as u32, expect.members);
    assert_eq!(secret(&group, &bob), expect.secret);

    match group
        .process_incoming(&bob, fixture!("member_app.msg"))
        .unwrap()
    {
        Incoming::Application(plain) => assert_eq!(plain, b"written by openmls 0.8.1"),
        other => panic!("expected an application message, got {other:?}"),
    }
    match group
        .process_incoming(&bob, fixture!("member_commit.msg"))
        .unwrap()
    {
        Incoming::CommitApplied { removed } => assert!(!removed),
        other => panic!("expected an applied commit, got {other:?}"),
    }
    assert_eq!(group.epoch(), expect.after_epoch);
    assert_eq!(group.member_count() as u32, expect.after_members);
    assert_eq!(secret(&group, &bob), expect.after_secret);
}

#[test]
fn openmls_0_8_1_welcome_joins_with_key_package_material_0_8_1_stored() {
    // `member.expect`'s post-commit values are the epoch the Welcome lands in.
    let expect = decode_expect(fixture!("member.expect"));
    let carol = decode_prejoin(fixture!("carol_prejoin.storage"));
    let group = ServerGroup::join(&carol, fixture!("carol.welcome")).unwrap();
    assert_eq!(group.epoch(), expect.after_epoch);
    assert_eq!(group.member_count() as u32, expect.after_members);
    assert_eq!(secret(&group, &carol), expect.after_secret);
}

/// Shared body for the two staged shapes: one restore merges, an independent restore aborts.
fn pending_commit_merges_and_aborts(snapshot: &[u8], expect: &Expect, removal: bool) {
    let (alice, mut merged) = restore_server(snapshot).unwrap();
    assert_eq!(
        merged.epoch(),
        expect.epoch,
        "the pending commit is not merged yet"
    );
    assert_eq!(secret(&merged, &alice), expect.secret);
    merged.merge_staged_self(&alice).unwrap();
    assert_eq!(merged.epoch(), expect.after_epoch);
    assert_eq!(merged.member_count() as u32, expect.after_members);
    assert_eq!(secret(&merged, &alice), expect.after_secret);
    let named = merged
        .member_device_ids()
        .iter()
        .any(|id| id.as_bytes().as_slice() == expect.device);
    assert_eq!(named, !removal, "the staged Add/Remove took effect");

    let (alice, mut aborted) = restore_server(snapshot).unwrap();
    aborted.abort_staged(&alice).unwrap();
    assert_eq!(
        aborted.epoch(),
        expect.epoch,
        "abort keeps the pre-stage epoch"
    );
    assert_eq!(aborted.member_count() as u32, expect.members);
    assert_eq!(
        secret(&aborted, &alice),
        expect.secret,
        "abort keeps the pre-stage epoch secrets"
    );
    // The aborted group is operational again: it can encrypt in the restored epoch.
    assert!(aborted
        .create_application_message(&alice, b"after abort")
        .is_ok());
}

#[test]
fn openmls_0_8_1_pending_stage_add_merges_and_aborts_after_restore() {
    let expect = decode_expect(fixture!("staged_add.expect"));
    pending_commit_merges_and_aborts(fixture!("staged_add.snapshot"), &expect, false);
}

#[test]
fn openmls_0_8_1_pending_stage_remove_merges_and_aborts_after_restore() {
    let expect = decode_expect(fixture!("staged_remove.expect"));
    pending_commit_merges_and_aborts(fixture!("staged_remove.snapshot"), &expect, true);
}

/// Writes the frozen fixtures. Run only on the openmls version named in the module docs:
/// `CATCOMS_MLS_FIXTURE_OUT=<dir> cargo test -p catcoms-mls generate_openmls_0_8_1 -- --ignored`
#[test]
#[ignore = "fixture generator; the committed fixtures are frozen"]
fn generate_openmls_0_8_1_fixtures() {
    let Ok(out) = std::env::var("CATCOMS_MLS_FIXTURE_OUT") else {
        panic!("set CATCOMS_MLS_FIXTURE_OUT to the output directory");
    };
    let write = |name: &str, bytes: &[u8]| {
        std::fs::write(format!("{out}/{name}"), bytes).unwrap();
    };

    let alice = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create_with_lifetime(&alice, forever()).unwrap();
    let gid = group.group_id();
    let bob = MlsDevice::generate().unwrap();
    let dave = MlsDevice::generate().unwrap();
    let carol = MlsDevice::generate().unwrap();
    let eve = MlsDevice::generate().unwrap();

    let to_bob = group
        .add_member(&alice, invite_key_package(&bob, &gid, [1; 16]))
        .unwrap();
    let mut bob_group = ServerGroup::join(&bob, &to_bob.welcome).unwrap();
    let to_dave = group
        .add_member(&alice, invite_key_package(&dave, &gid, [2; 16]))
        .unwrap();
    bob_group.process_incoming(&bob, &to_dave.commit).unwrap();
    let m1 = group
        .create_application_message(&alice, b"warm the ratchet")
        .unwrap();
    bob_group.process_incoming(&bob, &m1).unwrap();
    let b1 = bob_group
        .create_application_message(&bob, b"and back")
        .unwrap();
    group.process_incoming(&alice, &b1).unwrap();
    assert_eq!(secret(&group, &alice), secret(&bob_group, &bob));

    // Founder and member snapshots at the same epoch.
    let founder = Expect {
        epoch: group.epoch(),
        members: group.member_count() as u32,
        secret: secret(&group, &alice),
        after_epoch: 0,
        after_members: 0,
        after_secret: Vec::new(),
        device: alice.device_id().as_bytes().to_vec(),
    };
    write(
        "founder.snapshot",
        &snapshot_server(&alice, &group).unwrap(),
    );
    write("founder.expect", &encode_expect(&founder));
    write(
        "member.snapshot",
        &snapshot_server(&bob, &bob_group).unwrap(),
    );

    // Traffic the restored member must accept, and carol's Welcome.
    let m2 = group
        .create_application_message(&alice, b"written by openmls 0.8.1")
        .unwrap();
    let carol_kp = invite_key_package(&carol, &gid, [3; 16]);
    write("carol_prejoin.storage", &encode_prejoin(&carol));
    let to_carol = group.add_member(&alice, carol_kp).unwrap();
    write("member_app.msg", &m2);
    write("member_commit.msg", &to_carol.commit);
    write("carol.welcome", &to_carol.welcome);
    // Cross-check in 0.8.1 itself before freezing the expectations.
    bob_group.process_incoming(&bob, &m2).unwrap();
    bob_group.process_incoming(&bob, &to_carol.commit).unwrap();
    let carol_group = ServerGroup::join(&carol, &to_carol.welcome).unwrap();
    let member = Expect {
        after_epoch: group.epoch(),
        after_members: group.member_count() as u32,
        after_secret: secret(&group, &alice),
        device: bob.device_id().as_bytes().to_vec(),
        ..founder
    };
    assert_eq!(member.after_secret, secret(&bob_group, &bob));
    assert_eq!(member.after_secret, secret(&carol_group, &carol));
    write("member.expect", &encode_expect(&member));

    // Production staged shapes, snapshotted while pending.
    let before = (
        group.epoch(),
        group.member_count() as u32,
        secret(&group, &alice),
    );
    group
        .stage_add(&alice, invite_key_package(&eve, &gid, [4; 16]))
        .unwrap();
    write(
        "staged_add.snapshot",
        &snapshot_server(&alice, &group).unwrap(),
    );
    group.merge_staged_self(&alice).unwrap();
    let staged_add = Expect {
        epoch: before.0,
        members: before.1,
        secret: before.2,
        after_epoch: group.epoch(),
        after_members: group.member_count() as u32,
        after_secret: secret(&group, &alice),
        device: eve.device_id().as_bytes().to_vec(),
    };
    write("staged_add.expect", &encode_expect(&staged_add));

    let before = (
        group.epoch(),
        group.member_count() as u32,
        secret(&group, &alice),
    );
    group.stage_remove(&alice, &dave.device_id()).unwrap();
    write(
        "staged_remove.snapshot",
        &snapshot_server(&alice, &group).unwrap(),
    );
    group.merge_staged_self(&alice).unwrap();
    let staged_remove = Expect {
        epoch: before.0,
        members: before.1,
        secret: before.2,
        after_epoch: group.epoch(),
        after_members: group.member_count() as u32,
        after_secret: secret(&group, &alice),
        device: dave.device_id().as_bytes().to_vec(),
    };
    write("staged_remove.expect", &encode_expect(&staged_remove));
}
