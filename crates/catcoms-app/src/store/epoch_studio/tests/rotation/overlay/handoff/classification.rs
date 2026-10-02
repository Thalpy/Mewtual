//! P2's classification of a branch the handoff has already staged.
//!
//! Here rather than beside the other classification tests because the state needs this module's
//! interruption helper: a handoff stopped after its Source write and before Completed.
use super::*;
use catcoms_replication::studio::StudioOverlayEligibility as E;

/// A Prepared branch whose successor already holds its signed operations is pending resolution,
/// not stranded.
///
/// That is the normal shape after a crash between the Source write and the Completed write, and H1
/// settles it from the Prepared record alone. A review found the classifier reading it as
/// `successorNotPristine` - the successor does hold operations, they are this branch's own - which
/// would send a user to the manual path for work that transfers on the next run. Here the next run
/// is made, and it settles.
#[test]
fn a_prepared_branch_interrupted_after_its_source_write_reads_transferable_and_settles() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    super::fences::interrupt(&f, &mut store, basis, WriteTag::Completed);
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert!(
        state.handoff_prepared(),
        "precondition: the handoff stopped with Prepared durable"
    );
    assert!(
        f.load(&store).unwrap().op_count() > 0,
        "precondition: the successor already holds the branch's operations"
    );
    drop(state);

    assert_eq!(
        store
            .studio_overlay_eligibility(SERVER, &f.group, f.target, &f.device, Some(0))
            .unwrap(),
        Some(E::Transferable),
        "a Prepared branch pending resolution is not a manual case"
    );
    let mut b = budget(&mut store, &f);
    store
        .handoff_studio_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b,
        )
        .expect("and the next run settles it, as the classification said it would");
}
