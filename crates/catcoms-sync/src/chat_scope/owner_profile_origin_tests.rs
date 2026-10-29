//! This predicate test uses the existing authenticated join finisher. It does not claim the
//! legacy admit_now publication order is suitable for the forthcoming owner-evidence save.
use super::*;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn owner_profile_origin_requires_actual_founder_in_original_state_not_later_roster_or_welcome() {
    let founder = MlsDevice::generate().unwrap();
    let mut owner = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(980)),
        founder,
        ChaCha20Rng::seed_from_u64(980),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert!(owner
        .fresh_chat_admission()
        .unwrap()
        .is_original_founder_state(&owner.group, &owner.device));
    let member = MlsDevice::generate().unwrap();
    let invite = owner.mint_invite([13; 16], 10_000, Vec::new()).unwrap();
    let package = member
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let expected = member.key_package_reference(&package).unwrap();
    let (welcome, routing, signature) = owner
        .admit_now(&invite, &serialize_key_package(&package).unwrap(), 1_000)
        .unwrap();
    // This completion is still valid for local scope initialization; it is no longer a
    // genesis-install point. Checking epoch zero alone would not establish either fact.
    let source = owner.fresh_chat_admission().unwrap();
    assert!(!source.is_original_founder_state(&owner.group, &owner.device));
    let (joined, transferred) =
        finish_join(&member, &invite, &welcome, &signature, &routing, &expected).unwrap();
    let source = transferred.fresh_chat_admission.as_ref().unwrap();
    assert!(!source.is_original_founder_state(&joined, &member));
    assert!(source.matches_initial_state(&joined, &member));
}
