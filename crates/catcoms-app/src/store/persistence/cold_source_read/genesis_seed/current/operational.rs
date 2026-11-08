//! Original-owner composition for operational text storage.
//!
//! The read-only capture host preserves the complete original Current family and caches
//! physical observations before a final Current check. The separate first-promotion host
//! stages and selects only the empty operational O_G through actual affine work admissions.
//! Neither host can post, sign, issue projection support or activate native Chat V2.

mod host;
pub(in crate::store) use host::{
    OriginalV2TextResidentObservation, OriginalV2UnsignedPlanObservation,
    PreparedV2NextUnsignedAppend, PreparedV2TextResidentProjection,
    PreparedV2UnsignedOriginalCustody, V2AdvancedAppendBasis, V2TextResidentParentData,
};
pub(super) mod negative;
pub(super) mod owner;
pub use host::{PreparedV2TextHost, V2TextHostPolicy};

use crate::store::critical_namespaces::current_layout::V2CurrentLayoutStore;
use crate::store::persistence::v2_text::PreparedV2TextParentCapture;
use crate::store::persistence::{
    UnexposedColdV2GenesisCurrent, UnexposedRepairedV2GenesisCurrent, UnexposedV2GenesisCurrent,
};
use crate::AppError;
use owner::V2TextOwnedCurrent;

/// Read-only, original-owner custody. Construction only moves existing values; the
/// actual capture reserves its independent metadata payment before allocating or waiting.
/// The cached observation never supplies historical Current or permission to publish.
pub struct PreparedV2TextRead<T, R> {
    capture: Option<PreparedV2TextParentCapture>,
    owner: V2TextOwnedCurrent<T, R>,
}

impl<T, R> PreparedV2TextRead<T, R> {
    pub fn from_warm(current: UnexposedV2GenesisCurrent<T, R>) -> Self {
        Self::from_original_owner(V2TextOwnedCurrent::from_warm(current))
    }

    pub fn from_cold(current: UnexposedColdV2GenesisCurrent<T, R>) -> Self {
        Self::from_original_owner(V2TextOwnedCurrent::from_cold(current))
    }

    pub fn from_repaired(current: UnexposedRepairedV2GenesisCurrent<T, R>) -> Self {
        Self::from_original_owner(V2TextOwnedCurrent::from_repaired(current))
    }

    /// Same closed original family only. All public constructors delegate here; this
    /// inert move cannot adopt a raw node, decoded role or caller-selected permission.
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn from_original_owner(
        owner: V2TextOwnedCurrent<T, R>,
    ) -> Self {
        Self {
            capture: None,
            owner,
        }
    }

    /// Finish only the original family's existing activation-seed settlement. A fresh
    /// physical absence never replaces its original saved cleanup or covering barrier.
    pub fn retry_original_seed_settlement(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<(), AppError> {
        self.owner.retry_seed_settlement(layout, rng)
    }

    /// Observe an exact operational O/H under the real Current guard. Before O_G has
    /// actually been selected, the old genesis head refuses here; missing or corrupt
    /// controls never become an empty history. On error the same owner/preparation and
    /// any complete captured observation remain available for inspection or exact retry.
    pub fn retry_capture_current(&mut self, layout: &V2CurrentLayoutStore) -> Result<(), AppError> {
        let store = layout.checked_store()?;
        let owner = self.owner.borrow_current()?;
        if self.capture.is_none() {
            self.capture = Some(PreparedV2TextParentCapture::prepare(store, owner.source())?);
        }
        let capture = self
            .capture
            .as_mut()
            .expect("capture was cached before admission");
        // Clone the SAME paid resource body outside all writers. The callback borrows
        // the outer owning slot so a late refusal cannot drop its last payment under AD.
        let resources = capture.resources().clone();
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            capture.capture(&guard)?;
            Ok(())
        })
    }

    /// Revoke new use of this original owner. This read-only host owns no physical work
    /// or query worker, so this method is deliberately not a general work-drain receipt.
    pub fn cancel_original(&self) {
        self.owner.cancel_original();
    }
}

impl<T, R> std::fmt::Debug for PreparedV2TextRead<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2TextRead").finish_non_exhaustive()
    }
}
