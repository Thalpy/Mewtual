//! Read one immutable decision through the intent root selected by this
//! exact head. The remote ordinal never grants authority over a file.
use super::super::super::super::delivery::ReviewDelivery;
use super::*;
use catcoms_replication::chat::cold_index::MAX_INDEX_HEIGHT;
use catcoms_replication::chat::owner_bodies::{
    ReviewDecision, ReviewStreamPosition, ReviewStreamQuery,
};

struct CoveredReviewRecord {
    record: IndexedDecisionRecord,
    path: Vec<HeldDecisionFile>,
}

impl CoveredReviewRecord {
    fn require_current(&mut self) -> Result<(), AppError> {
        for page in &mut self.path {
            page.require()?;
        }
        self.record.frame.require()
    }
}

impl IndexedDecisionSnapshot {
    pub(in super::super::super::super) fn read_public_record(
        &mut self,
        root: &HeldDirectory,
        (key, pin): (&JournalKey, &FounderGenesisAnchorData),
        query: ReviewStreamQuery,
        mut recheck: impl FnMut(Option<ReviewStreamPosition>) -> Result<(), AppError>,
        check_approval: impl FnOnce(&ReviewStreamRecord<'_>) -> Result<bool, AppError>,
    ) -> Result<Option<ReviewDelivery>, AppError> {
        require_quiet_selection(root)?;
        let sequence = query.sequence_data();
        if sequence == 0 {
            return Err(denied());
        }
        // All new entries use the public format. If the selected first
        // entry predates it, even later approvals remain local. The
        // missing signed case transitions cannot be inferred or skipped.
        let mut genesis = self.prove_delivery_record(key, pin, 1)?;
        if genesis.record.entry.case_step_data().is_none() {
            return Err(denied());
        }
        if sequence > self.selected.sequence {
            recheck(None)?;
            genesis.require_current()?;
            self.require_current(root)?;
            require_quiet_selection(root)?;
            return Ok(None);
        }

        let (mut selected, mut genesis) = if sequence == 1 {
            (genesis, None)
        } else {
            (
                self.prove_delivery_record(key, pin, sequence)?,
                Some(genesis),
            )
        };
        let record = &selected.record;
        let step = record.entry.case_step_data().ok_or_else(denied)?;
        let position = record.entry.position_data();
        recheck(Some(position))?;

        // The writer covers outputs and Head before retiring its WAL.
        // Serving repeats the witness checks and never runs recovery or
        // opens a file for writing on behalf of an authenticated peer.
        let result = match query {
            ReviewStreamQuery::Header(_) => {
                Some(ReviewDelivery::Header(Box::new(record.entry.clone())))
            }
            ReviewStreamQuery::Approval { entry, .. }
                if entry == record.entry.id() && step.choice_data() == ReviewDecision::Approve =>
            {
                let plain = RetainedDecisionRecord::open(key, &record.frame.bytes)?;
                let retained = RetainedDecisionRecord::decode(pin, &plain)?;
                check_approval(&retained.record)?
                    .then(|| ReviewDelivery::Approval(Zeroizing::new(retained.frame.to_vec())))
            }
            ReviewStreamQuery::Approval { .. } => None,
        };
        recheck(Some(position))?;
        if let Some(genesis) = &mut genesis {
            genesis.require_current()?;
        }
        selected.require_current()?;
        self.require_current(root)?;
        require_quiet_selection(root)?;
        Ok(result)
    }

    fn prove_delivery_record(
        &self,
        key: &JournalKey,
        pin: &FounderGenesisAnchorData,
        sequence: u64,
    ) -> Result<CoveredReviewRecord, AppError> {
        // The record supplies an internal lookup hint. Only an exact
        // match from the selected intent root proves the old frame belongs
        // to this journal. The private intent never leaves the store.
        let record = IndexedDecisionRecord::read(&self.records, key, pin, sequence)?;
        let [intent, _] = self.selected.roots.ok_or_else(denied)?;
        let query = index::DecisionIndexQuery::Intent(record.row.intent);
        let mut path = Vec::with_capacity(1 + MAX_INDEX_HEIGHT as usize);
        let row = key.indexes().lookup(intent, query, |id| {
            let page = self.read_page(id)?;
            let bytes = page.bytes.clone();
            path.push(page);
            Ok(bytes)
        })?;
        if row != Some(record.row) {
            return Err(denied());
        }
        Ok(CoveredReviewRecord { record, path })
    }
}

pub(in super::super::super::super) fn require_quiet_selection(
    root: &HeldDirectory,
) -> Result<(), AppError> {
    root.require()?;
    absent(&root.child(HEAD_STAGE))?;
    absent(&root.child(SELECTION_SCRATCH))?;
    absent(&root.child(SELECTION_WAL))?;
    root.require()
}
