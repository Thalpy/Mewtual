//! Exact closed-root proof after the distinct ordinary lane entry has retired.
//! Normal root/admission APIs continue to require a registered live lane; only
//! this concrete opaque removed-lane owner may validate its SAME historical drain.
use super::*;
use crate::store::owner_evidence::canonical::text_lane::RetiredV2TextCreateLane;

impl V2TextOwnerNamespace<'_, '_> {
    pub(in crate::store) fn require_text_retirement_roots_after_lane_retired(
        &self,
        retired: &RetiredV2TextCreateLane,
        drained: &V2TextRetirementRootsDrained,
    ) -> Result<(), AppError> {
        let state = &drained.state;
        retired.require_lane_observation(&state.lane, &state.original_retain)?;
        retired.require_original(
            &state.original,
            &state.original_retain,
            &state.plan,
            &state.basis,
        )?;
        let admission = self.scope_admission();
        if !state.detached
            || !state.drained()
            || admission.group() != &state.group[..state.group_len]
            || admission.signer() != &state.signer
            || admission.source().data().source != state.source
            || self
                .text_lane_runtime()
                .queries
                .as_ref()
                .is_some_and(|buffer| {
                    buffer
                        .slots
                        .iter()
                        .flatten()
                        .any(|entry| state.matches(entry))
                })
        {
            return Err(refused());
        }
        self.validate_source_only()
    }
}
