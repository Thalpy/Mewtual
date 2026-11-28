//! Check the request against one authenticated intent and case root.
//! A saved nonce is resolved before consulting the current case head.
use super::*;
use crate::store::NativeTextInitialReviewRequest;
use catcoms_replication::chat::owner_bodies::ReviewStreamPosition;
use catcoms_replication::chat::owner_header::OwnerHeaderId;

pub(in super::super::super::super) enum IndexedReviewRequest {
    Saved(IndexedDecisionLookup),
    Superseded(IndexedReviewPredecessor),
    Next(IndexedReviewPredecessor),
}

pub(in super::super::super::super) struct IndexedReviewPredecessor {
    // The nonce must miss at the same selector used for the case.
    intent: IndexedDecisionLookup,
    case: IndexedDecisionLookup,
}

impl IndexedDecisionSnapshot {
    pub(in super::super::super::super) fn check_request(
        &mut self,
        root: &HeldDirectory,
        key: &JournalKey,
        pin: &FounderGenesisAnchorData,
        request: &NativeTextInitialReviewRequest,
    ) -> Result<IndexedReviewRequest, AppError> {
        let intent = self.lookup(
            root,
            key,
            pin,
            index::DecisionIndexQuery::Intent(request.intent),
        )?;
        if let Some(saved) = &intent.record {
            saved.require_request(request)?;
            return Ok(IndexedReviewRequest::Saved(intent));
        }
        let case = self.lookup(
            root,
            key,
            pin,
            index::DecisionIndexQuery::Case(request.case.id),
        )?;
        let actual = case.record.as_ref().map(|record| record.decision.id());
        if actual != request.previous {
            // An absent case cannot prove that a proposed predecessor has
            // been superseded. The same exact original must have a real,
            // independently authenticated selected case record already.
            case.record
                .as_ref()
                .ok_or_else(denied)?
                .require_case(request)?;
            let mut superseded = IndexedReviewPredecessor { intent, case };
            self.require_superseded(root, &mut superseded)?;
            return Ok(IndexedReviewRequest::Superseded(superseded));
        }
        if let Some(prior) = &case.record {
            prior.require_case(request)?;
        }
        let mut selected = IndexedReviewPredecessor { intent, case };
        selected.require_current()?;
        self.require_current(root)?;
        Ok(IndexedReviewRequest::Next(selected))
    }

    /// Keep the intent miss, current other case and actual Head on one
    /// selection. Any pending signing/selection evidence defeats the
    /// no-admission conclusion, even if it appears unrelated or damaged.
    pub(in super::super::super::super) fn require_superseded(
        &mut self,
        root: &HeldDirectory,
        proof: &mut IndexedReviewPredecessor,
    ) -> Result<(), AppError> {
        use super::super::super::super::control;
        control::require_control_names(root)?;
        control::require_no_signing_controls(root)?;
        require_quiet_selection(root)?;
        proof.require_current()?;
        self.require_current(root)?;
        control::require_control_names(root)?;
        control::require_no_signing_controls(root)?;
        require_quiet_selection(root)
    }

    pub(in super::super::super::super) fn next_position(
        &self,
        membership: (OwnerHeaderId, u64),
    ) -> Result<ReviewStreamPosition, AppError> {
        if membership.1 < self.tip.entry.position_data().membership_epoch {
            return Err(denied());
        }
        Ok(ReviewStreamPosition {
            sequence: self.selected.sequence.checked_add(1).ok_or_else(denied)?,
            previous: Some(self.tip.entry.id()),
            membership_header: membership.0,
            membership_epoch: membership.1,
        })
    }
}

impl IndexedReviewPredecessor {
    pub(in super::super::super::super) fn case_position(
        &self,
    ) -> Result<ReviewStreamPosition, AppError> {
        Ok(self
            .case
            .record
            .as_ref()
            .ok_or_else(denied)?
            .entry
            .position_data())
    }

    pub(in super::super::super::super) fn require_current(&mut self) -> Result<(), AppError> {
        self.intent.require_current()?;
        self.case.require_current()
    }

    pub(in super::super::super::super) fn case_data(
        &self,
    ) -> Option<(ReviewIndexValueData, &ExactReviewDecisionBody)> {
        self.case
            .record
            .as_ref()
            .map(|record| (record.row, &record.decision))
    }
}

impl IndexedDecisionRecord {
    fn require_request(&self, request: &NativeTextInitialReviewRequest) -> Result<(), AppError> {
        self.require_case(request)?;
        let spec = self.decision.decision().spec();
        if spec.intent != request.intent
            || spec.decision != request.decision
            || spec.previous != request.previous
            || self.row.request != *self.decision.id().as_bytes()
        {
            return Err(denied());
        }
        Ok(())
    }

    fn require_case(&self, request: &NativeTextInitialReviewRequest) -> Result<(), AppError> {
        let target = self.decision.decision().spec().target.spec();
        let exact = self.decision.original_data();
        if request.case.id != self.decision.case_id()
            || request.channel != target.channel
            || request.case.action != target.action
            || request.case.original != exact.digest
            || request.case.removal != (exact.remove_header, exact.remove_epoch)
        {
            return Err(denied());
        }
        Ok(())
    }
}
