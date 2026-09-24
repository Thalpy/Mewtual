use super::*;

#[test]
fn studio_overlay_handoff_candidate_requires_the_private_store_capability() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    super::fences::interrupt(&f, &mut store, basis, WriteTag::Source);
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let mut source = f.load(&store).unwrap();
    let before = source.unit.snapshot().unwrap();
    let observed = source.source.as_ref().map(SourceVersion::record);
    source
        .unit
        .edit_or_reseal(&f.device, &f.group, &mut rng(), &f.title(), 123)
        .unwrap();
    assert_eq!(
        state
            .handoff_metadata()
            .unwrap()
            .evidence(&source.unit, &state.ledger)
            .unwrap(),
        catcoms_replication::studio::StudioHandoffEvidence::Complete
    );
    let original = fs::read(f.path(&store)).unwrap();
    let mut b = budget(&mut store, &f);
    let mut wrote = false;
    let result = store.save_studio_source(
        SERVER,
        source.unit,
        observed,
        &before,
        WritePurpose::Ordinary,
        &mut rng(),
        &mut b.storage,
        WriteStep::new(WriteTag::Source),
        // Records whether a replacement was attempted at all.
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
        matches!(result,Err(AppError::Invalid(ref s)) if s.contains("prepared handoff blocks source replacement")),
        "ordinary writer bypassed the private handoff capability: {result:?}"
    );
    assert!(!wrote);
    assert_eq!(fs::read(f.path(&store)).unwrap(), original);
}

/// Simulate authenticated but conflicting saved evidence at an interrupted source write.
/// The alternate source is independently typed-admitted/signed; it is not a malformed blob.
fn substituted(
    f: &Fixture,
    store: &ServerStore,
    mutate_delta: bool,
) -> (Vec<u8>, StudioProjection) {
    let mut source = f.load(store).unwrap().unit;
    let mut signed_copy = StudioEpoch::restore(
        &source.snapshot().unwrap(),
        &f.group,
        f.target,
        f.device.device_id(),
    )
    .unwrap();
    let packet = signed_copy
        .edit_or_reseal(&f.device, &f.group, &mut rng(), &f.title(), 123)
        .unwrap();
    let original = f.signed(&packet);
    let signed = if mutate_delta {
        let mut expanded = automerge::Change::from_bytes(original.delta.clone())
            .unwrap()
            .decode();
        expanded.message = Some("different signed delta, same envelope and typed values".into());
        let change = automerge::Change::from(expanded);
        SignedOp::sign_domain(
            &f.device,
            f.logical.doc_type,
            source.doc_id(),
            change.raw_bytes().to_vec(),
            &f.title(),
        )
        .unwrap()
    } else {
        original.clone()
    };
    assert_eq!(
        signed.parsed_domain_op().unwrap(),
        original.parsed_domain_op().unwrap()
    );
    if mutate_delta {
        assert_ne!(signed.encode(), original.encode());
    }
    let packet = SealedOp::seal(&signed, &f.group, &f.device, &mut rng()).unwrap();
    assert_eq!(
        source.ingest(&packet, &f.group, &f.device).unwrap(),
        Admission::Accepted
    );
    let expected = signed_copy.projection().unwrap();
    assert_eq!(source.projection().unwrap(), expected);
    let scope = scope_bytes(SERVER, &f.logical).unwrap();
    let mut e = Encoder::new();
    e.put_bytes(&scope).unwrap();
    e.put_bytes(&f.target.channel()).unwrap();
    e.put_bytes(&source.snapshot().unwrap()).unwrap();
    e.put_u8(1);
    let sealed = seal(&store.keys.db_key().unwrap(), &e.finish(), &mut rng()).unwrap();
    (frame(&sealed), expected)
}

#[test]
fn studio_overlay_handoff_full_signed_digest_prevents_false_completion() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, expected) = prepare(&f, &mut store);
        let (substitute, projection) = substituted(&f, &store, true);
        assert_eq!(projection, expected);
        let mut b = budget(&mut store, &f);
        // Both decisions below observe it, so it cannot be held mutably by either alone.
        let hit = std::cell::Cell::new(false);
        let error = store
            .handoff_studio_overlay_with_io(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                basis,
                Some(0),
                &mut rng(),
                &mut b,
                // Substitute different bytes for the source record, then fail: the record on
                // disk is not the one this transaction signed. Replacing the bytes is what the
                // before decision is for; the write itself stays the capability's.
                &mut WriteHooks::Hooked {
                    before: Some(&mut |step: WriteTag, _: &Path, _: &[u8]| {
                        if step == WriteTag::Source {
                            hit.set(true);
                            return Intercept::Replace(substitute.clone());
                        }
                        Intercept::Continue
                    }),
                    before_sync: None,
                    before_unlink: None,
                    after: Some(&mut |op: CompletedOperation, step: WriteTag, _: &Path| {
                        if op == CompletedOperation::Write && step == WriteTag::Source {
                            return AfterIntercept::Fail(invalid("substituted signed source"));
                        }
                        AfterIntercept::Continue
                    }),
                },
            )
            .unwrap_err();
        assert!(hit.get() && error.to_string().contains("substituted signed source"));
        drop(store);
        let mut store = open(root.path());
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert!(state.handoff_prepared());
        let source = f.load(&store).unwrap();
        assert_eq!(source.projection().unwrap(), expected);
        assert!(source
            .unit
            .contains_exact_operation(f.device.device_id(), &f.title())
            .unwrap());
        let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
        let before = fs::read(store.epoch_intent_path(&scope)).unwrap();
        let mut b = budget(&mut store, &f);
        let result =
            store.resolve_studio_handoff(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b);
        assert!(
            matches!(result,Err(AppError::Invalid(ref s)) if s.contains("conflicting or incomplete signed evidence")),
            "handoff accepted a different complete signed operation: {result:?}"
        );
        assert_eq!(fs::read(store.epoch_intent_path(&scope)).unwrap(), before);
        assert!(store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .overlay()
            .is_some());
    }
}

#[test]
fn studio_overlay_handoff_partial_manifest_keeps_the_entire_branch() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    save(&f, &mut store, &close, basis.fingerprint(), f.title(), 123);
    let mut second = f.title();
    second.nonce = [77; 16];
    save(&f, &mut store, &close, basis.fingerprint(), second, 124);
    install(&f, &mut store, &close);
    let (substitute, _) = substituted(&f, &store, false);
    let mut b = budget(&mut store, &f);
    let error = store
        .handoff_studio_overlay_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis.fingerprint(),
            Some(0),
            &mut rng(),
            &mut b,
            &mut WriteHooks::Hooked {
                before: Some(&mut |step: WriteTag, _: &Path, _: &[u8]| {
                    if step == WriteTag::Source {
                        return Intercept::Replace(substitute.clone());
                    }
                    Intercept::Continue
                }),
                before_sync: None,
                before_unlink: None,
                after: Some(&mut |op: CompletedOperation, step: WriteTag, _: &Path| {
                    if op == CompletedOperation::Write && step == WriteTag::Source {
                        return AfterIntercept::Fail(invalid("partial signed source"));
                    }
                    AfterIntercept::Continue
                }),
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("partial signed source"));
    drop(store);
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    assert!(store
        .resolve_studio_handoff(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .is_err());
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert!(state.handoff_prepared());
    assert_eq!(state.local_draft().unwrap().unwrap().accepted(), 2);
    assert_eq!(f.load(&store).unwrap().op_count(), 1);
}

#[test]
fn studio_overlay_handoff_rechecks_source_after_prepared_before_candidate_write() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    let mut changed = f.load(&store).unwrap().unit;
    let conflict = Receipt::sign(
        f.logical.clone(),
        0,
        [99; 32],
        [88; 32],
        0,
        InheritedCheckpoint::EpochZero,
        &f.device,
    )
    .unwrap();
    assert_eq!(
        changed.seal(conflict, &f.group, 0).unwrap(),
        ReceiptIngest::Fault
    );
    assert_eq!(changed.op_count(), 0);
    let scope = scope_bytes(SERVER, &f.logical).unwrap();
    let mut e = Encoder::new();
    e.put_bytes(&scope).unwrap();
    e.put_bytes(&f.target.channel()).unwrap();
    e.put_bytes(&changed.snapshot().unwrap()).unwrap();
    e.put_u8(1);
    let changed = frame(&seal(&store.keys.db_key().unwrap(), &e.finish(), &mut rng()).unwrap());
    let source_path = f.path(&store);
    let mut b = budget(&mut store, &f);
    let mut hit = false;
    let result = store.handoff_studio_overlay_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        basis,
        Some(0),
        &mut rng(),
        &mut b,
        // Once the Prepared record is durable, change the source underneath it. The write
        // itself is the transaction's; this only disturbs a different file afterwards.
        &mut WriteHooks::Hooked {
            before: None,
            before_sync: None,
            before_unlink: None,
            after: Some(&mut |op: CompletedOperation, step: WriteTag, _: &Path| {
                if op == CompletedOperation::Write && step == WriteTag::Prepared {
                    hit = true;
                    write_for_test(&source_path, &changed).unwrap();
                }
                AfterIntercept::Continue
            }),
        },
    );
    assert!(hit);
    assert!(
        matches!(result,Err(AppError::Invalid(ref s)) if s.contains("candidate no longer matches Prepared")),
        "handoff overwrote source changed after Prepared: {result:?}"
    );
    assert_eq!(fs::read(source_path).unwrap(), changed);
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .handoff_prepared());
}
