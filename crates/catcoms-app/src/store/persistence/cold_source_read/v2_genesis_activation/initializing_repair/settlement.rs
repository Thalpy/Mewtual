//! Original repaired-file DATA loans to the shared actual settlement worker. No lookup,
//! identity initialization, path reopen, sync, source SDK or cleanup permission occurs here.
use super::super::settlement::{SeedPresentFiles, SelectedControl};
use super::*;

impl SavedColdV2GenesisRepairedHead {
    /// Lend ONLY the original already-selected complete candidate objects. The actual
    /// repaired head was cached after those identities/barriers; an impossible missing
    /// identity/candidate refuses rather than querying/filling a slot under a final proof.
    /// Cleanup's distinct guards and independent body still verify actual content/names.
    pub(in crate::store::persistence::cold_source_read::v2_genesis_activation) fn settlement_files(
        &self,
    ) -> Result<SeedPresentFiles<'_>, AppError> {
        let files = &self.seed.files;
        let pin = files.pin.get().ok_or_else(critical_namespaces::refused)?;
        let head = files.head.get().ok_or_else(critical_namespaces::refused)?;
        let candidates = files
            .candidates
            .get()
            .ok_or_else(critical_namespaces::refused)?;
        Ok(SeedPresentFiles {
            directory: (&files.group_path, &files.group_directory, files.group_id),
            seed: SelectedControl {
                file: &files.seed.object.file,
                id: files.seed.object.id,
                frame: &files.seed.frame,
            },
            pin: SelectedControl {
                file: &pin.file,
                id: pin
                    .identity
                    .get()
                    .copied()
                    .ok_or_else(critical_namespaces::refused)?,
                frame: &candidates.pin,
            },
            head: SelectedControl {
                file: &head.file,
                id: head
                    .identity
                    .get()
                    .copied()
                    .ok_or_else(critical_namespaces::refused)?,
                frame: &candidates.head,
            },
            expected: files.resources.expectation_data(),
        })
    }
}
