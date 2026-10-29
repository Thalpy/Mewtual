//! A genuine old signed response must not borrow the fresh init key of a later local attempt.
use super::*;
use catcoms_rt::{Hub, ManualClock};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn signed_old_welcome_with_available_old_provider_keys_refuses_a_fresh_attempt() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let mut node = ChannelSync::new(
        Hub::new().join(PeerId::from_u64(801)),
        group,
        owner,
        ChaCha20Rng::seed_from_u64(0x0080_1a77),
        Box::new(ManualClock::new(1_000)),
    );
    let invite = node.mint_invite([0x81; 16], 10_000, Vec::new()).unwrap();
    let joiner = MlsDevice::generate().unwrap();
    let local_group = ServerGroup::create(&joiner).unwrap();
    let old = joiner
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let old_reference = joiner.key_package_reference(&old).unwrap();
    let (welcome, routing, signature) = node
        .admit_now(&invite, &serialize_key_package(&old).unwrap(), 1_000)
        .unwrap();
    let fresh = joiner
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let expected = joiner.key_package_reference(&fresh).unwrap();
    let snapshot = snapshot_server(&joiner, &local_group).unwrap();
    let (restored, _) = restore_server(&snapshot).unwrap();
    assert!(
        ServerGroup::join(&restored, &welcome).is_ok(),
        "old init key is genuinely available"
    );
    assert!(
        finish_join(&joiner, &invite, &welcome, &signature, &routing, &expected).is_err(),
        "a real inviter signature does not bind another local attempt"
    );
    let mut invalid_signature = signature;
    invalid_signature[0] ^= 1;
    assert!(finish_join(
        &joiner,
        &invite,
        &welcome,
        &invalid_signature,
        &routing,
        &old_reference
    )
    .is_err());
    let (joined, _) = finish_join(
        &joiner,
        &invite,
        &welcome,
        &signature,
        &routing,
        &old_reference,
    )
    .unwrap();
    assert_eq!(
        joined.group_id(),
        invite.group_id,
        "both refusals happened before consuming the matching init material"
    );
}
