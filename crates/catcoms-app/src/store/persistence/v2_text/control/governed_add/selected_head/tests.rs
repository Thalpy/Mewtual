use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn governed_add_head2_preserves_the_old_head_envelope_and_refuses_legacy_selection() {
    let transition = GovernedArchiveLabels {
        context: TextContextIdData::from_bytes([1; 32]),
        before_head: TextHeadIdData::from_bytes([2; 32]),
        before_archive: ArchiveHeadId::from_bytes([3; 32]),
        header: OwnerHeaderId::from_bytes([4; 32]),
        state: OwnerStateCommitment::from_bytes([5; 32]),
        epoch: 1,
        authenticator: [6; 32],
    };
    let archive = ArchiveHeadId::from_bytes([7; 32]);
    let key = [28; 32];
    let mut rng = ChaCha20Rng::seed_from_u64(166);
    let data = GovernedHeadData::after_first_archive(transition, archive, 1).unwrap();
    assert_eq!(data.generation, 2);
    let raw = data.seal_data(&key, &mut rng).unwrap();
    assert_eq!(raw.len(), FRAME_LEN);
    assert!(GovernedHeadData::open_data(&key, &raw).unwrap() == data);
    assert!(matches!(
        inspect_known_control_mac_data(&key, &raw).unwrap(),
        TextControlMacInspectionData::Authenticated
    ));
    assert!(open_control_data(&key, KIND.id_data(&raw), &raw).is_err());
    assert!(GovernedHeadData::open_data(&key, &raw[1..]).is_err());
    assert!(
        GovernedHeadData::after_first_archive(transition, transition.before_archive, 1).is_err()
    );
    assert!(GovernedHeadData::after_first_archive(transition, archive, u64::MAX).is_err());
    let later = GovernedArchiveLabels {
        epoch: 2,
        ..transition
    };
    assert!(GovernedHeadData::after_first_archive(later, archive, 1).is_err());
    assert!(GovernedHeadData::after_first_archive(later, archive, 2).is_err());

    let limit = MAX_V2_GOVERNING_JOIN_STEPS as u64;
    assert_eq!(next_supported_epoch(limit - 1).unwrap(), limit);
    assert!(next_supported_epoch(limit).is_err());
    let parent = GovernedHeadData {
        transition: GovernedArchiveLabels {
            epoch: limit - 1,
            ..transition
        },
        generation: limit,
        archive,
        kind: GovernedHeadKind::Add,
    }
    .seal_kind(&key, GovernedHeadKind::Add, &mut rng)
    .unwrap();
    let removal = GovernedArchiveLabels {
        before_head: TextHeadIdData::of_data(&parent),
        before_archive: archive,
        header: OwnerHeaderId::from_bytes([8; 32]),
        state: OwnerStateCommitment::from_bytes([9; 32]),
        authenticator: [10; 32],
        epoch: limit,
        ..transition
    };
    let selected_archive = ArchiveHeadId::from_bytes([11; 32]);
    let selected = GovernedHeadData::after_successor(
        &key,
        &parent,
        removal,
        selected_archive,
        GovernedHeadKind::Remove,
    )
    .unwrap();
    let selected_bytes = selected
        .seal_kind(&key, GovernedHeadKind::Remove, &mut rng)
        .unwrap();
    let beyond = GovernedArchiveLabels {
        before_head: TextHeadIdData::of_data(&selected_bytes),
        before_archive: selected_archive,
        header: OwnerHeaderId::from_bytes([12; 32]),
        state: OwnerStateCommitment::from_bytes([13; 32]),
        authenticator: [14; 32],
        epoch: limit + 1,
        ..removal
    };
    assert!(GovernedHeadData::after_successor(
        &key,
        &selected_bytes,
        beyond,
        archive,
        GovernedHeadKind::Remove
    )
    .is_err());
    assert!(GovernedHeadData::open_governed_data(&key, &selected_bytes).unwrap() == selected);
}
