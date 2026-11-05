//! Real canonical reopen/parent barrier and scope/writer outcomes, not fabricated certificates.
use super::*;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Hub, ManualClock, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
const SECRET: &[u8] = b"canonical restore actual barrier fixture";
fn budget() -> OwnerEvidenceBudget {
    OwnerEvidenceBudget::checkpoint(
        4,
        16 * 1024 * 1024,
        8 * 1024 * 1024,
        1024 * 1024,
        512 * 1024,
        512 * 1024,
    )
    .unwrap()
}
fn open(path: &std::path::Path, rng: &mut ChaCha20Rng) -> ServerStore {
    let store = ServerStore::open(path, SECRET, rng).unwrap();
    let store = if store.chat_vault_format().unwrap() == catcoms_storage::VaultFormat::OwnerEvidence
    {
        store
    } else {
        store.into_owner_evidence_store(SECRET, rng).unwrap()
    };
    store.configure_owner_evidence_budget(budget()).unwrap();
    store
}
fn founder(
    store: &ServerStore,
    peer: u64,
) -> OwnerFounderInitialization<catcoms_rt::MemNetwork, ChaCha20Rng> {
    store
        .prepare_owner_evidence_founder(
            Hub::new().join(PeerId::from_u64(peer)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(peer),
            Box::new(ManualClock::new(1_000)),
            "restart fixture",
            9,
        )
        .unwrap()
}
fn decode(
    prepared: OwnerRestorePreparation,
    store: &ServerStore,
    peer: u64,
) -> OwnerRestoreInitialization<catcoms_rt::MemNetwork, ChaCha20Rng> {
    prepared
        .restore_private(
            store,
            Hub::new().join(PeerId::from_u64(peer)),
            ChaCha20Rng::seed_from_u64(peer),
            Box::new(ManualClock::new(1_000)),
            "exact private restore",
        )
        .unwrap()
}

#[test]
fn real_post_rename_crash_reopen_requires_exact_restore_barrier_before_current_writer() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xd001);
    let store = open(directory.path(), &mut rng);
    let mut initial = founder(&store, 0xd001);
    store
        .owner_evidence
        .canonical_fault
        .store(2, Ordering::SeqCst);
    assert!(matches!(
        initial.retry(&store, &mut rng),
        Err(AppError::CommittedButNotDurable(_))
    ));
    let group = initial.group().unwrap().to_vec();
    let signer = *initial.founder().unwrap();
    let profile = initial.profile().unwrap();
    // Actual file rename happened; no Current inventory or positive runtime escaped.
    let path = directory
        .path()
        .join("owner-state")
        .join(format!("{}.bin", hex::encode(key(&group, &signer))));
    let ciphertext = fs::read(&path).unwrap();
    let plain = Zeroizing::new(
        unseal(
            &store.keys.db_key().unwrap(),
            &crate::store::unframe(&ciphertext).unwrap(),
        )
        .unwrap(),
    );
    let mut fields = Decoder::new(&plain);
    fields.get_str().unwrap();
    for _ in 0..4 {
        fields.get_bytes().unwrap();
    } // group/signer/profile/domain
    let nonce: [u8; 32] = fields.get_bytes().unwrap().try_into().unwrap();
    fields.get_u64().unwrap();
    fields.get_u64().unwrap();
    fields.get_bytes().unwrap();
    fields.get_bytes().unwrap(); // parent/initial debt label
    let snapshot = *blake3::hash(fields.get_bytes().unwrap()).as_bytes();
    fields.finish().unwrap();

    drop(initial);
    drop(store);
    let reopened = open(directory.path(), &mut rng);
    let OwnerRestoreClassification::Initializing(prepared) = reopened
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("saved Active candidate must resume ORIGINAL external Initializing");
    };
    let mut restore = decode(prepared, &reopened, 0xd002);
    assert_eq!(restore.private.as_ref().unwrap().source().nonce(), nonce);
    assert_eq!(
        restore.private.as_ref().unwrap().source().snapshot_digest(),
        snapshot
    );
    assert!(restore
        .private
        .as_ref()
        .unwrap()
        .server()
        .unwrap()
        .sync
        .fresh_chat_admission()
        .is_none());
    assert!(restore
        .private
        .as_ref()
        .unwrap()
        .server()
        .unwrap()
        .sync
        .durable_send_context()
        .is_err());
    reopened
        .owner_evidence
        .restore_sync_failure
        .store(true, Ordering::SeqCst);
    assert!(restore.retry(&reopened, &mut rng).is_err());
    assert!(restore.saved.is_none());
    assert!(restore.writer.is_none());
    let identity = crate::store::ChatAdmissionKey::new(&group, signer).unwrap();
    assert!(matches!(
        reopened.resolve_chat_admission(&identity, nonce).unwrap(),
        ChatAdmissionResolution::Initializing(_)
    ));
    assert_eq!(fs::read(&path).unwrap(), ciphertext);
    // SAME opened-file+child-directory barrier retries; visible/authentic bytes were insufficient.
    assert!(restore.retry(&reopened, &mut rng).is_err());
    reopened
        .owner_evidence
        .restore_sync_failure
        .store(false, Ordering::SeqCst);
    let owned = restore.retry(&reopened, &mut rng).unwrap();
    assert!(owned.is_live());
    assert_eq!(owned.profile(), profile);
    assert_eq!(owned.group(), group);
    assert!(
        restore.private.is_none(),
        "read source/fee released outside final writers"
    );
    assert_eq!(
        reopened.owner_evidence.canonical.lock().unwrap().hot_bytes,
        0
    );
    assert!(matches!(
        reopened.resolve_chat_admission(&identity, nonce).unwrap(),
        ChatAdmissionResolution::Current(_)
    ));
    assert_eq!(
        fs::read(&path).unwrap(),
        ciphertext,
        "exact restore sync rewrites no signed evidence"
    );
    assert!(restore.retry(&reopened, &mut rng).is_err());
}

#[test]
fn current_reopen_refuses_second_live_writer_and_retired_classifies_without_transport() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xd003);
    let store = open(directory.path(), &mut rng);
    let mut initial = founder(&store, 0xd003);
    let owned = initial.retry(&store, &mut rng).unwrap();
    let group = owned.group();
    let signer = *owned.founder();
    let OwnerRestoreClassification::Current(prepared) = store
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("Current");
    };
    let mut duplicate = decode(prepared, &store, 0xd004);
    assert!(
        duplicate.retry(&store, &mut rng).is_err(),
        "one live writer including pre-registration owns the head"
    );
    assert!(duplicate.writer.is_none());
    // Drop the actual original revoking owner; a retained dummy identity/scope cannot keep it live.
    let old_authority = owned.publication_authority();
    drop(owned);
    assert!(old_authority.is_cancelled());
    let resumed = duplicate.retry(&store, &mut rng).unwrap();
    assert!(resumed.is_live());
    assert!(
        old_authority.is_cancelled(),
        "new registration never renews old queued authority"
    );
    resumed.retire_local_owner(&store, &mut rng).unwrap();
    assert!(!resumed.is_live());
    drop(resumed);
    let OwnerRestoreClassification::Retired(offline) = store
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("Retired before T/MLS");
    };
    assert_eq!(offline.group(), group);
    assert_eq!(offline.signer(), &signer);
    // No transport constructor is even accepted by the Retired variant. The private source is
    // retained, with no claim ordinary positive MLS restore can load a removed own leaf.
}

#[test]
fn authenticated_stripped_owner_tail_refuses_before_any_reservation_attachment() {
    let directory = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xd005);
    let store = open(directory.path(), &mut rng);
    let mut initial = founder(&store, 0xd005);
    let owned = initial.retry(&store, &mut rng).unwrap();
    let group = owned.group();
    let signer = *owned.founder();
    let path = directory
        .path()
        .join("owner-state")
        .join(format!("{}.bin", hex::encode(key(&group, &signer))));
    let original = fs::read(&path).unwrap();
    let plain = Zeroizing::new(
        unseal(
            &store.keys.db_key().unwrap(),
            &crate::store::unframe(&original).unwrap(),
        )
        .unwrap(),
    );
    let mut decoder = Decoder::new(&plain);
    let mut replacement = Encoder::with_capacity(plain.len());
    replacement.put_str(decoder.get_str().unwrap()).unwrap();
    for _ in 0..5 {
        replacement.put_bytes(decoder.get_bytes().unwrap()).unwrap();
    }
    replacement.put_u64(decoder.get_u64().unwrap());
    replacement.put_u64(decoder.get_u64().unwrap());
    replacement.put_bytes(decoder.get_bytes().unwrap()).unwrap();
    replacement.put_bytes(decoder.get_bytes().unwrap()).unwrap();
    let snapshot = decoder.get_bytes().unwrap();
    decoder.finish().unwrap();
    let tag = b"catcoms/owner-evidence-initial/v1";
    let at = snapshot
        .windows(tag.len())
        .position(|bytes| bytes == tag)
        .unwrap();
    assert!(at >= 8);
    // Remove exactly the outer owner-tail length plus inner tag framing/body. The preceding
    // actual Active stable-scope tail remains intact and would parse as a legacy snapshot.
    let stripped = &snapshot[..at - 8];
    replacement.put_bytes(stripped).unwrap();
    // Test-only authenticated malformed source: this is NOT a claim a peer without the DEK
    // can edit the canonical file. The parser/old-source defense must still refuse safely.
    let sealed = seal(
        &store.keys.db_key().unwrap(),
        &replacement.finish(),
        &mut rng,
    )
    .unwrap();
    let malformed = crate::store::frame(&sealed);
    fs::write(&path, &malformed).unwrap();
    drop(owned);
    drop(initial);
    drop(store);
    let reopened = open(directory.path(), &mut rng); // no in-memory known-head stamp survives
    let OwnerRestoreClassification::Current(prepared) = reopened
        .prepare_owner_evidence_restore(&group, signer)
        .unwrap()
    else {
        panic!("external row alone remains Current");
    };
    let refused = prepared.restore_private(
        &reopened,
        Hub::new().join(PeerId::from_u64(0xd006)),
        ChaCha20Rng::seed_from_u64(0xd006),
        Box::new(ManualClock::new(1_000)),
        "stripped owner marker",
    );
    assert!(refused.is_err());
    assert_eq!(
        fs::read(&path).unwrap(),
        malformed,
        "refusal never writes over retained source"
    );
    assert_eq!(
        reopened.owner_evidence.canonical.lock().unwrap().hot_bytes,
        0
    );
}
