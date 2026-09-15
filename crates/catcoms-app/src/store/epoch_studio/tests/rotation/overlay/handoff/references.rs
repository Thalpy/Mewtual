use super::*;
use crate::store::epoch_intents::{self, EpochIntentState};
use catcoms_replication::studio::StudioOverlayState;
use catcoms_replication::IntentLedger;

const MISSING: &str = "reference scan required handoff metadata missing or mismatched";

#[test]
fn studio_overlay_handoff_reference_scan_keeps_overlay_only_pixels_when_metadata_is_missing() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let group = hex::encode(f.group.group_id());
    let mut pix = crate::creative::tests::golden()[..23].to_vec();
    pix[4] = 191;
    pix[5] = 143;
    pix.extend((0..108).flat_map(|_| [255, 0]));
    crate::creative::validate_pix(&pix).unwrap();
    let mut blobs = store.blob_store(&group).unwrap();
    assert!(blobs.is_persistent());
    let cid = blobs.put(&pix).unwrap();
    let unreferenced = blobs.put(b"deletable control").unwrap();
    let (close, basis) = closing(&f, &mut store);
    let op = f.domain(
        FlipnoteOp::InsertFrame {
            frame: [44; 16],
            after: Some([1; 16]),
            cid: *cid.as_bytes(),
            bytes: pix.len() as u64,
        }
        .encode()
        .unwrap(),
        44,
    );
    let accepted = save(&f, &mut store, &close, basis.fingerprint(), op.clone(), 123);
    assert_eq!(accepted.accepted(), 1);
    install(&f, &mut store, &close);
    fences::interrupt(&f, &mut store, basis.fingerprint(), HandoffWrite::Source);

    // The operation is accepted locally but has not entered any canonical signed source.
    let source = f.load(&store).unwrap();
    assert_eq!((source.epoch(), source.op_count()), (1, 0));
    assert!(!source.unit.blob_cids().unwrap().contains(cid.as_bytes()));
    let scope = scope_bytes(SERVER, &f.logical).unwrap();
    let actual = store.read_studio_record(&scope).unwrap().unwrap();
    let (target, _, linked) = decode_record_link(&actual.plain, &scope, &f.logical).unwrap();
    assert_eq!(target, f.target);
    assert!(linked, "fixture must contain the durable dependency");
    assert!(canonical(&store).keys().all(|p| !p.ends_with(".recovery")));
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert!(state.handoff_prepared());
    let overlay = state.overlay().unwrap();
    assert!(!overlay.base_blob_cids().unwrap().contains(cid.as_bytes()));
    let id = op.id(&f.device.device_id());
    assert!(state.is_overlay(&id));
    assert_eq!(
        state
            .pending()
            .find(|(i, _)| **i == id)
            .unwrap()
            .1
            .operation,
        op
    );
    assert_eq!(
        state
            .pending()
            .filter(|(_, intent)| {
                catcoms_replication::studio::operation_blob_cid(&intent.operation).unwrap()
                    == Some(*cid.as_bytes())
            })
            .count(),
        1
    );
    let refs = store.creative_pinned_cids().unwrap();
    assert!(refs.for_group(&f.group.group_id()).any(|c| c == &cid));
    assert!(store.creative_references_known());
    assert!(!blobs.delete(&cid).unwrap());
    assert!(
        blobs.delete(&unreferenced).unwrap(),
        "healthy scans permit real deletion"
    );

    let path = store.epoch_intent_path(&epoch_intents::scope_bytes(SERVER, &f.logical).unwrap());
    let saved = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    drop(blobs);
    drop(store);
    let mut store = open(root.path());
    let mut blobs = store.blob_store(&group).unwrap();
    assert!(!store.creative_references_known());
    // Do not call the source-read fence first. Inventory alone must keep deletion disabled.
    let scan = store.creative_pinned_cids();
    let known = store.creative_references_known();
    let deletion = blobs.delete(&cid);
    let retained = blobs.get_bounded(&cid, pix.len()).unwrap();
    assert!(
        matches!(scan, Err(AppError::Invalid(ref s)) if s.contains(MISSING)),
        "missing handoff metadata completed a reference scan: {scan:?}; known={known}, deletion={deletion:?}, retained={}",
        retained.is_some()
    );
    assert!(!known);
    assert!(deletion.is_err());
    assert_eq!(retained.as_deref(), Some(pix.as_slice()));

    fs::write(&path, &saved).unwrap();
    let refs = store.creative_pinned_cids().unwrap();
    assert!(refs.for_group(&f.group.group_id()).any(|c| c == &cid));
    assert!(store.creative_references_known());
    assert!(!blobs.delete(&cid).unwrap());
    assert_eq!(blobs.get_bounded(&cid, pix.len()).unwrap(), Some(pix));
    assert_eq!(fs::read(path).unwrap(), saved);
}

/// Valid compact metadata isolates the inventory relationship from seed/receipt parsing.
/// These are test substitutions in authenticated local files, not handoff transitions.
fn compact(group: &[u8], target: StudioTarget) -> EpochIntentState {
    let ledger = IntentLedger::new(target.document(group).unwrap());
    let mut e = Encoder::new();
    e.put_u8(2);
    e.put_u8(if matches!(target, StudioTarget::Index { .. }) {
        0
    } else {
        1
    });
    e.put_bytes(&target.channel()).unwrap();
    if let StudioTarget::Flipnote { object, .. } = target {
        e.put_bytes(&object).unwrap();
    }
    e.put_u64(0);
    e.put_u8(0);
    e.put_u8(0);
    let overlay = Some(StudioOverlayState::decode_vault(&e.finish(), &ledger).unwrap());
    EpochIntentState { ledger, overlay }
}

#[test]
fn studio_overlay_handoff_reference_scan_requires_metadata_with_the_complete_scope_and_target() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    fences::interrupt(&f, &mut store, basis, HandoffWrite::Source);
    let original = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let path = store.epoch_intent_path(&epoch_intents::scope_bytes(SERVER, &f.logical).unwrap());
    let saved = fs::read(&path).unwrap();
    let mut ordinary = original.clone();
    ordinary.overlay = None;
    let mismatches = [
        ("ordinary ledger", SERVER, ordinary),
        ("numeric server", SERVER + 1, original),
        ("group", SERVER, compact(b"different group", f.target)),
        (
            "logical key",
            SERVER,
            compact(
                &f.group.group_id(),
                StudioTarget::Flipnote {
                    channel: f.target.channel(),
                    object: [55; 16],
                },
            ),
        ),
        (
            "document type",
            SERVER,
            compact(
                &f.group.group_id(),
                StudioTarget::Index { channel: [9; 16] },
            ),
        ),
        (
            "channel",
            SERVER,
            compact(
                &f.group.group_id(),
                StudioTarget::Flipnote {
                    channel: [55; 16],
                    object: [9; 16],
                },
            ),
        ),
    ];
    for (label, server, state) in mismatches {
        let document = state.ledger.document();
        let scope = epoch_intents::scope_bytes(server, document).unwrap();
        let substitute = store.epoch_intent_path(&scope);
        let plain = state.encode(&scope).unwrap();
        let sealed = seal(&store.keys.db_key().unwrap(), &plain, &mut rng()).unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(&substitute, frame(&sealed)).unwrap();
        drop(store);
        store = open(root.path());
        // Prove each negative supplies an independently authenticated, decodable record.
        let decoded = store.load_epoch_intents(server, document).unwrap();
        assert_eq!(decoded.encode(&scope).unwrap(), plain);
        let scan = store.creative_pinned_cids();
        assert!(
            matches!(scan, Err(AppError::Invalid(ref s)) if s.contains(MISSING)),
            "reference scan accepted mismatched handoff {label}: {scan:?}"
        );
        assert!(!store.creative_references_known(), "{label}");
        fs::remove_file(substitute).unwrap();
        fs::write(&path, &saved).unwrap();
        assert!(store.creative_pinned_cids().is_ok(), "restored {label}");
        assert!(store.creative_references_known());
    }
}
