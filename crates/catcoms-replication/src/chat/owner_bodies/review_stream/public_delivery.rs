//! Check what a recipient can verify without reading private decisions.
use super::*;

#[cfg(test)]
mod tests;

/// The sequence is group-wide. A body request also pins the selected
/// semantic entry, so following a different branch never releases text.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReviewStreamQuery {
    Header(u64),
    Approval {
        sequence: u64,
        entry: ReviewStreamEntryId,
    },
}

impl ReviewStreamQuery {
    pub fn sequence_data(self) -> u64 {
        match self {
            Self::Header(sequence) | Self::Approval { sequence, .. } => sequence,
        }
    }
}

catcoms_rt::redacted_debug! { ReviewStreamQuery; }

impl ReviewStreamEntry {
    /// Check one position against the independently selected global tip
    /// and that case's own tip. The caller still proves actual membership
    /// ancestry and covers both tips before it selects any new effect.
    pub fn verify_public_next_for_pin(
        &self,
        trusted: &FounderGenesisAnchorData,
        previous: Option<&Self>,
        previous_case: Option<(OwnerBodyId, ReviewCaseStep)>,
    ) -> Result<ReviewCaseStep, ChatError> {
        self.verify_next_for_pin(trusted, previous)?;
        let step = self.case_step_data().ok_or(ChatError::Dependency)?;
        if previous.is_some_and(|entry| entry.case_step_data().is_none()) {
            return Err(ChatError::Dependency);
        }
        if previous_case.is_some_and(|(id, _)| id == self.decision_id_data()) {
            return Err(ChatError::Dependency);
        }
        step.verify_transition(previous_case)?;
        Ok(step)
    }
}

impl<'a> ReviewStreamRecord<'a> {
    /// Verify a downloaded approval against an independently selected
    /// public position. A matching signature supplies no local archive
    /// or membership authority by itself.
    pub fn decode_public_approval(
        trusted: &FounderGenesisAnchorData,
        selected: ReviewStreamEntryId,
        raw: &'a [u8],
    ) -> Result<Self, ChatError> {
        let exact = Self::decode(trusted, raw)?;
        let entry = exact.entry_data();
        let step = entry.case_step_data().ok_or(ChatError::Dependency)?;
        if entry.id() != selected || step.choice_data() != ReviewDecision::Approve {
            return Err(ChatError::Scope);
        }
        Ok(exact)
    }
}
