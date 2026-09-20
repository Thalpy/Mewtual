use super::*;
use crate::store::epoch_intents::sync_intent;
use catcoms_replication::studio::{StudioClosingOverlayBasis, StudioLocalDraft};
use catcoms_replication::CloseRecord;
use std::collections::{BTreeMap, BTreeSet};

mod archive;
mod handoff;
mod source_version;

fn local(saved: catcoms_replication::studio::StudioOverlaySave) -> StudioLocalDraft {
    match saved {
        catcoms_replication::studio::StudioOverlaySave::Local(draft) => draft,
        _ => panic!("expected retained local draft"),
    }
}

#[test]
fn studio_overlay_store_changed_closing_source_refuses_first_acceptance() {
    source_version::check(false);
}

#[test]
fn studio_overlay_store_changed_closing_source_refuses_append_but_keeps_exact_retry() {
    source_version::check(true);
}

fn closing(f: &Fixture, store: &mut ServerStore) -> (CloseRecord, StudioClosingOverlayBasis) {
    eligible(f, store);
    seal_source(f, store)
}
fn seal_source(f: &Fixture, store: &mut ServerStore) -> (CloseRecord, StudioClosingOverlayBasis) {
    let mut state = f.load(store).unwrap();
    let decision = state
        .unit
        .new_owner_decision(&f.group, &f.device, 0, None)
        .unwrap();
    let close = decision.close().clone();
    let mut b = budget(store, f);
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
    (close, basis)
}
fn save(
    f: &Fixture,
    store: &mut ServerStore,
    close: &CloseRecord,
    basis: [u8; 32],
    op: DomainOp,
    ts: u64,
) -> StudioLocalDraft {
    let mut b = budget(store, f);
    local(
        store
            .save_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                close,
                Some(0),
                basis,
                op,
                ts,
                &mut rng(),
                &mut b,
            )
            .unwrap(),
    )
}
/// Publish a real PIX and return the reference a frame operation must carry. A local acceptance
/// may not name pixels the vault does not hold, so frame fixtures publish genuine bytes rather
/// than synthetic addresses.
fn published_pix(store: &ServerStore, f: &Fixture, tint: u8) -> ([u8; 32], u64) {
    let mut bytes = pix();
    bytes[8] = tint; // vary one palette channel so each fixture frame has a distinct CID
    let mut blobs = store.blob_store(&hex::encode(f.group.group_id())).unwrap();
    let cid = blobs.put(&bytes).unwrap();
    (*cid.as_bytes(), bytes.len() as u64)
}

fn canonical(store: &ServerStore) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(store.dir.join("servers"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_none_or(|e| e != "intents"))
        .map(|p| {
            (
                p.file_name().unwrap().to_str().unwrap().to_owned(),
                fs::read(p).unwrap(),
            )
        })
        .collect()
}
fn install(f: &Fixture, store: &mut ServerStore, close: &CloseRecord) -> EpochStudioState {
    let mut b = budget(store, f);
    let mut state = f.load(store).unwrap();
    let plan = state.unit.prepare_settlement(close, &f.group, 0).unwrap();
    // Exercise the actual included-only ledger writer before independent successor selection.
    store
        .retire_studio_intents_with_io(
            SERVER,
            &plan,
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
            |m: &EpochMutation<'_>, p: &Path, b: &[u8]| m.write(p, b),
            |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_intent(p, b),
        )
        .unwrap();
    let observed = state.source.as_ref().map(SourceVersion::record);
    let before = state.unit.snapshot().unwrap();
    let next = state.unit.checkpoint_successor(&plan, &f.group, 0).unwrap();
    store
        .save_studio_source(
            SERVER,
            next,
            observed,
            &before,
            WritePurpose::Settlement,
            &mut rng(),
            &mut b.storage,
            |m: &EpochMutation<'_>, p: &Path, b: &[u8]| m.write(p, b),
            |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_studio(p, b),
        )
        .unwrap()
}

#[test]
fn studio_overlay_store_restart_exact_retry_and_source_separation() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (close, basis) = closing(&f, &mut store);
        let before = canonical(&store);
        let view = save(&f, &mut store, &close, basis.fingerprint(), f.title(), 123);
        assert_eq!(view.accepted(), 1);
        assert_eq!(
            canonical(&store),
            before,
            "local acceptance mutated canonical records"
        );
        assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Closing);
        let expected = view.projection().clone();
        let bytes = fs::read(f.path(&store)).unwrap();
        drop(store);
        let mut store = open(root.path());
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert!(state.is_overlay(&f.title().id(&f.device.device_id())));
        assert_eq!(
            state.local_draft().unwrap().unwrap().projection(),
            &expected
        );
        assert_eq!(fs::read(f.path(&store)).unwrap(), bytes);
        let retry = save(&f, &mut store, &close, basis.fingerprint(), f.title(), 999);
        assert_eq!(
            retry.projection(),
            &expected,
            "retry changed the original authored timestamp"
        );
        assert_eq!(retry.accepted(), 1);
        assert_eq!(canonical(&store), before);
    }
}

#[test]
fn studio_overlay_store_reversed_hash_order_reconstructs_full_frame_history() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    let (insert_cid, insert_bytes) = published_pix(&store, &f, 0x41);
    let (replace_cid, replace_bytes) = published_pix(&store, &f, 0x42);
    let (second_cid, second_bytes) = published_pix(&store, &f, 0x43);
    let bodies = [
        FlipnoteOp::InsertFrame {
            frame: [2; 16],
            after: Some([1; 16]),
            cid: insert_cid,
            bytes: insert_bytes,
        },
        FlipnoteOp::ReplaceFrame {
            frame: [2; 16],
            cid: replace_cid,
            bytes: replace_bytes,
        },
        FlipnoteOp::InsertFrame {
            frame: [6; 16],
            after: Some([2; 16]),
            cid: second_cid,
            bytes: second_bytes,
        },
        FlipnoteOp::RemoveFrame { frame: [2; 16] },
    ];
    let mut nonces: Vec<_> = (100u128..116)
        .map(|n| {
            let mut op = f.title();
            op.nonce = n.to_be_bytes();
            (op.id(&f.device.device_id()), op.nonce)
        })
        .collect();
    nonces.sort_by(|a, b| b.0.cmp(&a.0));
    let mut last = None;
    for (i, body) in bodies.into_iter().enumerate() {
        let mut op = f.domain(body.encode().unwrap(), 0);
        op.nonce = nonces[i].1;
        last = Some(save(
            &f,
            &mut store,
            &close,
            basis.fingerprint(),
            op,
            200 + i as u64,
        ));
    }
    let expected = last.unwrap();
    let StudioProjection::Flipnote(p) = expected.projection() else {
        panic!("wrong type")
    };
    assert!(p.tombstones.contains_key(&[2; 16]));
    assert!(p.frames.contains_key(&[6; 16]));
    let pins = store.creative_pinned_cids().unwrap();
    for cid in [[3; 32], insert_cid, replace_cid, second_cid] {
        assert!(pins
            .for_group(&f.group.group_id())
            .any(|held| *held == catcoms_storage::Cid::from_bytes(cid)));
    }
    drop(store);
    let mut store = open(root.path());
    let actual = store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .local_draft()
        .unwrap()
        .unwrap();
    assert_eq!(actual.projection(), expected.projection());
    assert_eq!(actual.accepted(), 4);
    let pins = store.creative_pinned_cids().unwrap();
    for cid in [[3; 32], insert_cid, replace_cid, second_cid] {
        assert!(pins
            .for_group(&f.group.group_id())
            .any(|held| *held == catcoms_storage::Cid::from_bytes(cid)));
    }
}

#[test]
fn studio_overlay_store_uncertain_writes_and_changed_source_retry_at_physical_cap() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (close, basis) = closing(&f, &mut store);
        let before = canonical(&store);
        let mut b = budget(&mut store, &f);
        let error = store
            .save_studio_closing_overlay_with_io(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                basis.fingerprint(),
                f.title(),
                123,
                &mut rng(),
                &mut b,
                |_m, _, _| Err(AppError::Io("before overlay write".into())),
                |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_intent(p, b),
            )
            .unwrap_err();
        assert!(error.to_string().contains("before overlay write"));
        assert!(b.requires_reconciliation());
        assert!(store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .overlay()
            .is_none());
        assert_eq!(canonical(&store), before);

        let mut b = budget(&mut store, &f);
        let error = store
            .save_studio_closing_overlay_with_io(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                basis.fingerprint(),
                f.title(),
                123,
                &mut rng(),
                &mut b,
                |_m, p, bytes| {
                    write_for_test(p, bytes)?;
                    Err(AppError::Io("after overlay rename".into()))
                },
                |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_intent(p, b),
            )
            .unwrap_err();
        assert!(error.to_string().contains("after overlay rename"));
        assert!(b.requires_reconciliation());
        let expected = store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .local_draft()
            .unwrap()
            .unwrap();
        assert_eq!(expected.accepted(), 1);
        assert_eq!(canonical(&store), before);
        let next = install(&f, &mut store, &close);
        assert_eq!(next.phase(), EpochPhase::Open);

        // A real sparse orphan in the same inventoried namespace consumes the remaining
        // physical intent capacity. The scanner charges it without decrypting a partial file.
        let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
        let path = store
            .dir
            .join("servers")
            .join(format!("{}.intents", blake3::hash(&scope).to_hex()));
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
        let error = store
            .save_studio_closing_overlay_with_io(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                None,
                basis.fingerprint(),
                f.title(),
                999,
                &mut rng(),
                &mut b,
                |_m, _, _| panic!("exact retry allocated replacement"),
                |_m, _, _| Err(AppError::Io("retry sync failed".into())),
            )
            .unwrap_err();
        assert!(error.to_string().contains("retry sync failed"));
        assert!(b.requires_reconciliation());
        let mut b = budget(&mut store, &f);
        let retry = store
            .save_studio_closing_overlay_with_io(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                None,
                basis.fingerprint(),
                f.title(),
                999,
                &mut rng(),
                &mut b,
                |_m, _, _| panic!("exact retry allocated replacement"),
                |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_intent(p, b),
            )
            .unwrap();
        assert_eq!(local(retry).projection(), expected.projection());
        let mut changed = f.title();
        changed.nonce = [99; 16];
        let error = store
            .save_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                basis.fingerprint(),
                changed,
                999,
                &mut rng(),
                &mut b,
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&catcoms_replication::ReplError::EpochClosed.to_string()),
            "{error}"
        );
    }
}

#[test]
fn studio_overlay_store_failed_ordinary_intent_is_not_acceptance_and_ordinary_apply_cannot_promote()
{
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        eligible(&f, &mut store);
        let mut b = budget(&mut store, &f);
        let error = store
            .edit_studio_with_io(
                SERVER,
                &f.group,
                f.target,
                f.id,
                &f.device,
                f.title(),
                100,
                &mut rng(),
                &mut b,
                |m: &EpochMutation<'_>, p: &Path, b: &[u8]| m.write(p, b),
                |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_intent(p, b),
                |_m, _, _| Err(AppError::Io("ordinary source save failed".into())),
                |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_studio(p, b),
            )
            .unwrap_err();
        assert!(error.to_string().contains("ordinary source save failed"));
        assert!(!f
            .load(&store)
            .unwrap()
            .contains_exact_operation(f.device.device_id(), &f.title())
            .unwrap());
        let (close, basis) = seal_source(&f, &mut store);
        let mut b = budget(&mut store, &f);
        let error = store
            .save_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                basis.fingerprint(),
                f.title(),
                100,
                &mut rng(),
                &mut b,
            )
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("ordinary intent cannot become an accepted overlay"));
        assert!(store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .overlay()
            .is_none());
        let mut draft = f.title();
        draft.nonce = [8; 16];
        save(
            &f,
            &mut store,
            &close,
            basis.fingerprint(),
            draft.clone(),
            123,
        );
        let next = install(&f, &mut store, &close);
        let mut b = budget(&mut store, &f);
        let result = store.edit_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            next.doc_id(),
            &f.device,
            draft,
            123,
            &mut rng(),
            &mut b,
        );
        assert!(
            result.is_err(),
            "ordinary Apply admitted an annotated local draft"
        );
        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("local overlay cannot enter ordinary Apply"));
        assert_eq!(f.load(&store).unwrap().op_count(), 0);
    }
}

#[test]
fn studio_overlay_store_basis_scope_and_semantics_reject_before_acceptance() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (close, basis) = closing(&f, &mut store);
        let mut b = budget(&mut store, &f);
        let missing = store
            .prepare_studio_closing_overlay(
                SERVER, &f.group, f.target, &f.device, &close, None, &mut b,
            )
            .unwrap_err();
        assert!(missing.to_string().contains("observed owner tenure"));
        let mut stale = basis.fingerprint();
        stale[0] ^= 1;
        let error = store
            .save_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                stale,
                f.title(),
                123,
                &mut rng(),
                &mut b,
            )
            .unwrap_err();
        assert!(error.to_string().contains("basis changed"));
        let mut bad = f.title();
        bad.body = b"{}".to_vec();
        assert!(store
            .save_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                basis.fingerprint(),
                bad,
                123,
                &mut rng(),
                &mut b
            )
            .is_err());
        let stranger = MlsDevice::generate().unwrap();
        assert!(store
            .prepare_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &stranger,
                &close,
                Some(0),
                &mut b
            )
            .is_err());
        assert!(store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .overlay()
            .is_none());
        let mut changed = f.title();
        changed.nonce = [88; 16];
        save(
            &f,
            &mut store,
            &close,
            basis.fingerprint(),
            changed.clone(),
            123,
        );
        changed.body = f.insert().body;
        let mut b = budget(&mut store, &f);
        let error = store
            .save_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                basis.fingerprint(),
                changed,
                123,
                &mut rng(),
                &mut b,
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&catcoms_replication::ReplError::IntentConflict.to_string()),
            "{error}"
        );
        if let StudioTarget::Flipnote { object, .. } = f.target {
            let wrong = StudioTarget::Flipnote {
                channel: [8; 16],
                object,
            };
            let mut op = f.title();
            op.nonce = [88; 16];
            let error = store
                .save_studio_closing_overlay(
                    SERVER,
                    &f.group,
                    wrong,
                    &f.device,
                    &close,
                    Some(0),
                    basis.fingerprint(),
                    op,
                    123,
                    &mut rng(),
                    &mut b,
                )
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "epoch studio: {}",
                    catcoms_replication::ReplError::EpochScope
                )
            );
        }
        let conflict = f.receipt(&f.load(&store).unwrap(), 99);
        store
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
        assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
        let error = store
            .prepare_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                &mut b,
            )
            .unwrap_err();
        assert!(error
            .to_string()
            .contains(&catcoms_replication::ReplError::EpochClosed.to_string()));
    }
}

#[test]
fn studio_overlay_store_codec_binds_annotations_envelopes_and_sequence() {
    use catcoms_replication::studio::StudioOverlay;
    use catcoms_replication::{IntentLedger, ReplError};
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (close, basis) = closing(&f, &mut store);
        save(&f, &mut store, &close, basis.fingerprint(), f.title(), 123);
        let mut second = f.title();
        second.nonce = [44; 16];
        save(
            &f,
            &mut store,
            &close,
            basis.fingerprint(),
            second.clone(),
            124,
        );
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        let mut ledger = IntentLedger::new(f.logical.clone());
        for (_, i) in state.pending() {
            ledger.prepare(i.author, i.operation.clone()).unwrap();
        }
        let overlay = state.overlay().unwrap();
        let encoded = overlay.encode_vault(&ledger).unwrap();
        let restored = StudioOverlay::decode_vault(&encoded, &ledger).unwrap();
        assert_eq!(
            restored.read(&ledger).unwrap().projection(),
            state.local_draft().unwrap().unwrap().projection()
        );
        let mut d = Decoder::new(&encoded);
        assert_eq!(d.get_u8().unwrap(), 1);
        d.get_u8().unwrap();
        for _ in 0..6 {
            d.get_bytes().unwrap();
        }
        assert_eq!(d.get_u64().unwrap(), 3);
        let count_offset = encoded.len() - d.remaining();
        assert_eq!(d.get_u32().unwrap(), 2);
        let offset = encoded.len() - d.remaining();
        assert_eq!(d.remaining(), 176);
        let mut reversed = encoded.clone();
        reversed[offset..offset + 88].copy_from_slice(&encoded[offset + 88..]);
        reversed[offset + 88..].copy_from_slice(&encoded[offset..offset + 88]);
        assert!(
            matches!(
                StudioOverlay::decode_vault(&reversed, &ledger),
                Err(ReplError::IntentConflict)
            ),
            "overlay accepted reordered sequence"
        );
        let mut missing = IntentLedger::new(f.logical.clone());
        for (_, i) in state.pending().filter(|(_, i)| i.operation != second) {
            missing.prepare(i.author, i.operation.clone()).unwrap();
        }
        assert!(matches!(
            StudioOverlay::decode_vault(&encoded, &missing),
            Err(ReplError::Malformed)
        ));
        let mut substituted = IntentLedger::new(f.logical.clone());
        for (_, i) in state.pending() {
            let mut op = i.operation.clone();
            if op == second {
                op.body = f.insert().body;
            }
            substituted.prepare(i.author, op).unwrap();
        }
        assert!(
            matches!(
                StudioOverlay::decode_vault(&encoded, &substituted),
                Err(ReplError::IntentConflict)
            ),
            "overlay accepted different envelope with the same nonce"
        );
        let mut over = encoded.clone();
        over[count_offset..count_offset + 4].copy_from_slice(&257u32.to_be_bytes());
        assert!(matches!(
            StudioOverlay::decode_vault(&over, &ledger),
            Err(ReplError::EpochBound)
        ));
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert!(StudioOverlay::decode_vault(&trailing, &ledger).is_err());
        let mut version = encoded.clone();
        version[0] = 2;
        assert!(StudioOverlay::decode_vault(&version, &ledger).is_err());

        // An older enclosing decoder reaches the extension and refuses complete consumption.
        let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
        let path = store
            .dir
            .join("servers")
            .join(format!("{}.intents", blake3::hash(&scope).to_hex()));
        let sealed = fs::read(&path).unwrap();
        let plain = unseal(&store.keys.db_key().unwrap(), &unframe(&sealed).unwrap()).unwrap();
        let mut old = Decoder::new(&plain);
        assert_eq!(old.get_bytes().unwrap(), scope);
        IntentLedger::decode(old.get_bytes().unwrap()).unwrap();
        assert!(!old.is_empty());
        assert!(old.finish().is_err());

        // Corrupt authenticated local extension fails closed in the actual inventory, too.
        let mut damaged = plain.to_vec();
        let mut parts = Decoder::new(&plain);
        parts.get_bytes().unwrap();
        parts.get_bytes().unwrap();
        damaged[plain.len() - parts.remaining()] = 99;
        let blob = seal(&store.keys.db_key().unwrap(), &damaged, &mut rng()).unwrap();
        fs::write(&path, frame(&blob)).unwrap();
        assert!(
            store.creative_pinned_cids().is_err(),
            "corrupt overlay became an empty reference set"
        );
        assert!(!store.creative_references_known());
    }
}

#[test]
fn studio_overlay_store_seed_only_references_survive_without_the_source() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    save(&f, &mut store, &close, basis.fingerprint(), f.title(), 123);
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert!(state
        .overlay()
        .unwrap()
        .base_blob_cids()
        .unwrap()
        .contains(&[3; 32]));
    // Retire the source's included ordinary insert; the overlay title remains annotated.
    let next = install(&f, &mut store, &close);
    assert_eq!(next.phase(), EpochPhase::Open);
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert_eq!(state.pending().len(), 1);
    assert_eq!(
        catcoms_replication::studio::operation_blob_cid(
            &state.pending().next().unwrap().1.operation
        )
        .unwrap(),
        None
    );
    assert_eq!(
        store
            .load_epoch_recovery(SERVER, &f.logical)
            .unwrap()
            .retained()
            .len(),
        0
    );
    // Remove the independent canonical fixture copy, then discard every cache by reopening.
    // The only remaining durable source of this CID is the actual accepted overlay base.
    fs::remove_file(f.path(&store)).unwrap();
    drop(store);
    let mut store = open(root.path());
    assert!(f.load(&store).is_none());
    let pins = store.creative_pinned_cids().unwrap();
    assert!(
        pins.for_group(&f.group.group_id())
            .any(|cid| *cid == catcoms_storage::Cid::from_bytes([3; 32])),
        "overlay seed-only pixel reference was lost"
    );
}

#[test]
fn studio_overlay_store_operation_cap_preserves_the_last_accepted_branch() {
    use catcoms_replication::studio::{StudioOverlay, MAX_STUDIO_OVERLAY_OPS};
    use catcoms_replication::{IntentLedger, ReplError};
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis) = closing(&f, &mut store);
    let mut ledger = IntentLedger::new(f.logical.clone());
    let mut prefix = Vec::new();
    let mut records = Vec::new();
    for n in 0..MAX_STUDIO_OVERLAY_OPS {
        let mut op = f.title();
        op.nonce = (n as u128 + 100).to_be_bytes();
        let id = ledger.prepare(f.device.device_id(), op).unwrap();
        // Each annotation comes from real typed append. Assemble one bounded branch using
        // the documented consecutive sequences, then require full production decoding/read.
        let mut single = StudioOverlay::new(&basis);
        single.append(&basis, &ledger, id, n as u64).unwrap();
        let bytes = single.encode_vault(&ledger).unwrap();
        let split = bytes.len() - 88;
        if prefix.is_empty() {
            prefix.extend_from_slice(&bytes[..split]);
        }
        let mut record = bytes[split..].to_vec();
        record[72..80].copy_from_slice(&(n as u64 + 1).to_be_bytes());
        records.extend_from_slice(&record);
    }
    let end = prefix.len();
    prefix[end - 12..end - 4].copy_from_slice(&(MAX_STUDIO_OVERLAY_OPS as u64 + 1).to_be_bytes());
    prefix[end - 4..].copy_from_slice(&(MAX_STUDIO_OVERLAY_OPS as u32).to_be_bytes());
    prefix.extend_from_slice(&records);
    let mut overlay = StudioOverlay::decode_vault(&prefix, &ledger).unwrap();
    assert_eq!(
        overlay.read(&ledger).unwrap().accepted(),
        MAX_STUDIO_OVERLAY_OPS
    );
    let original = overlay.encode_vault(&ledger).unwrap();
    let mut op = f.title();
    op.nonce = [98; 16];
    let id = ledger.prepare(f.device.device_id(), op).unwrap();
    assert!(matches!(
        overlay.append(&basis, &ledger, id, 999),
        Err(ReplError::EpochBound)
    ));
    assert_eq!(overlay.encode_vault(&ledger).unwrap(), original);
    let restored = StudioOverlay::decode_vault(&original, &ledger).unwrap();
    assert_eq!(
        restored.read(&ledger).unwrap().accepted(),
        MAX_STUDIO_OVERLAY_OPS
    );
}

#[test]
fn studio_overlay_store_both_retirement_modes_preserve_annotated_mixed_entries() {
    for manual in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(false);
        let mut store = open(root.path());
        let (close, basis) = closing(&f, &mut store);
        let view = save(&f, &mut store, &close, basis.fingerprint(), f.title(), 123);
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert_eq!(state.pending().len(), 2);
        let included = state.pending().map(|(id, i)| (*id, i.clone())).collect();
        let mut b = budget(&mut store, &f);
        store
            .retire_overlay_mixture_for_test(
                SERVER,
                &f.logical,
                &included,
                manual,
                &mut rng(),
                &mut b.storage,
                &mut b.intents,
            )
            .expect("mixed retirement must retain overlays and retire ordinary entries");
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert_eq!(state.pending().len(), 1);
        assert!(state.is_overlay(&f.title().id(&f.device.device_id())));
        assert_eq!(
            state.local_draft().unwrap().unwrap().projection(),
            view.projection()
        );
        drop(store);
        let store = open(root.path());
        assert_eq!(
            store
                .load_epoch_intents(SERVER, &f.logical)
                .unwrap()
                .local_draft()
                .unwrap()
                .unwrap()
                .projection(),
            view.projection()
        );
    }
}

#[test]
fn studio_overlay_store_replacement_counts_base_and_orphans_without_refunding_old_final() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    let view = save(&f, &mut store, &close, basis.fingerprint(), f.title(), 123);
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let path = store
        .dir
        .join("servers")
        .join(format!("{}.intents", blake3::hash(&scope).to_hex()));
    let original = fs::read(&path).unwrap();
    let staging = path.with_file_name(format!(
        ".{}.mewtual-stage-7-99.tmp",
        path.file_name().unwrap().to_str().unwrap()
    ));
    File::create(&staging)
        .unwrap()
        .set_len(crate::store::MAX_VAULT_INTENT_BYTES - original.len() as u64)
        .unwrap();
    let b = budget(&mut store, &f);
    assert_eq!(b.intents.bytes(), crate::store::MAX_VAULT_INTENT_BYTES);
    // Enough space remains for the next operation's tiny body if the held final/seed is
    // wrongly refunded before replacement; the correct peak includes both complete files.
    let size = fs::metadata(&staging).unwrap().len();
    File::options()
        .write(true)
        .open(&staging)
        .unwrap()
        .set_len(size - 512)
        .unwrap();
    let mut b = budget(&mut store, &f);
    let mut op = f.title();
    op.nonce = [77; 16];
    let error = store
        .save_studio_closing_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &close,
            Some(0),
            basis.fingerprint(),
            op.clone(),
            124,
            &mut rng(),
            &mut b,
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("vault intent limit reached"),
        "{error}"
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .local_draft()
            .unwrap()
            .unwrap()
            .projection(),
        view.projection()
    );
    // Leave enough physical room for the complete replacement, then admit the same request.
    File::options()
        .write(true)
        .open(&staging)
        .unwrap()
        .set_len(size - original.len() as u64 - 4096)
        .unwrap();
    let accepted = save(&f, &mut store, &close, basis.fingerprint(), op, 124);
    assert_eq!(accepted.accepted(), 2);
}

/// I-3. A complete reference scan derives its pin set from durable state, so it cannot know a
/// pixel reference that only an in-flight acceptance names. The job-owned transient hold covers
/// that window, but a durable write does NOT repair a set the scan already installed: the
/// acceptance path must transfer protection to the ordinary conservative holds before its write,
/// while the transient owner is still alive. These cases exercise the transfer, not the window.
#[test]
fn studio_overlay_acceptance_transfers_pixel_protection_before_its_write() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let (close, basis) = closing(&f, &mut store);
    let group = hex::encode(f.group.group_id());

    let mut blobs = store.blob_store(&group).unwrap();
    let payload = pix();
    let pixels = payload.len() as u64;
    let cid = blobs.put(&payload).unwrap();
    let orphan = blobs.put(b"nothing will ever name this").unwrap();

    // Establish a KNOWN pin set that predates the acceptance and excludes both CIDs, so the
    // assertions below cannot pass merely because protection is fail-closed unknown.
    store.creative_pinned_cids().unwrap();
    assert!(store.creative_references_known());

    let op = f.domain(
        FlipnoteOp::InsertFrame {
            frame: [9; 16],
            after: None,
            cid: *cid.as_bytes(),
            bytes: pixels,
        }
        .encode()
        .unwrap(),
        7,
    );
    let draft = save(&f, &mut store, &close, basis.fingerprint(), op, 300);
    assert_eq!(draft.accepted(), 1);

    // Every transient and result owner is gone, and no scan has run since. Only the ordinary
    // holds installed during the acceptance can be protecting these bytes now.
    let mut blobs = store.blob_store(&group).unwrap();
    assert!(
        !blobs.delete(&cid).unwrap(),
        "an accepted operation's pixels were reclaimable before the next scan"
    );
    assert!(blobs.get_bounded(&cid, 100_000).unwrap().is_some());
    // The control: protection is genuinely known and still reclaims an unreferenced CID, so the
    // assertion above is not an unknown-protection refusal.
    assert!(store.creative_references_known());
    assert!(blobs.delete(&orphan).unwrap());
}

/// I-3 under uncertain persistence. A write that may have landed, and a write that fails leaving
/// its temporary sibling, must both leave the new references protected: the transfer runs before
/// the write attempt precisely so its outcome does not decide whether the pixels survive.
#[test]
fn studio_overlay_uncertain_acceptance_still_protects_its_pixels() {
    for after_rename in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let f = Fixture::new(true);
        let (close, basis) = closing(&f, &mut store);
        let group = hex::encode(f.group.group_id());
        let mut blobs = store.blob_store(&group).unwrap();
        let payload = pix();
        let pixels = payload.len() as u64;
        let cid = blobs.put(&payload).unwrap();
        store.creative_pinned_cids().unwrap();
        assert!(store.creative_references_known());

        let op = f.domain(
            FlipnoteOp::InsertFrame {
                frame: [9; 16],
                after: None,
                cid: *cid.as_bytes(),
                bytes: pixels,
            }
            .encode()
            .unwrap(),
            7,
        );
        let mut b = budget(&mut store, &f);
        let failed = store.save_studio_closing_overlay_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &close,
            Some(0),
            basis.fingerprint(),
            op,
            300,
            &mut rng(),
            &mut b,
            |_m, path, bytes| {
                if after_rename {
                    // The record really lands; only the caller's result is lost.
                    write_for_test(path, bytes)?;
                } else {
                    // A failure that leaves a temporary sibling behind.
                    let mut staged = path.to_path_buf();
                    staged.set_extension("intents.mewtual-stage-1-1.tmp");
                    std::fs::write(&staged, bytes).unwrap();
                }
                Err(crate::AppError::Io("interrupted".into()))
            },
            |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_intent(p, b),
        );
        assert!(failed.is_err(), "the injected writer must fail the save");

        let mut blobs = store.blob_store(&group).unwrap();
        assert!(
            !blobs.delete(&cid).unwrap(),
            "an uncertain acceptance left its pixels reclaimable"
        );
        assert!(blobs.get_bounded(&cid, 100_000).unwrap().is_some());
    }
}

/// I3-001. An already accepted request must be classified before any media admission. The pixel
/// hold is new-authoring work: a retry adds no reference and needs no possession, so it must not
/// be able to fail because the shared reference rails are full. This is the AG1-001 boundary
/// applied to I-3's first half.
#[test]
fn studio_overlay_exact_retry_is_acknowledged_without_media_admission() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let (close, basis) = closing(&f, &mut store);
    let group = hex::encode(f.group.group_id());
    let mut blobs = store.blob_store(&group).unwrap();
    let payload = pix();
    let pixels = payload.len() as u64;
    let cid = blobs.put(&payload).unwrap();

    let op = f.domain(
        FlipnoteOp::InsertFrame {
            frame: [9; 16],
            after: None,
            cid: *cid.as_bytes(),
            bytes: pixels,
        }
        .encode()
        .unwrap(),
        7,
    );
    let first = save(&f, &mut store, &close, basis.fingerprint(), op.clone(), 300);
    assert_eq!(first.accepted(), 1);
    let records = canonical(&store);

    // Occupy every job-owned hold slot with unrelated legitimate work, so any attempt at media
    // admission on the retry path must fail rather than silently succeed.
    let occupied: Vec<_> = (0..crate::store::creative_references::MAX_TRANSIENT_HOLD_OWNERS)
        .map(|n| {
            store
                .hold_creative_transient(&f.group.group_id(), BTreeSet::from([[n as u8; 32]]))
                .expect("the rail admits its stated number of owners")
        })
        .collect();
    assert!(store
        .hold_creative_transient(&f.group.group_id(), BTreeSet::from([[200u8; 32]]))
        .is_err());
    let live = store.live_transient_holds_for_test();

    // The exact accepted request must still be acknowledged. Call the store directly rather than
    // through the fixture helper, so the failure names this boundary instead of unwrapping.
    let mut b = budget(&mut store, &f);
    let retried = store.save_studio_closing_overlay(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        &close,
        Some(0),
        basis.fingerprint(),
        op,
        301,
        &mut rng(),
        &mut b,
    );
    let retried = match retried {
        Ok(saved) => local(saved),
        Err(error) => panic!("an accepted retry was refused by media admission: {error}"),
    };
    assert_eq!(retried.accepted(), 1);
    assert_eq!(
        retried.projection(),
        first.projection(),
        "an exact retry must return the same accepted draft"
    );
    assert_eq!(
        store.live_transient_holds_for_test(),
        live,
        "an accepted retry performed media admission"
    );
    assert_eq!(
        canonical(&store),
        records,
        "an exact retry changed durable records"
    );
    drop(occupied);
}

/// N12(a). The window C-4 exists for, exercised for the first time on the real staged path: a
/// complete reference scan installs a known set while an acceptance is detached between capture
/// and commit. Only the job-owned hold can protect the new pixels there, because nothing durable
/// names them yet and the scan derives its set from durable state alone.
#[test]
fn studio_overlay_detached_acceptance_survives_a_complete_scan_between_capture_and_commit() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let (close, basis) = closing(&f, &mut store);
    let group = hex::encode(f.group.group_id());
    let mut blobs = store.blob_store(&group).unwrap();
    let payload = pix();
    let pixel_bytes = payload.len() as u64;
    let cid = blobs.put(&payload).unwrap();
    let orphan = blobs.put(b"unreferenced throughout").unwrap();

    // A known pin set that predates the acceptance and excludes the new CID, so nothing below
    // can pass through fail-closed unknown protection.
    store.creative_pinned_cids().unwrap();
    assert!(store.creative_references_known());

    let op = f.domain(
        FlipnoteOp::InsertFrame {
            frame: [9; 16],
            after: None,
            cid: *cid.as_bytes(),
            bytes: pixel_bytes,
        }
        .encode()
        .unwrap(),
        7,
    );
    let authoring = store
        .admit_studio_overlay_authoring(f.target, &f.logical, &f.device, op)
        .unwrap();
    let capture = store
        .capture_studio_overlay_save(SERVER, &f.group, f.target, &f.device, basis, authoring, 300)
        .unwrap();

    // The detached stage. Custody is genuinely released here: `plan` owns authenticated plaintext
    // and the hold, and touches no store. While it is outstanding, another operation completes a
    // full reference scan and attempts protected deletion.
    let plan = capture.plan().unwrap();
    store.creative_pinned_cids().unwrap();
    let mut blobs = store.blob_store(&group).unwrap();
    assert!(
        !blobs.delete(&cid).unwrap(),
        "a complete scan reclaimed pixels held by a detached acceptance"
    );
    assert!(blobs.get_bounded(&cid, 100_000).unwrap().is_some());
    // The scan is genuinely complete and usable: an unreferenced CID still reclaims.
    assert!(store.creative_references_known());
    assert!(blobs.delete(&orphan).unwrap());

    // Commit under reacquired custody. The transfer runs before the write and the hold is
    // released only when this returns.
    let mut b = budget(&mut store, &f);
    let draft = store
        .commit_studio_overlay_save(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &close,
            Some(0),
            plan,
            &mut rng(),
            &mut b,
            |m: &EpochMutation<'_>, p: &Path, b: &[u8]| m.write(p, b),
            |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_intent(p, b),
        )
        .unwrap();
    assert_eq!(draft.accepted(), 1);
    assert_eq!(store.live_transient_holds_for_test(), 0);

    // After the commit, with every owner gone and before any further scan, the ordinary holds
    // installed during the transfer are what keep the pixels.
    let mut blobs = store.blob_store(&group).unwrap();
    assert!(
        !blobs.delete(&cid).unwrap(),
        "the committed acceptance left its pixels reclaimable"
    );
    assert!(blobs.get_bounded(&cid, 100_000).unwrap().is_some());
}

/// The staged commit must refuse a plan whose record changed while it was detached, rather than
/// writing a state derived from bytes that are no longer current.
#[test]
fn studio_overlay_detached_plan_is_refused_when_the_record_changed() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(false);
    let (close, basis) = closing(&f, &mut store);
    let basis_fingerprint = basis.fingerprint();
    let authoring = store
        .admit_studio_overlay_authoring(f.target, &f.logical, &f.device, f.title())
        .unwrap();
    let capture = store
        .capture_studio_overlay_save(SERVER, &f.group, f.target, &f.device, basis, authoring, 300)
        .unwrap();
    let plan = capture.plan().unwrap();

    // A different acceptance lands while the first plan is detached.
    let other = f.domain(
        IndexOp::SetTitle {
            object: [1; 16],
            title: "a different accepted title".into(),
        }
        .encode()
        .unwrap(),
        8,
    );
    let landed = save(&f, &mut store, &close, basis_fingerprint, other, 301);
    assert_eq!(landed.accepted(), 1);
    let records = canonical(&store);

    let mut b = budget(&mut store, &f);
    let refused = store.commit_studio_overlay_save(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        &close,
        Some(0),
        plan,
        &mut rng(),
        &mut b,
        |m: &EpochMutation<'_>, p: &Path, b: &[u8]| m.write(p, b),
        |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_intent(p, b),
    );
    assert!(
        refused.is_err(),
        "a plan derived from superseded bytes was committed"
    );
    assert_eq!(canonical(&store), records, "the refusal changed records");
    assert_eq!(
        store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .local_draft()
            .unwrap()
            .unwrap()
            .accepted(),
        1,
        "the refusal disturbed the accepted branch"
    );
}

/// FS-001 / N12(d). A new acceptance may not name pixels the vault does not hold. The reference
/// extractor only reads an address and the typed layer only checks declared sizes, so possession
/// is a separate obligation: once at S1b, and again at S3 because the bytes can disappear while
/// the append is detached.
#[test]
fn studio_overlay_new_acceptance_requires_pixels_at_admission_and_again_before_the_barrier() {
    // A CID that was never published must be refused before anything is accepted.
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let (close, basis) = closing(&f, &mut store);
    let records = canonical(&store);
    let absent = f.domain(
        FlipnoteOp::InsertFrame {
            frame: [9; 16],
            after: None,
            cid: [0xee; 32],
            bytes: 39,
        }
        .encode()
        .unwrap(),
        7,
    );
    let mut b = budget(&mut store, &f);
    let refused = store.save_studio_closing_overlay(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        &close,
        Some(0),
        basis.fingerprint(),
        absent,
        300,
        &mut rng(),
        &mut b,
    );
    assert!(
        refused.is_err(),
        "an operation naming absent pixels was accepted"
    );
    assert_eq!(canonical(&store), records, "the refusal changed records");
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .local_draft()
        .unwrap()
        .is_none());

    // Now the S3 case: admission succeeds, then the bytes are removed while the append is
    // detached. Every stamp and basis check still passes, so the refusal must come from the
    // possession recheck and not from an earlier stale-plan guard.
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let (close, basis) = closing(&f, &mut store);
    let group = hex::encode(f.group.group_id());
    let mut blobs = store.blob_store(&group).unwrap();
    let cid = blobs.put(&pix()).unwrap();
    let records = canonical(&store);

    let op = f.domain(
        FlipnoteOp::InsertFrame {
            frame: [9; 16],
            after: None,
            cid: *cid.as_bytes(),
            bytes: pix().len() as u64,
        }
        .encode()
        .unwrap(),
        7,
    );
    let authoring = store
        .admit_studio_overlay_authoring(f.target, &f.logical, &f.device, op)
        .unwrap();
    let capture = store
        .capture_studio_overlay_save(SERVER, &f.group, f.target, &f.device, basis, authoring, 300)
        .unwrap();
    let plan = capture.plan().unwrap();

    // Remove the bytes underneath the detached job, as external deletion or storage damage would.
    let path = store.dir.join("blobs").join(&group);
    for entry in fs::read_dir(&path).unwrap().flatten() {
        if entry.path().is_file() {
            fs::remove_file(entry.path()).unwrap();
        }
    }
    assert!(store
        .blob_store(&group)
        .unwrap()
        .get_bounded(&cid, 100_000)
        .unwrap()
        .is_none());

    let mut b = budget(&mut store, &f);
    let refused = store.commit_studio_overlay_save(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        &close,
        Some(0),
        plan,
        &mut rng(),
        &mut b,
        |m: &EpochMutation<'_>, p: &Path, b: &[u8]| m.write(p, b),
        |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_intent(p, b),
    );
    match refused {
        Err(error) => assert!(
            error.to_string().contains("no longer held"),
            "a new acceptance named absent pixels: {error}"
        ),
        Ok(_) => panic!("a new acceptance named absent pixels"),
    }
    assert_eq!(canonical(&store), records, "the refusal changed records");
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .local_draft()
        .unwrap()
        .is_none());
}

/// A-001. The media facts and the operation that consumes them must be one value. Pairing media
/// admitted for operation A with an intent carrying operation B would make the detached plan append
/// B while S3 rechecked, and the job-owned hold protected, A's pixels: a durable acceptance naming
/// pixels nothing verified, and unprotected pixels for the ones it does name.
///
/// Production cannot express that pairing at all, because `AdmittedOverlayAuthoring` has private
/// fields and `admit_studio_overlay_authoring` is its only constructor. That is a type-level fact
/// and nothing can execute it, so the binding is also rechecked at capture and at commit, and this
/// test forces the mismatch through a test-only constructor to prove those rechecks fire.
#[test]
fn admitted_media_cannot_be_paired_with_another_operation() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open(root.path());
    let f = Fixture::new(true);
    let (close, basis) = closing(&f, &mut store);
    let (first_cid, first_bytes) = published_pix(&store, &f, 11);
    let (second_cid, second_bytes) = published_pix(&store, &f, 12);
    assert_ne!(first_cid, second_cid, "the fixture frames share pixels");
    // A second basis for the positive control, derived from the same unchanged Closing source.
    let control_basis = {
        let mut b = budget(&mut store, &f);
        store
            .prepare_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                &mut b,
            )
            .unwrap()
    };
    assert_eq!(control_basis.fingerprint(), basis.fingerprint());
    let records = canonical(&store);

    let frame = |frame: [u8; 16], cid: [u8; 32], bytes: u64, nonce: u8| {
        f.domain(
            FlipnoteOp::InsertFrame {
                frame,
                after: None,
                cid,
                bytes,
            }
            .encode()
            .unwrap(),
            nonce,
        )
    };
    // Two legitimately admitted requests, each naming its own published pixels.
    let a = store
        .admit_studio_overlay_authoring(
            f.target,
            &f.logical,
            &f.device,
            frame([9; 16], first_cid, first_bytes, 7),
        )
        .unwrap();
    let b = store
        .admit_studio_overlay_authoring(
            f.target,
            &f.logical,
            &f.device,
            frame([10; 16], second_cid, second_bytes, 8),
        )
        .unwrap();
    let live = store.live_transient_holds_for_test();
    assert_eq!(live, 2, "each admitted request takes its own hold");

    // B's intent with A's media. Capture must refuse before any custody is released, so no plan
    // and no detached stage ever exists for the mismatched pair.
    let intent = catcoms_replication::LocalIntent {
        author: f.device.device_id(),
        operation: frame([10; 16], second_cid, second_bytes, 8),
    };
    let swapped =
        crate::store::epoch_studio::overlay_capture::AdmittedOverlayAuthoring::mismatched_for_test(
            intent, a,
        );
    let refused = store
        .capture_studio_overlay_save(SERVER, &f.group, f.target, &f.device, basis, swapped, 300);
    match refused {
        Err(error) => assert!(
            error
                .to_string()
                .contains("admitted media does not belong to this authoring request"),
            "the mismatch was refused for an unrelated reason: {error}"
        ),
        Ok(_) => panic!("media admitted for one operation was captured against another"),
    }
    assert_eq!(
        canonical(&store),
        records,
        "a refused capture changed durable records"
    );

    // Positive control: the same capture with B's own admitted authoring is accepted and reaches a
    // plan, so the refusal above is the binding and not the fixture.
    let capture = store
        .capture_studio_overlay_save(SERVER, &f.group, f.target, &f.device, control_basis, b, 300)
        .expect("a correctly paired request was refused");
    let plan = capture.plan().unwrap();
    let mut budget = budget(&mut store, &f);
    let draft = store
        .commit_studio_overlay_save(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &close,
            Some(0),
            plan,
            &mut rng(),
            &mut budget,
            |m: &EpochMutation<'_>, p: &Path, b: &[u8]| m.write(p, b),
            |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_intent(p, b),
        )
        .unwrap();
    assert_eq!(draft.accepted(), 1);
    // A's hold died with the refused capture; B's died with its commit.
    assert_eq!(store.live_transient_holds_for_test(), 0);
}

/// Every file this group's blob namespace holds, staging included, by path and exact bytes. Used
/// to prove a refused request neither promoted anything nor left a staged sibling behind.
fn blob_namespace(store: &ServerStore, f: &Fixture) -> BTreeMap<String, Vec<u8>> {
    let root = store
        .dir
        .join("blobs")
        .join(hex::encode(f.group.group_id()));
    let mut out = BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let name = path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(name, fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// FS-002. Classification answers "has this already been accepted"; it does not answer "may this
/// be authored now". A request carrying a basis the document has legitimately moved past is
/// neither an accepted retry nor authorized, so it must be refused as stale before any pixel is
/// read, promoted or held and before the shared reference rails are consulted. Otherwise a stale
/// editor is told its artwork is missing, or a request destined for refusal promotes a blob and
/// takes a rail slot on its way out.
///
/// Both hazards the reviewer named are exercised against the same advanced state, each with a
/// positive control proving media admission was genuinely reachable and would have refused.
#[test]
fn studio_overlay_stale_basis_is_refused_before_any_media_admission() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    eligible(&f, &mut store);

    // A separate sender's valid signed edit, withheld until the receiver has sealed. Ingesting it
    // afterwards advances the persisted Closing source, which is what makes the first basis stale:
    // no gate field, snapshot or stamp is edited by the fixture.
    let mut sender = f.load(&store).unwrap();
    let mut late = f.title();
    late.nonce = [76; 16];
    let packet = sender
        .unit
        .edit_or_reseal(&f.device, &f.group, &mut rng(), &late, 100)
        .unwrap();
    let (close, stale) = seal_source(&f, &mut store);

    warm(&f, &mut store);
    let mut b = budget(&mut store, &f);
    let (admitted, _) = store
        .ingest_studio_epoch_reusing(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &packet,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(admitted, Admission::Quarantined);
    drop(store);

    // Reopen so a cached source cannot stand in for the changed persisted one.
    let mut store = open(root.path());
    let mut b = budget(&mut store, &f);
    let fresh = store
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
    assert_ne!(
        fresh.fingerprint(),
        stale.fingerprint(),
        "the Closing source did not actually advance"
    );
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let intent_path = store
        .dir
        .join("servers")
        .join(format!("{}.intents", blake3::hash(&scope).to_hex()));
    let intents_before = fs::read(&intent_path).ok();

    let attempt = |store: &mut ServerStore, basis: [u8; 32], op: DomainOp| {
        let mut b = budget(store, &f);
        store
            .save_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                Some(0),
                basis,
                op,
                456,
                &mut rng(),
                &mut b,
            )
            .map(|_| ())
            .unwrap_err()
            .to_string()
    };

    // Hazard 1: the pixels this new frame names were never published. A stale request must not
    // reach the possession check at all, so the editor learns its basis moved rather than being
    // told, wrongly, that its artwork is missing.
    let absent = f.domain(
        FlipnoteOp::InsertFrame {
            frame: [9; 16],
            after: None,
            cid: [0xAB; 32],
            bytes: pix().len() as u64,
        }
        .encode()
        .unwrap(),
        7,
    );
    let live = store.live_transient_holds_for_test();
    let blobs_before = blob_namespace(&store, &f);
    assert_eq!(
        attempt(&mut store, stale.fingerprint(), absent.clone()),
        invalid("Closing overlay basis changed").to_string(),
        "a stale request was classified by its media instead of its basis"
    );
    assert_eq!(
        store.live_transient_holds_for_test(),
        live,
        "a stale request took a job-owned hold"
    );
    assert_eq!(
        blob_namespace(&store, &f),
        blobs_before,
        "a stale request changed the blob namespace"
    );
    assert_eq!(fs::read(&intent_path).ok(), intents_before);
    // Positive control: with the current basis this identical request does reach media admission,
    // and is refused there. The stale refusal above was ordering, not an inert request.
    assert!(
        attempt(&mut store, fresh.fingerprint(), absent).contains("publish the frame PIX"),
        "the absent-pixel hazard was not reachable, so the stale case proves nothing"
    );

    // Hazard 2: the pixels exist, but every job-owned hold slot is legitimately occupied. A stale
    // request must not consume rail capacity, nor be refused for a rail it had no business
    // consulting.
    let (cid, bytes) = published_pix(&store, &f, 3);
    let held = f.domain(
        FlipnoteOp::InsertFrame {
            frame: [10; 16],
            after: None,
            cid,
            bytes,
        }
        .encode()
        .unwrap(),
        8,
    );
    let occupied: Vec<_> = (0..crate::store::creative_references::MAX_TRANSIENT_HOLD_OWNERS)
        .map(|n| {
            store
                .hold_creative_transient(&f.group.group_id(), BTreeSet::from([[n as u8; 32]]))
                .expect("the rail admits its stated number of owners")
        })
        .collect();
    let live = store.live_transient_holds_for_test();
    let blobs_before = blob_namespace(&store, &f);
    assert_eq!(
        attempt(&mut store, stale.fingerprint(), held.clone()),
        invalid("Closing overlay basis changed").to_string(),
        "a stale request consulted the reference rails before its basis"
    );
    assert_eq!(
        store.live_transient_holds_for_test(),
        live,
        "a stale request disturbed the job-owned hold rail"
    );
    assert_eq!(
        blob_namespace(&store, &f),
        blobs_before,
        "a stale request promoted a blob on its way to refusal"
    );
    assert_eq!(fs::read(&intent_path).ok(), intents_before);
    // Positive control: the saturated rail does refuse this request once its basis is current.
    assert_eq!(
        attempt(&mut store, fresh.fingerprint(), held),
        AppError::Invalid("creative reference scan incomplete, unsupported or over bound".into())
            .to_string(),
        "the saturated-rail hazard was not reachable, so the stale case proves nothing"
    );
    drop(occupied);

    // Nothing durable was accepted by any of the four attempts.
    assert_eq!(fs::read(&intent_path).ok(), intents_before);
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .overlay()
        .is_none());
}

/// A valid PIX payload at the 192x144 the Flipnote frame rules require: a four-entry palette and
/// 108 maximal runs, which is exactly 27,648 pixels. Consecutive equal indices are legal here
/// because a maximal run clears the non-maximal-run rule.
fn pix() -> Vec<u8> {
    let mut bytes = vec![0x50, 0x49, 0x58, 0x31, 191, 143, 3];
    for entry in [
        [1, 0x13, 0x12, 0x18],
        [2, 0xe8, 0xe6, 0xf0],
        [3, 0x97, 0x7d, 0xf2],
        [0, 0xe0, 0x7a, 0xb8],
    ] {
        bytes.extend(entry);
    }
    for _ in 0..108 {
        bytes.extend([255, 0]);
    }
    crate::creative::validate_pix(&bytes).expect("the fixture must be a valid PIX");
    bytes
}
