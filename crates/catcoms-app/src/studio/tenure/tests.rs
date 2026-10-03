//! V7's two halves, anchored separately.
//!
//! Revision-4's finding 1 is the reason these are two tests rather than one: an implementation can
//! satisfy every sync-layer test and still launder `Imported` into `Known` at the app boundary, or
//! convert faithfully and then let `require_` succeed for it anyway. Either alone would let a
//! device author under a tenure it cannot verify.
use super::*;
use catcoms_sync::ObservedOwnerTenure as Observed;

/// The mapping, stated as data rather than as prose.
///
/// This is deliberately a table over **every** `ObservedOwnerTenure` variant. The conversion under
/// test has no catch-all arm, so adding a variant breaks the build there; this makes it also break
/// here, where a human has to decide which of the three app states it belongs to.
fn conversion() -> [(Observed, StudioOwnerTenure); 3] {
    [
        (Observed::Observed(7), StudioOwnerTenure::Known(7)),
        (Observed::Imported(7), StudioOwnerTenure::Imported(7)),
        (Observed::Unknown, StudioOwnerTenure::Unknown),
    ]
}

/// M22g: the conversion is total and lossy in one direction only.
#[test]
fn the_app_conversion_never_launders_imported_into_known() {
    for (observed, expected) in conversion() {
        assert_eq!(
            convert(observed),
            expected,
            "{observed:?} must convert to {expected:?}"
        );
    }
    // The claim that actually matters, stated on its own so it cannot be satisfied by a table that
    // happens to agree: an imported start and an observed start with the SAME number are different
    // app states. An implementation that mapped on the number alone would pass every row above.
    assert_ne!(
        convert(Observed::Imported(7)),
        convert(Observed::Observed(7)),
        "an imported tenure and an observed one are different states at the same start"
    );
    assert_eq!(
        convert(Observed::Imported(7)),
        StudioOwnerTenure::Imported(7),
        "and the imported one keeps its start, because verification still needs it"
    );
}

/// M24b: `require_` succeeds for `Known` alone, and says which fail-closed state it hit.
///
/// The two refusals carry different text on purpose: `Unknown` means the device holds nothing,
/// `Imported` that it holds a value it cannot vouch for. Not because they end differently. Both
/// end at the next contiguous step that derives a tenure here (an owner change, or the committer's
/// membership restarting), and neither ends by waiting alone; `catcoms-sync` pins that.
#[test]
fn requiring_a_tenure_succeeds_for_known_alone_and_distinguishes_the_two_refusals() {
    assert_eq!(require(StudioOwnerTenure::Known(7)).unwrap(), 7);

    let imported = require(StudioOwnerTenure::Imported(7))
        .expect_err("an unverified imported tenure must not authorise authoring");
    assert!(
        imported.to_string().contains("imported snapshot"),
        "said: {imported}"
    );
    let unknown = require(StudioOwnerTenure::Unknown)
        .expect_err("an unobserved tenure must not authorise authoring");
    assert!(
        unknown.to_string().contains("has not observed"),
        "said: {unknown}"
    );
    assert_ne!(
        imported.to_string(),
        unknown.to_string(),
        "the two fail-closed states are different situations and must not share a message"
    );
}

/// V6: `Imported` carries a usable start for verification even though authoring refuses it.
///
/// Hiding the value would be the worse failure. `complete_checkpoint_head_scoped` accepts a proof's
/// *claimed* tenure when the local value is absent, so an app boundary that reported `Unknown` for
/// an imported tenure would turn a refusal into an acceptance of someone else's number.
#[test]
fn an_imported_tenure_keeps_a_start_for_verification_while_refusing_to_author() {
    let StudioOwnerTenure::Imported(start) = convert(Observed::Imported(11)) else {
        panic!("an imported observation must stay imported")
    };
    assert_eq!(start, 11);
    assert!(require(StudioOwnerTenure::Imported(start)).is_err());
}
