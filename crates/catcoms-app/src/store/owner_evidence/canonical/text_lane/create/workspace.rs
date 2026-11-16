//! Actual readonly workspace custody and the distinct closed logical predecessor.
//!
//! A workspace read reserves an affine control end BEFORE Files open, but binds no
//! transaction. Only the genuine Clear issuer plus actual selected/query/idle proof can
//! bind a predecessor. Neither owner is an A stage, selector commit or signer permit.
use super::*;
mod signing;
mod write;
use crate::store::persistence::v2_text::OriginalV2RetainObservation;
use std::sync::OnceLock;
pub(in crate::store) use write::V2TextCreateControlWriteEnd;

pub(super) struct CreateWorkControl {
    active: AtomicBool,
    binding_busy: AtomicBool,
    original: OnceLock<OriginalV2RetainObservation>,
}
impl CreateWorkControl {
    /// Cached binding/work tail for an already removed ordinary lane. Unlike
    /// signing this does not require a live lane: retirement permanently cancels
    /// that caller. The genuine retired receipt separately proves cancellation
    /// and identity; this helper exposes no mutable work state or new authority.
    pub(super) fn require_bound_idle_negative(
        &self,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.original
            .get()
            .ok_or_else(refused)?
            .require_same_original(original)?;
        if self.active.load(Ordering::Acquire) {
            return Err(refused());
        }
        Ok(())
    }

    pub(super) fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            binding_busy: AtomicBool::new(false),
            original: OnceLock::new(),
        }
    }

    fn require_original_if_bound(
        &self,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        if let Some(bound) = self.original.get() {
            bound.require_same_original(original)?;
        }
        Ok(())
    }
}

/// Actual negative finite read ownership, including an UNSTARTED or failed Source
/// capture. The active bundle declares it after Source/key and OS lease. No general
/// constructor, Clone, caller count setter or Drop-based registry retirement exists.
pub(in crate::store) struct V2TextCreateWorkspaceEnd {
    control: Arc<CreateLaneControl>,
}
impl Drop for V2TextCreateWorkspaceEnd {
    fn drop(&mut self) {
        // Source/key, helper-local Files and physical lease have all already drained.
        let previous = self.control.work.active.swap(false, Ordering::AcqRel);
        debug_assert!(
            previous,
            "one actual end per same-lane workspace reservation"
        );
    }
}

/// Negative SAME-request serialization only. The canonical owner cannot construct a
/// Current-layout predecessor or a positive SDK admission from this claim. There is no
/// raw source/SDK signer, frame adopter, filesystem effect or mutable raw lane borrower.
pub(in crate::store) struct V2TextCreateBinding {
    original: OriginalV2RetainObservation,
    lane: OriginalV2TextCreateLaneObservation,
    control: Arc<CreateLaneControl>,
    // LAST: no second same-lane binding owner can allocate while this one's original/
    // observer metadata still drains. Same outside-slot reobservation uses no new end.
    _end: CreateBindingEnd,
}

struct CreateBindingEnd {
    control: Arc<CreateLaneControl>,
}
impl Drop for CreateBindingEnd {
    fn drop(&mut self) {
        let previous = self.control.work.binding_busy.swap(false, Ordering::AcqRel);
        debug_assert!(previous, "one affine logical binding per lane");
    }
}

pub(super) const CREATE_WORKSPACE_CONTROL_LAYOUT_BYTES: usize =
    std::mem::size_of::<V2TextCreateWorkspaceEnd>()
        + std::mem::size_of::<V2TextCreateControlWriteEnd>()
        + std::mem::size_of::<V2TextCreateBinding>()
        + std::mem::size_of::<CreateBindingEnd>()
        + 2 * std::mem::size_of::<OriginalV2RetainObservation>()
        + 2 * 64;

impl V2TextCreateLane {
    /// Negative observation only. No caller value is accepted by idle/admission proofs.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn workspace_active_for_test(&self) -> bool {
        self.control.work.active.load(Ordering::Acquire)
    }

    /// Pure final negative check, used BEFORE the physical Clear issuer opens ephemeral
    /// controls and again after every physical/account wait. Logical root readers may
    /// remain live; finite query/control IO is checked separately under the same writer.
    pub(in crate::store) fn require_workspace_idle(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.require_registered(namespace)?;
        self.control.work.require_original_if_bound(original)?;
        if self.control.work.active.load(Ordering::Acquire)
            || self
                .original_lane
                .body
                .control_write_active
                .load(Ordering::Acquire)
        {
            return Err(refused());
        }
        Ok(())
    }

    /// The actual guard just verified the SAME selected/query parent. Reserve no new
    /// permission here: this negative count only blocks other same-lane control work
    /// until the actual Source/native cleanup ends. A dirty workspace must never bind a
    /// new request permanently, so the original-plan OnceLock is deliberately untouched.
    pub(in crate::store) fn reserve_workspace_capture_end(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
        original: &OriginalV2RetainObservation,
    ) -> Result<V2TextCreateWorkspaceEnd, AppError> {
        self.require_workspace_idle(namespace, original)?;
        self.control
            .work
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        Ok(V2TextCreateWorkspaceEnd {
            control: self.control.clone(),
        })
    }

    /// Negative same-plan serialization only. This canonical owner cannot construct the
    /// Current-layout Bound predecessor, which separately requires the concrete physical
    /// ReadyClear/current/query proof. No SDK/signing/effect permission follows here.
    pub(in crate::store) fn bind_create_original(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
        original: &OriginalV2RetainObservation,
    ) -> Result<V2TextCreateBinding, AppError> {
        self.require_workspace_idle(namespace, original)?;
        self.control
            .work
            .binding_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        // The actual canonical writer serializes this immutable first binding. Existing
        // same-original reobservation produces only the same negative claim, never a new
        // physical/predecessor permission. The caller caches its paired role immediately.
        if self.control.work.original.get().is_none() {
            self.control
                .work
                .original
                .set(original.clone())
                .expect("same canonical writer serializes first original binding");
        }
        Ok(V2TextCreateBinding {
            original: original.clone(),
            lane: self.observe_original(),
            control: self.control.clone(),
            _end: CreateBindingEnd {
                control: self.control.clone(),
            },
        })
    }
}

impl V2TextCreateWorkspaceEnd {
    pub(in crate::store) fn require_same_lane(
        &self,
        lane: &V2TextCreateLane,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.control, &lane.control)
            || !self.control.work.active.load(Ordering::Acquire)
        {
            return Err(refused());
        }
        Ok(())
    }
}

impl V2TextCreateBinding {
    pub(in crate::store) fn require_same_original(
        &self,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)
    }

    pub(in crate::store) fn require_same_lane_original(
        &self,
        lane: &V2TextCreateLane,
        original: &OriginalV2RetainObservation,
    ) -> Result<(), AppError> {
        self.require_same_original(original)?;
        self.lane.require_same_original(lane)?;
        if !Arc::ptr_eq(&self.control, &lane.control) {
            return Err(refused());
        }
        self.control
            .work
            .original
            .get()
            .ok_or_else(refused)?
            .require_same_original(original)
    }
}

impl std::fmt::Debug for V2TextCreateWorkspaceEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextCreateWorkspaceEnd")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for V2TextCreateBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextCreateBinding")
            .finish_non_exhaustive()
    }
}
