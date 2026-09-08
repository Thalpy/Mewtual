use super::*;
use catcoms_replication::studio::{FlipnoteHeader, StudioExpiry, StudioKind};
use catcoms_replication::{epoch_zero_id, InheritedCheckpoint, SignedOp};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SERVER: u64 = 73;
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(123)
}
fn open(path: &Path) -> ServerStore {
    ServerStore::open(path, b"studio-vault-test", &mut rng()).unwrap()
}
fn target(frame: bool) -> StudioTarget {
    if frame {
        StudioTarget::Flipnote {
            channel: [7; 16],
            object: [9; 16],
        }
    } else {
        StudioTarget::Index { channel: [7; 16] }
    }
}
fn inventory(store: &mut ServerStore) -> EpochStorageInventory {
    let mut scan = store.scan_epoch_storage_with_studio().unwrap();
    while !scan.step().unwrap().complete {}
    scan.finish().unwrap()
}
fn budget(store: &mut ServerStore, f: &Fixture) -> EpochStudioBudget {
    let inv = inventory(store);
    store.studio_storage_budget(SERVER, &f.group, &inv).unwrap()
}
struct Fixture {
    device: MlsDevice,
    group: ServerGroup,
    target: StudioTarget,
    logical: LogicalDocument,
    id: u128,
}
impl Fixture {
    fn new(frame: bool) -> Self {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let target = target(frame);
        let logical = target.document(&group.group_id()).unwrap();
        let id = epoch_zero_id(logical.doc_type, &logical.logical_key);
        Self {
            device,
            group,
            target,
            logical,
            id,
        }
    }
    fn domain(&self, body: Vec<u8>, n: u8) -> DomainOp {
        DomainOp {
            nonce: [n; 16],
            doc_type: self.logical.doc_type,
            logical_key: self.logical.logical_key.clone(),
            body,
        }
    }
    fn insert(&self) -> DomainOp {
        self.domain(
            match self.target {
                StudioTarget::Index { .. } => IndexOp::PutObject {
                    object: [1; 16],
                    kind: StudioKind::Flipnote,
                    title: "private moon".into(),
                    created_by: self.device.device_id(),
                    ts: 100,
                    expiry: StudioExpiry::Never,
                }
                .encode()
                .unwrap(),
                _ => FlipnoteOp::InsertFrame {
                    frame: [1; 16],
                    after: None,
                    cid: [3; 32],
                    bytes: 10,
                }
                .encode()
                .unwrap(),
            },
            1,
        )
    }
    fn title(&self) -> DomainOp {
        self.domain(
            match self.target {
                StudioTarget::Index { .. } => IndexOp::SetTitle {
                    object: [1; 16],
                    title: "private renamed".into(),
                }
                .encode()
                .unwrap(),
                _ => FlipnoteOp::SetHeader(FlipnoteHeader::Title("private renamed".into()))
                    .encode()
                    .unwrap(),
            },
            2,
        )
    }
    fn load(&self, store: &ServerStore) -> Option<EpochStudioState> {
        store
            .load_studio_epoch(SERVER, &self.group, self.target, &self.device)
            .unwrap()
    }
    fn path(&self, store: &ServerStore) -> PathBuf {
        store.studio_epoch_path(&scope_bytes(SERVER, &self.logical).unwrap())
    }
    fn intents(&self, store: &ServerStore) -> usize {
        store
            .load_epoch_intents(SERVER, &self.logical)
            .unwrap()
            .pending()
            .len()
    }
    fn edit(
        &self,
        store: &mut ServerStore,
        budget: &mut EpochStudioBudget,
        op: DomainOp,
    ) -> (SealedOp, EpochStudioState) {
        store
            .edit_studio_epoch(
                SERVER,
                &self.group,
                self.target,
                self.id,
                &self.device,
                op,
                100,
                &mut rng(),
                budget,
            )
            .unwrap()
    }
    fn signed(&self, op: &SealedOp) -> SignedOp {
        op.open(
            &self
                .group
                .channel_secret(&self.device, op.doc_type, op.doc_id)
                .unwrap(),
        )
        .unwrap()
    }
    fn receipt(&self, state: &EpochStudioState, close: u8) -> Receipt {
        Receipt::sign(
            self.logical.clone(),
            state.epoch(),
            [close; 32],
            state
                .projection()
                .unwrap()
                .checkpoint([close; 32])
                .unwrap()
                .change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &self.device,
        )
        .unwrap()
    }
}

#[test]
fn studio_store_save_reopen_and_retry_preserves_exact_signed_work_and_ledger() {
    for frame in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(frame);
        let mut store = open(root.path());
        let mut b = budget(&mut store, &f);
        let (first, _) = f.edit(&mut store, &mut b, f.insert());
        let (_, state) = f.edit(&mut store, &mut b, f.title());
        let projection = state.projection().unwrap();
        let saved = fs::read(f.path(&store)).unwrap();
        assert!(!saved.windows(b"private".len()).any(|s| s == b"private"));
        assert_eq!(f.intents(&store), 2);
        drop(store);
        let mut store = open(root.path());
        assert_eq!(f.load(&store).unwrap().projection().unwrap(), projection);
        assert!(
            store
                .edit_studio_epoch(
                    SERVER,
                    &f.group,
                    f.target,
                    f.id,
                    &f.device,
                    f.insert(),
                    100,
                    &mut rng(),
                    &mut b
                )
                .is_err(),
            "old mount budget"
        );
        let mut b = budget(&mut store, &f);
        let (retry, state) = f.edit(&mut store, &mut b, f.insert());
        assert_eq!(f.signed(&first), f.signed(&retry));
        assert_eq!(state.op_count(), 2);
        assert_eq!(state.projection().unwrap(), projection);
        assert_eq!(fs::read(f.path(&store)).unwrap(), saved);
        assert_eq!(f.intents(&store), 2);
        assert!(!format!("{state:?} {b:?}").contains("private"));
    }
}

#[test]
fn studio_store_reopens_frame_pointing_to_real_promoted_pix_bytes() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    // Exercise the existing vault blob namespace, not an in-memory placeholder. These are
    // 192x144 canonical pixels; production publication/retention coordination is a later layer.
    let mut pix = crate::creative::tests::golden()[..23].to_vec();
    pix[4] = 191;
    pix[5] = 143;
    pix.extend((0..108).flat_map(|_| [255, 0]));
    crate::creative::validate_pix(&pix).unwrap();
    let cid = {
        let mut blobs = store.blob_store("studio-test").unwrap();
        assert!(blobs.is_persistent());
        let cid = blobs.put_staged(&pix).unwrap();
        assert!(blobs.get_bounded(&cid, pix.len()).unwrap().is_none());
        blobs.promote_staged_bounded(&cid, pix.len()).unwrap();
        cid
    };
    let op = f.domain(
        FlipnoteOp::InsertFrame {
            frame: [1; 16],
            after: None,
            cid: *cid.as_bytes(),
            bytes: pix.len() as u64,
        }
        .encode()
        .unwrap(),
        1,
    );
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, op);
    drop(store);
    let store = open(root.path());
    let StudioProjection::Flipnote(p) = f.load(&store).unwrap().projection().unwrap() else {
        panic!("wrong persisted type");
    };
    let frame = &p.frames[&[1; 16]].pixels.selected.value;
    assert_eq!(frame.cid, *cid.as_bytes());
    assert_eq!(frame.bytes, pix.len() as u64);
    let blobs = store.blob_store("studio-test").unwrap();
    assert_eq!(
        blobs
            .get_bounded(&cid, frame.bytes as usize)
            .unwrap()
            .unwrap(),
        pix
    );
    assert!(blobs.get_bounded(&cid, pix.len() - 1).is_err());
}

#[test]
fn studio_store_authenticated_scope_corruption_and_file_bounds_fail_closed() {
    for mode in 0..4 {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(true);
        let mut store = open(root.path());
        let mut b = budget(&mut store, &f);
        f.edit(&mut store, &mut b, f.insert());
        let path = f.path(&store);
        match mode {
            0 => {
                // Wrong vault ciphertext, not an empty/new document.
                let mut bytes = fs::read(&path).unwrap();
                *bytes.last_mut().unwrap() ^= 1;
                fs::write(&path, bytes).unwrap();
            }
            1 => {
                // Authenticated bytes copied to another canonical object path.
                let other = StudioTarget::Flipnote {
                    channel: [7; 16],
                    object: [8; 16],
                }
                .document(&f.group.group_id())
                .unwrap();
                let other_path = store.studio_epoch_path(&scope_bytes(SERVER, &other).unwrap());
                fs::copy(&path, other_path).unwrap();
            }
            2 => {
                // Bound the filesystem length before opening/reading an attacker-sized file.
                fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_len(MAX_SEALED_BYTES as u64 + 1)
                    .unwrap();
            }
            _ => {
                // Unsupported authenticated version is not treated as missing.
                let scope = scope_bytes(SERVER, &f.logical).unwrap();
                let held = store.read_studio_record(&scope).unwrap().unwrap();
                let (_, snapshot) = decode_record(&held.plain, &scope, &f.logical).unwrap();
                let mut snapshot = snapshot.to_vec();
                snapshot[0] = 255;
                let mut e = Encoder::new();
                e.put_bytes(&scope).unwrap();
                e.put_bytes(&f.target.channel()).unwrap();
                e.put_bytes(&snapshot).unwrap();
                let sealed = seal(&store.keys.db_key().unwrap(), &e.finish(), &mut rng()).unwrap();
                fs::write(&path, frame(&sealed)).unwrap();
            }
        }
        let mut scan = store.scan_epoch_storage_with_studio().unwrap();
        assert!((|| {
            while !scan.step()?.complete {}
            Ok::<_, AppError>(())
        })()
        .is_err());
        drop(scan);
        if mode != 1 {
            assert!(store
                .load_studio_epoch(SERVER, &f.group, f.target, &f.device)
                .is_err());
            assert!(store
                .edit_studio_epoch(
                    SERVER,
                    &f.group,
                    f.target,
                    f.id,
                    &f.device,
                    f.title(),
                    100,
                    &mut rng(),
                    &mut b
                )
                .is_err());
            assert!(b.requires_reconciliation());
        }
        assert_eq!(f.intents(&store), 1);
    }
}

#[test]
fn studio_store_temporary_copies_are_charged_and_only_explicit_cleanup_removes_them() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    let held = fs::read(f.path(&store)).unwrap();
    let temp = staging_candidate(&f.path(&store), 100);
    fs::write(&temp, b"partial copy").unwrap();
    let inv = inventory(&mut store);
    assert_eq!(inv.orphans().len(), 1);
    let current = store.studio_storage_budget(SERVER, &f.group, &inv).unwrap();
    assert_eq!(current.usage().content, b.usage().content + 12);
    assert_eq!(current.usage().settlement, 0);
    let mut narrow = store.cleanup_epoch_storage_staging_with_registry().unwrap();
    while !narrow.step().unwrap().complete {}
    drop(narrow);
    assert!(
        temp.exists(),
        "legacy four-family cleanup must not silently expand"
    );
    let mut cleanup = store.cleanup_epoch_storage_staging_with_studio().unwrap();
    while !cleanup.step().unwrap().complete {}
    let mut scan = cleanup.into_inventory().unwrap();
    while !scan.step().unwrap().complete {}
    let inv = scan.finish().unwrap();
    assert_eq!(inv.orphans().len(), 0);
    assert_eq!(fs::read(f.path(&store)).unwrap(), held);
    assert!(!temp.exists());
    let current = store.studio_storage_budget(SERVER, &f.group, &inv).unwrap();
    assert_eq!(current.usage(), b.usage());
    // An unpublished first write has no authenticated scope from which to charge its bytes.
    let unowned = store
        .dir
        .join("servers")
        .join(format!("{}.studio-epoch", "ab".repeat(32)));
    fs::write(staging_candidate(&unowned, 101), []).unwrap();
    let inv = inventory(&mut store);
    assert_eq!(inv.unresolved_orphans(), 1);
    assert!(store.studio_storage_budget(SERVER, &f.group, &inv).is_err());
}

#[test]
fn studio_store_invalid_receipt_is_rejected_before_disk_or_inventory_mutation() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    let (_, state) = f.edit(&mut store, &mut b, f.insert());
    let mut receipt = f.receipt(&state, 7);
    receipt.signature[0] ^= 1;
    let generation = store.studio_generation.clone();
    // If receipt authentication is delayed until core sealing, this lost source will poison
    // the budget first. A forged receipt must never reach that expensive I/O path.
    fs::remove_file(f.path(&store)).unwrap();
    let err = store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            receipt,
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap_err();
    assert!(!err.to_string().contains("source missing"));
    assert!(!b.requires_reconciliation());
    assert!(Arc::ptr_eq(&generation, &store.studio_generation));
    assert_eq!(f.intents(&store), 1);
}

#[test]
fn reference_scan_keeps_an_overwritten_checkpoint_register_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let mut blobs = store.blob_store(&hex::encode(f.group.group_id())).unwrap();
    let a = blobs.put(b"birth pixels").unwrap();
    let b = blobs.put(b"seed replacement").unwrap();
    let c = blobs.put(b"successor replacement").unwrap();
    let mut unit = StudioEpoch::new(&f.group, f.target, f.device.device_id()).unwrap();
    for (n, body) in [
        FlipnoteOp::InsertFrame {
            frame: [1; 16],
            after: None,
            cid: *a.as_bytes(),
            bytes: 10,
        },
        FlipnoteOp::ReplaceFrame {
            frame: [1; 16],
            cid: *b.as_bytes(),
            bytes: 10,
        },
    ]
    .into_iter()
    .enumerate()
    {
        unit.edit_or_reseal(
            &f.device,
            &f.group,
            &mut rng(),
            &f.domain(body.encode().unwrap(), n as u8),
            100,
        )
        .unwrap();
    }
    let source = EpochStudioState { unit };
    let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
    let mut next = StudioEpoch::from_checkpoint(
        &f.group,
        f.target,
        f.device.device_id(),
        f.receipt(&source, 7),
        0,
        seed.bytes(),
    )
    .unwrap();
    next.edit_or_reseal(
        &f.device,
        &f.group,
        &mut rng(),
        &f.domain(
            FlipnoteOp::ReplaceFrame {
                frame: [1; 16],
                cid: *c.as_bytes(),
                bytes: 10,
            }
            .encode()
            .unwrap(),
            3,
        ),
        101,
    )
    .unwrap();
    // Test-only installed source: runtime checkpoint installation remains gate 4 work.
    let mut budget = budget(&mut store, &f);
    store
        .save_studio_source(
            SERVER,
            next,
            None,
            &[],
            WritePurpose::Ordinary,
            &mut rng(),
            &mut budget.storage,
            atomic_write,
            sync_studio,
        )
        .unwrap();
    store.creative_pinned_cids().unwrap();
    assert!(!blobs.delete(&b).unwrap());
    drop(store);
    let mut store = open(root.path());
    let mut blobs = store.blob_store(&hex::encode(f.group.group_id())).unwrap();
    assert_eq!(
        store
            .creative_pinned_cids()
            .unwrap()
            .for_group(&f.group.group_id())
            .count(),
        3
    );
    for cid in [a, b, c] {
        assert!(!blobs.delete(&cid).unwrap());
    }
}

#[test]
fn studio_store_full_content_allows_exact_retry_and_receipt_but_not_new_intent() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    let (first, state) = f.edit(&mut store, &mut b, f.insert());
    let original = fs::read(f.path(&store)).unwrap();
    let inv = inventory(&mut store);
    let mut records = inv.records_for_server(SERVER, &f.group.group_id()).unwrap();
    // Private test-only ballast avoids allocating 2 GiB. The production wrapper can only be
    // minted from the actual completed scan; the generic budget algorithm is unchanged.
    records.push(StorageRecord {
        id: [255; 32],
        document: [255; 32],
        footprint: Footprint {
            content: super::super::epoch_budget::CONTENT_ALLOWANCE_BYTES - b.usage().content,
            ..Footprint::default()
        },
    });
    b.storage = EpochStorageBudget::from_inventory(b.scope.clone(), records).unwrap();
    let (retry, _) = f.edit(&mut store, &mut b, f.insert());
    assert_eq!(f.signed(&retry), f.signed(&first));
    assert_eq!(fs::read(f.path(&store)).unwrap(), original);
    assert!(store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            f.id,
            &f.device,
            f.title(),
            100,
            &mut rng(),
            &mut b
        )
        .is_err());
    assert!(
        !b.requires_reconciliation(),
        "known preflight refusal is not uncertain I/O"
    );
    assert_eq!(f.intents(&store), 1);
    let (_, closing) = store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            f.receipt(&state, 7),
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(closing.phase(), EpochPhase::Closing);
    assert_eq!(
        b.usage().content,
        super::super::epoch_budget::CONTENT_ALLOWANCE_BYTES
    );
    assert!(b.usage().protocol > 0);
    assert_eq!(
        b.usage().settlement,
        0,
        "replacement temporary is retired after durability"
    );
    assert_eq!(
        f.load(&store).unwrap().projection().unwrap(),
        state.projection().unwrap()
    );
}

#[test]
fn studio_store_crash_matrix_has_intent_first_and_no_ciphertext_before_both_barriers() {
    for stage in 0..2 {
        for mode in 0..3 {
            let root = tempfile::tempdir().unwrap();
            let f = Fixture::new(true);
            let mut store = open(root.path());
            let mut b = budget(&mut store, &f);
            let fail = |path: &Path, bytes: &[u8]| {
                if mode == 1 {
                    atomic_write(path, bytes)?;
                }
                if mode == 2 {
                    panic!("injected Studio writer panic");
                }
                Err(AppError::Io("injected persistence failure".into()))
            };
            let intent_scope =
                super::super::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
            let intent_path = store
                .dir
                .join("servers")
                .join(format!("{}.intents", blake3::hash(&intent_scope).to_hex()));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                store.edit_studio_with_io(
                    SERVER,
                    &f.group,
                    f.target,
                    f.id,
                    &f.device,
                    f.insert(),
                    100,
                    &mut rng(),
                    &mut b,
                    |p, bytes| {
                        if stage == 0 {
                            fail(p, bytes)
                        } else {
                            atomic_write(p, bytes)
                        }
                    },
                    super::super::epoch_intents::sync_intent,
                    |p, bytes| {
                        assert!(intent_path.exists());
                        if stage == 1 {
                            fail(p, bytes)
                        } else {
                            atomic_write(p, bytes)
                        }
                    },
                    sync_studio,
                )
            }));
            assert!(result.is_err() || result.unwrap().is_err());
            assert!(b.requires_reconciliation());
            assert_eq!(f.intents(&store), usize::from(stage == 1 || mode == 1));
            assert_eq!(
                f.load(&store).map_or(0, |s| s.op_count()),
                usize::from(stage == 1 && mode == 1)
            );
            let pins = store.creative_pinned_cids().unwrap();
            assert_eq!(
                pins.for_group(&f.group.group_id())
                    .any(|cid| cid.as_bytes() == &[3; 32]),
                stage == 1 || mode == 1,
                "a durable intent pins even if its epoch write failed"
            );
            drop(store);
            let mut store = open(root.path());
            assert_eq!(
                store
                    .creative_pinned_cids()
                    .unwrap()
                    .for_group(&f.group.group_id())
                    .count(),
                usize::from(stage == 1 || mode == 1),
                "same intent/source holds after restart"
            );
            let mut b = budget(&mut store, &f);
            let (_, state) = f.edit(&mut store, &mut b, f.insert());
            assert_eq!(state.op_count(), 1);
            assert_eq!(f.intents(&store), 1);
        }
    }
}

#[test]
fn studio_store_duplicate_flush_failure_preserves_bytes_and_requires_rescan() {
    for stage in 0..2 {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(true);
        let mut store = open(root.path());
        let mut b = budget(&mut store, &f);
        let (first, _) = f.edit(&mut store, &mut b, f.insert());
        let saved = fs::read(f.path(&store)).unwrap();
        let result = store.edit_studio_with_io(
            SERVER,
            &f.group,
            f.target,
            f.id,
            &f.device,
            f.insert(),
            100,
            &mut rng(),
            &mut b,
            |_, _| panic!("duplicate intent rewrote"),
            |p, n| {
                if stage == 0 {
                    Err(AppError::Io("flush failed".into()))
                } else {
                    super::super::epoch_intents::sync_intent(p, n)
                }
            },
            |_, _| panic!("duplicate epoch rewrote"),
            |p, n| {
                if stage == 1 {
                    Err(AppError::Io("flush failed".into()))
                } else {
                    sync_studio(p, n)
                }
            },
        );
        assert!(result.is_err());
        assert!(b.requires_reconciliation());
        assert_eq!(fs::read(f.path(&store)).unwrap(), saved);
        let mut b = budget(&mut store, &f);
        let (retry, _) = f.edit(&mut store, &mut b, f.insert());
        assert_eq!(f.signed(&first), f.signed(&retry));
    }
}

#[test]
fn studio_store_refuses_bad_targets_scope_and_lost_indexed_source_before_intents() {
    for frame in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(frame);
        let mut store = open(root.path());
        let mut b = budget(&mut store, &f);
        let bad = f.domain(
            if frame {
                FlipnoteOp::InsertFrame {
                    frame: [1; 16],
                    after: Some([2; 16]),
                    cid: [3; 32],
                    bytes: 10,
                }
                .encode()
                .unwrap()
            } else {
                IndexOp::SetTitle {
                    object: [1; 16],
                    title: "unknown".into(),
                }
                .encode()
                .unwrap()
            },
            1,
        );
        assert!(store
            .edit_studio_epoch(
                SERVER,
                &f.group,
                f.target,
                f.id,
                &f.device,
                bad,
                100,
                &mut rng(),
                &mut b
            )
            .is_err());
        assert_eq!(f.intents(&store), 0);
        assert!(f.load(&store).is_none());
        f.edit(&mut store, &mut b, f.insert());
        let mut conflict = f.title();
        conflict.nonce = [1; 16];
        assert!(store
            .edit_studio_epoch(
                SERVER,
                &f.group,
                f.target,
                f.id,
                &f.device,
                conflict,
                100,
                &mut rng(),
                &mut b
            )
            .is_err());
        assert_eq!(f.intents(&store), 1);
        assert!(store
            .edit_studio_epoch(
                SERVER,
                &f.group,
                f.target,
                f.id + 1,
                &f.device,
                f.title(),
                100,
                &mut rng(),
                &mut b
            )
            .is_err());
        if frame {
            assert!(store
                .load_studio_epoch(
                    SERVER,
                    &f.group,
                    StudioTarget::Flipnote {
                        channel: [8; 16],
                        object: [9; 16]
                    },
                    &f.device
                )
                .is_err());
        }
        // Simulated external disk loss is confined to this test's temporary vault.
        fs::remove_file(f.path(&store)).unwrap();
        assert!(store
            .edit_studio_epoch(
                SERVER,
                &f.group,
                f.target,
                f.id,
                &f.device,
                f.title(),
                100,
                &mut rng(),
                &mut b
            )
            .is_err());
        assert_eq!(f.intents(&store), 1);
        assert!(b.requires_reconciliation());
    }
}

#[test]
fn studio_store_coverage_generation_ingress_and_cleanup_bound_accounting() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let mut scan = store.scan_epoch_storage_with_registry().unwrap();
    while !scan.step().unwrap().complete {}
    let narrow = scan.finish().unwrap();
    assert!(store
        .studio_storage_budget(SERVER, &f.group, &narrow)
        .is_err());
    let inv = inventory(&mut store);
    let mut first = store.studio_storage_budget(SERVER, &f.group, &inv).unwrap();
    assert!(
        store.studio_storage_budget(SERVER, &f.group, &inv).is_err(),
        "one mint per captured inventory"
    );
    let stale = inventory(&mut store);
    let mut source = StudioEpoch::new(&f.group, f.target, f.device.device_id()).unwrap();
    let op = source
        .edit_or_reseal(&f.device, &f.group, &mut rng(), &f.insert(), 100)
        .unwrap();
    store
        .ingest_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &op,
            &mut rng(),
            &mut first,
        )
        .unwrap();
    assert!(
        store
            .studio_storage_budget(SERVER, &f.group, &stale)
            .is_err(),
        "source-only ingress invalidates old scans"
    );
    assert_eq!(f.intents(&store), 0);
    let inv = inventory(&mut store);
    assert_eq!(
        inv.records()
            .filter(|e| e.kind == EpochRecordKind::Studio)
            .count(),
        1
    );
    let mut next = store.studio_storage_budget(SERVER, &f.group, &inv).unwrap();
    assert!(store
        .ingest_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &op,
            &mut rng(),
            &mut first
        )
        .is_err());
    assert_eq!(
        store
            .ingest_studio_epoch(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &op,
                &mut rng(),
                &mut next
            )
            .unwrap()
            .0,
        Admission::Duplicate
    );
    let captured = inventory(&mut store);
    let mut cleanup = store.cleanup_epoch_storage_staging_with_studio().unwrap();
    while !cleanup.step().unwrap().complete {}
    drop(cleanup);
    assert!(store
        .studio_storage_budget(SERVER, &f.group, &captured)
        .is_err());
    assert!(store
        .ingest_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &op,
            &mut rng(),
            &mut next
        )
        .is_err());
}

#[test]
fn studio_store_fault_persists_before_reporting_and_failed_fault_save_can_retry() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    let (_, state) = f.edit(&mut store, &mut b, f.insert());
    let first = f.receipt(&state, 7);
    let conflict = f.receipt(&state, 8);
    let (_, state) = store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            first,
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(state.phase(), EpochPhase::Closing);
    assert_eq!(state.op_count(), 1);
    assert!(store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            f.id,
            &f.device,
            f.title(),
            100,
            &mut rng(),
            &mut b
        )
        .is_err());
    assert_eq!(f.intents(&store), 1);
    assert!(store
        .seal_studio_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            conflict.clone(),
            0,
            &mut rng(),
            &mut b,
            |_, _| Err(AppError::Io("fault write failed".into()))
        )
        .is_err());
    assert!(b.requires_reconciliation());
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Closing);
    let mut b = budget(&mut store, &f);
    let (_, state) = store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            conflict,
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(state.phase(), EpochPhase::Fault);
    let projection = state.projection().unwrap();
    drop(store);
    let store = open(root.path());
    let restored = f.load(&store).unwrap();
    assert_eq!(restored.phase(), EpochPhase::Fault);
    assert_eq!(restored.projection().unwrap(), projection);
    assert_eq!(f.intents(&store), 1);
}
