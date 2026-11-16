//! Negative DATA recovery from the exact original SDK preparation. Declared with
//! Root's real candidate consumer; this never signs, derives or authorizes storage.
use super::*;

impl PreparedV2AppendData {
    /// Borrow only an already cached complete Header and its original immutable DATA,
    /// including after a failed post-materialization Source check. Opaque identity is
    /// checked BEFORE presence; neither equal bytes nor an unrelated plan is adopted.
    /// No Source/node/key, verification, encoding, replay or allocation occurs here.
    pub fn borrow_existing_materialized_append_data(
        &self,
        original_plan: &V2AppendPlanObservation,
    ) -> Result<V2AppendDataView<'_>, V2CreateRefusal> {
        if !original_plan.matches_original_data(self) {
            return Err(V2CreateRefusal::RequestMismatch);
        }
        self.body.raw.as_ref().ok_or(V2CreateRefusal::Terminal)?;
        let header = self.body.header.as_ref().ok_or(V2CreateRefusal::Terminal)?;
        Ok(V2AppendDataView {
            action: &self.body.action,
            record: &self.body.record,
            body: &self.body.body,
            header,
            image: self.body.unsigned.image_data(),
        })
    }
}
