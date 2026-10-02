//! P2 at the store: the reasons the store-level classifier adds on top of the successor hold.
//!
//! The successor reasons themselves are pinned to the handoff precondition in
//! `catcoms-replication`; these cover what only the store can see - authorship against the
//! requesting device, an absent or unreadable source, and the tenure the handoff would sign under -
//! on a real vault, through the ordinary Save path.
use super::archive::frame_branch;
use super::*;
use catcoms_replication::studio::{StudioOverlayEligibility as E, StudioOverlayManualReason as R};

fn classify(f: &Fixture, store: &ServerStore, tenure: Option<u64>) -> Option<E> {
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
        classify(&f, &store, Some(0)),
        None,
        "no branch, no classification"
    );

    let (close, basis) = closing(&f, &mut store);
    frame_branch(&f, &mut store, &close, &basis);
    assert_eq!(
        classify(&f, &store, Some(0)),
        Some(E::Manual(R::SuccessorMissing)),
        "on the still-Closing source the successor does not exist yet"
    );
    let other = MlsDevice::generate().unwrap();
    assert_eq!(
        store
            .studio_overlay_eligibility(SERVER, &f.group, f.target, &other, Some(0))
            .unwrap(),
        Some(E::Manual(R::NotCurrentAuthor)),
        "only the branch's author may transfer it"
    );

    install(&f, &mut store, &close);
    assert_eq!(
        classify(&f, &store, Some(0)),
        Some(E::Transferable),
        "the pristine successor, the author and the receipt's own tenure: the handoff would run"
    );
    assert_eq!(
        classify(&f, &store, None),
        Some(E::Manual(R::TenureUnknown)),
        "without an observed tenure the handoff cannot sign, so the draft is manual"
    );
    assert_eq!(
        classify(&f, &store, Some(7)),
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
        classify(&f, &store, Some(0)),
        Some(E::Manual(R::SourceUnreadable))
    );

    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        classify(&f, &store, Some(0)),
        Some(E::Manual(R::SourceMissing))
    );
}
