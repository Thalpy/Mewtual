//! P2 at the store: the reasons the store-level classifier adds on top of the successor hold.
//!
//! The successor reasons themselves are pinned to the handoff precondition in
//! `catcoms-replication`; these cover what only the store can see - authorship against the
//! requesting device, an absent or unreadable source, and the tenure the handoff would sign under -
//! on a real vault, through the ordinary Save path.
use super::archive::frame_branch;
use super::*;
use crate::studio::StudioOwnerTenure::{self, Imported, Known, Unknown};
use catcoms_replication::studio::{StudioOverlayEligibility as E, StudioOverlayManualReason as R};

fn classify(f: &Fixture, store: &ServerStore, tenure: StudioOwnerTenure) -> Option<E> {
    store
        .studio_overlay_eligibility(SERVER, &f.group, f.target, &f.device, tenure)
        .unwrap()
}

#[test]
fn the_lifecycle_classification_names_each_store_level_reason() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    assert_eq!(
        classify(&f, &store, Known(0)),
        None,
        "no branch, no classification"
    );

    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    assert_eq!(
        classify(&f, &store, Known(0)),
        Some(E::Manual(R::SuccessorMissing)),
        "on the still-Closing source the successor does not exist yet"
    );
    let other = MlsDevice::generate().unwrap();
    assert_eq!(
        store
            .studio_overlay_eligibility(SERVER, &f.group, f.target, &other, Known(0))
            .unwrap(),
        Some(E::Manual(R::NotCurrentAuthor)),
        "only the branch's author may transfer it"
    );

    install(&f, &mut store, &close);
    let restores = crate::store::epoch_studio::source::studio_full_restores_for_test();
    assert_eq!(
        classify(&f, &store, Known(0)),
        Some(E::Transferable),
        "the pristine successor, the author and the receipt's own tenure: the handoff would run"
    );
    assert_eq!(
        crate::store::epoch_studio::source::studio_full_restores_for_test(),
        restores,
        "the lifecycle row runs under custody on every read and must not restore the source"
    );
    assert_eq!(
        classify(&f, &store, Unknown),
        Some(E::Manual(R::TenureUnknown)),
        "without an observed tenure the handoff cannot sign, so the draft is manual"
    );
    assert_eq!(
        classify(&f, &store, Imported(0)),
        Some(E::Manual(R::TenureImported)),
        "an imported tenure is refused too, and named apart: the device holds a value it cannot \
         vouch for, not nothing"
    );
    assert_eq!(
        classify(&f, &store, Known(7)),
        Some(E::Manual(R::ReceiptChanged)),
        "under a tenure the receipt was not issued in, its owner is not the current one"
    );

    // An unreadable source is a reason, not an error that hides the branch.
    let path = f.path(&store);
    let original = std::fs::read(&path).unwrap();
    let mut corrupt = original.clone();
    let middle = corrupt.len() / 2;
    corrupt[middle] ^= 0xff;
    std::fs::write(&path, &corrupt).unwrap();
    assert_eq!(
        classify(&f, &store, Known(0)),
        Some(E::Manual(R::SourceUnreadable))
    );

    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        classify(&f, &store, Known(0)),
        Some(E::Manual(R::SourceMissing))
    );
}

/// The classification and the REAL handoff agree, in both directions, on the one durable refusal
/// the successor precondition does not cover: an Index entry naming a Flipnote with no source.
///
/// A review found the classifier calling such a branch `Transferable` while H1 refused it with
/// "overlay references an unavailable Flipnote", so a user would be told to wait for a transfer
/// that never comes. Here the handoff is run, not reasoned about: `ObjectMissing` while it
/// refuses, then `Transferable` once the Flipnote exists, and the handoff then succeeds.
#[test]
fn an_index_entry_naming_a_missing_flipnote_is_manual_exactly_while_the_handoff_refuses_it() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    let object = [15; 16];
    let op = f.domain(
        IndexOp::PutObject {
            object,
            kind: StudioKind::Flipnote,
            title: "saved branch".into(),
            created_by: f.device.device_id(),
            ts: 123,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
        55,
    );
    save(&f, &mut store, &close, basis.fingerprint(), op, 123);
    install(&f, &mut store, &close);
    let handoff = |store: &mut ServerStore| {
        let mut b = budget(store, &f);
        store.handoff_studio_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis.fingerprint(),
            Some(0),
            &mut rng(),
            &mut b,
        )
    };

    assert_eq!(
        classify(&f, &store, Known(0)),
        Some(E::Manual(R::ObjectMissing))
    );
    let refused = handoff(&mut store).unwrap_err().to_string();
    assert!(
        refused.contains("unavailable Flipnote"),
        "the handoff must refuse for the reason the classifier named, got: {refused}"
    );

    // Create the Flipnote the entry names, and the two must agree the other way.
    let flipnote = StudioTarget::Flipnote {
        channel: f.target.channel(),
        object,
    };
    let logical = flipnote.document(&f.group.group_id()).unwrap();
    let edit = DomainOp {
        doc_type: logical.doc_type,
        logical_key: logical.logical_key.clone(),
        nonce: [55; 16],
        body: FlipnoteOp::SetHeader(FlipnoteHeader::Title("saved branch".into()))
            .encode()
            .unwrap(),
    };
    let mut b = budget(&mut store, &f);
    store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            flipnote,
            epoch_zero_id(logical.doc_type, &logical.logical_key),
            &f.device,
            edit,
            123,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    assert_eq!(classify(&f, &store, Known(0)), Some(E::Transferable));
    handoff(&mut store).expect("a branch classified transferable must transfer");
}
