//! Single-use, device-bound invite tests: the happy path plus the adversarial
//! cases the design review flagged — single use, cross-group replay, expiry,
//! revocation, forged signatures, and non-member inviters.

use catcoms_mls::{InviteError, InviteLedger, InviteToken, MlsDevice, MlsError, ServerGroup};

const NOW: u64 = 1_000;
const SOON: u64 = 10_000;

fn nonce(b: u8) -> [u8; 16] {
    [b; 16]
}

/// Founder + a fresh group.
fn founded() -> (MlsDevice, ServerGroup) {
    let alice = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&alice).unwrap();
    (alice, group)
}

#[test]
fn invite_admits_a_device_and_consumes_the_nonce() {
    let (alice, mut group) = founded();
    let bob = MlsDevice::generate().unwrap();
    let mut ledger = InviteLedger::new();

    let token = group.mint_invite(&alice, nonce(1), SOON, vec![]).unwrap();
    let bob_kp = bob
        .key_package_for_invite(&group.group_id(), token.invite_nonce)
        .unwrap();

    let welcome = group
        .add_member_via_invite(&alice, bob_kp, &token, &mut ledger, NOW)
        .unwrap();
    let bob_group = ServerGroup::join(&bob, &welcome).unwrap();

    assert_eq!(group.member_count(), 2);
    assert!(group.contains_device(&bob.device_id()));
    assert_eq!(bob_group.epoch(), group.epoch());
    assert!(ledger.is_consumed(&token.invite_nonce));
}

#[test]
fn an_invite_is_single_use() {
    let (alice, mut group) = founded();
    let bob = MlsDevice::generate().unwrap();
    let carol = MlsDevice::generate().unwrap();
    let mut ledger = InviteLedger::new();

    let token = group.mint_invite(&alice, nonce(1), SOON, vec![]).unwrap();
    let bob_kp = bob
        .key_package_for_invite(&group.group_id(), token.invite_nonce)
        .unwrap();
    group
        .add_member_via_invite(&alice, bob_kp, &token, &mut ledger, NOW)
        .unwrap();

    // Re-presenting the same token (even for a different device) is rejected.
    let carol_kp = carol
        .key_package_for_invite(&group.group_id(), token.invite_nonce)
        .unwrap();
    let err = group
        .add_member_via_invite(&alice, carol_kp, &token, &mut ledger, NOW)
        .unwrap_err();
    assert!(matches!(err, MlsError::Invite(InviteError::AlreadyUsed)));
}

#[test]
fn key_package_cannot_be_replayed_into_another_group() {
    // Two independent groups.
    let (alice, group_a) = founded();
    let (carol, mut group_c) = founded();
    let bob = MlsDevice::generate().unwrap();

    // Bob mints a KeyPackage bound to group A + a nonce A.
    let token_a = group_a.mint_invite(&alice, nonce(1), SOON, vec![]).unwrap();
    let bob_kp_for_a = bob
        .key_package_for_invite(&group_a.group_id(), token_a.invite_nonce)
        .unwrap();

    // Carol tries to admit Bob's group-A KeyPackage into group C via her own invite.
    let token_c = group_c.mint_invite(&carol, nonce(2), SOON, vec![]).unwrap();
    let mut ledger_c = InviteLedger::new();
    let err = group_c
        .add_member_via_invite(&carol, bob_kp_for_a, &token_c, &mut ledger_c, NOW)
        .unwrap_err();
    assert!(matches!(
        err,
        MlsError::Invite(InviteError::CredentialMismatch)
    ));
    assert_eq!(group_c.member_count(), 1);
}

#[test]
fn expired_invite_is_rejected() {
    let (alice, mut group) = founded();
    let bob = MlsDevice::generate().unwrap();
    let mut ledger = InviteLedger::new();

    let token = group.mint_invite(&alice, nonce(1), 500, vec![]).unwrap(); // expires at 500
    let bob_kp = bob
        .key_package_for_invite(&group.group_id(), token.invite_nonce)
        .unwrap();
    let err = group
        .add_member_via_invite(&alice, bob_kp, &token, &mut ledger, NOW) // NOW = 1000 > 500
        .unwrap_err();
    assert!(matches!(err, MlsError::Invite(InviteError::Expired)));
}

#[test]
fn revoked_invite_is_rejected() {
    let (alice, mut group) = founded();
    let bob = MlsDevice::generate().unwrap();
    let mut ledger = InviteLedger::new();

    let token = group.mint_invite(&alice, nonce(1), SOON, vec![]).unwrap();
    ledger.revoke(token.invite_nonce);
    let bob_kp = bob
        .key_package_for_invite(&group.group_id(), token.invite_nonce)
        .unwrap();
    let err = group
        .add_member_via_invite(&alice, bob_kp, &token, &mut ledger, NOW)
        .unwrap_err();
    assert!(matches!(err, MlsError::Invite(InviteError::Revoked)));
}

#[test]
fn forged_invite_signature_is_rejected() {
    let (alice, mut group) = founded();
    let bob = MlsDevice::generate().unwrap();
    let mut ledger = InviteLedger::new();

    let mut token = group.mint_invite(&alice, nonce(1), SOON, vec![]).unwrap();
    token.signature[0] ^= 0xFF; // tamper
    let bob_kp = bob
        .key_package_for_invite(&group.group_id(), token.invite_nonce)
        .unwrap();
    let err = group
        .add_member_via_invite(&alice, bob_kp, &token, &mut ledger, NOW)
        .unwrap_err();
    assert!(matches!(err, MlsError::Invite(InviteError::BadSignature)));
}

#[test]
fn invite_from_a_non_member_is_rejected() {
    let (_alice, mut group) = founded();
    let stranger = MlsDevice::generate().unwrap();
    let bob = MlsDevice::generate().unwrap();
    let mut ledger = InviteLedger::new();

    // A token naming a non-member as the inviter. The membership check fires
    // before signature verification, so the (here irrelevant) signature is never
    // reached — a stranger simply cannot invite, however they sign.
    let token = InviteToken {
        group_id: group.group_id(),
        inviter_device_id: stranger.device_id(),
        inviter_public_key: stranger.public_key_bytes(),
        invite_nonce: nonce(1),
        expires_at_ms: SOON,
        bootstrap: vec![],
        signature: [0u8; 64],
    };

    let bob_kp = bob
        .key_package_for_invite(&group.group_id(), token.invite_nonce)
        .unwrap();
    let err = group
        .add_member_via_invite(&stranger, bob_kp, &token, &mut ledger, NOW)
        .unwrap_err();
    assert!(matches!(
        err,
        MlsError::Invite(InviteError::InviterNotMember)
    ));
}

#[test]
fn token_encode_decode_roundtrips() {
    let (alice, group) = founded();
    let token = group
        .mint_invite(
            &alice,
            nonce(3),
            SOON,
            vec!["/dns/relay.example/tcp/4001".into()],
        )
        .unwrap();
    let decoded = InviteToken::decode(&token.encode()).unwrap();
    assert_eq!(decoded, token);
}
