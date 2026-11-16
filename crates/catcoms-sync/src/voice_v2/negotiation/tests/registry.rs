//! Registry publication/destruction uses original identities, not phase liveness snapshots.
use super::*;
use catcoms_crypto::DeviceKeypair;

fn bound(resources: &Arc<Resources>, seed: u8) -> Result<Arc<Exchange>, VoiceError> {
    let scope = Scope::for_group(b"registered SDP owners", Some([7; 32]), 44);
    Exchange::new_bound(
        resources.clone(),
        Binding {
            scope,
            local: DeviceKeypair::from_seed(&[seed; 32]).device_id(),
            join: [seed; 32],
            key_ref: [2; 32],
            peer: super::super::super::packets::PairBinding {
                peer: DeviceKeypair::from_seed(&[seed + 1; 32]).device_id(),
                join: [3; 32],
                key_ref: [4; 32],
                handshake: [5; 32],
            },
            call: CallLease::default(),
            pair: CallLease::default(),
            helpers: Vec::new(),
        },
        Arc::new(super::super::super::crypto_actions::slots::Slots::default()).reserve(2)?,
    )
}

#[test]
fn sdp_registry_competing_original_owner_cannot_replace_or_refund_current_capacity() {
    let resources = Arc::new(Resources::default());
    let first = bound(&resources, 1).unwrap();
    let group = first.group;
    let before = resources.counts();
    assert_eq!(bound(&resources, 3).err(), Some(VoiceError::Capacity));
    assert!(Arc::ptr_eq(&resources.exchange(group).unwrap(), &first));
    assert_eq!(resources.counts(), before);
    resources.register_exchange(group, &first).unwrap();
    assert_eq!(resources.counts(), before, "same allocation is idempotent");
    let foreign = Arc::new(Resources::default());
    assert_eq!(
        foreign.register_exchange(group, &first),
        Err(VoiceError::Scope)
    );
    assert_eq!(
        resources.register_exchange([9; 32], &first),
        Err(VoiceError::Scope)
    );
    assert_eq!(foreign.counts(), (0, 0));
}

#[test]
fn sdp_registry_closed_handle_or_actual_lower_blocks_replacement_until_real_last_drop() {
    for lower in [false, true] {
        let resources = Arc::new(Resources::default());
        let first = bound(&resources, 1).unwrap();
        let group = first.group;
        let independent = resources.charge(group, 17, ResourceKind::Retained).unwrap();
        let retained_handle = (!lower).then(|| first.clone());
        let retained_lower = lower.then(|| first.charge(Stage::Lower, 128).unwrap());
        first.close();
        drop(first);
        let before = resources.counts();
        assert_eq!(bound(&resources, 3).err(), Some(VoiceError::Capacity));
        assert_eq!(resources.counts(), before);
        assert_eq!(
            resources.retained_phase_indices_for_test(group),
            (false, true)
        );
        drop((retained_handle, retained_lower));
        // Check the actual index, not Weak::upgrade/weak_count after strong-zero. Counts is
        // still alive because this unrelated original same-group payment has not returned.
        assert_eq!(
            resources.retained_phase_indices_for_test(group),
            (false, false)
        );
        assert_eq!(resources.counts(), (17, 0));
        let next = bound(&resources, 3).unwrap();
        assert!(Arc::ptr_eq(&resources.exchange(group).unwrap(), &next));
        drop((next, independent));
        assert_eq!(resources.counts(), (0, 0));
    }
}

#[test]
fn sdp_registry_late_original_cleanup_cannot_clear_a_newly_installed_identity() {
    let resources = Arc::new(Resources::default());
    let first = bound(&resources, 1).unwrap();
    let group = first.group;
    // Prepare the new allocation while old is still alive so allocator address reuse cannot
    // make this raw-identity comparison a synthetic ABA. No pointer is ever dereferenced.
    let next = Exchange::new(resources.clone(), group).unwrap();
    let original = Arc::as_ptr(&first);
    drop(first);
    resources.register_exchange(group, &next).unwrap();
    // Resume the exact destructor clear with its old captured identity after replacement.
    resources.unregister_exchange(group, original);
    assert!(Arc::ptr_eq(&resources.exchange(group).unwrap(), &next));
}
