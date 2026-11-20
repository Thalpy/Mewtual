//! Exact successful empty-promotion lane completion.
//!
//! This is not ordinary WC, a suspension path, or permission to abandon pre-Saved debt.
//! The first REAL stage admission binds a lane permanently to its opaque promotion plan;
//! only that plan's selected Saved O_G may close and retire it. Actual marker ownership,
//! rather than a caller-reported count, determines when the final work has drained.
use super::*;
mod create;
mod repair;
use crate::store::persistence::v2_text::OriginalV2TextPromotionObservation;
use std::sync::atomic::AtomicU8;
use std::sync::{Condvar, Mutex, OnceLock};

const OPEN_IDLE: u8 = 0;
const ACTIVE: u8 = 1;
const CLOSING: u8 = 2;
const CLOSING_IDLE: u8 = CLOSING;

pub(super) struct PromotionWorkControl {
    state: AtomicU8,
    plan: OnceLock<OriginalV2TextPromotionObservation>,
    // The end owner takes this ONLY outside global writers, so a waiter cannot miss the
    // final clear/notification between observing ACTIVE and entering Condvar::wait.
    wait: Mutex<()>,
    changed: Condvar,
    // LAST: every escaping control/weak observation retains this SAME entry payment.
    _fee: Arc<CatalogReadReservation>,
}
impl PromotionWorkControl {
    pub(super) fn new(fee: Arc<CatalogReadReservation>) -> Self {
        Self {
            state: AtomicU8::new(OPEN_IDLE),
            plan: OnceLock::new(),
            wait: Mutex::new(()),
            changed: Condvar::new(),
            _fee: fee,
        }
    }

    fn require_plan(&self, original: &OriginalV2TextPromotionObservation) -> Result<(), AppError> {
        self.plan
            .get()
            .ok_or_else(refused)?
            .require_same_original(original)
    }
}

/// Exactly one active admitted Stage OR Commit marker may own this value for a lane.
/// The marker declares it LAST, after the SAME physical OS lease, and the finite worker
/// keeps that marker alive through all workspace Files and local cleanup on Ok AND Err.
pub(in crate::store) struct V2TextPromotionWorkEnd {
    control: Arc<PromotionWorkControl>,
}
impl Drop for V2TextPromotionWorkEnd {
    fn drop(&mut self) {
        // A poisoned wait lock must not strand the real end owner. It protects only the
        // notification protocol; no positive authority or user data is recovered here.
        let _wait = self
            .control
            .wait
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = self.control.state.fetch_and(!ACTIVE, Ordering::AcqRel);
        debug_assert_ne!(previous & ACTIVE, 0, "one affine end owner per reservation");
        self.control.changed.notify_all();
    }
}

/// Constructed only after fresh selected Saved O_G closes this exact lane's admissions.
/// An owning caller caches it outside the Scope callback before a late postcheck can fail.
pub(in crate::store) struct V2TextPromotionLaneDrain {
    control: Arc<PromotionWorkControl>,
    original: OriginalV2TextLaneObservation,
}

/// Actual closed same-control drain, not an integer supplied by a caller or a weak expiry.
pub(in crate::store) struct V2TextPromotionLaneDrained {
    control: Arc<PromotionWorkControl>,
    original: OriginalV2TextLaneObservation,
}

/// Both actual removed registry ownership and the unique caller move here before the
/// Scope postcheck. Drop this outside ALL writers; its source/entry fees may be last.
pub(in crate::store) struct RetiredV2TextPromotionLane {
    caller: V2TextLane,
    registered: Arc<LaneBody>,
}

pub(super) const PROMOTION_WORK_CONTROL_METADATA_BYTES: usize =
    std::mem::size_of::<PromotionWorkControl>()
        + std::mem::size_of::<V2TextPromotionWorkEnd>()
        + std::mem::size_of::<V2TextPromotionLaneDrain>()
        + std::mem::size_of::<V2TextPromotionLaneDrained>()
        + std::mem::size_of::<RetiredV2TextPromotionLane>()
        + 4 * 64;

impl V2TextLane {
    /// Called ONLY by the actual guard after its complete stage physical proof and final
    /// original negatives. No allocation/account/IO follows successful reserve/binding.
    /// No independent path may bind a new lane to an old Ready or completed Saved plan.
    pub(in crate::store) fn reserve_promotion_stage_end(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
        original: &OriginalV2TextPromotionObservation,
    ) -> Result<V2TextPromotionWorkEnd, AppError> {
        self.require_registered(namespace)?;
        if let Some(plan) = self.body.promotion.plan.get() {
            plan.require_same_original(original)?;
        }
        self.body
            .promotion
            .state
            .compare_exchange(OPEN_IDLE, ACTIVE, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        // The actual canonical mutex serializes ALL constructors of this private binding.
        // The successful first stage admission is the only caller that can reach this set.
        // Arc/Weak cloning is already paid and infallible; no fee/refund or hook follows.
        if self.body.promotion.plan.get().is_none() {
            self.body
                .promotion
                .plan
                .set(original.clone())
                .expect("same canonical writer serializes the first actual stage binding");
        }
        Ok(V2TextPromotionWorkEnd {
            control: self.body.promotion.clone(),
        })
    }

    /// Commit cannot perform the first binding: it follows this SAME lane's real Stage.
    pub(in crate::store) fn reserve_promotion_commit_end(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
        original: &OriginalV2TextPromotionObservation,
    ) -> Result<V2TextPromotionWorkEnd, AppError> {
        self.require_registered(namespace)?;
        self.body.promotion.require_plan(original)?;
        self.body
            .promotion
            .state
            .compare_exchange(OPEN_IDLE, ACTIVE, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        Ok(V2TextPromotionWorkEnd {
            control: self.body.promotion.clone(),
        })
    }

    /// The caller has just proved actual selected Saved O_G under the same Current guard.
    /// Close is permanent even if the generic Scope postcheck refuses after this transfer.
    pub(in crate::store) fn close_promotion_into(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
        original: &OriginalV2TextPromotionObservation,
        closed: &mut Option<V2TextPromotionLaneDrain>,
    ) -> Result<(), AppError> {
        self.require_registered(namespace)?;
        self.body.promotion.require_plan(original)?;
        if let Some(previous) = closed.as_ref() {
            previous.require_same_closed_lane(self)?;
            return Ok(());
        }
        let drain = V2TextPromotionLaneDrain {
            control: self.body.promotion.clone(),
            original: self.observe_original(),
        };
        self.body
            .promotion
            .state
            .fetch_or(CLOSING, Ordering::AcqRel);
        *closed = Some(drain);
        Ok(())
    }
}

impl V2TextPromotionLaneDrain {
    fn require_same_closed_lane(&self, lane: &V2TextLane) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.control, &lane.body.promotion)
            || !self
                .original
                .identity
                .ptr_eq(&Arc::downgrade(&lane.body.identity))
            || self.control.state.load(Ordering::Acquire) & CLOSING == 0
        {
            return Err(refused());
        }
        Ok(())
    }

    /// OUTSIDE all writers. Wait only for the real affine end owner, which drops AFTER
    /// physical File cleanup and the original OS lease. Source death cannot fake a drain.
    /// Owning errors preserve this SAME closing observation, never reopen admissions.
    pub(in crate::store) fn join(self) -> Result<V2TextPromotionLaneDrained, (Self, AppError)> {
        let outcome = (|| -> Result<(), AppError> {
            let mut wait = self.control.wait.lock().map_err(|_| refused())?;
            while self.control.state.load(Ordering::Acquire) & ACTIVE != 0 {
                wait = self.control.changed.wait(wait).map_err(|_| refused())?;
            }
            if self.control.state.load(Ordering::Acquire) != CLOSING_IDLE {
                return Err(refused());
            }
            Ok(())
        })();
        match outcome {
            Err(error) => Err((self, error)),
            Ok(()) => Ok(V2TextPromotionLaneDrained {
                control: self.control,
                original: self.original,
            }),
        }
    }
}

impl V2TextPromotionLaneDrained {
    fn require_same_closed_lane(&self, lane: &V2TextLane) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.control, &lane.body.promotion)
            || !self
                .original
                .identity
                .ptr_eq(&Arc::downgrade(&lane.body.identity))
            || self.control.state.load(Ordering::Acquire) != CLOSING_IDLE
        {
            return Err(refused());
        }
        Ok(())
    }
}

impl V2TextOwnerNamespace<'_, '_> {
    /// Distinct successful-promotion removal. Normal `require_registered` is never
    /// weakened to accept absent/retired entries, and no caller-supplied count is trusted.
    pub(in crate::store) fn retire_promotion_lane_into(
        &mut self,
        caller: &mut Option<V2TextLane>,
        original: &OriginalV2TextPromotionObservation,
        drained: &V2TextPromotionLaneDrained,
        retired: &mut Option<RetiredV2TextPromotionLane>,
    ) -> Result<(), AppError> {
        self.validate_source_only()?;
        if let Some(previous) = retired.as_ref() {
            if caller.is_some() || !Arc::ptr_eq(&previous.caller.body, &previous.registered) {
                return Err(refused());
            }
            previous.registered.require_admission(self)?;
            previous.registered.promotion.require_plan(original)?;
            return drained.require_same_closed_lane(&previous.caller);
        }
        let lane = caller.as_ref().ok_or_else(refused)?;
        lane.require_registered(self)?;
        lane.body.promotion.require_plan(original)?;
        drained.require_same_closed_lane(lane)?;
        let runtime = self.text_lane_runtime_mut();
        let slots = &mut runtime.buffer.as_mut().ok_or_else(refused)?.slots;
        let index = slots
            .iter()
            .position(|slot| {
                slot.as_ref()
                    .is_some_and(|entry| Arc::ptr_eq(entry, &lane.body))
            })
            .ok_or_else(refused)?;
        // Closed admission is monotone and actual work is zero. All fallible checks have
        // completed: neither a disappearing weak owner nor a destructor frees this slot.
        let registered = slots[index].take().expect("same actual slot checked above");
        let caller = caller.take().expect("same unique caller checked above");
        *retired = Some(RetiredV2TextPromotionLane { caller, registered });
        Ok(())
    }
}

macro_rules! redacted_debug {
    ($($ty:ident),+ $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_debug!(
    V2TextPromotionWorkEnd,
    V2TextPromotionLaneDrain,
    V2TextPromotionLaneDrained,
    RetiredV2TextPromotionLane
);
