//! Real canonical input/lineage classification. Final restored-head barrier and handoff are
//! separately exercised by the actual canonical restore consumer, not a fabricated receipt.
use super::*;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &[u8] = b"private canonical restore fixture";

type Owned =
    crate::store::owner_evidence::canonical::OwnedOwnerServer<catcoms_rt::MemNetwork, ChaCha20Rng>;
struct Fixture {
    directory: tempfile::TempDir,
    store: ServerStore,
    rng: ChaCha20Rng,
    group: Vec<u8>,
    signer: [u8; 32],
    owned: Owned,
}

fn initialized() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xd001);
    let store = ServerStore::open(directory.path(), SECRET, &mut rng)
        .unwrap()
        .into_owner_evidence_store(SECRET, &mut rng)
        .unwrap();
    store
        .configure_owner_evidence_budget(
            crate::store::owner_evidence::canonical::OwnerEvidenceBudget::checkpoint(
                4,
                16 * 1024 * 1024,
                8 * 1024 * 1024,
                1024 * 1024,
                512 * 1024,
                512 * 1024,
            )
            .unwrap(),
        )
        .unwrap();
    let device = MlsDevice::generate().unwrap();
    let signer = device.public_key_bytes().try_into().unwrap();
    let mut initialization = store
        .prepare_owner_evidence_founder(
            Hub::new().join(PeerId::from_u64(0xd001)),
            device,
            ChaCha20Rng::seed_from_u64(0xd001),
            Box::new(ManualClock::new(1_000)),
            "actual canonical restore source",
            9,
        )
        .unwrap();
    let owned = initialization.retry(&store, &mut rng).unwrap();
    let group = owned.group();
    Fixture {
        directory,
        store,
        rng,
        group,
        signer,
        owned,
    }
}

#[test]
fn current_canonical_decode_is_private_exact_and_has_no_fresh_or_publication_authority() {
    let Fixture {
        directory: _directory,
        store,
        rng: _rng,
        group,
        signer,
        owned,
    } = initialized();
    drop(owned);
    let source = store
        .prepare_owner_canonical_restore(
            store.capture_mounted_owner_profile(&group).unwrap(),
            signer,
        )
        .unwrap();
    let original_digest = source.snapshot_digest();
    let CanonicalOwnerRestore::Current(attempt) =
        store.classify_owner_canonical_restore(source).unwrap()
    else {
        panic!("the actual persisted admission is Current");
    };
    let restored = attempt
        .restore_private(
            &store,
            Hub::new().join(PeerId::from_u64(0xd002)),
            ChaCha20Rng::seed_from_u64(0xd002),
            Box::new(ManualClock::new(2_000)),
            "still private after exact decode",
        )
        .unwrap();
    assert_eq!(restored.source().snapshot_digest(), original_digest);
    assert_eq!(restored.server().unwrap().group_id(), group);
    assert!(restored
        .server()
        .unwrap()
        .sync
        .fresh_chat_admission()
        .is_none());
    assert!(restored
        .server()
        .unwrap()
        .sync
        .durable_send_context()
        .is_err());
    // No snapshot/re-encoding is used to compare the loaded provider. This exact private decode
    // still needs the distinct real same-file barrier, Current and saved restored-head handoff.
}

#[test]
fn durable_retirement_of_captured_active_canonical_bytes_keeps_capsule_without_transport() {
    let Fixture {
        directory,
        store,
        mut rng,
        group,
        signer,
        owned,
    } = initialized();
    let source = store
        .prepare_owner_canonical_restore(
            store.capture_mounted_owner_profile(&group).unwrap(),
            signer,
        )
        .unwrap();
    let nonce = source.nonce();
    let digest = source.snapshot_digest();
    let identity = ChatAdmissionKey::new(&group, signer).unwrap();
    let ChatAdmissionResolution::Current(binding) =
        store.resolve_chat_admission(&identity, nonce).unwrap()
    else {
        panic!("actual saved Current scope");
    };
    let path = fs::read_dir(directory.path().join("owner-state"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "bin"))
        .unwrap();
    let original_sealed = fs::read(&path).unwrap();
    // Release the original read fee before loading another source from the same bounded pool.
    drop(source);
    store.retire_chat_admission(&binding, &mut rng).unwrap();
    assert!(!owned.is_live());
    drop(owned);

    let retained = store
        .prepare_owner_canonical_restore(
            store.capture_mounted_owner_profile(&group).unwrap(),
            signer,
        )
        .unwrap();
    let result = store.classify_owner_canonical_restore(retained).unwrap();
    let capsule = match result {
        CanonicalOwnerRestore::Retired(capsule) => capsule,
        CanonicalOwnerRestore::Current(_) | CanonicalOwnerRestore::Initializing(_) => {
            panic!("retired bytes must never reach transport construction");
        }
    };
    assert_eq!(capsule.source().snapshot_digest(), digest);
    assert_eq!(capsule.source().nonce(), nonce);
    assert!(!capsule.source().snapshot().is_empty());
    assert_eq!(fs::read(path).unwrap(), original_sealed);
    // This claims retained original evidence only. Offline history projection is a separate
    // bounded reader; the capsule has no Server, transport, signer or run/subscribe method.
}

#[test]
fn retained_current_preparation_cannot_restore_after_its_original_admission_retires() {
    let Fixture {
        directory: _directory,
        store,
        mut rng,
        group,
        signer,
        owned,
    } = initialized();
    let source = store
        .prepare_owner_canonical_restore(
            store.capture_mounted_owner_profile(&group).unwrap(),
            signer,
        )
        .unwrap();
    let nonce = source.nonce();
    let CanonicalOwnerRestore::Current(attempt) =
        store.classify_owner_canonical_restore(source).unwrap()
    else {
        panic!("original classification is Current");
    };
    let identity = ChatAdmissionKey::new(&group, signer).unwrap();
    let ChatAdmissionResolution::Current(binding) =
        store.resolve_chat_admission(&identity, nonce).unwrap()
    else {
        panic!("actual same original admission");
    };
    store.retire_chat_admission(&binding, &mut rng).unwrap();
    assert!(!owned.is_live());
    assert!(attempt
        .restore_private(
            &store,
            Hub::new().join(PeerId::from_u64(0xd003)),
            ChaCha20Rng::seed_from_u64(0xd003),
            Box::new(ManualClock::new(3_000)),
            "stale preparation",
        )
        .is_err());
    assert!(matches!(
        store.resolve_chat_admission(&identity, nonce).unwrap(),
        ChatAdmissionResolution::Retired
    ));
    // The immutable canonical source remains available through a fresh Retired classification;
    // a retained earlier Current result cannot translate that retirement into a new admission.
    assert!(matches!(
        store
            .classify_owner_canonical_restore(
                store
                    .prepare_owner_canonical_restore(
                        store.capture_mounted_owner_profile(&group).unwrap(),
                        signer,
                    )
                    .unwrap()
            )
            .unwrap(),
        CanonicalOwnerRestore::Retired(_)
    ));
}
