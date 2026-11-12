//! Active capture custody, not dormant recovery DATA or a permission to repair/unlink.
//!
//! Both concrete bundles declare held source Files BEFORE their same-vault physical lease.
//! They live in an OUTSIDE caller slot and are only borrowed by the HRTB Current callback.
//! A failed capture therefore cannot leave detached Files after its lease was released.
use super::*;
use crate::store::persistence::v2_text::V2TextPromotionRecoveryResources;

macro_rules! capture_bundle {
    ($bundle:ident, $prepared:ident, $source:ident, $slot:ident) => {
        pub(in crate::store) struct $bundle {
            source: Option<$source>,
            original: OriginalV2TextPromotionObservation,
            // Every successfully opened source File/buffer drains before physical unlock.
            physical: PreparedV2TextPhysicalLease,
            // LAST: fee-only custody pays this WHOLE bundle even while source=None and
            // after the original PreparedRole dropped. No File/payload follows the lease.
            _recovery: V2TextPromotionRecoveryResources,
        }
        impl $bundle {
            /// Negative observation only: no source/File/work permission escapes. Tests
            /// pin that the real prior-work gate runs BEFORE even inert source caching.
            #[cfg(all(test, windows))]
            pub(in crate::store) fn has_source_for_test(&self) -> bool {
                self.source.is_some()
            }

            /// Prepare immediately OUTSIDE all writers. Inputs remain borrowed unchanged
            /// on error. Capture-source constructors have their OWN paid4MiB account; this
            /// actual OS lease neither replaces that credit nor grants positive permission.
            pub(in crate::store) fn prepare(
                store: &ServerStore,
                source: &GenesisSourceDataRef<'_>,
                prepared: &$prepared,
            ) -> Result<Self, AppError> {
                prepared.validate_preparation(store, source)?;
                let recovery = prepared.recovery_resources().clone();
                let physical =
                    PreparedV2TextPhysicalLease::prepare(store, source, prepared.resources())?;
                prepared.validate_preparation(store, source)?;
                Ok(Self {
                    source: None,
                    original: prepared.observe_original(),
                    physical,
                    _recovery: recovery,
                })
            }
            pub(super) fn require_same_promotion(
                &self,
                original: &OriginalV2TextPromotionObservation,
            ) -> Result<(), AppError> {
                self.original.require_same_original(original)
            }
            pub(super) fn validate_resources(
                &self,
                guard: &V2TextCurrentGroupGuard<'_, '_>,
            ) -> Result<(), AppError> {
                self.physical.validate_under_current_guard(guard)
            }
            pub(super) fn require_original_negative(
                &self,
                guard: &V2TextCurrentGroupGuard<'_, '_>,
            ) -> Result<(), AppError> {
                self.physical.require_original_negative(guard)
            }
            pub(super) fn source_data(&self) -> Result<&$source, AppError> {
                self.source.as_ref().ok_or_else(refused)
            }
            pub(super) fn source_slot(&mut self) -> $slot<'_> {
                $slot {
                    source: &mut self.source,
                }
            }
        }

        /// A narrow fill loan, never an owned source/Option accessor. The physical family
        /// caches its initial inert role before opening dependent Files, then fills it in
        /// place before each fallible read/proof. It cannot detach those Files from lease.
        pub(in crate::store) struct $slot<'capture> {
            source: &'capture mut Option<$source>,
        }
        impl $slot<'_> {
            pub(in crate::store) fn get(&self) -> Option<&$source> {
                self.source.as_ref()
            }
            pub(in crate::store) fn get_mut(&mut self) -> Option<&mut $source> {
                self.source.as_mut()
            }
            /// Infallible cache-initial edge under the same exclusive fill loan. The
            /// private caller must not replace an existing source with a new selection.
            pub(in crate::store) fn cache_initial(&mut self, source: $source) {
                assert!(
                    self.source.is_none(),
                    "original source already cached in active bundle"
                );
                *self.source = Some(source);
            }
        }
        impl std::fmt::Debug for $bundle {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($bundle)).finish_non_exhaustive()
            }
        }
        impl std::fmt::Debug for $slot<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($slot)).finish_non_exhaustive()
            }
        }
    };
}

capture_bundle!(
    PreparedV2TextPromotionSeedRepairCapture,
    PreparedV2TextPromotionSeedRepair,
    CapturedV2TextPromotionSeedRepair,
    V2TextPromotionSeedRepairSourceSlot
);
capture_bundle!(
    PreparedV2TextPromotionScratchCapture,
    PreparedV2TextPromotionScratch,
    CapturedV2TextPromotionScratch,
    V2TextPromotionScratchSourceSlot
);
