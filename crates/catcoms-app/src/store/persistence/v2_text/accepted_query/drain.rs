//! Negative-only actual logical reader disposal OUTSIDE all writers. No resource,
//! source, root-close, drain, deletion or completion permission follows from None.
use super::*;
impl PreparedV2TextAcceptedQuery {
    /// Drop the SAME completed native registry lease; no caller count or synthetic
    /// End is accepted. All physical Source Files/OSlease already drained in the
    /// admitted query's finish_into before this completed lease could exist.
    pub(in crate::store::persistence::v2_text) fn drain_completed_reader_outside_writers(
        &mut self,
    ) {
        drop(self.completed_reader.take());
    }
}

impl OriginalV2TextAcceptedQueryObservation {
    /// Exact original selected-O DATA for the authoritative SAME-Runtime registry
    /// scan. A hash/absence here never supplies current or root-close authority.
    pub(in crate::store) fn selected_head_id_data(&self) -> TextHeadIdData {
        self.parent.selected_head
    }
}

#[cfg(all(test, windows))]
impl PreparedV2TextAcceptedQuery {
    /// Actual same registered reader's cached lifetime/closing proof only. No FS,
    /// Source/current proof or caller counts are substituted by this observation.
    pub(in crate::store::persistence::v2_text) fn completed_reader_live_for_test(&self) -> bool {
        self.completed_reader
            .as_ref()
            .is_some_and(|reader| reader.require_live_query(&self.observe_original()).is_ok())
    }
}
