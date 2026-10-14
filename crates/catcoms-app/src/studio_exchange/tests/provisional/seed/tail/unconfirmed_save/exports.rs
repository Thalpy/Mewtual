//! The same held-byte barrier applies to an export in a real Unconfirmed draft. A ready preview
//! and a valid ticket grant local authoring, not evidence that the mounted vault has any media.
use super::*;
use crate::store::{StudioOverlayCapture, StudioOverlayPlan};

fn package() -> Vec<u8> {
    let vector: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catcoms-replication/tests/fixtures/studio-export-v1.json"
    )))
    .unwrap();
    let bytes = hex::decode(vector["pixaHex"].as_str().unwrap()).unwrap();
    crate::creative::validate_pixa(&bytes).unwrap();
    bytes
}

fn export(target: StudioTarget, cid: crate::Cid, bytes: u64, nonce: u8) -> DomainOp {
    domain(
        target,
        FlipnoteOp::SetExport {
            export: [77; 16],
            cid: *cid.as_bytes(),
            bytes,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
        nonce,
    )
}

fn publish(p: &Pair, bytes: &[u8]) -> crate::Cid {
    p.b_store
        .blob_store(&hex::encode(p.bob.group_id()))
        .unwrap()
        .put(bytes)
        .unwrap()
}

fn remove_externally(p: &Pair, cid: crate::Cid) {
    // Deliberately bypass application reclamation, as external filesystem damage can. This is
    // the exact temporary fixture's CID file, not a directory-wide deletion.
    let path = p
        .b_root
        .path()
        .join("blobs")
        .join(hex::encode(p.bob.group_id()))
        .join(cid.to_hex());
    std::fs::remove_file(path).unwrap();
}

fn capture(
    p: &mut Pair,
    prepared: &ServerPreparedProvisionalStudioSeed,
    ticket: ([u8; 32], [u8; 32]),
    operation: DomainOp,
) -> StudioOverlayCapture {
    let target = p.watch.target;
    let start = mint(p, prepared);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let store = &mut p.b_store;
    let start = p
        .bob
        .sync
        .with_registry_context(|g, d, clock, rng| {
            store.start_studio_overlay(
                SERVER,
                g,
                target,
                d,
                StudioOverlayMint::unconfirmed(start),
                ticket.0,
                ticket.1,
                operation,
                clock.now_ms(),
                rng,
                &mut b,
            )
        })
        .unwrap();
    let StudioOverlayStart::Captured(capture) = start else {
        panic!("new export must reach detached authoring")
    };
    *capture
}

fn commit(
    p: &mut Pair,
    prepared: &ServerPreparedProvisionalStudioSeed,
    plan: StudioOverlayPlan,
) -> Result<StudioLocalDraft, AppError> {
    let target = p.watch.target;
    let fresh = mint(p, prepared);
    let mut b = budget(&mut p.bob, &mut p.b_store);
    let store = &mut p.b_store;
    p.bob.sync.with_registry_context(|g, d, _, rng| {
        store.commit_studio_overlay_with(
            SERVER,
            g,
            target,
            d,
            StudioOverlayMint::unconfirmed(fresh),
            plan,
            rng,
            &mut b,
        )
    })
}

#[tokio::test]
async fn unconfirmed_export_requires_exact_valid_held_bytes_and_retries_without_media() {
    let target = target();
    let mut p = preview_pair(target).await;
    let prepared = complete_preview(&mut p).await;
    let basis = mint(&p, &prepared).unwrap();
    let ticket = ticket(&mut p, target, &basis);
    let bytes = package();
    let cid = crate::Cid::of(&bytes);
    let operation = export(target, cid, bytes.len() as u64, 77);
    let error = save(&mut p, target, &prepared, ticket, operation.clone()).unwrap_err();
    assert!(
        refusal(error, StudioOverlaySaveRefusal::ExportUnpublished).contains("publish the PIXA")
    );
    assert_eq!(recorded(&mut p, target), None);
    assert_eq!(p.b_store.live_transient_holds_for_test(), 0);
    assert_eq!(publish(&p, &bytes), cid);
    for size in [bytes.len() as u64 - 1, bytes.len() as u64 + 1] {
        let error = save(
            &mut p,
            target,
            &prepared,
            ticket,
            export(target, cid, size, 77),
        )
        .unwrap_err();
        assert!(
            refusal(error, StudioOverlaySaveRefusal::ExportInvalid).contains("byte declaration")
        );
        assert_eq!(recorded(&mut p, target), None);
    }
    let invalid = b"PIXA-not-a-valid-package";
    let bad = publish(&p, invalid);
    let error = save(
        &mut p,
        target,
        &prepared,
        ticket,
        export(target, bad, invalid.len() as u64, 77),
    )
    .unwrap_err();
    assert!(refusal(error, StudioOverlaySaveRefusal::ExportInvalid).contains("invalid PIXA"));
    assert_eq!(recorded(&mut p, target), None);

    let draft = local(save(&mut p, target, &prepared, ticket, operation.clone()).unwrap());
    assert_eq!(draft.accepted(), 1);
    let StudioProjection::Flipnote(view) = draft.projection() else {
        panic!()
    };
    assert_eq!(view.sound.exports().len(), 1);
    assert!(matches!(
        recorded(&mut p, target).unwrap().0,
        Some(StudioOverlayProvenance::Unconfirmed { .. })
    ));
    assert!(
        !installed(&mut p, target),
        "acceptance installed an unconfirmed source"
    );
    assert_eq!(p.b_store.live_transient_holds_for_test(), 0);
    let mut blobs = p
        .b_store
        .blob_store(&hex::encode(p.bob.group_id()))
        .unwrap();
    p.b_store.creative_pinned_cids().unwrap();
    assert!(
        !blobs.delete(&cid).unwrap(),
        "accepted export lost conservative protection"
    );
    remove_externally(&p, cid);
    assert_eq!(
        acknowledged(
            save_with(
                &mut p,
                target,
                no_preview("preview gone"),
                ticket,
                operation
            )
            .unwrap()
        ),
        (ticket.0, 1),
        "an accepted retry must not re-author, read media or require a new preview"
    );
    assert_eq!(p.b_store.live_transient_holds_for_test(), 0);
    for reason in [
        StudioOverlaySaveRefusal::ExportUnpublished,
        StudioOverlaySaveRefusal::ExportInvalid,
        StudioOverlaySaveRefusal::ExportMissing,
    ] {
        assert_eq!(
            reason.action(),
            catcoms_replication::studio::StudioOverlaySaveAction::NewTicket
        );
    }
}

#[tokio::test]
async fn unconfirmed_export_s3_missing_refuses_before_intent_and_releases_its_hold() {
    let target = target();
    let mut p = preview_pair(target).await;
    let prepared = complete_preview(&mut p).await;
    let basis = mint(&p, &prepared).unwrap();
    let ticket = ticket(&mut p, target, &basis);
    let bytes = package();
    let cid = publish(&p, &bytes);
    let captured = capture(
        &mut p,
        &prepared,
        ticket,
        export(target, cid, bytes.len() as u64, 77),
    );
    assert_eq!(p.b_store.live_transient_holds_for_test(), 1);
    let plan = captured.plan().unwrap();
    remove_externally(&p, cid);
    let error = commit(&mut p, &prepared, plan).unwrap_err();
    assert!(refusal(error, StudioOverlaySaveRefusal::ExportMissing).contains("no longer held"));
    assert_eq!(
        recorded(&mut p, target),
        None,
        "missing media became a durable reference"
    );
    assert_eq!(p.b_store.live_transient_holds_for_test(), 0);
}

#[tokio::test]
async fn unconfirmed_export_detached_scan_and_cancellation_preserve_exact_hold_lifetime() {
    for cancel_after_plan in [false, true] {
        let target = target();
        let mut p = preview_pair(target).await;
        let prepared = complete_preview(&mut p).await;
        let basis = mint(&p, &prepared).unwrap();
        let ticket = ticket(&mut p, target, &basis);
        let bytes = package();
        let cid = publish(&p, &bytes);
        let orphan = publish(&p, b"never referenced");
        p.b_store.creative_pinned_cids().unwrap();
        assert!(p.b_store.creative_references_known());
        let captured = capture(
            &mut p,
            &prepared,
            ticket,
            export(target, cid, bytes.len() as u64, 77),
        );
        let owner: Box<dyn std::any::Any> = if cancel_after_plan {
            Box::new(captured.plan().unwrap())
        } else {
            Box::new(captured)
        };
        assert_eq!(p.b_store.live_transient_holds_for_test(), 1);
        // A complete concurrent scan cannot see the new reference in durable state yet.
        p.b_store.creative_pinned_cids().unwrap();
        let mut blobs = p
            .b_store
            .blob_store(&hex::encode(p.bob.group_id()))
            .unwrap();
        assert!(!blobs.delete(&cid).unwrap(), "scan reclaimed detached PIXA");
        assert!(
            blobs.delete(&orphan).unwrap(),
            "fail-closed unknown pins masked the test"
        );
        drop(owner);
        assert_eq!(p.b_store.live_transient_holds_for_test(), 0);
        assert!(
            blobs.delete(&cid).unwrap(),
            "cancelled job stranded its transient export hold"
        );
        assert_eq!(recorded(&mut p, target), None);
    }
}

#[tokio::test]
async fn unconfirmed_export_stale_detached_plan_releases_hold_without_accepting_reference() {
    let target = target();
    let mut p = preview_pair(target).await;
    let prepared = complete_preview(&mut p).await;
    let basis = mint(&p, &prepared).unwrap();
    let ticket = ticket(&mut p, target, &basis);
    let bytes = package();
    let cid = publish(&p, &bytes);
    p.b_store.creative_pinned_cids().unwrap();
    let plan = capture(
        &mut p,
        &prepared,
        ticket,
        export(target, cid, bytes.len() as u64, 77),
    )
    .plan()
    .unwrap();
    let other = draft_op(&p, target, 78, "another request won the intent stamp");
    local(save(&mut p, target, &prepared, ticket, other).unwrap());
    let before = recorded(&mut p, target);
    let error = commit(&mut p, &prepared, plan).unwrap_err();
    assert!(refusal(error, StudioOverlaySaveRefusal::Retry).contains("record or context changed"));
    assert_eq!(recorded(&mut p, target), before);
    assert_eq!(p.b_store.live_transient_holds_for_test(), 0);
    let mut blobs = p
        .b_store
        .blob_store(&hex::encode(p.bob.group_id()))
        .unwrap();
    assert!(
        blobs.delete(&cid).unwrap(),
        "stale export plan leaked a hold"
    );
    let logical = target.document(&p.bob.group_id()).unwrap();
    let state = p.b_store.load_epoch_intents(SERVER, &logical).unwrap();
    let draft = state.local_draft().unwrap().unwrap();
    let StudioProjection::Flipnote(view) = draft.projection() else {
        panic!()
    };
    assert!(view.sound.exports().is_empty());
}

#[tokio::test]
async fn unconfirmed_export_scan_between_capture_and_commit_transfers_hold_before_write() {
    let target = target();
    let mut p = preview_pair(target).await;
    let prepared = complete_preview(&mut p).await;
    let basis = mint(&p, &prepared).unwrap();
    let ticket = ticket(&mut p, target, &basis);
    let bytes = package();
    let cid = publish(&p, &bytes);
    let orphan = publish(&p, b"not an export");
    let plan = capture(
        &mut p,
        &prepared,
        ticket,
        export(target, cid, bytes.len() as u64, 77),
    )
    .plan()
    .unwrap();
    p.b_store.creative_pinned_cids().unwrap();
    assert!(p.b_store.creative_references_known());
    let mut blobs = p
        .b_store
        .blob_store(&hex::encode(p.bob.group_id()))
        .unwrap();
    assert!(!blobs.delete(&cid).unwrap());
    assert!(blobs.delete(&orphan).unwrap());
    let draft = commit(&mut p, &prepared, plan).unwrap();
    assert_eq!(draft.accepted(), 1);
    assert_eq!(p.b_store.live_transient_holds_for_test(), 0);
    // No scan after commit: only the before-write conservative hold transfer can protect this
    // CID now that the detached owner has gone. A durable intent alone cannot repair old pins.
    assert!(
        !blobs.delete(&cid).unwrap(),
        "successful acceptance lost export protection"
    );
    assert_eq!(
        blobs.get_bounded(&cid, bytes.len()).unwrap().unwrap(),
        bytes
    );
}
