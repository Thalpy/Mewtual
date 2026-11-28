//! Publish only the verified append outputs under the original head.
use super::super::super::append::IndexedAppend;
use super::*;

pub(in super::super::super::super) struct CoveredAppendFiles {
    predecessor: IndexedDecisionRecord,
    selected_intent: IndexedDecisionLookup,
    selected_case: IndexedDecisionLookup,
    pages: Vec<HeldDecisionFile>,
}

impl IndexedDecisionSnapshot {
    pub(in super::super::super::super) fn select_append(
        &mut self,
        root: &HeldDirectory,
        wal: &mut HeldDecisionFile,
        append: &IndexedAppend<'_>,
        selected: DecisionHead,
        mut require_source: impl FnMut() -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        if wal.path != root.child(SELECTION_WAL)
            || append.predecessor != self.head.bytes.as_slice()
            || self.selected.sequence.checked_add(1) != Some(selected.sequence)
        {
            return Err(denied());
        }
        require_source()?;
        self.require_current(root)?;
        absent(&root.child(SELECTION_SCRATCH))?;
        wal.sync_complete()?;
        root.sync()?;
        wal.require()?;

        let name = format!("{:016x}.record", selected.sequence);
        let mut record = HeldDecisionFile::cover(
            self.records.child(&name),
            append.record,
            MAX_RECORD_BYTES,
            false,
        )?;
        let page_count = append
            .changes
            .iter()
            .map(|change| change.outputs.len())
            .sum();
        let mut pages = Vec::with_capacity(page_count);
        for changes in &append.changes {
            for (reference, bytes) in &changes.outputs {
                self.records.require()?;
                wal.require()?;
                let path = self.records.child(&index_name(reference.page));
                pages.push(HeldDecisionFile::cover(
                    path,
                    bytes,
                    MAX_INDEX_FRAME_BYTES,
                    false,
                )?);
            }
        }
        self.records.sync()?;
        let stage_path = root.child(HEAD_STAGE);
        let head_path = root.child(HEAD_CURRENT);
        let mut stage = ensure_exact_held(&stage_path, append.head)?;
        root.sync()?;

        // The callback can block. Reprove the exact captured nodes and
        // both selector names afterward, immediately before rename.
        require_source()?;
        self.require_current(root)?;
        wal.require()?;
        record.require()?;
        for page in &mut pages {
            page.require()?;
        }
        absent(&root.child(SELECTION_SCRATCH))?;
        require_exact_held(&stage_path, &mut stage, append.head)?;
        self.head.require()?;
        install_catalog_stage(&stage_path, &head_path).map_err(io)?;
        #[cfg(test)]
        if take_fault(ReviewControlFault::AfterSelectionHeadRename) {
            return Err(AppError::Io(
                "injected stop after review append head rename".into(),
            ));
        }

        absent(&stage_path)?;
        require_exact_held(&head_path, &mut stage, append.head)?;
        sync_selected_catalog(&stage.0).map_err(io)?;
        root.sync()?;
        require_source()?;
        self.records.require()?;
        record.require()?;
        for page in &mut pages {
            page.require()?;
        }
        wal.require()?;
        require_exact_held(&head_path, &mut stage, append.head)?;
        root.require()
    }

    pub(in super::super::super::super) fn cover_committed_append(
        &mut self,
        root: &HeldDirectory,
        key: &JournalKey,
        pin: &FounderGenesisAnchorData,
        append: &IndexedAppend<'_>,
    ) -> Result<CoveredAppendFiles, AppError> {
        let (selected, row) = append.verify_committed(key, pin)?;
        if selected != self.selected
            || self.head.bytes.as_slice() != append.head
            || self.tip.frame.bytes.as_slice() != append.record
            || self.tip.row != row
        {
            return Err(denied());
        }
        let previous = DecisionHead::open(key, append.predecessor)?;
        let predecessor = IndexedDecisionRecord::read(&self.records, key, pin, previous.sequence)?;
        previous.require_record(&predecessor.entry, &predecessor.frame.bytes)?;
        self.tip
            .entry
            .verify_next_for_pin(pin, Some(&predecessor.entry))
            .map_err(|_| denied())?;
        let selected_intent = self.lookup(
            root,
            key,
            pin,
            index::DecisionIndexQuery::Intent(row.intent),
        )?;
        let selected_case =
            self.lookup(root, key, pin, index::DecisionIndexQuery::Case(row.case))?;
        if selected_intent.record.as_ref().map(|record| record.row) != Some(row)
            || selected_case.record.as_ref().map(|record| record.row) != Some(row)
        {
            return Err(denied());
        }
        let mut pages = Vec::with_capacity(
            append
                .changes
                .iter()
                .map(|change| change.outputs.len())
                .sum(),
        );
        for change in &append.changes {
            for (reference, bytes) in &change.outputs {
                self.records.require()?;
                let path = self.records.child(&index_name(reference.page));
                pages.push(HeldDecisionFile::cover(
                    path,
                    bytes,
                    MAX_INDEX_FRAME_BYTES,
                    true,
                )?);
            }
        }
        self.tip.frame.sync_complete()?;
        self.records.sync()?;
        self.head.sync_complete()?;
        root.sync()?;
        let mut files = CoveredAppendFiles {
            predecessor,
            selected_intent,
            selected_case,
            pages,
        };
        files.require_current(self, root)?;
        Ok(files)
    }

    pub(in super::super::super::super) fn retire_old_index_pages(
        &mut self,
        root: &HeldDirectory,
        key: &JournalKey,
        append: &IndexedAppend<'_>,
        mut recheck: impl FnMut(&mut Self) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        use crate::store::persistence::v2_text::private_index::PrivateIndexKindData;
        use catcoms_replication::chat::cold_index::{NodePlacementData, MAX_INDEX_HEIGHT};

        let indexes = key.indexes();
        let selected = self.selected.roots.ok_or_else(denied)?;
        let kinds = [
            PrivateIndexKindData::ReviewIntent,
            PrivateIndexKindData::ReviewCase,
        ];
        for ((replacement, kind), change) in selected.into_iter().zip(kinds).zip(&append.changes) {
            for (depth, old) in change.old_path.iter().copied().enumerate() {
                let mut pages = Vec::with_capacity(MAX_INDEX_HEIGHT as usize + 1);
                indexes.require_unreachable(replacement, kind, old, |page| {
                    let file = self.read_page(page)?;
                    let bytes = file.bytes.clone();
                    pages.push(file);
                    Ok(bytes)
                })?;
                let path = self.records.child(&index_name(old.page));
                let obsolete = HeldDecisionFile::read(path.clone(), MAX_INDEX_FRAME_BYTES)?;
                if let Some(held) = &obsolete {
                    let placement = if depth == 0 {
                        NodePlacementData::Root
                    } else {
                        NodePlacementData::Child
                    };
                    indexes.require_old_page(kind, placement, old, &held.bytes)?;
                }
                recheck(self)?;
                self.require_current(root)?;
                for page in &mut pages {
                    page.require()?;
                }
                if let Some(held) = obsolete {
                    held.retire(&self.records)?;
                    #[cfg(test)]
                    if take_fault(ReviewControlFault::AfterRetiringIndexPage) {
                        return Err(AppError::Io(
                            "injected stop after retiring a review index page".into(),
                        ));
                    }
                } else {
                    absent(&path)?;
                }
            }
        }
        Ok(())
    }
}

impl CoveredAppendFiles {
    pub(in super::super::super::super) fn require_current(
        &mut self,
        snapshot: &mut IndexedDecisionSnapshot,
        root: &HeldDirectory,
    ) -> Result<(), AppError> {
        snapshot.require_current(root)?;
        absent(&root.child(HEAD_STAGE))?;
        absent(&root.child(SELECTION_SCRATCH))?;
        self.predecessor.frame.require()?;
        self.selected_intent.require_current()?;
        self.selected_case.require_current()?;
        for page in &mut self.pages {
            page.require()?;
        }
        snapshot.require_current(root)
    }
}
