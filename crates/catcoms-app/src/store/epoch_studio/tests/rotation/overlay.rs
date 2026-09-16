use super::*;
use crate::store::epoch_intents::sync_intent;
use catcoms_replication::studio::{StudioClosingOverlayBasis, StudioLocalDraft};
use catcoms_replication::CloseRecord;
use std::collections::{BTreeMap, BTreeSet};

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
            atomic_write,
            sync_intent,
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
            atomic_write,
            sync_studio,
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
    let bodies = [
        FlipnoteOp::InsertFrame {
            frame: [2; 16],
            after: Some([1; 16]),
            cid: [4; 32],
            bytes: 12,
        },
        FlipnoteOp::ReplaceFrame {
            frame: [2; 16],
            cid: [5; 32],
            bytes: 13,
        },
        FlipnoteOp::InsertFrame {
            frame: [6; 16],
            after: Some([2; 16]),
            cid: [6; 32],
            bytes: 14,
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
    for cid in [[3; 32], [4; 32], [5; 32], [6; 32]] {
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
    for cid in [[3; 32], [4; 32], [5; 32], [6; 32]] {
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
                |_, _| Err(AppError::Io("before overlay write".into())),
                sync_intent,
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
                |p, bytes| {
                    atomic_write(p, bytes)?;
                    Err(AppError::Io("after overlay rename".into()))
                },
                sync_intent,
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
                |_, _| panic!("exact retry allocated replacement"),
                |_, _| Err(AppError::Io("retry sync failed".into())),
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
                |_, _| panic!("exact retry allocated replacement"),
                sync_intent,
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
                atomic_write,
                sync_intent,
                |_, _| Err(AppError::Io("ordinary source save failed".into())),
                sync_studio,
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
    let cid = blobs
        .put(b"pixels named only by the new operation")
        .unwrap();
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
            bytes: 39,
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
    assert!(blobs.get_bounded(&cid, 200).unwrap().is_some());
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
        let cid = blobs.put(b"pixels for an uncertain write").unwrap();
        store.creative_pinned_cids().unwrap();
        assert!(store.creative_references_known());

        let op = f.domain(
            FlipnoteOp::InsertFrame {
                frame: [9; 16],
                after: None,
                cid: *cid.as_bytes(),
                bytes: 39,
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
            |path, bytes| {
                if after_rename {
                    // The record really lands; only the caller's result is lost.
                    atomic_write(path, bytes)?;
                } else {
                    // A failure that leaves a temporary sibling behind.
                    let mut staged = path.to_path_buf();
                    staged.set_extension("intents.mewtual-stage-1-1.tmp");
                    std::fs::write(&staged, bytes).unwrap();
                }
                Err(crate::AppError::Io("interrupted".into()))
            },
            sync_intent,
        );
        assert!(failed.is_err(), "the injected writer must fail the save");

        let mut blobs = store.blob_store(&group).unwrap();
        assert!(
            !blobs.delete(&cid).unwrap(),
            "an uncertain acceptance left its pixels reclaimable"
        );
        assert!(blobs.get_bounded(&cid, 200).unwrap().is_some());
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
    let cid = blobs.put(b"pixels for the accepted operation").unwrap();

    let op = f.domain(
        FlipnoteOp::InsertFrame {
            frame: [9; 16],
            after: None,
            cid: *cid.as_bytes(),
            bytes: 39,
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
