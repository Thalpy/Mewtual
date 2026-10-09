use super::*;
use catcoms_replication::studio::{StudioOverlay, StudioOverlayState};
use catcoms_replication::IntentLedger;

/// V8, and a regression the branch wiring first introduced: a transferred branch's acknowledgement
/// stays owed after a **newer** branch has been admitted, and it needs no tenure.
///
/// `classify_request` derives the transferred branch's identity from the *current* generation, so
/// it can only recognise it until the next admission moves the generation on. The bare
/// `completed_retry` that S1 used to call keyed the same acknowledgement on basis and operation
/// instead, so it survived that. Replacing it outright made the delayed retry `Unmatched`: refused
/// as stale under a known tenure, and refused for its tenure under `Imported` or `Unknown` - which
/// for `Imported` is permanent. An acknowledgement is not an acceptance, so nothing about the
/// namespace requires giving it up.
///
/// Sequence: G1 accepted and transferred; a new Closing basis; G2 admitted and live at generation
/// 2; then G1's own delayed request, with **no** tenure. It must be acknowledged with G1's outcome,
/// write nothing new and leave G2 untouched.
#[test]
fn a_transferred_branch_is_still_acknowledged_after_a_newer_branch_is_admitted() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (first_close, first_basis) = closing(&f, &mut store);
    let first_branch = request_branch(&f, &mut store, &first_close);
    save(
        &f,
        &mut store,
        &first_close,
        first_basis.fingerprint(),
        f.title(),
        123,
    );
    install(&f, &mut store, &first_close);
    let first_outcome = transfer(&f, &mut store, first_basis.fingerprint());

    // A new Closing basis, and G2 admitted on it.
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
    assert_eq!(
        metadata.branch_generation(),
        2,
        "G2 must be live at a newer generation, or this is not the case under test"
    );
    assert!(
        metadata.has_completed(),
        "G1's transfer manifest must still be retained"
    );
    let live = metadata.branch_id();
    drop(state);
    let before =
        fs::read(store.epoch_intent_path(
            &crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap(),
        ))
        .unwrap();

    // G1's delayed request, exactly as its client sent it, with no tenure at all.
    let mut b = budget(&mut store, &f);
    let retried = store
        .save_studio_closing_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &first_close,
            StudioOwnerTenure::Unknown,
            first_basis.fingerprint(),
            first_branch,
            f.title(),
            999,
            &mut rng(),
            &mut b,
        )
        .unwrap_or_else(|e| panic!("the transferred branch's acknowledgement was refused: {e}"));
    assert!(
        matches!(retried, StudioOverlaySave::HandedOff(ref value) if value == &first_outcome),
        "acknowledged with the wrong outcome: {retried:?}"
    );
    assert_eq!(
        fs::read(store.epoch_intent_path(
            &crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap()
        ))
        .unwrap(),
        before,
        "an acknowledgement wrote new content"
    );
    assert_eq!(
        store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .handoff_metadata()
            .unwrap()
            .branch_id(),
        live,
        "acknowledging G1 disturbed the live G2 branch"
    );
}

/// I-4 audit M-3. Every page serve of a target whose handoff completed runs the publication
/// check, which flushes the completed record. The flush stays a mutation for inventory purposes,
/// but repeating one this mount already made, of a file nothing has written since, rotated the
/// token on every serve, so a peer polling pages could restart any inventory job spanning visits
/// for as long as it polled. In a quiet vault the second check must not rotate; after any
/// five-family write the next one flushes, and rotates, again.
#[test]
fn a_completed_handoff_is_flushed_once_per_quiet_period_not_on_every_serve() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    let _ = request_branch(&f, &mut store, &close);
    save(&f, &mut store, &close, basis.fingerprint(), f.title(), 123);
    install(&f, &mut store, &close);
    transfer(&f, &mut store, basis.fingerprint());
    {
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        let metadata = state.handoff_metadata().unwrap();
        assert!(
            metadata.has_completed() && !metadata.is_prepared(),
            "precondition: a completed handoff that is not prepared, which is what flushes"
        );
    }
    let check = |store: &mut ServerStore| {
        store
            .check_studio_handoff_publication(SERVER, &f.group, f.target)
            .unwrap()
    };
    let before = store.inventory_generation();
    check(&mut store);
    let quiet = store.inventory_generation();
    assert!(
        !std::sync::Arc::ptr_eq(&before, &quiet),
        "the first check skipped a flush this mount had never made"
    );
    check(&mut store);
    assert!(
        std::sync::Arc::ptr_eq(&quiet, &store.inventory_generation()),
        "a repeat flush of an unchanged record rotated the token"
    );
    // Any five-family write (here the mutation guard alone, which is what rotates) makes the memo
    // stale.
    let _ = store.epoch_mutation_guard();
    let moved = store.inventory_generation();
    check(&mut store);
    assert!(
        !std::sync::Arc::ptr_eq(&moved, &store.inventory_generation()),
        "the flush after a five-family write was skipped"
    );
}

#[test]
fn studio_overlay_handoff_rollover_floor_rejects_forgotten_retry_after_rewind() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (first_close, first_basis) = closing(&f, &mut store);
    // The branch the first Save's ticket named, kept as its client would keep it.
    let first_branch = request_branch(&f, &mut store, &first_close);
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
    //
    // **This record is now v3, not v2, and that is correct.** The Save above minted a branch where
    // none existed - `active` was `None` after the first transfer - which is a generation event, so
    // this document is on branch generation 2. A generation other than 1 cannot be expressed in v2.
    // Agent 2's lifecycle slice made that increment happen; before it, a second branch silently
    // reused generation 1 and would have inherited the transferred branch's identity.
    //
    // The consequence for this test is only that the synthesised floor-only record must carry the v3
    // tail as well: eight bytes of generation, a provenance byte and a disposal-presence byte, all at
    // the end. They are copied from the original rather than rebuilt, so this test does not restate
    // the layout it is checking.
    let encoded = metadata.encode_vault(&state.ledger).unwrap();
    let mut d = Decoder::new(&encoded);
    assert_eq!(d.get_u8().unwrap(), 3);
    assert_eq!(d.get_u8().unwrap(), 1);
    d.get_bytes().unwrap();
    d.get_bytes().unwrap();
    assert_eq!(d.get_u64().unwrap(), 2);
    assert_eq!(d.get_u8().unwrap(), 0);
    const V3_TAIL: usize = 10;
    let completed_offset = encoded.len() - d.remaining();
    let mut floor_only = encoded[..completed_offset].to_vec();
    floor_only.push(0);
    floor_only.extend_from_slice(&encoded[encoded.len() - V3_TAIL..]);
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

    // Two independent defences now stand in front of the forgotten retry, and each is proved on
    // its own rather than letting either one's refusal stand in for the other.
    //
    // The first is the branch namespace. The forgotten request resends the branch its own ticket
    // named, generation 1 of the first basis. Both branches since have been transferred and the
    // document is on generation 2, so that identity names nothing and S1b refuses it as stale -
    // before any media work, and before the plan where the floor lives.
    let result = store.save_studio_closing_overlay(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        &first_close,
        StudioOwnerTenure::Known(0),
        first_basis.fingerprint(),
        first_branch,
        f.title(),
        123,
        &mut rng(),
        &mut b,
    );
    assert!(
        matches!(result, Err(AppError::Invalid(ref s)) if s.contains("stale branch")),
        "a forgotten retry naming a long-gone branch was not refused by the namespace: {result:?}"
    );
    assert_eq!(fs::read(store.epoch_intent_path(&scope)).unwrap(), original);

    // The second is the rollover floor, now isolated. A request prepared *after* the rewind is
    // handed the branch the next admission would open, so the namespace admits it, and the only
    // thing between it and a new branch on a basis the document has legitimately moved past is
    // `minimum_new_basis_closed_epoch`. Design 6.3 step 3: the detached plan refuses it with
    // `EpochScope` before any write. Before the namespace existed the floor was reachable only
    // because nothing refused earlier; this is the first version of the test that shows it holding
    // on its own.
    let fresh_branch = request_branch(&f, &mut store, &first_close);
    assert_ne!(
        fresh_branch, first_branch,
        "the rewound document handed out the forgotten branch again"
    );
    let mut b = budget(&mut store, &f);
    let result = store.save_studio_closing_overlay(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        &first_close,
        StudioOwnerTenure::Known(0),
        first_basis.fingerprint(),
        fresh_branch,
        f.title(),
        124,
        &mut rng(),
        &mut b,
    );
    assert!(
        matches!(result, Err(AppError::Invalid(ref s)) if s.contains(&ReplError::EpochScope.to_string())),
        "a freshly prepared request on a rewound basis crossed the persisted floor: {result:?}"
    );
    assert_eq!(fs::read(store.epoch_intent_path(&scope)).unwrap(), original);
}

#[test]
fn studio_overlay_handoff_capacity_preflight_and_full_cap_completed_sync_retry() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis, _) = prepare(&f, &mut store);
    // The accepted Save's branch, which both completed retries below resend.
    let branch = live_branch(&f, &store);
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
        StudioOwnerTenure::Unknown,
        basis,
        branch,
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
            StudioOwnerTenure::Unknown,
            basis,
            branch,
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
