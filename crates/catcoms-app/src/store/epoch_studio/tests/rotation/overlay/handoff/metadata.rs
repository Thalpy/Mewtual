use super::*;
use catcoms_replication::studio::{StudioOverlay, StudioOverlayState};
use catcoms_replication::IntentLedger;

#[test]
fn studio_overlay_handoff_rollover_floor_rejects_forgotten_retry_after_rewind() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (first_close, first_basis) = closing(&f, &mut store);
    let rewind = f.load(&store).unwrap().unit.snapshot().unwrap();
    save(
        &f,
        &mut store,
        &first_close,
        first_basis.fingerprint(),
        f.title(),
        123,
    );
    install(&f, &mut store, &first_close);
    transfer(&f, &mut store, first_basis.fingerprint());
    grow(&f, &mut store);
    let mut source = f.load(&store).unwrap();
    let previous = source.unit.receipt_head().unwrap().cloned().unwrap();
    let decision = source
        .unit
        .new_owner_decision(&f.group, &f.device, 0, Some(&previous))
        .unwrap();
    let close = decision.close().clone();
    let mut b = budget(&mut store, &f);
    store
        .seal_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            decision.receipt().clone(),
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    let basis = store
        .prepare_studio_closing_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &close,
            Some(0),
            &mut b,
        )
        .unwrap();
    let mut second = f.title();
    second.nonce = [77; 16];
    save(&f, &mut store, &close, basis.fingerprint(), second, 300);
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let metadata = state.handoff_metadata().unwrap();
    assert!(metadata.overlay().is_some() && metadata.has_completed());
    let encoded = metadata.encode_vault(&state.ledger).unwrap();
    assert_eq!(
        StudioOverlayState::decode_vault(&encoded, &state.ledger)
            .unwrap()
            .target(),
        f.target
    );
    install(&f, &mut store, &close);
    assert!(!store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .pending()
        .any(|(id, _)| *id == f.title().id(&f.device.device_id())));
    transfer(&f, &mut store, basis.fingerprint());
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let metadata = state.handoff_metadata().unwrap();
    assert_eq!(metadata.minimum_new_basis_closed_epoch(), 2);
    assert!(metadata
        .completed_branch(f.target, f.device.device_id(), first_basis.fingerprint())
        .unwrap()
        .is_none());
    // A valid floor-only record retains complete target binding even with no branch or ack.
    let encoded = metadata.encode_vault(&state.ledger).unwrap();
    let mut d = Decoder::new(&encoded);
    assert_eq!(d.get_u8().unwrap(), 2);
    assert_eq!(d.get_u8().unwrap(), 1);
    d.get_bytes().unwrap();
    d.get_bytes().unwrap();
    assert_eq!(d.get_u64().unwrap(), 2);
    assert_eq!(d.get_u8().unwrap(), 0);
    let completed_offset = encoded.len() - d.remaining();
    let mut floor_only = encoded[..completed_offset].to_vec();
    floor_only.push(0);
    let empty = IntentLedger::new(f.logical.clone());
    let floor = StudioOverlayState::decode_vault(&floor_only, &empty).unwrap();
    assert_eq!(floor.target(), f.target);
    assert_eq!(floor.minimum_new_basis_closed_epoch(), 2);
    assert!(!floor.has_completed() && floor.overlay().is_none());
    // Restore only the old actual source, preserving the new mandatory metadata and ledger.
    let mut current = f.load(&store).unwrap();
    let observed = current.source.as_ref().map(SourceVersion::record);
    let before = current.unit.snapshot().unwrap();
    let unit = StudioEpoch::restore(&rewind, &f.group, f.target, f.device.device_id()).unwrap();
    let mut b = budget(&mut store, &f);
    store
        .save_studio_source(
            SERVER,
            unit,
            observed,
            &before,
            WritePurpose::Ordinary,
            &mut rng(),
            &mut b.storage,
            WriteStep::new(WriteTag::Source),
            &mut WriteHooks::None,
        )
        .unwrap();
    drop(store);
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    let fresh = store
        .prepare_studio_closing_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &first_close,
            Some(0),
            &mut b,
        )
        .unwrap();
    assert_eq!(
        fresh.fingerprint(),
        first_basis.fingerprint(),
        "rewind did not reproduce the old otherwise eligible basis"
    );
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let original = fs::read(store.epoch_intent_path(&scope)).unwrap();
    let result = store.save_studio_closing_overlay(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        &first_close,
        Some(0),
        first_basis.fingerprint(),
        f.title(),
        123,
        &mut rng(),
        &mut b,
    );
    assert!(
        matches!(result,Err(AppError::Invalid(ref s)) if s.contains(&ReplError::EpochScope.to_string())),
        "forgotten overlay retry crossed persisted floor: {result:?}"
    );
    assert_eq!(fs::read(store.epoch_intent_path(&scope)).unwrap(), original);
}

#[test]
fn studio_overlay_handoff_capacity_preflight_and_full_cap_completed_sync_retry() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis, _) = prepare(&f, &mut store);
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let path = store.epoch_intent_path(&scope);
    let staging = path.with_file_name(format!(
        ".{}.mewtual-stage-7-99.tmp",
        path.file_name().unwrap().to_str().unwrap()
    ));
    let size = fs::metadata(&path).unwrap().len();
    File::create(&staging)
        .unwrap()
        .set_len(crate::store::MAX_VAULT_INTENT_BYTES - size)
        .unwrap();
    let mut b = budget(&mut store, &f);
    let original = canonical(&store);
    let mut wrote = false;
    let result = store.handoff_studio_overlay_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        basis,
        Some(0),
        &mut rng(),
        &mut b,
        // Records whether any replacement was reached; the transaction still performs it.
        &mut WriteHooks::Hooked {
            before: Some(&mut |_: WriteTag, _: &Path, _: &[u8]| {
                wrote = true;
                Intercept::Continue
            }),
            before_sync: None,
            before_unlink: None,
            after: None,
        },
    );
    assert!(
        matches!(result,Err(AppError::Invalid(ref s)) if s.contains("vault intent limit reached")),
        "handoff exceeded physical intent cap: {result:?}"
    );
    assert!(!wrote);
    assert_eq!(canonical(&store), original);
    File::options()
        .write(true)
        .open(&staging)
        .unwrap()
        .set_len(0)
        .unwrap();
    let outcome = transfer(&f, &mut store, basis);
    let size = fs::metadata(&path).unwrap().len();
    File::options()
        .write(true)
        .open(&staging)
        .unwrap()
        .set_len(crate::store::MAX_VAULT_INTENT_BYTES - size)
        .unwrap();
    let original = fs::read(&path).unwrap();
    let mut b = budget(&mut store, &f);
    assert_eq!(b.intents.bytes(), crate::store::MAX_VAULT_INTENT_BYTES);
    let result = store.save_studio_closing_overlay_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        &close,
        None,
        basis,
        f.title(),
        999,
        &mut rng(),
        &mut b,
        // A completed retry must reach the flush, never a replacement.
        &mut WriteHooks::Hooked {
            before: Some(&mut |_: WriteTag, _: &Path, _: &[u8]| {
                panic!("completed retry allocated replacement")
            }),
            before_sync: Some(&mut |_: WriteTag, _: &Path, _: u64| {
                AfterIntercept::Fail(invalid("injected completed retry sync"))
            }),
            before_unlink: None,
            after: None,
        },
    );
    assert!(
        matches!(result,Err(AppError::Invalid(ref s)) if s.contains("injected completed retry sync"))
    );
    assert!(b.requires_reconciliation());
    let mut b = budget(&mut store, &f);
    let mut synced = false;
    let result = store
        .save_studio_closing_overlay_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &close,
            None,
            basis,
            f.title(),
            999,
            &mut rng(),
            &mut b,
            // The flush still happens; this only records that the transaction reached it.
            &mut WriteHooks::Hooked {
                before: Some(&mut |_: WriteTag, _: &Path, _: &[u8]| {
                    panic!("completed retry allocated replacement")
                }),
                before_sync: Some(&mut |_: WriteTag, _: &Path, _: u64| {
                    synced = true;
                    AfterIntercept::Continue
                }),
                before_unlink: None,
                after: None,
            },
        )
        .unwrap();
    assert!(synced);
    assert!(matches!(result,StudioOverlaySave::HandedOff(ref value) if value==&outcome));
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn studio_overlay_handoff_preflights_later_source_peak_before_prepared_write() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    for n in 70..78 {
        let mut op = f.title();
        op.nonce = [n; 16];
        save(&f, &mut store, &close, basis.fingerprint(), op, n as u64);
    }
    install(&f, &mut store, &close);
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let old = fs::metadata(store.epoch_intent_path(&scope)).unwrap().len();
    let mut source = f.load(&store).unwrap().unit;
    let candidate = state
        .handoff_metadata()
        .unwrap()
        .prepare_handoff(
            &mut source,
            &state.ledger,
            &f.device,
            &f.group,
            0,
            &mut rng(),
        )
        .unwrap();
    let (mut candidate, metadata) = candidate.into_parts();
    let mut staged = state.clone();
    staged.overlay = Some(metadata);
    let prepared = staged.encode(&scope).unwrap().len() as u64 + 40;
    let source_scope = scope_bytes(SERVER, &f.logical).unwrap();
    let mut e = Encoder::new();
    e.put_bytes(&source_scope).unwrap();
    e.put_bytes(&f.target.channel()).unwrap();
    e.put_bytes(&candidate.snapshot().unwrap()).unwrap();
    e.put_u8(1);
    let next = e.finish().len() as u64 + 40 - candidate.storage_protocol_bytes().unwrap() as u64;
    assert!(
        next > old,
        "fixture needs a later source peak larger than Prepared's reclaimed predecessor"
    );
    let mut b = budget(&mut store, &f);
    let inv = inventory(&mut store);
    let mut records = inv.records_for_server(SERVER, &f.group.group_id()).unwrap();
    records.push(StorageRecord {
        id: [255; 32],
        document: [255; 32],
        footprint: Footprint {
            content: crate::store::epoch_budget::CONTENT_ALLOWANCE_BYTES
                - b.usage().content
                - prepared,
            ..Footprint::default()
        },
    });
    b.storage = EpochStorageBudget::from_inventory(b.scope.clone(), records).unwrap();
    let original = fs::read(store.epoch_intent_path(&scope)).unwrap();
    let mut wrote = false;
    let result = store.handoff_studio_overlay_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        basis.fingerprint(),
        Some(0),
        &mut rng(),
        &mut b,
        // Records whether any replacement was reached; the transaction still performs it.
        &mut WriteHooks::Hooked {
            before: Some(&mut |_: WriteTag, _: &Path, _: &[u8]| {
                wrote = true;
                Intercept::Continue
            }),
            before_sync: None,
            before_unlink: None,
            after: None,
        },
    );
    assert!(result.is_err());
    assert!(
        !wrote,
        "handoff wrote Prepared before checking the later source peak"
    );
    assert_eq!(fs::read(store.epoch_intent_path(&scope)).unwrap(), original);
    assert!(!b.requires_reconciliation());
}

#[test]
fn studio_overlay_handoff_codec_migrates_v1_and_retains_target_without_a_ledger() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, _) = prepare(&f, &mut store);
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        let v1 = state
            .overlay()
            .unwrap()
            .encode_vault(&state.ledger)
            .unwrap();
        let migrated = StudioOverlayState::decode_vault(&v1, &state.ledger).unwrap();
        assert_eq!(migrated.target(), f.target);
        assert_eq!(
            migrated.encode_vault(&state.ledger).unwrap(),
            v1,
            "reading v1 rewrote it"
        );
        let v2 = state
            .handoff_metadata()
            .unwrap()
            .encode_vault(&state.ledger)
            .unwrap();
        assert_eq!(v2[0], 2);
        assert!(StudioOverlay::decode_vault(&v2, &state.ledger).is_err());
        for index in [1, 2] {
            let mut malformed = v2.clone();
            malformed[index] = 99;
            assert!(StudioOverlayState::decode_vault(&malformed, &state.ledger).is_err());
        }
        let mut trailing = v2.clone();
        trailing.push(0);
        assert!(StudioOverlayState::decode_vault(&trailing, &state.ledger).is_err());
        transfer(&f, &mut store, basis);
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        let bytes = state
            .handoff_metadata()
            .unwrap()
            .encode_vault(&state.ledger)
            .unwrap();
        let empty = IntentLedger::new(f.logical.clone());
        let restored = StudioOverlayState::decode_vault(&bytes, &empty).unwrap();
        assert_eq!(restored.minimum_new_basis_closed_epoch(), 1);
        assert!(restored.overlay().is_none());
        assert!(restored
            .completed_branch(f.target, f.device.device_id(), basis)
            .unwrap()
            .is_some());
        assert!(
            bytes.len() < 1024,
            "completed metadata retained the large base"
        );
        if let StudioTarget::Flipnote { object, .. } = f.target {
            let wrong = StudioTarget::Flipnote {
                channel: [66; 16],
                object,
            };
            assert!(matches!(
                restored.completed_branch(wrong, f.device.device_id(), basis),
                Err(ReplError::EpochScope)
            ));
            let mut mismatch = bytes.clone();
            // Canonical enclosing target's channel starts after version, kind and byte length.
            mismatch[6..22].fill(66);
            assert!(matches!(
                StudioOverlayState::decode_vault(&mismatch, &empty),
                Err(ReplError::EpochScope)
            ));
        }
    }
}
