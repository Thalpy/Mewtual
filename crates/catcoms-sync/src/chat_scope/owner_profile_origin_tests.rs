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

/// This exercises the existing actual MLS key custodian, not a fake DeviceKeypair carried
/// alongside it. Historical grant verification and save-before-publication are separate gates.
#[test]
fn owner_evidence_actual_finished_member_signs_original_typed_action_without_exporting_key() {
    use catcoms_replication::chat::{
        ActionSpec, BasisId, Body, ChatError, GrantId, Realm, SignedAction,
    };
    let mut owner = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(989)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(989),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let member = MlsDevice::generate().unwrap();
    let invite = owner.mint_invite([19; 16], 10_000, Vec::new()).unwrap();
    let package = member
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let expected = member.key_package_reference(&package).unwrap();
    // Legacy admit_now is used only to obtain a genuine authenticated finished member in this
    // bridge regression. The owner-evidence publication transaction must not call it on live state.
    let (welcome, routing, signature) = owner
        .admit_now(&invite, &serialize_key_package(&package).unwrap(), 1_000)
        .unwrap();
    let (joined, transfer) =
        finish_join(&member, &invite, &welcome, &signature, &routing, &expected).unwrap();
    assert!(transfer
        .fresh_chat_admission
        .as_ref()
        .unwrap()
        .matches_initial_state(&joined, &member));
    let spec = ActionSpec {
        realm: Realm::new(&joined.group_id(), 9).unwrap(),
        // Labels do not supply grants. The archive must later verify actual owner certificates.
        basis: BasisId::from_bytes([31; 32]),
        grant: GrantId::from_bytes([32; 32]),
        nonce: [19; 16],
        presented_ms: 1_000,
        parents: vec![],
        body: Body::Create {
            text: "from actual fresh-KP member".into(),
            reply_to: None,
        },
    };
    let member_key: [u8; 32] = member.public_key_bytes().try_into().unwrap();
    let action = SignedAction::sign_using(spec.clone(), member_key, |bytes| {
        member.sign(bytes).map_err(|_| ChatError::Signature)
    })
    .unwrap();
    assert!(action.verify());
    assert_eq!(action.signer(), &member_key);
    assert_eq!(SignedAction::decode(&action.encode()).unwrap(), action);
    assert_eq!(
        SignedAction::sign_using(spec, member_key, |bytes| {
            owner.device.sign(bytes).map_err(|_| ChatError::Signature)
        }),
        Err(ChatError::Signature),
        "a different actual MLS signer cannot sign as the admitted member"
    );
}
