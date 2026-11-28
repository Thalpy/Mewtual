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
use catcoms_storage::private_chat_page::PrivateChatPageId;

pub(in super::super) mod indexed;
mod selected;
#[cfg(test)]
mod tests;
pub(in super::super) use selected::FirstDecisionFiles;

pub(in super::super) const RECORD_DIRECTORY: &str = "records";
pub(super) const HEAD_CURRENT: &str = super::super::HEAD_NAME;
pub(super) const HEAD_STAGE: &str = "head.stage";
pub(in super::super) const SELECTION_WAL: &str = "selection.pending";
pub(in super::super) const SELECTION_SCRATCH: &str = "selection.scratch";

pub(in super::super) const NAMES: [&str; 5] = [
    RECORD_DIRECTORY,
    HEAD_CURRENT,
    HEAD_STAGE,
    SELECTION_WAL,
    SELECTION_SCRATCH,
];

pub(in super::super) fn require_no_selection(
    root: &HeldDirectory,
    key: &JournalKey,
) -> Result<(), AppError> {
    absent(&root.child(SELECTION_SCRATCH))?;
    absent(&root.child(SELECTION_WAL))?;
    require_unstarted_outputs(root, Some(&key.initial_head()))
}

fn require_unstarted_outputs(root: &HeldDirectory, initial: Option<&[u8]>) -> Result<(), AppError> {
    absent(&root.child(HEAD_STAGE))?;
    let path = root.child(HEAD_CURRENT);
    if let Some(initial) = initial {
        let mut head = HeldDecisionFile::read(path, HEAD_BYTES)?.ok_or_else(denied)?;
        if head.bytes.as_slice() != initial {
            return Err(denied());
        }
        head.require()?;
    } else {
        absent(&path)?;
    }
    if observe_directory(&root.child(RECORD_DIRECTORY))? != Presence::Absent {
        return Err(denied());
    }
    root.require()
}

pub(in super::super) struct HeldDecisionFile {
    pub(in super::super) path: PathBuf,
    file: (File, SessionId),
    pub(in super::super) bytes: Zeroizing<Vec<u8>>,
}

impl HeldDecisionFile {
    pub(in super::super) fn read(path: PathBuf, maximum: usize) -> Result<Option<Self>, AppError> {
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

    pub(in super::super) fn require(&mut self) -> Result<(), AppError> {
        require_exact_held(&self.path, &mut self.file, &self.bytes)
    }

    pub(in super::super) fn sync_complete(&mut self) -> Result<(), AppError> {
        self.require()?;
        let mut writer = open_exact_write(&self.path, WriteMode::Existing)?;
        if writer.1 != self.file.1 {
            return Err(denied());
        }
        require_exact_held(&self.path, &mut writer, &self.bytes)?;
        sync_selected_catalog(&writer.0).map_err(io)?;
        self.require()
    }

    pub(in super::super) fn try_clone(&mut self) -> Result<Self, AppError> {
        self.require()?;
        Ok(Self {
            path: self.path.clone(),
            file: (self.file.0.try_clone().map_err(io)?, self.file.1),
            bytes: self.bytes.clone(),
        })
    }

    pub(in super::super) fn same_original(&self, other: &Self) -> bool {
        self.path == other.path && self.file.1 == other.file.1 && self.bytes == other.bytes
    }

    pub(in super::super) fn retire(mut self, directory: &HeldDirectory) -> Result<(), AppError> {
        directory.require()?;
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(denied)?;
        if self.path != directory.child(name) {
            return Err(denied());
        }
        self.require()?;
        remove_listed(&self.path)?;
        let path = self.path.clone();
        // Close the original before inspecting a Windows delete-pending name.
        drop(self);
        absent(&path)?;
        directory.sync()?;
        absent(&path)
    }

    fn cover(
        path: PathBuf,
        bytes: &[u8],
        maximum: usize,
        already_selected: bool,
    ) -> Result<Self, AppError> {
        let mut held = if already_selected {
            let existing = Self::read(path, maximum)?.ok_or_else(denied)?;
            if existing.bytes.as_slice() != bytes {
                return Err(denied());
            }
            existing
        } else {
            let file = ensure_exact_held(&path, bytes)?;
            Self {
                path,
                file,
                bytes: Zeroizing::new(bytes.to_vec()),
            }
        };
        held.sync_complete()?;
        Ok(held)
    }
}

pub(in super::super) fn index_name(page: PrivateChatPageId) -> String {
    format!("{}.page", hex::encode(page.as_bytes()))
}

pub(in super::super) fn require_fresh_append_slots(
    root: &HeldDirectory,
    next_sequence: u64,
) -> Result<(), AppError> {
    absent(&root.child(SELECTION_SCRATCH))?;
    require_unstarted_append(root, next_sequence)
}

fn require_unstarted_append(root: &HeldDirectory, next_sequence: u64) -> Result<(), AppError> {
    absent(&root.child(SELECTION_WAL))?;
    absent(&root.child(HEAD_STAGE))?;
    let records = HeldDirectory::open(root.child(RECORD_DIRECTORY))?;
    absent(&records.child(&format!("{next_sequence:016x}.record")))?;
    records.require()?;
    root.require()
}

/// Keep the incomplete first write in a private scratch name. Only a
/// complete authenticated claim can survive promotion to the WAL role.
/// The caller verifies the original source and old/current Head on every
/// cut. A missing frame requests the exact pre-selection absence proof.
pub(in super::super) fn cover_selection_wal(
    root: &HeldDirectory,
    initial: Option<&[u8]>,
    verify: impl FnMut(Option<&[u8]>) -> Result<(), AppError>,
    prepare: impl FnOnce() -> Result<Zeroizing<Vec<u8>>, AppError>,
) -> Result<HeldDecisionFile, AppError> {
    cover_wal(
        root,
        (MAX_SELECTION_BYTES, DecisionSelection::incomplete_scratch),
        || require_unstarted_outputs(root, initial),
        verify,
        prepare,
    )
}

pub(in super::super) fn cover_indexed_append(
    root: &HeldDirectory,
    next: u64,
    verify: impl FnMut(Option<&[u8]>) -> Result<(), AppError>,
    prepare: impl FnOnce() -> Result<Zeroizing<Vec<u8>>, AppError>,
) -> Result<HeldDecisionFile, AppError> {
    use super::append::{IndexedAppend, MAX_APPEND_BYTES};
    cover_wal(
        root,
        (MAX_APPEND_BYTES, IndexedAppend::incomplete_scratch),
        || require_unstarted_append(root, next),
        verify,
        prepare,
    )
}

fn cover_wal(
    root: &HeldDirectory,
    (maximum, incomplete): (usize, fn(&[u8]) -> bool),
    mut require_unstarted: impl FnMut() -> Result<(), AppError>,
    mut verify: impl FnMut(Option<&[u8]>) -> Result<(), AppError>,
    prepare: impl FnOnce() -> Result<Zeroizing<Vec<u8>>, AppError>,
) -> Result<HeldDecisionFile, AppError> {
    root.require()?;
    let pending_path = root.child(SELECTION_WAL);
    let scratch_path = root.child(SELECTION_SCRATCH);
    if let Some(mut pending) = HeldDecisionFile::read(pending_path.clone(), maximum)? {
        absent(&scratch_path)?;
        verify(Some(&pending.bytes))?;
        pending.sync_complete()?;
        root.sync()?;
        pending.require()?;
        verify(Some(&pending.bytes))?;
        return Ok(pending);
    }

    let mut scratch = if observe(&scratch_path)? == Presence::Present {
        let (file, identity, bytes) = read_bounded_held(&scratch_path, maximum, true)?;
        let mut original = HeldDecisionFile {
            path: scratch_path.clone(),
            file: (file, identity),
            bytes,
        };
        if incomplete(&original.bytes) {
            verify(None)?;
            require_unstarted()?;
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
            require_unstarted()?;
            original.sync_complete()?;
            Some(original)
        }
    } else {
        None
    };

    if scratch.is_none() {
        verify(None)?;
        require_unstarted()?;
        absent(&pending_path)?;
        let bytes = prepare()?;
        verify(Some(&bytes))?;
        require_unstarted()?;
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
    require_unstarted()?;
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
    initial: Option<&[u8]>,
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
    } else if old.as_ref().map(|head| head.bytes.as_slice())
        != transaction.previous_head.or(initial)
    {
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
    let mut outputs = vec![(
        format!("{:016x}.record", selected.sequence),
        transaction.record,
        MAX_RECORD_BYTES,
    )];
    match (selected.roots, transaction.pages) {
        (Some(roots), Some(pages)) => {
            for (root, frame) in roots.into_iter().zip(pages) {
                outputs.push((index_name(root.page), frame, MAX_INDEX_FRAME_BYTES));
            }
        }
        (None, None) => (),
        _ => return Err(denied()),
    }
    let require_output_names = || {
        let names = bounded_entries(&records, outputs.len())?;
        if names.len() > outputs.len()
            || names
                .iter()
                .any(|name| !outputs.iter().any(|(output, _, _)| name == output.as_str()))
        {
            return Err(denied());
        }
        Ok(())
    };
    require_output_names()?;
    records.sync()?;
    root.sync()?;
    if !already_selected {
        require_current()?;
    }
    let mut captured = Vec::with_capacity(outputs.len());
    for (name, frame, cap) in &outputs {
        records.require()?;
        root.require()?;
        wal.require()?;
        captured.push(HeldDecisionFile::cover(
            records.child(name),
            frame,
            *cap,
            already_selected,
        )?);
    }
    let mut require_outputs = || -> Result<(), AppError> {
        require_output_names()?;
        for file in &mut captured {
            file.require()?;
        }
        Ok(())
    };
    records.sync()?;
    require_outputs()?;
    root.require()?;

    if already_selected {
        let mut head = old.take().ok_or_else(denied)?;
        head.sync_complete()?;
        root.sync()?;
        require_current()?;
        absent(&stage_path)?;
        records.require()?;
        require_outputs()?;
        head.require()?;
        wal.require()?;
        return Ok(head);
    }

    let mut stage = ensure_exact_held(&stage_path, transaction.head)?;
    root.sync()?;
    require_current()?;
    records.require()?;
    require_outputs()?;
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
    require_outputs()?;
    wal.require()?;
    require_exact_held(&head_path, &mut stage, transaction.head)?;
    Ok(HeldDecisionFile {
        path: head_path,
        file: stage,
        bytes: Zeroizing::new(transaction.head.to_vec()),
    })
}
