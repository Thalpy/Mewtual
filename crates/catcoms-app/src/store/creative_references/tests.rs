use super::*;
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::{
    epoch_zero_id,
    studio::{FlipnoteOp, StudioEpoch, StudioRecovery, StudioTarget},
    RecoveryReason,
};
use catcoms_rt::ManualClock;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(818)
}
fn open(path: &Path) -> ServerStore {
    ServerStore::open(path, b"references", &mut rng()).unwrap()
}
fn inventory(store: &mut ServerStore) -> EpochStorageInventory {
    let mut scan = store.scan_epoch_storage_with_studio().unwrap();
    while !scan.step().unwrap().complete {}
    scan.finish().unwrap()
}
fn op(target: StudioTarget, group: &ServerGroup, cid: Cid, n: u8) -> DomainOp {
    let logical = target.document(&group.group_id()).unwrap();
    DomainOp {
        nonce: [n; 16],
        doc_type: logical.doc_type,
        logical_key: logical.logical_key,
        body: FlipnoteOp::InsertFrame {
            frame: [n; 16],
            after: None,
            cid: *cid.as_bytes(),
            bytes: 10,
        }
        .encode()
        .unwrap(),
    }
}
fn save(
    store: &mut ServerStore,
    server: u64,
    group: &ServerGroup,
    device: &MlsDevice,
    target: StudioTarget,
    cid: Cid,
    n: u8,
) {
    let inv = inventory(store);
    let mut budget = store.studio_storage_budget(server, group, &inv).unwrap();
    let logical = target.document(&group.group_id()).unwrap();
    store
        .edit_studio_epoch(
            server,
            group,
            target,
            epoch_zero_id(logical.doc_type, &logical.logical_key),
            device,
            op(target, group, cid, n),
            100,
            &mut rng(),
            &mut budget,
        )
        .unwrap();
}

#[test]
fn reference_sets_are_bounded_and_union_full_group_namespaces() {
    let mut refs = CreativeReferences::default();
    refs.add(b"one", [[7; 32], [7; 32]]).unwrap();
    refs.add(b"two", [[7; 32]]).unwrap();
    assert_eq!(refs.len(), 2);
    assert_eq!(refs.for_group(b"one").count(), 1);
    assert_eq!(refs.for_group(b"other").count(), 0);
    refs.count = MAX_CREATIVE_REFERENCES;
    assert!(refs.add(b"one", [[8; 32]]).is_err());
    assert!(!format!("{refs:?}").contains("one"));
}

#[test]
fn deletion_keeps_the_shared_fence_locked_inside_the_actual_store_call() {
    // Inspect synchronously from inside unlink's test substitute. This catches unlock-before-
    // delete without scheduler timing, sleeps, or a thread that could be stranded on failure.
    struct Probe {
        inner: catcoms_storage::MemoryBlobStore,
        protection: SharedProtection,
    }
    impl BlobStore for Probe {
        fn delete(&mut self, c: &Cid) -> Result<bool, StorageError> {
            assert!(
                self.protection.try_lock().is_err(),
                "writer must not add a reference between the guard check and unlink"
            );
            self.inner.delete(c)
        }
        fn put(&mut self, b: &[u8]) -> Result<Cid, StorageError> {
            self.inner.put(b)
        }
        fn get(&self, c: &Cid) -> Result<Option<Vec<u8>>, StorageError> {
            self.inner.get(c)
        }
        fn get_bounded(&self, c: &Cid, n: usize) -> Result<Option<Vec<u8>>, StorageError> {
            self.inner.get_bounded(c, n)
        }
        fn has(&self, c: &Cid) -> bool {
            self.inner.has(c)
        }
        fn cids(&self) -> Vec<Cid> {
            self.inner.cids()
        }
        fn put_staged(&mut self, b: &[u8]) -> Result<Cid, StorageError> {
            self.inner.put_staged(b)
        }
        fn promote_staged(&mut self, c: &Cid) -> Result<bool, StorageError> {
            self.inner.promote_staged(c)
        }
        fn promote_staged_bounded(&mut self, c: &Cid, n: usize) -> Result<bool, StorageError> {
            self.inner.promote_staged_bounded(c, n)
        }
        fn drop_staged(&mut self, c: &Cid) -> Result<bool, StorageError> {
            self.inner.drop_staged(c)
        }
        fn clear_staging(&mut self) -> Result<usize, StorageError> {
            self.inner.clear_staging()
        }
    }
    let root = tempfile::tempdir().unwrap();
    let store = open(root.path());
    let protection = store.creative_protection.clone();
    let inner = Probe {
        inner: Default::default(),
        protection: protection.clone(),
    };
    let mut blobs = ProtectedBlobs {
        inner: Box::new(inner),
        group: b"group".to_vec(),
        protection,
    };
    let cid = blobs.put(b"unreferenced").unwrap();
    assert!(blobs.delete(&cid).unwrap());
    store.hold_creative(b"group", Ok(BTreeSet::from([*cid.as_bytes()])));
    assert!(blobs.get_bounded(&cid, 100).unwrap().is_none());
}

#[test]
fn source_holds_cover_two_handles_aliases_restart_and_fail_closed_scans() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let key = hex::encode(group.group_id());
    let mut first = store.blob_store(&key).unwrap();
    let mut second = store.blob_store(&key).unwrap();
    let a = first.put(b"frame A").unwrap();
    let b = first.put(b"frame B").unwrap();
    let orphan = first.put(b"orphan").unwrap();
    let target = StudioTarget::Flipnote {
        channel: [7; 16],
        object: [9; 16],
    };
    save(&mut store, 1, &group, &device, target, a, 1);
    save(&mut store, 2, &group, &device, target, b, 2); // same group, distinct native server alias
    assert!(!first.delete(&a).unwrap());
    assert!(!second.delete(&b).unwrap());
    assert!(second.delete(&orphan).unwrap());
    assert_eq!(
        store
            .creative_pinned_cids()
            .unwrap()
            .for_group(&group.group_id())
            .count(),
        2
    );
    assert!(!first.delete(&a).unwrap());
    assert!(!second.delete(&b).unwrap());
    let mut other = store.blob_store("other-group").unwrap();
    other.put(b"frame A").unwrap();
    assert!(other.delete(&a).unwrap(), "full groups do not share holds");
    assert!(store.blob_store(&format!("../{key}")).is_err());
    drop(store);
    assert!(first.delete(&a).is_err(), "old mount handle is revoked");
    let mut store = open(root.path());
    let mut current = store.blob_store(&key).unwrap();
    assert!(
        current.delete(&a).is_err(),
        "restored references initially Unknown"
    );
    store.creative_pinned_cids().unwrap();
    assert!(!current.delete(&a).unwrap());
    assert!(
        first.delete(&a).is_err(),
        "new scan cannot revive old mount"
    );
    // A malformed canonical metadata record never grants an empty completed scan.
    fs::write(
        root.path()
            .join("servers")
            .join(format!("{}.studio-epoch", "ab".repeat(32))),
        b"broken",
    )
    .unwrap();
    assert!(store.creative_pinned_cids().is_err());
    assert!(current.delete(&a).is_err());
    assert!(current.get_bounded(&a, 100).unwrap().is_some());
}

#[test]
fn transient_preholds_survive_budget_scans_but_only_complete_reference_scans_can_unpin() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let group = b"group";
    let mut blobs = store.blob_store(&hex::encode(group)).unwrap();
    let cid = blobs.put(b"published before intent").unwrap();
    store.hold_creative(group, Ok(BTreeSet::from([*cid.as_bytes()])));
    inventory(&mut store); // the scan between PIX promotion and the intent barrier
    assert!(!blobs.delete(&cid).unwrap());
    let protection = store.creative_protection.clone();
    let mut scan = store.scan_epoch_storage_with_studio().unwrap();
    scan.collect_creative_references().unwrap();
    while !scan.step().unwrap().complete {}
    protection.lock().unwrap().unknown(); // generation changed before installation
    assert!(scan.finish_creative_references().is_err());
    assert!(blobs.delete(&cid).is_err());
    store.creative_pinned_cids().unwrap(); // no saved record ever claimed this orphan
    assert!(blobs.delete(&cid).unwrap());
    // The opposite ordering: deletion before hold leaves missing bytes, never invented success.
    let gone = blobs.put(b"deleted first").unwrap();
    assert!(blobs.delete(&gone).unwrap());
    store.hold_creative(group, Ok(BTreeSet::from([*gone.as_bytes()])));
    assert!(blobs.get_bounded(&gone, 100).unwrap().is_none());
    store
        .creative_protection
        .lock()
        .unwrap()
        .pins
        .as_mut()
        .unwrap()
        .count = MAX_CREATIVE_REFERENCES;
    store.hold_creative(group, Ok(BTreeSet::from([[42; 32]])));
    assert!(
        blobs.delete(&gone).is_err(),
        "overflow is Unknown, never truncated permission"
    );
}

#[test]
fn retained_and_staged_recovery_hold_pixels_and_expiry_releases_only_evicted_versions() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let target = StudioTarget::Flipnote {
        channel: [7; 16],
        object: [9; 16],
    };
    let logical = target.document(&group.group_id()).unwrap();
    let mut blobs = store.blob_store(&hex::encode(group.group_id())).unwrap();
    let clock = ManualClock::new(100);
    let mut cids = Vec::new();
    for n in 1..=3 {
        let cid = blobs.put(&[n; 10]).unwrap();
        cids.push(cid);
        let mut unit = StudioEpoch::new(&group, target, device.device_id()).unwrap();
        unit.edit_or_reseal(
            &device,
            &group,
            &mut rng(),
            &op(target, &group, cid, n),
            100,
        )
        .unwrap();
        let snapshot = StudioRecovery::snapshot(
            &unit.projection().unwrap(),
            None,
            RecoveryReason::Rewound,
            [0; 32],
            &BTreeMap::new(),
        )
        .unwrap();
        store
            .update_epoch_recovery(
                1,
                &logical,
                EpochRecoveryAction::Stage(snapshot),
                &clock,
                &mut rng(),
            )
            .unwrap();
        assert!(
            !blobs.delete(&cid).unwrap(),
            "hold installed before returning from writer"
        );
    }
    let state = store.load_epoch_recovery(1, &logical).unwrap();
    assert_eq!(state.retained().len(), 2);
    assert!(state.staged().is_some());
    assert_eq!(
        store
            .creative_pinned_cids()
            .unwrap()
            .for_group(&group.group_id())
            .count(),
        3
    );
    drop(store);
    let mut store = open(root.path());
    let mut blobs = store.blob_store(&hex::encode(group.group_id())).unwrap();
    store.creative_pinned_cids().unwrap();
    for cid in &cids {
        assert!(!blobs.delete(cid).unwrap());
    }
    clock.advance_ms(8 * 24 * 60 * 60 * 1000);
    store
        .update_epoch_recovery(
            1,
            &logical,
            EpochRecoveryAction::AdvanceTime,
            &clock,
            &mut rng(),
        )
        .unwrap();
    assert!(
        !blobs.delete(&cids[0]).unwrap(),
        "write only adds, never drops old holds"
    );
    assert_eq!(
        store
            .creative_pinned_cids()
            .unwrap()
            .for_group(&group.group_id())
            .count(),
        2
    );
    assert!(blobs.delete(&cids[0]).unwrap());
    assert!(!blobs.delete(&cids[1]).unwrap());
    assert!(!blobs.delete(&cids[2]).unwrap());
}

#[test]
fn unknown_recovery_and_partial_temporary_never_enable_reclamation() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let mut blobs = store.blob_store("group").unwrap();
    let cid = blobs.put(b"held").unwrap();
    let document =
        LogicalDocument::new(b"group".to_vec(), DocType::StudioObject, vec![9; 16]).unwrap();
    let snapshot = catcoms_replication::RecoverySnapshot {
        doc_type: document.doc_type,
        logical_key: document.logical_key.clone(),
        epoch: 0,
        base_close_record_hash: None,
        reason: RecoveryReason::Rewound,
        projection: vec![255],
        tombstones: vec![],
        elements: vec![],
        conflicts: vec![],
        applied_ops: vec![],
    };
    store
        .update_epoch_recovery(
            1,
            &document,
            EpochRecoveryAction::Stage(snapshot),
            &ManualClock::new(0),
            &mut rng(),
        )
        .unwrap();
    assert!(
        blobs.delete(&cid).is_err(),
        "generic writer did not bless unsupported typed data"
    );
    assert!(store.creative_pinned_cids().is_err());
    let empty = tempfile::tempdir().unwrap();
    let mut clean = open(empty.path());
    let final_path = empty
        .path()
        .join("servers")
        .join(format!("{}.studio-epoch", "aa".repeat(32)));
    fs::write(super::super::staging_candidate(&final_path, 1), b"partial").unwrap();
    assert!(clean.creative_pinned_cids().is_err());
}

#[tokio::test]
async fn fileshare_unlisting_and_upload_cleanup_consult_actual_saved_studio_references() {
    use catcoms_rt::{Hub, PeerId};
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let hub = Hub::new();
    let mut server = crate::Server::found(
        hub.join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(100)),
        "alice",
    )
    .unwrap();
    server.set_blob_store(store.blob_store(&hex::encode(server.group_id())).unwrap());
    server.open_files().await.unwrap();
    let file = server
        .add_file(
            "shared.bin",
            "application/octet-stream",
            "test",
            b"shared bytes",
        )
        .await
        .unwrap();
    let entry = server
        .files()
        .into_iter()
        .find(|entry| entry.cid == file.as_bytes())
        .unwrap();
    let manifest = catcoms_storage::FileManifest::decode_or_legacy(&entry.file_ref).unwrap();
    let chunk = manifest.chunks[0].clone();
    // A fileshare's untrusted manifest may alias another consumer's held CID. The guard's
    // decision is by actual address; it cannot rely on assuming file/PIX bytes never coincide.
    let target = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [9; 16],
    };
    server.sync.with_registry_context(|group, device, _, _| {
        save(
            &mut store,
            1,
            group,
            device,
            target,
            chunk.ciphertext_cid,
            1,
        )
    });
    server.discard_upload_chunks(std::slice::from_ref(&chunk));
    assert!(server.sync.has_blob(&chunk.ciphertext_cid));
    server.delete_file_at(&file, "test").await.unwrap();
    assert!(server.files().is_empty(), "unlisting still succeeds");
    assert!(
        server.sync.has_blob(&chunk.ciphertext_cid),
        "byte cleanup does not destroy Studio data"
    );
    // Unrelated cache data still reclaims normally in a known mount.
    let other = server
        .add_file("other.bin", "application/octet-stream", "", b"other bytes")
        .await
        .unwrap();
    let entry = server
        .files()
        .into_iter()
        .find(|entry| entry.cid == other.as_bytes())
        .unwrap();
    let other_chunks = catcoms_storage::FileManifest::decode_or_legacy(&entry.file_ref)
        .unwrap()
        .chunks;
    server.delete_file(&other).await.unwrap();
    for chunk in other_chunks {
        assert!(!server.sync.has_blob(&chunk.ciphertext_cid));
    }
    let staged = server
        .seal_upload_chunk(b"unpublished", "application/octet-stream")
        .unwrap();
    server.discard_upload_chunks(&[staged]); // staging cleanup remains independent of holds
}
