//! P2's classification of a branch the handoff has already staged.
//!
//! Here rather than beside the other classification tests because the states need this module's
//! interruption and substitution helpers: a handoff stopped before its Source write, after it, and
//! after a partial one.
use super::*;
use crate::studio::StudioOwnerTenure;
use catcoms_replication::studio::{
    StudioHandoffEvidence as Evidence, StudioOverlayEligibility as E,
    StudioOverlayManualReason as R,
};

fn classify(f: &Fixture, store: &ServerStore) -> Option<E> {
    store
        .studio_overlay_eligibility(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            StudioOwnerTenure::Known(0),
        )
        .unwrap()
}

/// The resolution's evidence, read two ways: from the restored source, as H1 reads it, and from
/// the vault record's framing, as the lifecycle row reads it. They must agree.
fn evidence_both_ways(f: &Fixture, store: &ServerStore) -> Evidence {
    let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let metadata = state.handoff_metadata().unwrap();
    let restored = metadata
        .evidence(&f.load(store).unwrap().unit, &state.ledger)
        .unwrap();
    let structural = store
        .with_vault_source(SERVER, &f.group, f.target, |bytes| {
            metadata.evidence_in_vault(bytes, &state.ledger)
        })
        .unwrap()
        .expect("a Prepared branch has a source");
    assert_eq!(
        structural, restored,
        "the vault evidence must agree with the evidence the resolution actually uses"
    );
    restored
}

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

    assert_eq!(evidence_both_ways(&f, &store), Evidence::Complete);
    assert_eq!(
        classify(&f, &store),
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

/// Prepared, but the Source write never happened: the resolution returns the branch to active.
#[test]
fn a_prepared_branch_interrupted_before_its_source_write_reads_transferable() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    super::fences::interrupt(&f, &mut store, basis, WriteTag::Source);
    assert_eq!(evidence_both_ways(&f, &store), Evidence::Absent);
    assert_eq!(classify(&f, &store), Some(E::Transferable));
}

/// Prepared, and the source holds only part of the branch's signed operations. The resolution can
/// neither complete it nor return it to active, so it is stuck, and the row must say so rather than
/// tell the user it will transfer.
#[test]
fn a_prepared_branch_with_a_partial_source_is_stuck_exactly_while_the_resolution_refuses_it() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis) = closing(&f, &mut store);
    save(&f, &mut store, &close, basis.fingerprint(), f.title(), 123);
    let mut second = f.title();
    second.nonce = [77; 16];
    save(&f, &mut store, &close, basis.fingerprint(), second, 124);
    install(&f, &mut store, &close);
    let (substitute, _) = super::evidence::substituted(&f, &store, false);
    let mut b = budget(&mut store, &f);
    store
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

    assert_eq!(evidence_both_ways(&f, &store), Evidence::Hold);
    assert_eq!(classify(&f, &store), Some(E::Manual(R::PreparedStuck)));
    let mut b = budget(&mut store, &f);
    assert!(
        store
            .resolve_studio_handoff(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
            .is_err(),
        "the resolution refuses exactly the state the row calls stuck"
    );
}
