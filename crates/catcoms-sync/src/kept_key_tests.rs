//! The supported absent-routing-transfer state has no file key, including after restore.
//! It may recover signed metadata, but that is insufficient authority to execute a Keep.
use super::*;
use automerge::{transaction::Transactable, ROOT};
use catcoms_rt::ManualClock;
use catcoms_storage::{
    kept::{KeepPlan, KeepScope, KeptBlobStore},
    BlobStore, MemoryBlobStore,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[tokio::test]
async fn absent_transfer_key_refuses_retention_without_reservation_or_spend_after_restore() {
    let (hub, mut members, _) = crate::tests::build_members(2).await;
    let joined = members.pop().unwrap();
    let mut source = members.pop().unwrap();
    let (reference, ciphertext) = source
        .seal_file(b"genuine encrypted member file", "application/octet-stream")
        .unwrap();
    assert!(source.open_file(&ciphertext, &reference).is_ok());
    let encoded = reference.encode();
    source.open_channel(DocType::FileIndex, 0).await.unwrap();
    source
        .post(DocType::FileIndex, 0, |doc| {
            doc.put(ROOT, "reference", hex::encode(&encoded))
        })
        .await
        .unwrap();
    // Real MLS admission supplied this group/device. The public compatibility constructor
    // explicitly permits an absent routing transfer; it must not invent a group file key.
    let mut keyless = ChannelSync::new_joined(
        hub.join(PeerId::from_u64(88)),
        joined.group,
        joined.device,
        ChaCha20Rng::seed_from_u64(88),
        Box::new(ManualClock::new(1_000)),
        RoutingState::default(),
    );
    assert!(!keyless.has_file_key());
    let saved = keyless.snapshot().unwrap();
    drop(keyless);
    let mut restored = ChannelSync::restore(
        &saved,
        hub.join(PeerId::from_u64(88)),
        ChaCha20Rng::seed_from_u64(89),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert!(!restored.has_file_key());
    restored.open_channel(DocType::FileIndex, 0).await.unwrap();
    let (imported, _) = tokio::join!(
        restored.request_catchup(source.local_peer(), DocType::FileIndex, 0),
        source.run_once()
    );
    assert!(
        imported.unwrap() > 0,
        "original signed file metadata remains recoverable"
    );
    let plan = KeepPlan {
        cid: reference.plaintext_cid,
        version: *blake3::hash(&encoded).as_bytes(),
        manifest: encoded,
        chunks: vec![(reference.ciphertext_cid, ciphertext.len() as u64)],
    };
    let dir = tempfile::tempdir().unwrap();
    let make_store = || {
        KeptBlobStore::open(
            Box::new(MemoryBlobStore::new()),
            dir.path().join("kept"),
            [6; 32],
            [8; 32],
            ChaCha20Rng::seed_from_u64(90),
        )
    };
    restored.set_blob_store(Box::new(make_store()));
    let empty = restored.kept_files();
    assert!(
        restored
            .begin_keep_with_listing(plan.clone(), plan.version)
            .is_err(),
        "keyless admission must refuse before creating durable ownership"
    );
    assert_eq!(restored.kept_files(), empty);
    assert!(
        restored.begin_keep(plan.clone()).is_err(),
        "the exact-plan compatibility seam is guarded too"
    );
    assert!(
        restored
            .begin_keep_variants(vec![plan.clone(), plan.clone()], 0, plan.version)
            .is_err(),
        "multi-candidate admission must reject the missing key before storage validation"
    );
    assert_eq!(restored.kept_files(), empty);
    restored.set_blob_store(Box::new(MemoryBlobStore::new()));
    let mut store = make_store();
    store
        .configure_keep_scope(KeepScope {
            group: restored.group.group_id(),
            device: restored.device.device_id(),
        })
        .unwrap();
    let run = store.begin_keep(plan.clone()).unwrap();
    store.put_keep(run, &ciphertext).unwrap();
    store.pause_keep(run).unwrap();
    let existing = store.kept_files();
    restored.set_blob_store(Box::new(store));
    assert!(
        restored.automatic_keep_candidate(None).is_none(),
        "saved intent does not grant a missing decryption key"
    );
    assert!(!restored.keep_auto_source_available(&plan.cid));
    assert!(
        restored
            .fallback_keep(run, &plan.version, &[plan.version], false)
            .is_err(),
        "fallback cannot mint a run without the native file key"
    );
    assert_eq!(
        restored.kept_files(),
        existing,
        "no debit, blocked verdict or intent change"
    );
    drop(restored);
    drop(dir);
}
