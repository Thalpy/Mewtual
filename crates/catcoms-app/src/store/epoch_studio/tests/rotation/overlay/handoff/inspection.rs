use super::*;
use crate as app;
use crate::store::{StudioInspectedDraft, StudioInspectionStamp};
#[path = "../../../../../../../tests/support/studio_inspection_shapes.rs"]
mod shapes;

fn inspect(f: &Fixture, store: &ServerStore) -> (StudioInspectionStamp, StudioInspectedDraft) {
    let capture = store
        .capture_studio_inspection(SERVER, &f.group.group_id(), f.target, f.device.device_id())
        .unwrap();
    std::thread::spawn(move || capture.rebuild())
        .join()
        .unwrap()
        .unwrap()
}
fn current(f: &Fixture, store: &ServerStore, stamp: &StudioInspectionStamp) -> bool {
    store
        .studio_inspection_is_current(
            SERVER,
            &f.group.group_id(),
            f.target,
            f.device.device_id(),
            stamp,
        )
        .unwrap()
}

#[test]
fn studio_inspection_full_wrapper_change_with_unchanged_draft_is_stale() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, expected) = prepare(&f, &mut store);
        let (stamp, value) = inspect(&f, &store);
        assert!(current(&f, &store, &stamp));
        assert_eq!(value.draft.unwrap().projection(), &expected);
        let mut operation = f.title();
        operation.nonce = [99; 16];
        let mut b = budget(&mut store, &f);
        store
            .prepare_epoch_intent(
                SERVER,
                &f.logical,
                operation,
                &f.device,
                &f.group,
                &mut rng(),
                &mut b.storage,
                &mut b.intents,
            )
            .unwrap();
        let (_, unchanged) = inspect(&f, &store);
        let draft = unchanged.draft.unwrap();
        assert_eq!((draft.basis(), draft.accepted()), (basis, 1));
        assert_eq!(draft.projection(), &expected);
        assert!(
            !current(&f, &store, &stamp),
            "changed complete wrapper escaped inspection currency fence"
        );
    }
}

#[test]
fn studio_inspection_same_size_authenticated_replacement_is_stale() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, expected) = prepare(&f, &mut store);
        let author = f.device.device_id();
        let mut ordinary = f.title();
        ordinary.nonce = [99; 16];
        let ordinary_id = ordinary.id(&author);
        let mut b = budget(&mut store, &f);
        store
            .prepare_epoch_intent(
                SERVER,
                &f.logical,
                ordinary.clone(),
                &f.device,
                &f.group,
                &mut rng(),
                &mut b.storage,
                &mut b.intents,
            )
            .unwrap();
        let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
        let path = store.epoch_intent_path(&scope);
        let before_bytes = fs::read(&path).unwrap();
        let before_raw = store.read_epoch_intent_plain(&path).unwrap().unwrap();
        let before = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert_eq!(before.encode(&scope).unwrap(), before_raw.plain);
        assert!(!before.is_overlay(&ordinary_id));
        let metadata = before
            .handoff_metadata()
            .unwrap()
            .encode_vault(&before.ledger)
            .unwrap();
        let sources = canonical(&store);
        let (stamp, original) = inspect(&f, &store);
        assert!(current(&f, &store, &stamp));
        assert_eq!(fs::read(&path).unwrap(), before_bytes);

        // Construct a valid fixture replacement, not an authorized production retirement:
        // copy every envelope and change only the unannotated ordinary envelope's nonce.
        let mut replacement = before.clone();
        replacement.ledger = catcoms_replication::IntentLedger::new(f.logical.clone());
        let mut replaced = 0;
        for (id, intent) in before.pending() {
            let mut operation = intent.operation.clone();
            if *id == ordinary_id {
                assert_eq!(intent.author, author);
                assert_eq!(operation, ordinary);
                operation.nonce = [100; 16];
                replaced += 1;
            }
            replacement
                .ledger
                .prepare(intent.author, operation)
                .unwrap();
        }
        assert_eq!(replaced, 1);
        assert_eq!(replacement.pending().len(), before.pending().len());
        let mut b = budget(&mut store, &f);
        store
            .write_prepared_intents(
                SERVER,
                &f.logical,
                replacement,
                Some(before_raw.physical_bytes),
                false,
                &mut rng(),
                &mut b.storage,
                &mut b.intents,
                atomic_write,
                sync_intent,
            )
            .unwrap();
        let after_bytes = fs::read(&path).unwrap();
        let after_raw = store.read_epoch_intent_plain(&path).unwrap().unwrap();
        let after = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert_eq!(after.encode(&scope).unwrap(), after_raw.plain);
        assert_eq!(before_raw.physical_bytes, before_bytes.len() as u64);
        assert_eq!(after_raw.physical_bytes, fs::metadata(&path).unwrap().len());
        assert_eq!(after_raw.physical_bytes, after_bytes.len() as u64);
        assert_eq!(before_raw.physical_bytes, after_raw.physical_bytes);
        assert_ne!(
            blake3::hash(&before_raw.plain),
            blake3::hash(&after_raw.plain)
        );
        assert_eq!(
            after
                .handoff_metadata()
                .unwrap()
                .encode_vault(&after.ledger)
                .unwrap(),
            metadata,
            "ordinary replacement changed accepted branch metadata"
        );
        for state in [&before, &after] {
            let overlay = state.overlay().unwrap();
            assert_eq!((overlay.target(), overlay.author()), (f.target, author));
            let draft = state.local_draft().unwrap().unwrap();
            assert_eq!((draft.basis(), draft.accepted()), (basis, 1));
            assert_eq!(draft.projection(), &expected);
        }
        let (fresh, updated) = inspect(&f, &store);
        for value in [original, updated] {
            assert_eq!(value.target, f.target);
            assert!(!value.prepared);
            let draft = value.draft.unwrap();
            assert_eq!((draft.basis(), draft.accepted()), (basis, 1));
            assert_eq!(draft.projection(), &expected);
        }
        assert_eq!(fs::read(&path).unwrap(), after_bytes);
        assert!(current(&f, &store, &fresh));
        assert_eq!(fs::read(&path).unwrap(), after_bytes);
        let obsolete_is_current = current(&f, &store, &stamp);
        assert_eq!(fs::read(&path).unwrap(), after_bytes);
        assert_eq!(canonical(&store), sources);
        assert!(
            !obsolete_is_current,
            "same-sized authenticated replacement escaped inspection digest fence"
        );
    }
}

#[test]
fn studio_inspection_absence_deletion_remount_and_scope_are_distinct() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let (absent, value) = inspect(&f, &store);
    assert!(value.draft.is_none());
    assert!(current(&f, &store, &absent));
    let mut b = budget(&mut store, &f);
    f.edit(&mut store, &mut b, f.insert());
    assert!(!current(&f, &store, &absent), "absence became presence");
    let (stamp, ordinary) = inspect(&f, &store);
    assert!(
        ordinary.draft.is_none(),
        "ordinary pending intent became local acceptance"
    );
    assert!(current(&f, &store, &stamp));
    let group = f.group.group_id();
    let author = f.device.device_id();
    for (server, group, target, author) in [
        (SERVER + 1, group.clone(), f.target, author),
        (SERVER, vec![99], f.target, author),
        (
            SERVER,
            group.clone(),
            StudioTarget::Index { channel: [99; 16] },
            author,
        ),
        (
            SERVER,
            group,
            f.target,
            MlsDevice::generate().unwrap().device_id(),
        ),
    ] {
        assert!(!store
            .studio_inspection_is_current(server, &group, target, author, &stamp)
            .unwrap());
    }
    let path = store
        .epoch_intent_path(&crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap());
    let bytes = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(
        !current(&f, &store, &stamp),
        "deleted wrapper remained current"
    );
    fs::write(&path, &bytes).unwrap();
    assert!(current(&f, &store, &stamp));
    drop(store);
    let store = open(root.path());
    assert!(
        !current(&f, &store, &stamp),
        "remounted identical bytes remained current"
    );
    let (stamp, _) = inspect(&f, &store);
    fs::write(&path, b"corrupt sealed record").unwrap();
    assert!(store
        .capture_studio_inspection(SERVER, &f.group.group_id(), f.target, author)
        .is_err());
    assert!(store
        .studio_inspection_is_current(SERVER, &f.group.group_id(), f.target, author, &stamp)
        .is_err());
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(store
        .capture_studio_inspection(SERVER, &f.group.group_id(), f.target, author)
        .is_err());
}

#[test]
fn studio_inspection_author_and_full_channel_are_validated_detached() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    prepare(&f, &mut store);
    let (_, value) = inspect(&f, &store);
    assert_eq!(value.draft.unwrap().accepted(), 1);
    let wrong = MlsDevice::generate().unwrap().device_id();
    let capture = store
        .capture_studio_inspection(SERVER, &f.group.group_id(), f.target, wrong)
        .unwrap();
    assert!(
        matches!(capture.rebuild(), Err(AppError::Invalid(e)) if e.contains("overlay inspection author mismatch")),
        "foreign author escaped detached inspection"
    );
    let wrong_target = StudioTarget::Flipnote {
        channel: [99; 16],
        object: [9; 16],
    };
    assert_eq!(
        wrong_target.document(&f.group.group_id()).unwrap(),
        f.logical
    );
    let capture = store
        .capture_studio_inspection(
            SERVER,
            &f.group.group_id(),
            wrong_target,
            f.device.device_id(),
        )
        .unwrap();
    assert!(
        capture.rebuild().is_err(),
        "foreign channel escaped detached inspection"
    );
}

#[test]
fn studio_inspection_prepared_and_completed_never_resolve_on_read() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, expected) = prepare(&f, &mut store);
        let mut b = budget(&mut store, &f);
        let result = store.handoff_studio_overlay_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b,
            &mut |step, p, bytes| {
                if step == HandoffWrite::Source {
                    return Err(AppError::Io("pause after Prepared".into()));
                }
                atomic_write(p, bytes)
            },
            &mut flush,
        );
        assert!(result.is_err());
        let path = store.epoch_intent_path(
            &crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap(),
        );
        let bytes = fs::read(&path).unwrap();
        let sources = canonical(&store);
        let (stamp, value) = inspect(&f, &store);
        assert!(value.prepared);
        assert_eq!(value.draft.unwrap().projection(), &expected);
        assert!(current(&f, &store, &stamp));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(canonical(&store), sources);
        drop(store);
        let mut store = open(root.path());
        transfer(&f, &mut store, basis);
        let bytes = fs::read(&path).unwrap();
        let sources = canonical(&store);
        let (_, value) = inspect(&f, &store);
        assert!(value.draft.is_none());
        assert!(!value.prepared);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(canonical(&store), sources);
        if art {
            let target = StudioTarget::Flipnote {
                channel: [99; 16],
                object: [9; 16],
            };
            let capture = store
                .capture_studio_inspection(
                    SERVER,
                    &f.group.group_id(),
                    target,
                    f.device.device_id(),
                )
                .unwrap();
            assert!(
                capture.rebuild().is_err(),
                "completed metadata lost full channel binding"
            );
        }
    }
}

#[test]
fn studio_inspection_maximum_accepted_operation_count() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (basis, expected) = performance::fixture(&f, &mut store, 256);
        let (stamp, value) = inspect(&f, &store);
        let draft = value.draft.unwrap();
        assert_eq!((draft.basis(), draft.accepted()), (basis, 256));
        assert_eq!(draft.projection(), &expected);
        assert!(current(&f, &store, &stamp));
    }
}

#[test]
fn studio_inspection_maximal_canonical_seed_and_bounded_record_input() {
    use catcoms_replication::studio::StudioOverlayState;
    use catcoms_wire::{Decoder, Encoder};
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        prepare(&f, &mut store);
        let mut state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        let old = state
            .overlay()
            .unwrap()
            .encode_vault(&state.ledger)
            .unwrap();
        let mut d = Decoder::new(&old);
        let mut e = Encoder::new();
        e.put_u8(d.get_u8().unwrap());
        e.put_u8(d.get_u8().unwrap());
        for _ in 0..4 {
            e.put_bytes(d.get_bytes().unwrap()).unwrap();
        }
        let receipt = Receipt::decode(d.get_bytes().unwrap()).unwrap();
        let _small_seed = d.get_bytes().unwrap();
        let mut large = shapes::maximal(&f.group.group_id(), f.target);
        match &mut large {
            StudioProjection::Index(p) => p.epoch = 0,
            StudioProjection::Flipnote(p) => p.epoch = 0,
        }
        let seed = large.checkpoint(receipt.close_record_hash).unwrap();
        let receipt = Receipt::sign(
            f.logical.clone(),
            0,
            receipt.close_record_hash,
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &f.device,
        )
        .unwrap();
        e.put_bytes(&receipt.encode()).unwrap();
        e.put_bytes(seed.bytes()).unwrap();
        e.put_u64(d.get_u64().unwrap());
        let count = d.get_u32().unwrap();
        e.put_u32(count);
        for _ in 0..count {
            e.put_bytes(d.get_bytes().unwrap()).unwrap();
            e.put_bytes(d.get_bytes().unwrap()).unwrap();
            e.put_u64(d.get_u64().unwrap());
            e.put_u64(d.get_u64().unwrap());
        }
        d.finish().unwrap();
        // Codec-sized sealed fixture: decoded local metadata only, never a fresh append basis.
        // Genuine store acceptance and actor-produced delivery are covered separately.
        state.overlay = Some(StudioOverlayState::decode_vault(&e.finish(), &state.ledger).unwrap());
        let expected = state.local_draft().unwrap().unwrap().projection().clone();
        let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
        let (_, old) = store.read_epoch_intent_record(&scope, &f.logical).unwrap();
        let mut b = budget(&mut store, &f);
        store
            .write_prepared_intents(
                SERVER,
                &f.logical,
                state,
                old,
                false,
                &mut rng(),
                &mut b.storage,
                &mut b.intents,
                atomic_write,
                sync_intent,
            )
            .unwrap();
        let path = store.epoch_intent_path(&scope);
        let bytes = fs::read(&path).unwrap();
        let (stamp, value) = inspect(&f, &store);
        assert_eq!(value.draft.unwrap().projection(), &expected);
        assert!(current(&f, &store, &stamp));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::write(
            &path,
            vec![0; crate::store::epoch_intents::MAX_SEALED_BYTES + 1],
        )
        .unwrap();
        assert!(
            store
                .capture_studio_inspection(
                    SERVER,
                    &f.group.group_id(),
                    f.target,
                    f.device.device_id()
                )
                .is_err(),
            "oversized sealed input escaped inspection cap"
        );
    }
}
