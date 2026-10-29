use super::*;
use catcoms_rt::{Hub, ManualClock};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn owner_evidence_private_profile_comes_from_actual_found_and_never_mutates_or_exports() {
    let mut owner = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(985)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(985),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let original = owner.group.epoch_authenticator_id();
    let a = owner.prepare_owner_evidence_checkpoint(9).unwrap();
    let b = owner.prepare_owner_evidence_checkpoint(10).unwrap();
    assert!(a.profile.verify());
    assert_eq!(
        a.profile_id(),
        b.profile_id(),
        "channel grants cannot choose a different group root"
    );
    assert_eq!(a.group(), owner.group_id());
    assert_eq!(a.founder().as_slice(), owner.device.public_key_bytes());
    assert_eq!(a.initial_channel(), 9);
    assert_eq!(owner.group.epoch_authenticator_id(), original);
    assert!(owner.outbox.is_empty());
    assert!(a.matches_original_mls_state(&owner));
    // Ordinary snapshot restore cannot recover this process-local fresh source or self-pin.
    let snapshot = owner.snapshot().unwrap();
    let restored = ChannelSync::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(986)),
        ChaCha20Rng::seed_from_u64(986),
        Box::new(ManualClock::new(2_000)),
    )
    .unwrap();
    assert!(matches!(
        restored.prepare_owner_evidence_checkpoint(9),
        Err(OwnerEvidencePrepareError::Context)
    ));
    assert!(
        a.matches_original_mls_state(&restored),
        "MLS equality alone is intentionally not the original physical/native lease"
    );
    let _scope_source = owner.take_fresh_chat_admission().unwrap();
    assert!(a.matches_original_mls_state(&owner));
    assert!(
        matches!(
            owner.prepare_owner_evidence_checkpoint(9),
            Err(OwnerEvidencePrepareError::Context)
        ),
        "a consumed source cannot mint another preparation even at unchanged epoch"
    );
    let newcomer = MlsDevice::generate().unwrap();
    let package = newcomer
        .key_package_for_invite(&owner.group_id(), [14; 16])
        .unwrap();
    owner
        .with_observed_mls_transition(|node| node.group.add_member(&node.device, package))
        .unwrap();
    assert!(!a.matches_original_mls_state(&owner));
    assert!(matches!(
        owner.prepare_owner_evidence_checkpoint(9),
        Err(OwnerEvidencePrepareError::Context)
    ));
}

#[test]
fn owner_evidence_preparation_types_remain_send_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<PreparedOwnerEvidence>();
}
