//! Actual finite control-writer exclusion, separate from readonly workspace ownership.
//!
//! The typed Current A stage/commit admission verifies its real physical issuer first.
//! This final native edge atomically excludes NEW query captures and actual same-lane
//! control work. IO-zero alone never constructs an effect permit, Saved or SDK grant.
use super::*;

/// Affine owner of both actual writer gate and cap1 control reservation. The consuming
/// stage/commit marker declares it LAST after ALL physical children and OS-session lease.
/// An UNSTARTED admitted marker also owns the gate; Drop never retires durable lane debt.
pub(in crate::store) struct V2TextCreateControlWriteEnd {
    control: Arc<CreateLaneControl>,
    lane: Arc<LaneBody>,
    #[cfg(all(test, windows))]
    after_gate_clear: Option<Box<dyn FnOnce() + Send>>,
}

impl V2TextCreateLane {
    /// Called under the SAME canonical Current writer only AFTER complete typed physical
    /// stage/Ready proof and final source negatives. No account/FS/codec wait may follow
    /// reservation before its actual marker is cached in the outside owning slot.
    pub(in crate::store) fn reserve_create_control_write_end(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
        original: &OriginalV2RetainObservation,
    ) -> Result<V2TextCreateControlWriteEnd, AppError> {
        self.require_workspace_idle(namespace, original)?;
        // A negative workspace read never sets this original. Only its separately
        // proved native predecessor can have established this SAME immutable binding.
        self.control
            .work
            .original
            .get()
            .ok_or_else(refused)?
            .require_same_original(original)?;
        namespace.require_text_control_io_quiescent()?;
        // Query admissions require this SAME canonical lock. Existing IO may only
        // finish/decrement; no new Source can slip between IO-zero and gate install.
        self.control
            .work
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        let previous = self
            .original_lane
            .body
            .control_write_active
            .swap(true, Ordering::AcqRel);
        debug_assert!(
            !previous,
            "same canonical writer serializes actual gate install"
        );
        Ok(V2TextCreateControlWriteEnd {
            control: self.control.clone(),
            lane: self.original_lane.body.clone(),
            #[cfg(all(test, windows))]
            after_gate_clear: None,
        })
    }
}

impl V2TextCreateControlWriteEnd {
    /// Deterministic actual cleanup interleaving only. The marker has already drained
    /// Files/OSlease before this hook; no production callback or authority is introduced.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_after_gate_clear_for_test(
        &mut self,
        hook: Box<dyn FnOnce() + Send>,
    ) {
        self.after_gate_clear = Some(hook);
    }

    /// Cached exact actual-control observation only. No source/SDK/FS or positive
    /// Current reacquisition is performed; already admitted finite work may end after Stop.
    pub(in crate::store) fn require_same_lane(
        &self,
        lane: &V2TextCreateLane,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.control, &lane.control)
            || !Arc::ptr_eq(&self.lane, &lane.original_lane.body)
            || !self.control.work.active.load(Ordering::Acquire)
            || !self.lane.control_write_active.load(Ordering::Acquire)
        {
            return Err(refused());
        }
        Ok(())
    }
}

impl Drop for V2TextCreateControlWriteEnd {
    fn drop(&mut self) {
        // The typed marker/finite worker has already closed every File, native helper,
        // key and physical lease. New readonly captures may start once this flag clears.
        // The cap remains active until the next line, so another writer still cannot
        // race that tiny transition; its own IO-zero check includes any new capture.
        let previous_gate = self.lane.control_write_active.swap(false, Ordering::AcqRel);
        #[cfg(all(test, windows))]
        if let Some(hook) = self.after_gate_clear.take() {
            hook();
        }
        let previous_work = self.control.work.active.swap(false, Ordering::AcqRel);
        debug_assert!(
            previous_gate && previous_work,
            "one affine finite writer end"
        );
    }
}

impl std::fmt::Debug for V2TextCreateControlWriteEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextCreateControlWriteEnd")
            .finish_non_exhaustive()
    }
}
