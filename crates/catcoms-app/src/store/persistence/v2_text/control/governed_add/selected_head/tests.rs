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
    let data = GovernedHeadData::after_archive(transition, archive, 1).unwrap();
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
    assert!(GovernedHeadData::after_archive(transition, transition.before_archive, 1).is_err());
    assert!(GovernedHeadData::after_archive(transition, archive, u64::MAX).is_err());
}
