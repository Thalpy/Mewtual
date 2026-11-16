//! Boundary-only fixtures around actual Fresh factory reservations. Manually exhausting the
//! private revision counter below tests negative persistence, never historical proof authority.
use super::*;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SECRET: &[u8] = b"unexposed local lineage boundary fixture";

fn open(path: &std::path::Path, rng: &mut ChaCha20Rng) -> ServerStore {
    let store = ServerStore::open(path, SECRET, rng).unwrap();
    let store = if store.chat_vault_format().unwrap() == catcoms_storage::VaultFormat::OwnerEvidence
    {
        store
    } else {
        store.into_owner_evidence_store(SECRET, rng).unwrap()
    };
    store
        .configure_owner_evidence_budget(
            crate::store::OwnerEvidenceBudget::checkpoint(
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
    store
}

fn reserve(store: &ServerStore, rng: &mut ChaCha20Rng) -> ChatAdmissionPreparation {
    let mut private = store
        .private_managed_founder(
            Hub::new().join(PeerId::from_u64(0xe100)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(0xe100),
            Box::new(ManualClock::new(1_000)),
            "actual unexposed source",
        )
        .unwrap();
    private.reserve_scope(store, rng).unwrap()
}

#[test]
fn unexposed_original_capture_cannot_adopt_reopened_mount_even_with_the_same_vault_domain() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe101);
    let store = open(directory.path(), &mut rng);
    let scope = reserve(&store, &mut rng);
    let mut old = store
        .try_prepare_unexposed_owner_initialization(&scope.key)
        .unwrap()
        .unwrap();
    let domain = store.chat_vault_domain().unwrap();
    drop(store);
    let reopened = open(directory.path(), &mut rng);
    assert_eq!(reopened.chat_vault_domain().unwrap(), domain);
    assert!(old.retry(&reopened, &mut rng).is_err());
    assert!(matches!(
        reopened
            .resolve_chat_admission(&scope.key, scope.nonce())
            .unwrap(),
        ChatAdmissionResolution::Initializing(_)
    ));
    let mut fresh_negative = reopened
        .try_prepare_unexposed_owner_initialization(&scope.key)
        .unwrap()
        .unwrap();
    assert_eq!(
        fresh_negative.retry(&reopened, &mut rng).unwrap(),
        OwnerUnexposedRetirement::Retired
    );
}

#[test]
fn source_appearing_after_negative_capture_is_preserved_without_retiring_its_nonce() {
    for extension in ["bin", "replace.tmp"] {
        let directory = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xe102);
        let store = open(directory.path(), &mut rng);
        let scope = reserve(&store, &mut rng);
        let mut recovery = store
            .try_prepare_unexposed_owner_initialization(&scope.key)
            .unwrap()
            .unwrap();
        let mut hash = blake3::Hasher::new();
        hash.update(b"catcoms/owner-canonical-key/v1\0");
        hash.update(&(scope.key.group().len() as u32).to_be_bytes());
        hash.update(scope.key.group());
        hash.update(scope.key.signer());
        let path = store.owner_canonical_directory().unwrap().join(format!(
            "{}.{extension}",
            hex::encode(hash.finalize().as_bytes())
        ));
        let original = b"original retained bytes; authentication remains unproven";
        fs::write(&path, original).unwrap();
        assert_eq!(
            recovery.retry(&store, &mut rng).unwrap(),
            OwnerUnexposedRetirement::SourceRetained
        );
        assert_eq!(fs::read(path).unwrap(), original);
        assert!(matches!(
            store
                .resolve_chat_admission(&scope.key, scope.nonce())
                .unwrap(),
            ChatAdmissionResolution::Initializing(_)
        ));
    }
}

#[test]
fn exact_unexposed_negative_debt_still_closes_when_its_reserved_revision_is_exhausted() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe103);
    let store = open(directory.path(), &mut rng);
    let scope = reserve(&store, &mut rng);
    {
        let mut runtime = store.chat_mount.locked().unwrap();
        let row = runtime.table.as_mut().unwrap().get_mut(&scope.key).unwrap();
        row.revision = u64::MAX; // storage-boundary exhaustion fixture, not a fresh-proof source
        store.flush_chat_admissions(&mut runtime, &mut rng).unwrap();
    }
    let mut recovery = store
        .try_prepare_unexposed_owner_initialization(&scope.key)
        .unwrap()
        .unwrap();
    store.chat_parent_sync_failure.store(true, Ordering::SeqCst);
    assert!(matches!(
        recovery.retry(&store, &mut rng),
        Err(AppError::CommittedButNotDurable(_))
    ));
    assert_eq!(
        recovery.retry(&store, &mut rng).unwrap(),
        OwnerUnexposedRetirement::Retired
    );
    assert_eq!(
        recovery.retry(&store, &mut rng).unwrap(),
        OwnerUnexposedRetirement::Retired
    );
    let runtime = store.chat_mount.locked().unwrap();
    let row = runtime.table.as_ref().unwrap().get(&scope.key).unwrap();
    assert_eq!(row.revision, u64::MAX);
    assert_eq!(row.phase, Phase::Retired);
    assert!(!runtime.dirty);
}

#[test]
fn scope_only_vault_and_legacy_bootstrap_cannot_enter_fresh_owner_absence_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xe104);
    let store = ServerStore::open(directory.path(), SECRET, &mut rng).unwrap();
    store
        .enable_chat_scope_management(SECRET, &mut rng)
        .unwrap();
    let identity = ChatAdmissionKey::new(&[1], [2; 32]).unwrap();
    assert!(store
        .try_prepare_unexposed_owner_initialization(&identity)
        .is_err());
    let capture = store.capture_chat_admission(identity.clone()).unwrap();
    let legacy = store
        .prepare_legacy_chat_admission(&capture, b"exact old snapshot fixture", &mut rng)
        .unwrap();
    let store = store.into_owner_evidence_store(SECRET, &mut rng).unwrap();
    // Even critical vault upgrade cannot relabel a previously managed legacy reservation.
    assert!(store
        .try_prepare_unexposed_owner_initialization(&identity)
        .unwrap()
        .is_none());
    assert!(matches!(
        store
            .resolve_chat_admission(&identity, legacy.nonce())
            .unwrap(),
        ChatAdmissionResolution::Initializing(_)
    ));
}
