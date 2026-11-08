//! Receiver-only actual second-A loan. No parsed token, old WC label or arbitrary
//! Header/model can create this. Root must finish fresh sameA+acceptedquery proof
//! before moving its SAME resident owner into new paid SDK unsigned custody.
use super::super::*;
use crate::store::persistence::OriginalV2TextResidentObservation;
pub(in crate::store) struct NextRetainedAResidentSourceData<'a> {
    original: &'a OriginalV2TextResidentObservation,
    saved: &'a Arc<SavedV2RetainedAction>,
}
impl PreparedV2TextRetainTransaction {
    pub(in crate::store) fn borrow_next_resident_unsigned_source_data(
        &self,
    ) -> Result<NextRetainedAResidentSourceData<'_>, AppError> {
        let origin = self.completed.as_ref().ok_or_else(refused)?;
        let parent = self.parent.completed.as_ref().ok_or_else(refused)?;
        origin
            .original
            .require_same_original(&parent.observe_completed_original())?;
        let saved = self.saved.as_ref().ok_or_else(refused)?;
        saved
            .original
            .require_same_original(&self.observe_original())?;
        if !Arc::ptr_eq(&saved.parent, &self.parent)
            || saved.selected.as_slice()
                != self.candidate.as_ref().ok_or_else(refused)?.frame_data()
        {
            return Err(refused());
        }
        // Parent's constructor came ONLY from the actual completed Bound/Ready and
        // these owned A bytes came only from the actual original SDK custody.
        Ok(NextRetainedAResidentSourceData {
            original: origin.original.original_resident(),
            saved,
        })
    }
}
impl NextRetainedAResidentSourceData<'_> {
    pub(in crate::store) fn original_resident(&self) -> &OriginalV2TextResidentObservation {
        self.original
    }
    pub(in crate::store) fn saved_data(&self) -> &Arc<SavedV2RetainedAction> {
        self.saved
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        self.saved.resources()
    }
}
