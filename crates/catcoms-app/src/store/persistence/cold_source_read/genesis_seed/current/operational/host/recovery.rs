//! Same-owner promotion recovery dispatcher. Classification is readonly DATA; each
//! chosen role still needs its own fresh Current, idle lane and finite IO admission.
//!
//! In particular, a repair refusal is never retried as scratch deletion. Cached actual
//! completion is installed before consulting positive Source again, including after a
//! deliberately lost worker reply or cancellation. No branch signs or reseals bytes.
use super::super::owner::V2TextOwnerRef;
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextPromotionScratch, AdmittedV2TextPromotionSeedRepair,
    PreparedV2TextPromotionScratchCapture, PreparedV2TextPromotionSeedRepairCapture,
};
use crate::store::persistence::v2_text::{
    PreparedV2TextPromotionScratch, PreparedV2TextPromotionSeedRepair,
    V2TextPromotionRecoveryClassData,
};
use crate::store::ServerStore;

/// Exactly one independently paid recovery role at a time. The optional capture is
/// ACTIVE custody (Files before its physical lease), never detached dormant source DATA.
/// Every callback borrows these outer slots; none can drop owning Files under a writer.
pub(super) enum PromotionRecovery {
    Seed {
        role: PreparedV2TextPromotionSeedRepair,
        capture: Option<PreparedV2TextPromotionSeedRepairCapture>,
        admitted: Option<AdmittedV2TextPromotionSeedRepair>,
    },
    Scratch {
        role: PreparedV2TextPromotionScratch,
        capture: Option<PreparedV2TextPromotionScratchCapture>,
        admitted: Option<AdmittedV2TextPromotionScratch>,
    },
}

// The physical role reserves a separate 4 MiB invoice including 8 KiB for this actual
// host sum/Option metadata. Original16MiB, SDK and read payments provide no overlap credit.
const _: () = assert!(std::mem::size_of::<PromotionRecovery>() <= 8192);

impl PromotionRecovery {
    /// Run only already-admitted work and install its actual outcome. No Source, key,
    /// Clock, RNG or Current lookup is needed to move this same immutable result home.
    /// A lost reply retains the role and its completion for the next call, unchanged.
    fn drain_into(&mut self, original: &mut PreparedV2TextPromotion) -> Result<bool, AppError> {
        match self {
            Self::Seed { role, admitted, .. } => {
                if let Some(work) = admitted.take() {
                    role.run(work)?;
                }
                if role.has_completed_ready_data() {
                    role.install_ready_into(original)?;
                    return Ok(true);
                }
            }
            Self::Scratch { role, admitted, .. } => {
                if let Some(work) = admitted.take() {
                    role.run(work)?;
                }
                if role.has_completed_cleanup_data() {
                    role.install_cleanup_into(original)?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Dispatch before taking a new original owner loan. Destroy the completed role and
    /// its resource-only plan outside all writers, after its actual result has moved home.
    pub(super) fn drain_cached(
        slot: &mut Option<Self>,
        original: &mut PreparedV2TextPromotion,
    ) -> Result<(), AppError> {
        let complete = match slot.as_mut() {
            Some(recovery) => recovery.drain_into(original)?,
            None => false,
        };
        if complete {
            drop(slot.take());
        }
        Ok(())
    }

    /// A readonly positive classification chooses the ONE role; it never supplies its
    /// effect capability. Unknown, foreign, corrupt and missing-history results propagate
    /// unchanged. Clean/unrenamed complete seed work goes through ordinary stage admission.
    pub(super) fn before_stage(
        slot: &mut Option<Self>,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        lane: &V2TextLane,
        original: &mut PreparedV2TextPromotion,
    ) -> Result<(), AppError> {
        Self::drain_cached(slot, original)?;
        // This checks only the SAME private cached outcome, never a disk/error fallback.
        // A just-installed Ready must go on to fresh commit, not allocate another repair.
        if original.ready_selection().is_ok() {
            return Ok(());
        }
        if slot.is_none() {
            let resources = original.resources().clone();
            let class = store.with_v2_text_current(owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                original.classify_recovery_under_current_guard(&guard)
            })?;
            *slot = match class {
                V2TextPromotionRecoveryClassData::CompleteMain => Some(Self::Seed {
                    role: PreparedV2TextPromotionSeedRepair::prepare(
                        store,
                        owner.source(),
                        original,
                    )?,
                    capture: None,
                    admitted: None,
                }),
                V2TextPromotionRecoveryClassData::FirstSeedScratch => Some(Self::Scratch {
                    role: PreparedV2TextPromotionScratch::prepare(store, owner.source(), original)?,
                    capture: None,
                    admitted: None,
                }),
                V2TextPromotionRecoveryClassData::CleanUnstarted
                | V2TextPromotionRecoveryClassData::CompleteSeedStage => None,
            };
        }
        let result = match slot.as_mut() {
            Some(Self::Seed {
                role,
                capture,
                admitted,
            }) => {
                if capture.is_none() {
                    *capture = Some(PreparedV2TextPromotionSeedRepairCapture::prepare(
                        store,
                        owner.source(),
                        role,
                    )?);
                }
                let resources = role.resources().clone();
                let result = store.with_v2_text_current(owner, |admission| {
                    let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                    guard.admit_text_promotion_seed_repair_into(lane, role, capture, admitted)
                });
                // A late callback failure can leave EITHER the whole unadmitted capture
                // or the admitted marker. Close the former here OUTSIDE every writer;
                // the latter is dispatched below even when that callback returned Err.
                drop(capture.take());
                result
            }
            Some(Self::Scratch {
                role,
                capture,
                admitted,
            }) => {
                if capture.is_none() {
                    *capture = Some(PreparedV2TextPromotionScratchCapture::prepare(
                        store,
                        owner.source(),
                        role,
                    )?);
                }
                let resources = role.resources().clone();
                let result = store.with_v2_text_current(owner, |admission| {
                    let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                    guard.admit_text_promotion_scratch_into(lane, role, capture, admitted)
                });
                drop(capture.take());
                result
            }
            None => return Ok(()),
        };
        // Complete already-admitted finite work before reporting a late admission error.
        // If run itself returns Err AFTER caching a result, the next call installs that
        // result first. An error never becomes permission to choose another role.
        Self::drain_cached(slot, original)?;
        result
    }
}
