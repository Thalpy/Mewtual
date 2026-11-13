//! Genuine encrypted selected3 tuples; classification alone is not a writing
//! claim, native command, membership proof or SDK activation test.
use super::super::{
    data::Backend,
    preparation::{Policy, PREPARATION_FEE},
    rename,
};
use super::*;
use crate::store::critical_namespaces::native_registry::preparation::tests::{
    current_policy, initialize, records, SECRET,
};
use crate::store::{
    CriticalCatalogReadPool, NativeIndexPolicy, NativeIndexUpgrade, NativeRegistryInstallation,
    NativeRegistryPolicy,
};
use catcoms_crypto::seal;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(in crate::store::critical_namespaces::native_registry) fn mounted(
    path: &Path,
    rng: &mut ChaCha20Rng,
) -> super::super::super::operational::upgrade_source::owning::Store {
    initialize(path, Some(&records()), rng);
    let first = NativeRegistryPolicy::new(
        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
        NativeRegistryPolicy::required_persistent_capacity(),
    )
    .unwrap();
    let mut install =
        NativeRegistryInstallation::prepare(path, SECRET, rng, current_policy(), first).unwrap();
    install.retry().unwrap();
    let second = NativeIndexPolicy::new(
        CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity()).unwrap(),
        NativeIndexPolicy::required_persistent_capacity(),
    )
    .unwrap();
    let mut upgrade =
        NativeIndexUpgrade::prepare(install.into_native_registry().unwrap(), second, rng).unwrap();
    upgrade.retry().unwrap();
    upgrade
        .into_index_store()
        .unwrap()
        .into_lifecycle3_for_test(rng)
        .unwrap()
}
fn stage(path: &Path) -> PathBuf {
    path.join(crate::store::critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
        .join(super::super::super::preparation::INDEX_STAGE_NAME)
}
fn seal_plain(store: &ServerStore, plain: &[u8], rng: &mut ChaCha20Rng) -> Vec<u8> {
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    let derived = Zeroizing::new(blake3::derive_key(preparation::INDEX_KEY, key.as_slice()));
    crate::store::frame(&seal(&derived, plain, rng).unwrap())
}
fn invoice() -> (
    CriticalCatalogReadPool,
    crate::store::critical_namespaces::read_pool::CatalogReadReservation,
) {
    let pool = CriticalCatalogReadPool::new(PREPARATION_FEE).unwrap();
    let paid = Policy::new(pool.clone(), Policy::required_persistent_capacity())
        .unwrap()
        .reserve()
        .unwrap();
    (pool, paid)
}

#[test]
fn lifecycle3_wal_classifies_exact_duplicate_short_and_actual_bad_mac_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc501);
    let owner = mounted(dir.path(), &mut rng);
    let store = owner.actual_store_for_test().unwrap();
    let (pool, paid) = invoice();
    let selected = {
        let inventory = UpgradeInventory::capture(store, &paid, &pool).unwrap();
        inventory.index().to_vec()
    };
    let mut samples = vec![(selected.clone(), Decision::Duplicate)];
    for length in [0, 1, 23, 24, 39] {
        samples.push((vec![0; length], Decision::DiscardShort));
    }
    let mut bad_mac = selected.clone();
    *bad_mac.last_mut().unwrap() ^= 1;
    samples.push((bad_mac, Decision::DiscardBadMac));
    for (bytes, decision) in samples {
        fs::write(stage(dir.path()), &bytes).unwrap();
        let inventory = UpgradeInventory::capture(store, &paid, &pool).unwrap();
        let proof = classify(store, &inventory).unwrap();
        assert_eq!(proof.decision(), decision);
        proof.require_inventory(&inventory).unwrap();
        assert_eq!(fs::read(stage(dir.path())).unwrap(), bytes); // no classification side effect
        drop(inventory);
        fs::remove_file(stage(dir.path())).unwrap();
    }
}

#[test]
fn lifecycle3_wal_preserves_authenticated_invalid_future_reencrypted_and_unsupported_data() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc502);
    let owner = mounted(dir.path(), &mut rng);
    let store = owner.actual_store_for_test().unwrap();
    let (pool, paid) = invoice();
    let selected = {
        let inventory = UpgradeInventory::capture(store, &paid, &pool).unwrap();
        preparation::decode_selected_data(store, &inventory)
            .unwrap()
            .0
    };
    let reencrypted = seal_plain(store, &selected.encode_data().unwrap(), &mut rng);
    let mut retired =
        Index::decode_data(&selected.encode_data().unwrap(), selected.capability).unwrap();
    let parent = {
        let inventory = UpgradeInventory::capture(store, &paid, &pool).unwrap();
        *blake3::hash(inventory.index()).as_bytes()
    };
    retired.generation += 1;
    retired.parent = Some(parent);
    let Backend::Legacy { origin, invite } = &retired.rows[0].backend else {
        panic!("real imported Legacy fixture")
    };
    retired.rows[0].backend = Backend::RetiredLegacy {
        origin: origin.clone(),
        invite: invite.clone(),
        terminal: [88; 32],
    };
    assert!(matches!(
        selected.successor_data(&retired, parent),
        Ok(Change::Retire)
    ));
    let samples = [
        seal_plain(store, b"", &mut rng),
        seal_plain(store, b"future-index-v4", &mut rng),
        reencrypted,
        seal_plain(store, &retired.encode_data().unwrap(), &mut rng),
    ];
    for bytes in samples {
        fs::write(stage(dir.path()), &bytes).unwrap();
        let inventory = UpgradeInventory::capture(store, &paid, &pool).unwrap();
        assert!(classify(store, &inventory).is_err());
        assert_eq!(fs::read(stage(dir.path())).unwrap(), bytes);
        drop(inventory);
        fs::remove_file(stage(dir.path())).unwrap();
    }
}

#[test]
fn lifecycle3_wal_rename_proof_binds_this_inventory_and_rejects_two_simultaneous_wals() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xc503);
    let owner = mounted(dir.path(), &mut rng);
    let store = owner.actual_store_for_test().unwrap();
    let (pool, paid) = invoice();
    let original = UpgradeInventory::capture(store, &paid, &pool).unwrap();
    let (selected, _) = preparation::decode_selected_data(store, &original).unwrap();
    let first = &selected.rows[0];
    let request = rename::Request {
        id: first.id,
        identity: first.identity,
        expected_revision: first.metadata_revision,
        name: "new local name".into(),
    };
    let rename::Plan::Change { candidate, .. } = rename::prepare(
        selected,
        &request,
        *blake3::hash(original.index()).as_bytes(),
    )
    .unwrap() else {
        panic!("one rename")
    };
    let frame = seal_plain(store, &candidate.encode_data().unwrap(), &mut rng);
    let warm = verify_warm_rename(store, &original, &candidate, &frame).unwrap();
    assert!(!warm.has_stage());
    warm.require_inventory(&original).unwrap();
    drop(original);
    fs::write(stage(dir.path()), &frame).unwrap();
    let observed = UpgradeInventory::capture(store, &paid, &pool).unwrap();
    let proof = classify(store, &observed).unwrap();
    assert_eq!(proof.decision(), Decision::Rename);
    assert!(proof.has_stage());
    let other = UpgradeInventory::capture(store, &paid, &pool).unwrap();
    assert!(proof.require_inventory(&other).is_err());
    proof.require_inventory(&observed).unwrap();
    drop(other);
    drop(observed);
    let catalog_stage = dir
        .path()
        .join(crate::store::critical_namespaces::Slot::Stage.name());
    let current = fs::read(
        dir.path()
            .join(crate::store::critical_namespaces::Slot::Current.name()),
    )
    .unwrap();
    fs::write(&catalog_stage, &current).unwrap();
    let both = UpgradeInventory::capture(store, &paid, &pool).unwrap();
    assert!(classify(store, &both).is_err());
    assert_eq!(fs::read(stage(dir.path())).unwrap(), frame);
    assert_eq!(fs::read(catalog_stage).unwrap(), current);
}
