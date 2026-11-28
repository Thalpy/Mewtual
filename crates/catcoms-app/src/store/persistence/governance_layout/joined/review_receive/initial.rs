//! The one empty manifest covered by the original namespace declaration.
//! This writer is never used to reopen an already selected journal.
use super::{manifest::ManifestKey, *};
use crate::store::persistence::v2_member_outbox::fs::{
    bounded_entries, finish_exact_write, open_exact_write, WriteMode,
};

pub(super) struct InitialManifest {
    file: (File, SessionId),
}

impl InitialManifest {
    pub(super) fn open(
        directory: &HeldDirectory,
        key: &ManifestKey,
        exact: &[u8],
        mode: WriteMode,
    ) -> Result<Self, AppError> {
        if !key.open(exact)?.is_empty() {
            return Err(denied());
        }
        require_names(directory, matches!(mode, WriteMode::Create))?;
        let file = open_exact_write(&directory.child(MANIFEST_NAME), mode)?;
        Ok(Self { file })
    }

    pub(super) fn cover(
        &mut self,
        directory: &HeldDirectory,
        exact: &[u8],
    ) -> Result<(), AppError> {
        require_names(directory, false)?;
        finish_exact_write(&directory.child(MANIFEST_NAME), &mut self.file, exact)?;
        directory.sync()?;
        self.require(directory, exact)
    }

    pub(super) fn require(
        &mut self,
        directory: &HeldDirectory,
        exact: &[u8],
    ) -> Result<(), AppError> {
        require_names(directory, false)?;
        require_exact_held(&directory.child(MANIFEST_NAME), &mut self.file, exact)?;
        directory.require()
    }
}

fn require_names(directory: &HeldDirectory, allow_empty: bool) -> Result<(), AppError> {
    let names = bounded_entries(directory, 1)?;
    match names.as_slice() {
        [] if allow_empty => Ok(()),
        [name] if name == MANIFEST_NAME => Ok(()),
        _ => Err(denied()),
    }
}
