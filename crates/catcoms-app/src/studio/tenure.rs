//! The live-tenure seam Agents 1 and 3 consume (design 9.4).
//!
//! The sync layer already decides what this device has observed. This module is the **app
//! boundary** over that decision, and it exists because revision-4's finding 1 was precisely that
//! an implementation can satisfy every sync-layer test while still laundering `Imported` into
//! `Known` on the way out. The conversion below is therefore total, written as an exhaustive match
//! with no catch-all arm, and anchored by its own tests.
//!
//! **Fail closed means new authoring is refused, not that everything is refused (V8).** Under
//! `Imported` and under `Unknown` an exact accepted Save retry, a completed-handoff
//! acknowledgement, and resolution of an already durable `Prepared` handoff all stay reachable.
//! That is why this module offers two different things - a value anyone may read, and a
//! requirement only new authoring should call - rather than one function that refuses.
//!
//! **This module does not refuse anything itself, by design (A-1).** The app wrappers that read
//! `authoring_owner_tenure_start()` pass the `Option<u64>` through unchanged, and the Flow S
//! wrappers pass the typed [`StudioOwnerTenure`] instead, so the store can refuse at the stage that
//! needs it. A wrapper that refused early would move the
//! decision away from the code that knows whether this particular call is new authoring or a
//! retry, and V8 is the list of things that would then break.
use super::*;
use catcoms_sync::ObservedOwnerTenure;

/// What this device can say about the current owner's tenure, at the app boundary.
///
/// Three states rather than an `Option<u64>` because "I have not observed a tenure" and "I hold a
/// value I cannot verify" are different facts with different consequences, and collapsing them is
/// how a device ends up authoring under a tenure it only inherited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioOwnerTenure {
    /// Observed under the leaf-aware rule, or migrated from a provably safe v1 state. Usable for
    /// verification and for authoring.
    Known(u64),
    /// A v1 snapshot whose `start < epoch`, with no leaf-continuity evidence (9.3 part 5).
    ///
    /// **Usable for verification, fail-closed for authoring**, and never promoted: not by a
    /// restart, a save and reload, elapsed time, a fresh owner proof, or a peer's agreement (V6).
    /// It stays visible to verification precisely because hiding it would make the device fall back
    /// to accepting a proof's *claimed* tenure instead of comparing against what it holds.
    Imported(u64),
    /// No evidence at all.
    Unknown,
}

/// The conversion, as a free function so V7's two halves can be anchored without a live `Server`.
///
/// That is not a testing convenience: revision-4's finding 1 is that the laundering happens *at
/// this boundary*, so the boundary needs an anchor that does not depend on being able to construct
/// a device in the state under test. Building a genuine `Imported` device requires a migrated v1
/// snapshot, which is N-T7's job; this is the mapping itself.
///
/// Written as an exhaustive match with **no catch-all arm** so that a new `ObservedOwnerTenure`
/// variant is a compile error here rather than silently becoming whatever the fallback said.
fn convert(observed: ObservedOwnerTenure) -> StudioOwnerTenure {
    match observed {
        ObservedOwnerTenure::Observed(start) => StudioOwnerTenure::Known(start),
        ObservedOwnerTenure::Imported(start) => StudioOwnerTenure::Imported(start),
        ObservedOwnerTenure::Unknown => StudioOwnerTenure::Unknown,
    }
}

/// The requirement, likewise free-standing and likewise exhaustive.
///
/// `pub(crate)` so each Flow S authoring stage can apply it at its own point (A-1): the Server
/// passes the `Copy` value down, and S1b and S3 each call this after classification. A single
/// `Result` could not be carried instead, because `AppError` is not `Clone` and both stages need
/// the value.
pub(crate) fn require(tenure: StudioOwnerTenure) -> Result<u64, AppError> {
    match tenure {
        StudioOwnerTenure::Known(start) => Ok(start),
        StudioOwnerTenure::Imported(_) => Err(invalid(
            "this device holds an unverified owner tenure from an imported snapshot and cannot \
             author under it",
        )),
        StudioOwnerTenure::Unknown => Err(invalid(
            "this device has not observed the current owner's tenure and cannot author under it",
        )),
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// The observed tenure, converted without loss except in the one permitted direction (V7).
    pub fn observed_owner_tenure(&self) -> StudioOwnerTenure {
        convert(self.sync.observed_owner_tenure())
    }

    /// The tenure for **new authoring**, which succeeds for `Known` alone (V1, V5, V7).
    ///
    /// `Imported` and `Unknown` are refused separately rather than with one message, because they
    /// are different situations for whoever reads the error: `Unknown` means this device has never
    /// observed the owner take office, and `Imported` means it holds a value from a snapshot it
    /// cannot verify. The second is not fixed by waiting.
    ///
    /// Callers that are **not** new authoring must not use this. Agent 3 takes it for repair
    /// issuance and holds on both fail-closed values (V5); an exact retry, an acknowledgement or
    /// the resolution of an already durable handoff must keep working and so must read the value
    /// rather than require it (V8).
    ///
    /// First spent by Agent 1 at Closing-overlay preparation, which is pure authoring with no
    /// terminal path to strand.
    pub(crate) fn require_observed_owner_tenure(&self) -> Result<u64, AppError> {
        require(self.observed_owner_tenure())
    }
}

#[cfg(test)]
mod tests;
