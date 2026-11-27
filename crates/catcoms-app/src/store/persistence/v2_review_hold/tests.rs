use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::{
    acceptance_tree::TreeContext,
    cold_actions::PreparedActionPages,
    owner_bodies::{AcceptanceRecord, AcceptanceSpec},
    owner_facts::OwnerFactId,
    owner_header::OwnerTenureId,
    ActionSpec, BasisId, Body, GrantId, Realm, SignedAction,
};
use catcoms_wire::Encoder;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

pub(super) const KEY: [u8; 32] = [80; 32];

pub(super) fn scope() -> SharedHistoryScope {
    SharedHistoryScope {
        vault: [1; 32],
        group: [2; 16],
        profile: OwnerFactId::from_bytes([3; 32]),
        tenure: OwnerTenureId::from_bytes([4; 32]),
        creation: [5; 32],
        channel: 6,
    }
}

/// Build wire DATA as a decrypted vault would supply it. This fixture never
/// constructs provenance. Core tests cover the signed governing reproof.
pub(super) fn fixture(
    scope: SharedHistoryScope,
    admission: u8,
    nonce: u8,
) -> (Zeroizing<Vec<u8>>, HeldCase) {
    let action = SignedAction::sign(
        &DeviceKeypair::from_seed(&[7; 32]),
        ActionSpec {
            realm: Realm::new(&scope.group, scope.channel).unwrap(),
            basis: BasisId::from_bytes([8; 32]),
            grant: GrantId::from_bytes([9; 32]),
            nonce: [nonce; 16],
            presented_ms: 11,
            parents: vec![],
            body: Body::Create {
                text: "unproven after the removal".into(),
                reply_to: None,
            },
        },
    )
    .unwrap();
    let target = AcceptanceRecord::new(AcceptanceSpec {
        group: scope.group.to_vec(),
        context: TreeContext {
            profile: scope.profile,
            admission: OwnerFactId::from_bytes([admission; 32]),
        },
        channel: scope.channel,
        action: action.id(),
        subject: *action.signer(),
        grant: action.spec().grant,
        basis: action.spec().basis,
        create: None,
    })
    .unwrap();
    let original = Zeroizing::new(action.encode());
    let digest = PreparedActionPages::prepare(scope.vault, scope.profile, &action)
        .unwrap()
        .original_digest();
    let removal = (OwnerHeaderId::from_bytes([10; 32]), 2);
    let mut writer = Encoder::with_capacity(MAX_REMOVED_CREATE_RECORD_BYTES);
    writer
        .put_bytes(b"mewtual/chat/removed-create-hold/v1\0")
        .unwrap();
    writer.put_bytes(&target.encode()).unwrap();
    writer.put_bytes(removal.0.as_bytes()).unwrap();
    writer.put_u64(removal.1);
    writer.put_bytes(digest.as_bytes()).unwrap();
    writer.put_bytes(&original).unwrap();
    (
        Zeroizing::new(writer.finish()),
        HeldCase {
            id: ReviewCaseId::from_target_data(&target),
            action: action.id(),
            original: digest,
            removal,
        },
    )
}

#[test]
fn encrypted_hold_preserves_exact_case_and_binds_every_namespace_coordinate() {
    let coordinates = scope();
    let original = HoldScope::new(coordinates);
    let (plain, expected) = fixture(coordinates, 13, 14);
    let mut rng = ChaCha20Rng::seed_from_u64(12);
    let (identity, sealed) = original.seal(&KEY, &plain, &mut rng).unwrap();
    assert_eq!(identity, expected);
    let reopened = original.open(&KEY, &sealed).unwrap();
    assert_eq!(reopened.identity, expected);
    assert_eq!(reopened.plain.as_slice(), plain.as_slice());
    assert_ne!(original.seal(&KEY, &plain, &mut rng).unwrap().1, sealed);
    assert!(original.open(&[240; 32], &sealed).is_err());
    for foreign in [
        SharedHistoryScope {
            vault: [61; 32],
            ..coordinates
        },
        SharedHistoryScope {
            group: [61; 16],
            ..coordinates
        },
        SharedHistoryScope {
            profile: OwnerFactId::from_bytes([61; 32]),
            ..coordinates
        },
        SharedHistoryScope {
            tenure: OwnerTenureId::from_bytes([61; 32]),
            ..coordinates
        },
        SharedHistoryScope {
            creation: [61; 32],
            ..coordinates
        },
        SharedHistoryScope {
            channel: 61,
            ..coordinates
        },
    ] {
        let foreign = HoldScope::new(foreign);
        assert!(foreign.open(&KEY, &sealed).is_err());
        assert_ne!(
            original.file_label(&KEY, expected.id),
            foreign.file_label(&KEY, expected.id)
        );
    }
    // The same signed bytes associated with a different claimed Admission
    // can parse as DATA, but they never alias the first retained case.
    let (_, other) = fixture(coordinates, 15, 14);
    assert_eq!(other.original, expected.original);
    assert_ne!(
        original.file_label(&KEY, other.id),
        original.file_label(&KEY, expected.id)
    );
}

#[test]
fn encrypted_hold_rejects_tampering_even_inside_an_authenticated_outer_frame() {
    let coordinates = scope();
    let original = HoldScope::new(coordinates);
    let mut rng = ChaCha20Rng::seed_from_u64(12);
    let (mut plain, _) = fixture(coordinates, 13, 14);
    let (_, mut sealed) = original.seal(&KEY, &plain, &mut rng).unwrap();
    *sealed.last_mut().unwrap() ^= 1;
    assert!(original.open(&KEY, &sealed).is_err());
    assert!(original.open(&KEY, &vec![0; MAX_FRAME + 1]).is_err());
    *plain.last_mut().unwrap() ^= 1;
    let authentic_but_invalid =
        frame(&catcoms_crypto::seal(&original.key(&KEY), &plain, &mut rng).unwrap());
    assert!(original.open(&KEY, &authentic_but_invalid).is_err());
}
