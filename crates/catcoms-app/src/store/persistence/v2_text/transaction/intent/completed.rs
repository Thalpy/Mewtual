//! Closed owning I/nextA association before semantic codec dispatch. The initial
//! first-I verifier is unchanged; completed mode cannot invent arbitrary carry.
use super::*;
impl PreparedV2TextUnsignedIntent {
    pub(super) fn require_original_completed_carry(
        &self,
        carry: &RetirementCarryData,
    ) -> Result<(), AppError> {
        let original = RetirementCarryData::decode_data(&self.retained.carry)?;
        self.retained.require_completed_carry_data(&original)?;
        let mut roots = original.roots_data().to_vec();
        roots.push(TextHeadIdData::of_data(&self.parent.head_frame));
        roots.sort_unstable();
        if carry.roots_data() != roots || carry.entries_data() != original.entries_data() {
            return Err(refused());
        }
        Ok(())
    }
}
