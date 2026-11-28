//! A single physical Head and its authenticated, directed record reads.
//! The journal owner holds this selection through any successor attempt.
use super::*;
use crate::store::persistence::v2_text::private_index::ReviewIndexValueData;

mod preflight;
mod write;
pub(in super::super::super) use preflight::IndexedReviewPredecessor;
pub(in super::super::super) use preflight::IndexedReviewRequest;

pub(in super::super::super) struct IndexedDecisionSnapshot {
    head: HeldDecisionFile,
    selected: DecisionHead,
    records: HeldDirectory,
    tip: IndexedDecisionRecord,
}

pub(in super::super::super) struct IndexedDecisionRecord {
    frame: HeldDecisionFile,
    pub(in super::super::super) row: ReviewIndexValueData,
    pub(in super::super::super) entry: ReviewStreamEntry,
    pub(in super::super::super) decision: ExactReviewDecisionBody,
}

pub(in super::super::super) struct IndexedDecisionLookup {
    pub(in super::super::super) record: Option<IndexedDecisionRecord>,
    // Authenticated pages opened for this observed miss or hit. A live
    // writer holds these identities until its final predecessor check.
    path: Vec<HeldDecisionFile>,
}

impl IndexedDecisionSnapshot {
    pub(in super::super::super) fn read(
        root: &HeldDirectory,
        key: &JournalKey,
        pin: &FounderGenesisAnchorData,
    ) -> Result<Self, AppError> {
        root.require()?;
        let head =
            HeldDecisionFile::read(root.child(HEAD_CURRENT), HEAD_BYTES)?.ok_or_else(denied)?;
        let selected = DecisionHead::open(key, &head.bytes)?;
        selected.roots.ok_or_else(denied)?;
        let records = HeldDirectory::open(root.child(RECORD_DIRECTORY))?;
        let tip = IndexedDecisionRecord::read(&records, key, pin, selected.sequence)?;
        selected.require_record(&tip.entry, &tip.frame.bytes)?;
        let mut snapshot = Self {
            head,
            selected,
            records,
            tip,
        };
        snapshot.require_current(root)?;
        Ok(snapshot)
    }

    pub(in super::super::super) fn selected_data(
        &self,
    ) -> (DecisionHead, &[u8], &ReviewStreamEntry) {
        (self.selected, &self.head.bytes, &self.tip.entry)
    }

    pub(in super::super::super) fn signing_base(
        &mut self,
        root: &HeldDirectory,
    ) -> Result<(HeldDecisionFile, u64), AppError> {
        self.require_current(root)?;
        let next = self.selected.sequence.checked_add(1).ok_or_else(denied)?;
        Ok((self.head.try_clone()?, next))
    }

    pub(in super::super::super) fn records_directory(&self) -> Result<HeldDirectory, AppError> {
        self.records.try_clone()
    }

    pub(in super::super::super) fn lookup(
        &mut self,
        root: &HeldDirectory,
        key: &JournalKey,
        pin: &FounderGenesisAnchorData,
        query: index::DecisionIndexQuery,
    ) -> Result<IndexedDecisionLookup, AppError> {
        self.require_current(root)?;
        let [intent, case] = self.selected.roots.ok_or_else(denied)?;
        let pointer = match query {
            index::DecisionIndexQuery::Intent(_) => intent,
            index::DecisionIndexQuery::Case(_) => case,
        };
        let mut path = Vec::with_capacity(
            1 + catcoms_replication::chat::cold_index::MAX_INDEX_HEIGHT as usize,
        );
        let row = key.indexes().lookup(pointer, query, |id| {
            let page = self.read_page(id)?;
            let bytes = page.bytes.clone();
            path.push(page);
            Ok(bytes)
        })?;
        let record = row
            .map(|row| {
                if row.sequence > self.selected.sequence {
                    return Err(denied());
                }
                let record = IndexedDecisionRecord::read(&self.records, key, pin, row.sequence)?;
                if record.row != row {
                    return Err(denied());
                }
                Ok(record)
            })
            .transpose()?;
        let mut found = IndexedDecisionLookup { record, path };
        self.require_current(root)?;
        found.require_current()?;
        Ok(found)
    }

    pub(in super::super::super) fn require_current(
        &mut self,
        root: &HeldDirectory,
    ) -> Result<(), AppError> {
        root.require()?;
        self.records.require()?;
        self.tip.frame.require()?;
        self.head.require()?;
        root.require()
    }

    fn read_page(&self, id: PrivateChatPageId) -> Result<HeldDecisionFile, AppError> {
        self.records.require()?;
        HeldDecisionFile::read(self.records.child(&index_name(id)), MAX_INDEX_FRAME_BYTES)?
            .ok_or_else(denied)
    }
}

impl IndexedDecisionRecord {
    fn read(
        directory: &HeldDirectory,
        key: &JournalKey,
        pin: &FounderGenesisAnchorData,
        sequence: u64,
    ) -> Result<Self, AppError> {
        let mut frame = HeldDecisionFile::read(
            directory.child(&format!("{sequence:016x}.record")),
            MAX_RECORD_BYTES,
        )?
        .ok_or_else(denied)?;
        let plain = RetainedDecisionRecord::open(key, &frame.bytes)?;
        let record = RetainedDecisionRecord::decode(pin, &plain)?;
        let row = index::DecisionIndexes::row(&record, &frame.bytes);
        if row.sequence != sequence {
            return Err(denied());
        }
        let entry = record.record.entry_data().clone();
        let decision = record.record.decision_data().clone();
        frame.require()?;
        directory.require()?;
        Ok(Self {
            frame,
            row,
            entry,
            decision,
        })
    }
}

impl IndexedDecisionLookup {
    pub(in super::super::super) fn require_current(&mut self) -> Result<(), AppError> {
        for page in &mut self.path {
            page.require()?;
        }
        if let Some(record) = &mut self.record {
            record.frame.require()?;
        }
        Ok(())
    }
}
