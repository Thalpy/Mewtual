//! Exact encrypted WAL output recovery. A covered append is visible
//! only after the head pointing to that exact record has been covered.
#[cfg(test)]
use super::super::control::{take_fault, ReviewControlFault};
use super::*;
use crate::store::persistence::v2_member_outbox::fs::{
    bounded_entries, create_namespace, ensure_exact_held, finish_exact_write, observe,
    observe_directory, open_exact_write, read_bounded_held, remove_listed, require_exact_held,
    HeldDirectory, Presence, WriteMode,
};
use catcoms_storage::kept::SessionId;

#[cfg(test)]
mod tests;

pub(super) const RECORD_DIRECTORY: &str = "records";
pub(super) const HEAD_CURRENT: &str = "head.current";
pub(super) const HEAD_STAGE: &str = "head.stage";
pub(super) const SELECTION_WAL: &str = "selection.pending";
pub(super) const SELECTION_SCRATCH: &str = "selection.scratch";

pub(in super::super) const NAMES: [&str; 5] = [
    RECORD_DIRECTORY,
    HEAD_CURRENT,
    HEAD_STAGE,
    SELECTION_WAL,
    SELECTION_SCRATCH,
];

pub(in super::super) fn require_no_selection(root: &HeldDirectory) -> Result<(), AppError> {
    absent(&root.child(SELECTION_SCRATCH))?;
    absent(&root.child(SELECTION_WAL))?;
    require_unstarted_outputs(root)
}

fn require_unstarted_outputs(root: &HeldDirectory) -> Result<(), AppError> {
    absent(&root.child(HEAD_STAGE))?;
    absent(&root.child(HEAD_CURRENT))?;
    if observe_directory(&root.child(RECORD_DIRECTORY))? != Presence::Absent {
        return Err(denied());
    }
    root.require()
}

pub(in super::super) struct HeldDecisionFile {
    path: PathBuf,
    file: (File, SessionId),
    pub(in super::super) bytes: Zeroizing<Vec<u8>>,
}

impl HeldDecisionFile {
    pub(super) fn read(path: PathBuf, maximum: usize) -> Result<Option<Self>, AppError> {
        if observe(&path)? == Presence::Absent {
            return Ok(None);
        }
        let (file, identity, bytes) = read_bounded_held(&path, maximum, false)?;
        Ok(Some(Self {
            path,
            file: (file, identity),
            bytes,
        }))
    }

    fn require(&mut self) -> Result<(), AppError> {
        require_exact_held(&self.path, &mut self.file, &self.bytes)
    }

    fn sync_complete(&mut self) -> Result<(), AppError> {
        self.require()?;
        let mut writer = open_exact_write(&self.path, WriteMode::Existing)?;
        if writer.1 != self.file.1 {
            return Err(denied());
        }
        require_exact_held(&self.path, &mut writer, &self.bytes)?;
        sync_selected_catalog(&writer.0).map_err(io)?;
        self.require()
    }
}

/// Keep the incomplete first write in a private scratch name. Only a
/// complete authenticated claim can survive promotion to the WAL role.
/// The caller verifies the original source and old/current Head on every
/// cut. A missing frame requests the exact pre-selection absence proof.
pub(in super::super) fn cover_selection_wal(
    root: &HeldDirectory,
    mut verify: impl FnMut(Option<&[u8]>) -> Result<(), AppError>,
    prepare: impl FnOnce() -> Result<Zeroizing<Vec<u8>>, AppError>,
) -> Result<HeldDecisionFile, AppError> {
    root.require()?;
    let pending_path = root.child(SELECTION_WAL);
    let scratch_path = root.child(SELECTION_SCRATCH);
    if let Some(mut pending) = HeldDecisionFile::read(pending_path.clone(), MAX_SELECTION_BYTES)? {
        absent(&scratch_path)?;
        verify(Some(&pending.bytes))?;
        pending.sync_complete()?;
        root.sync()?;
        pending.require()?;
        verify(Some(&pending.bytes))?;
        return Ok(pending);
    }

    let mut scratch = if observe(&scratch_path)? == Presence::Present {
        let (file, identity, bytes) = read_bounded_held(&scratch_path, MAX_SELECTION_BYTES, true)?;
        let mut original = HeldDecisionFile {
            path: scratch_path.clone(),
            file: (file, identity),
            bytes,
        };
        if DecisionSelection::incomplete_scratch(&original.bytes) {
            verify(None)?;
            require_unstarted_outputs(root)?;
            absent(&pending_path)?;
            original.require()?;
            remove_listed(&scratch_path)?;
            drop(original);
            absent(&scratch_path)?;
            root.sync()?;
            absent(&scratch_path)?;
            None
        } else {
            verify(Some(&original.bytes))?;
            require_unstarted_outputs(root)?;
            original.sync_complete()?;
            Some(original)
        }
    } else {
        None
    };

    if scratch.is_none() {
        verify(None)?;
        require_unstarted_outputs(root)?;
        absent(&pending_path)?;
        let bytes = prepare()?;
        verify(Some(&bytes))?;
        require_unstarted_outputs(root)?;
        let mut file = open_exact_write(&scratch_path, WriteMode::Create)?;
        #[cfg(test)]
        if take_fault(ReviewControlFault::ShortSelectionScratch) {
            finish_exact_write(&scratch_path, &mut file, &bytes[..bytes.len() / 2])?;
            return Err(AppError::Io("injected short selection scratch".into()));
        }
        finish_exact_write(&scratch_path, &mut file, &bytes)?;
        scratch = Some(HeldDecisionFile {
            path: scratch_path.clone(),
            file,
            bytes,
        });
    }
    let mut scratch = scratch.ok_or_else(denied)?;
    root.sync()?;
    verify(Some(&scratch.bytes))?;
    require_unstarted_outputs(root)?;
    absent(&pending_path)?;
    scratch.require()?;
    install_catalog_stage(&scratch_path, &pending_path).map_err(io)?;
    scratch.path = pending_path;
    absent(&scratch_path)?;
    scratch.sync_complete()?;
    root.sync()?;
    scratch.require()?;
    verify(Some(&scratch.bytes))?;
    #[cfg(test)]
    if take_fault(ReviewControlFault::AfterSelectionWal) {
        return Err(AppError::Io("injected stop after selection WAL".into()));
    }
    Ok(scratch)
}

/// Selection requires a live source and an already authenticated WAL
/// naming the exact independently checked predecessor and signed Raw.
/// The writer never chooses a different record or head on retry.
pub(in super::super) fn select_covered_wal(
    root: &HeldDirectory,
    wal: &mut HeldDecisionFile,
    transaction: &DecisionSelection<'_>,
    selected: DecisionHead,
    mut require_current: impl FnMut() -> Result<(), AppError>,
) -> Result<HeldDecisionFile, AppError> {
    root.require()?;
    require_current()?;
    if wal.path != root.child(SELECTION_WAL)
        || transaction.previous_head.is_some()
        || selected.sequence != 1
    {
        return Err(denied());
    }
    wal.sync_complete()?;
    root.sync()?;
    wal.require()?;
    require_current()?;

    let head_path = root.child(HEAD_CURRENT);
    let stage_path = root.child(HEAD_STAGE);
    let mut old = HeldDecisionFile::read(head_path.clone(), HEAD_BYTES)?;
    let already_selected = old
        .as_ref()
        .is_some_and(|head| head.bytes.as_slice() == transaction.head);
    if already_selected {
        // A completed rename may have stopped before the directory barrier.
        absent(&stage_path)?;
    } else if old.as_ref().map(|head| head.bytes.as_slice()) != transaction.previous_head {
        return Err(denied());
    }

    let records_path = root.child(RECORD_DIRECTORY);
    if observe_directory(&records_path)? == Presence::Absent {
        if already_selected || transaction.previous_head.is_some() {
            return Err(denied());
        }
        require_current()?;
        root.require()?;
        wal.require()?;
        create_namespace(&records_path)?;
    }
    let records = HeldDirectory::open(records_path)?;
    let record_name = format!("{:016x}.record", selected.sequence);
    let entries = bounded_entries(&records, 1)?;
    if entries.len() > 1
        || entries
            .first()
            .is_some_and(|name| name != record_name.as_str())
    {
        return Err(denied());
    }
    records.sync()?;
    root.sync()?;
    let record_path = records.child(&record_name);
    let mut record = if already_selected {
        let existing = HeldDecisionFile::read(record_path, MAX_RECORD_BYTES)?.ok_or_else(denied)?;
        if existing.bytes.as_slice() != transaction.record {
            return Err(denied());
        }
        existing
    } else {
        require_current()?;
        records.require()?;
        root.require()?;
        wal.require()?;
        let file = ensure_exact_held(&record_path, transaction.record)?;
        HeldDecisionFile {
            path: record_path,
            file,
            bytes: Zeroizing::new(transaction.record.to_vec()),
        }
    };
    record.sync_complete()?;
    records.sync()?;
    record.require()?;
    root.require()?;

    if already_selected {
        let mut head = old.take().ok_or_else(denied)?;
        head.sync_complete()?;
        root.sync()?;
        require_current()?;
        absent(&stage_path)?;
        records.require()?;
        record.require()?;
        head.require()?;
        wal.require()?;
        return Ok(head);
    }

    let mut stage = ensure_exact_held(&stage_path, transaction.head)?;
    root.sync()?;
    require_current()?;
    records.require()?;
    record.require()?;
    wal.require()?;
    require_exact_held(&stage_path, &mut stage, transaction.head)?;
    if let Some(previous) = old.as_mut() {
        previous.require()?;
    } else {
        absent(&head_path)?;
    }
    install_catalog_stage(&stage_path, &head_path).map_err(io)?;
    #[cfg(test)]
    if take_fault(ReviewControlFault::AfterSelectionHeadRename) {
        return Err(AppError::Io(
            "injected stop after review head rename".into(),
        ));
    }
    absent(&stage_path)?;
    require_exact_held(&head_path, &mut stage, transaction.head)?;
    sync_selected_catalog(&stage.0).map_err(io)?;
    root.sync()?;
    require_current()?;
    records.require()?;
    record.require()?;
    wal.require()?;
    require_exact_held(&head_path, &mut stage, transaction.head)?;
    Ok(HeldDecisionFile {
        path: head_path,
        file: stage,
        bytes: Zeroizing::new(transaction.head.to_vec()),
    })
}
