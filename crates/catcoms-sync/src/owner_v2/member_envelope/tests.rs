use super::*;
use catcoms_replication::chat::{ActionDraft, ActionSpec, BasisId, Body, GrantId, Realm};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn member_envelope_uses_the_real_mls_group_and_exact_signed_original() {
    let alice = MlsDevice::generate().unwrap();
    let bob = MlsDevice::generate().unwrap();
    let mut alice_group = ServerGroup::create(&alice).unwrap();
    let group = alice_group.group_id();
    let package = bob.key_package_for_invite(&group, [1; 16]).unwrap();
    let added = alice_group.add_member(&alice, package).unwrap();
    let bob_group = ServerGroup::join(&bob, &added.welcome).unwrap();
    let scope = V2MemberEnvelopeScopeData {
        group: group.try_into().unwrap(),
        profile: OwnerFactId::from_bytes([2; 32]),
        governing: OwnerHeaderId::from_bytes([3; 32]),
        epoch: alice_group.epoch(),
        channel: 42,
    };
    let signer = bob.public_key_bytes().try_into().unwrap();
    let draft = ActionDraft::new(
        ActionSpec {
            realm: Realm::new(&scope.group, scope.channel).unwrap(),
            basis: BasisId::from_bytes([4; 32]),
            grant: GrantId::from_bytes([5; 32]),
            nonce: [6; 16],
            presented_ms: 1,
            parents: vec![],
            body: Body::Create {
                text: "The original".into(),
                reply_to: None,
            },
        },
        signer,
    )
    .unwrap();
    let raw = bob
        .sign_retained_ed25519_bounded(&signer, draft.signing_bytes_data(), MAX_ACTION_BYTES)
        .unwrap();
    let action = draft.complete_raw64(raw).unwrap();
    let original = action.encode();
    let admission = OwnerFactId::from_bytes([7; 32]);
    let mut rng = ChaCha20Rng::seed_from_u64(8);
    assert!(seal_v2_member_envelope_data(
        &alice_group,
        &alice,
        &scope,
        admission,
        &original,
        &mut rng
    )
    .is_err());
    let frame =
        seal_v2_member_envelope_data(&bob_group, &bob, &scope, admission, &original, &mut rng)
            .unwrap();
    let topic = v2_member_envelope_topic_data(&alice_group, &alice, &scope).unwrap();
    assert_eq!(
        topic,
        v2_member_envelope_topic_data(&bob_group, &bob, &scope).unwrap()
    );
    let retry =
        seal_v2_member_envelope_data(&bob_group, &bob, &scope, admission, &original, &mut rng)
            .unwrap();
    assert_ne!(frame, retry);
    for wire in [&frame, &retry] {
        let opened = open_v2_member_envelope_data(&alice_group, &alice, &scope, wire).unwrap();
        assert_eq!(opened.admission_data(), admission);
        assert_eq!(opened.action_data().id(), action.id());
        assert_eq!(opened.original_data(), original);
    }
    for changed in [
        V2MemberEnvelopeScopeData {
            channel: 43,
            ..scope
        },
        V2MemberEnvelopeScopeData {
            group: [9; 16],
            ..scope
        },
        V2MemberEnvelopeScopeData {
            epoch: scope.epoch + 1,
            ..scope
        },
        V2MemberEnvelopeScopeData {
            profile: OwnerFactId::from_bytes([10; 32]),
            ..scope
        },
        V2MemberEnvelopeScopeData {
            governing: OwnerHeaderId::from_bytes([11; 32]),
            ..scope
        },
    ] {
        assert!(open_v2_member_envelope_data(&alice_group, &alice, &changed, &frame).is_err());
    }
    let mut damaged = frame.clone();
    *damaged.last_mut().unwrap() ^= 1;
    assert!(open_v2_member_envelope_data(&alice_group, &alice, &scope, &damaged).is_err());
    let mut trailing = frame.clone();
    trailing.push(0);
    assert!(open_v2_member_envelope_data(&alice_group, &alice, &scope, &trailing).is_err());
    assert!(open_v2_member_envelope_data(
        &alice_group,
        &alice,
        &scope,
        &vec![0; MAX_V2_MEMBER_TRANSPORT_BYTES + 1]
    )
    .is_err());

    alice_group.remove_member(&alice, &bob.device_id()).unwrap();
    let current = V2MemberEnvelopeScopeData {
        epoch: alice_group.epoch(),
        ..scope
    };
    assert!(current.epoch > scope.epoch);
    assert_ne!(
        topic,
        v2_member_envelope_topic_data(&alice_group, &alice, &current).unwrap()
    );
    assert!(open_v2_member_envelope_data(&alice_group, &alice, &scope, &frame).is_err());
    assert!(open_v2_member_envelope_data(&alice_group, &alice, &current, &frame).is_err());
}
