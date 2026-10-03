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
    classify_under(f, store, StudioOwnerTenure::Known(0))
}

/// Every classification here also proves it restored nothing. Reading the Prepared evidence from
/// the record's framing is the whole reason `evidence_in_vault` exists, and swapping it for
/// `evidence` over a restored source would otherwise pass every assertion below.
fn classify_under(f: &Fixture, store: &ServerStore, tenure: StudioOwnerTenure) -> Option<E> {
    let restores = crate::store::epoch_studio::source::studio_full_restores_for_test();
    let class = store
        .studio_overlay_eligibility(SERVER, &f.group, f.target, &f.device, tenure)
        .unwrap();
    assert_eq!(
        crate::store::epoch_studio::source::studio_full_restores_for_test(),
        restores,
        "the lifecycle row runs under custody on every read and must not restore the source"
    );
    class
}

fn handoff(
    f: &Fixture,
    store: &mut ServerStore,
    basis: [u8; 32],
    tenure: Option<u64>,
) -> Result<StudioHandoffOutcome, AppError> {
    let mut b = budget(store, f);
    store.handoff_studio_overlay(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        basis,
        tenure,
        &mut rng(),
        &mut b,
    )
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
    assert_eq!(
        classify_under(&f, &store, StudioOwnerTenure::Unknown),
        Some(E::Transferable),
        "Complete evidence settles without a tenure (V8), so none is asked of it"
    );
    handoff(&f, &mut store, basis, None)
        .expect("and the next run settles it with no tenure, as the classification said it would");
}

/// Prepared, but the Source write never happened: the resolution returns the branch to active,
/// and H1 then carries on exactly as for an active branch. So the classification must too.
///
/// A review found it stopping at `Transferable` for every Absent branch, so a device with no
/// observed tenure was told the draft would transfer while H1 returned it to active and refused
/// in the same call. Here both directions are run, not reasoned about, each from a freshly
/// interrupted Prepared state: the tenure-less run durably returns its branch to active, so the
/// transfer is proved on a second fixture that never left Prepared.
#[test]
fn a_prepared_branch_interrupted_before_its_source_write_is_classified_as_the_active_branch_it_returns_to(
) {
    let interrupted = |root: &Path| {
        let f = Fixture::new(true);
        let mut store = open(root);
        let (_, basis, _) = prepare(&f, &mut store);
        super::fences::interrupt(&f, &mut store, basis, WriteTag::Source);
        assert!(store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .handoff_prepared());
        assert_eq!(evidence_both_ways(&f, &store), Evidence::Absent);
        (f, store, basis)
    };

    let root = tempfile::tempdir().unwrap();
    let (f, mut store, basis) = interrupted(root.path());
    assert_eq!(classify(&f, &store), Some(E::Transferable));
    handoff(&f, &mut store, basis, Some(0))
        .expect("the Prepared branch the row called transferable transfers");

    let root = tempfile::tempdir().unwrap();
    let (f, mut store, basis) = interrupted(root.path());
    assert_eq!(
        classify_under(&f, &store, StudioOwnerTenure::Unknown),
        Some(E::Manual(R::TenureUnknown)),
        "after returning it to active, H1 needs a tenure to sign under"
    );
    let refused = handoff(&f, &mut store, basis, None).unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("overlay handoff needs observed owner tenure"),
        "the handoff refuses for the reason the row gave: {refused}"
    );
}

/// Prepared with Absent evidence, but the source under it has since been faulted. The resolution
/// returns the branch to active and the successor precondition then refuses a faulted source, so
/// the row must say `Fault`, not `Transferable`.
#[test]
fn a_prepared_branch_over_a_faulted_source_is_manual_exactly_while_the_handoff_refuses_it() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    super::fences::interrupt(&f, &mut store, basis, WriteTag::Source);
    let faulted = super::evidence::faulted_source(&f, &store);
    write_for_test(&f.path(&store), &faulted).unwrap();
    assert_eq!(f.load(&store).unwrap().phase(), EpochPhase::Fault);
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .handoff_prepared());

    assert_eq!(
        evidence_both_ways(&f, &store),
        Evidence::Absent,
        "the faulted source holds none of the branch's operations"
    );
    assert_eq!(classify(&f, &store), Some(E::Manual(R::Fault)));
    // The successor precondition's own refusal of a source that is no longer Open, not an
    // incidental one from the inventory or the source read.
    let refused = handoff(&f, &mut store, basis, Some(0)).unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("epoch does not accept operations"),
        "the handoff refuses the state the row calls faulted: {refused}"
    );
}

/// Prepared against the successor, but the source record is now the Closing one the branch was
/// based past: another epoch and another document id. Both evidence readings answer `Hold` from
/// the scope check alone, before any operation is compared, and the resolution refuses.
///
/// Without this, breaking the vault path's epoch or document comparison left every other test
/// green: the remaining tests all keep the source at the Prepared epoch.
#[test]
fn a_prepared_branch_whose_source_went_back_a_generation_is_stuck() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _, closing_record) = super::prepare_keeping_closing(&f, &mut store);
    super::fences::interrupt(&f, &mut store, basis, WriteTag::Source);
    write_for_test(&f.path(&store), &closing_record).unwrap();
    assert_eq!(
        f.load(&store).unwrap().epoch(),
        0,
        "precondition: the source is the Closing epoch again, behind the Prepared one"
    );

    assert_eq!(evidence_both_ways(&f, &store), Evidence::Hold);
    assert_eq!(classify(&f, &store), Some(E::Manual(R::PreparedStuck)));
    let mut b = budget(&mut store, &f);
    let refused = store
        .resolve_studio_handoff(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("prepared handoff retains conflicting or incomplete signed evidence"),
        "the resolution refuses with its own Hold, the state the row calls stuck: {refused}"
    );
}

/// A Prepared branch with no source record, or an unreadable one, is named for that rather than
/// read as transferable.
#[test]
fn a_prepared_branch_whose_source_is_missing_or_unreadable_says_so() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    super::fences::interrupt(&f, &mut store, basis, WriteTag::Source);
    let path = f.path(&store);
    let original = std::fs::read(&path).unwrap();

    let mut corrupt = original.clone();
    let middle = corrupt.len() / 2;
    corrupt[middle] ^= 0xff;
    std::fs::write(&path, &corrupt).unwrap();
    assert_eq!(classify(&f, &store), Some(E::Manual(R::SourceUnreadable)));
    // No resolution is attempted here: its storage budget comes from a full inventory scan, which
    // refuses the corrupt record first, so nothing that writes can start while it stands.

    std::fs::remove_file(&path).unwrap();
    assert_eq!(classify(&f, &store), Some(E::Manual(R::SourceMissing)));
    let mut b = budget(&mut store, &f);
    let refused = store
        .resolve_studio_handoff(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .unwrap_err();
    assert!(
        refused.to_string().contains("source missing"),
        "with no source the resolution cannot read its evidence either: {refused}"
    );
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
    let refused = store
        .resolve_studio_handoff(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("prepared handoff retains conflicting or incomplete signed evidence"),
        "the resolution refuses with its own Hold, exactly the state the row calls stuck: {refused}"
    );
}
