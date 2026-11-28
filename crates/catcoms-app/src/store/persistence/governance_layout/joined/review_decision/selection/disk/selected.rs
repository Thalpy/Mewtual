//! Read the independently selected immutable record and its two roots.
//! This is used after signing controls have started to retire.
use super::*;

pub(in super::super::super) struct FirstDecisionFiles {
    head: HeldDecisionFile,
    records: HeldDirectory,
    record: HeldDecisionFile,
    pages: [HeldDecisionFile; 2],
}

impl FirstDecisionFiles {
    pub(in super::super::super) fn read(
        root: &HeldDirectory,
        key: &JournalKey,
    ) -> Result<Option<Self>, AppError> {
        root.require()?;
        let Some(head) = HeldDecisionFile::read(root.child(HEAD_CURRENT), HEAD_BYTES)? else {
            return Ok(None);
        };
        if head.bytes.as_slice() == key.initial_head() {
            return Ok(None);
        }
        let selected = DecisionHead::open(key, &head.bytes)?;
        if selected.sequence != 1 {
            return Ok(None);
        }
        let Some(roots) = selected.roots else {
            // The original first-decision journal still uses its complete
            // signing controls and matching write-ahead file for recovery.
            return Ok(None);
        };
        absent(&root.child(HEAD_STAGE))?;
        absent(&root.child(SELECTION_SCRATCH))?;
        let records = HeldDirectory::open(root.child(RECORD_DIRECTORY))?;
        let record =
            HeldDecisionFile::read(records.child("0000000000000001.record"), MAX_RECORD_BYTES)?
                .ok_or_else(denied)?;
        let read_page = |id| {
            HeldDecisionFile::read(records.child(&index_name(id)), MAX_INDEX_FRAME_BYTES)?
                .ok_or_else(denied)
        };
        let pages = [read_page(roots[0].page)?, read_page(roots[1].page)?];
        let mut files = Self {
            head,
            records,
            record,
            pages,
        };
        files.require_current(root)?;
        Ok(Some(files))
    }

    /// Membership and final native publication are the caller's separate
    /// obligations. This read checks the selected head and data, including
    /// the actual encrypted leaf frames that head refers to.
    pub(in super::super::super) fn with_record<T>(
        &self,
        key: &JournalKey,
        pin: &FounderGenesisAnchorData,
        use_record: impl FnOnce(&RetainedDecisionRecord<'_>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let head = DecisionHead::open(key, &self.head.bytes)?;
        let roots = head.roots.ok_or_else(denied)?;
        let plain = RetainedDecisionRecord::open(key, &self.record.bytes)?;
        let record = RetainedDecisionRecord::decode(pin, &plain)?;
        head.require_record(record.record.entry_data(), &self.record.bytes)?;
        head.require_predecessor(record.record.entry_data(), None)?;
        key.indexes().require_first(
            roots,
            self.pages.each_ref().map(|page| page.bytes.as_slice()),
            &record,
            &self.record.bytes,
        )?;
        use_record(&record)
    }

    pub(in super::super::super) fn require_wal_outputs(
        &self,
        transaction: &DecisionSelection<'_>,
    ) -> Result<(), AppError> {
        if transaction.previous_head.is_some()
            || transaction.head != self.head.bytes.as_slice()
            || transaction.record != self.record.bytes.as_slice()
            || transaction.pages != Some(self.pages.each_ref().map(|page| page.bytes.as_slice()))
        {
            return Err(denied());
        }
        Ok(())
    }

    pub(in super::super::super) fn cover(&mut self, root: &HeldDirectory) -> Result<(), AppError> {
        self.require_current(root)?;
        self.record.sync_complete()?;
        for page in &mut self.pages {
            page.sync_complete()?;
        }
        self.records.sync()?;
        self.head.sync_complete()?;
        root.sync()?;
        self.require_current(root)
    }

    pub(in super::super::super) fn require_current(
        &mut self,
        root: &HeldDirectory,
    ) -> Result<(), AppError> {
        root.require()?;
        absent(&root.child(HEAD_STAGE))?;
        absent(&root.child(SELECTION_SCRATCH))?;
        self.records.require()?;
        let names = bounded_entries(&self.records, 3)?;
        let exact = [&self.record, &self.pages[0], &self.pages[1]];
        if names.len() != exact.len()
            || exact.iter().any(|file| {
                !names
                    .iter()
                    .any(|name| file.path.file_name() == Some(name.as_os_str()))
            })
        {
            return Err(denied());
        }
        self.record.require()?;
        for page in &mut self.pages {
            page.require()?;
        }
        self.head.require()?;
        root.require()
    }
}
