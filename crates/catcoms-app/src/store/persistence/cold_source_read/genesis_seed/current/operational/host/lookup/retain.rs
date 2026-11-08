//! Owning transfer into the actual A cell, declared with that concrete consumer.
use super::*;
use crate::store::persistence::v2_text::SelectedV2TextHead;

impl PendingTokenLookup {
    /// The original completed query, including its logical root reader, must move with
    /// the transaction. A parsed None cannot replace it. All refusal checks precede the
    /// mutable slot loan; the Physical constructor checks again before taking either
    /// this value or the packet/SDK custody. No Current or writer guard is retained.
    pub(in super::super) fn retain_inputs(
        &mut self,
    ) -> Result<(&SelectedV2TextHead, &mut Option<PreparedV2TextTokenQuery>), AppError> {
        if self.admitted.is_some() {
            return Err(refused());
        }
        let query = self.query.as_ref().ok_or_else(refused)?;
        if !query.has_completed_data() || query.token_value_data()?.is_some() {
            return Err(refused());
        }
        let selected = self.parent.selected_data().ok_or_else(refused)?;
        query.require_token_parent_data(selected, self.token)?;
        Ok((selected, &mut self.query))
    }
}
