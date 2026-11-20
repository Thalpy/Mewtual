//! Actual finite O/H capture cleanup, separate from retained logical root readers.
//!
//! Both owners are reserved under the SAME canonical edge before Source opens. The
//! active bundle drops Source/key, physical OS lease, then this end; only its logical
//! reader survives in a completed result. IO-zero is NOT a WD or writer admission.
use super::*;

pub(in crate::store) struct V2TextQueryControlIoEnd {
    generation: Arc<QueryGeneration>,
}

impl QueryGeneration {
    /// The caller has just reserved this SAME generation's actual root pin under the
    /// canonical writer. IO cannot exceed its64 affine reader reservations. There is no
    /// fallible gap between reserving the two owners or caching them in outside custody.
    pub(super) fn reserve_control_io_end(self: &Arc<Self>) -> V2TextQueryControlIoEnd {
        let previous = self.control_io.fetch_add(1, Ordering::AcqRel);
        debug_assert!(previous < MAX_QUERY_READERS);
        V2TextQueryControlIoEnd {
            generation: self.clone(),
        }
    }
}

impl V2TextQueryControlIoEnd {
    pub(super) fn require_same_reader(&self, reader: &V2TextQueryLease) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.generation, &reader.generation)
            || self.generation.control_io.load(Ordering::Acquire) == 0
        {
            return Err(refused());
        }
        Ok(())
    }
}

impl Drop for V2TextQueryControlIoEnd {
    fn drop(&mut self) {
        // Actual Source/native child cleanup and OS unlock happened BEFORE this field.
        // The wait mutex/notification contains no user callback or physical cleanup.
        let _wait = self
            .generation
            .wait
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = self.generation.control_io.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "one affine end per actual capture");
        self.generation.changed.notify_all();
    }
}

impl V2TextOwnerNamespace<'_, '_> {
    /// Pure final negative check under the actual canonical writer. Future typed stage/
    /// commit admission must combine this IO-zero proof with installing its SAME active
    /// control-writer gate in that edge. Unlocking between check and gate would race a
    /// new Source capture. Logical completed-query readers intentionally need NOT be zero.
    pub(in crate::store) fn require_text_control_io_quiescent(&self) -> Result<(), AppError> {
        self.validate_source_only()?;
        let admission = self.scope_admission();
        if self
            .text_lane_runtime()
            .queries
            .as_ref()
            .is_some_and(|buffer| {
                buffer.slots.iter().flatten().any(|generation| {
                    generation.key.group() == admission.group()
                        && &generation.key.signer == admission.signer()
                        && generation.control_io.load(Ordering::Acquire) != 0
                })
            })
        {
            return Err(refused());
        }
        self.validate_source_only()
    }

    /// A new capture cannot open an obsolete control after a typed finite writer was
    /// admitted. Only actual later stage/commit marker ownership may set this gate; the
    /// original raw lane constructor initializes it inactive and grants no writer work.
    pub(in crate::store) fn require_text_control_write_idle(&self) -> Result<(), AppError> {
        self.validate_source_only()?;
        let admission = self.scope_admission();
        if self
            .text_lane_runtime()
            .buffer
            .as_ref()
            .is_some_and(|buffer| {
                buffer.slots.iter().flatten().any(|lane| {
                    lane.key.group() == admission.group()
                        && &lane.key.signer == admission.signer()
                        && lane.control_write_active.load(Ordering::Acquire)
                })
            })
        {
            return Err(refused());
        }
        self.validate_source_only()
    }
}

impl std::fmt::Debug for V2TextQueryControlIoEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextQueryControlIoEnd")
            .finish_non_exhaustive()
    }
}

/// Negative observation of ACTUAL finite IO ends; never accepted as a caller proof.
#[cfg(all(test, windows))]
pub(in crate::store) fn text_query_control_io_for_test(store: &ServerStore) -> usize {
    let canonical = store.owner_evidence.canonical.lock().unwrap();
    canonical.text_lanes.queries.as_ref().map_or(0, |buffer| {
        buffer
            .slots
            .iter()
            .flatten()
            .map(|generation| generation.control_io.load(Ordering::Acquire))
            .sum()
    })
}
