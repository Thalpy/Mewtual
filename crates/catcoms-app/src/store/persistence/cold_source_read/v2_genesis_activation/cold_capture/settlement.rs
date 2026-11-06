//! Private original-file view for the new independent cleanup worker. This is NEVER a
//! replacement warm Saved receipt and does not invoke the old mandatory-seed validator.
use super::super::settlement::{SeedPresentFiles, SelectedControl};
use super::*;

impl ColdV2GenesisCurrentHead {
    pub(in crate::store::persistence::cold_source_read::v2_genesis_activation) fn settlement_files(
        &self,
    ) -> Result<SeedPresentFiles<'_>, AppError> {
        let c = &self.closure;
        // The independently restored settled role has NO original seed to remove. Never
        // fabricate a ReadControl/ID or turn it into a second cleanup invocation.
        let seed = c.seed.present()?;
        Ok(SeedPresentFiles {
            directory: (&c.group_path, &c.group_directory, c.group_id),
            seed: SelectedControl {
                file: &seed.object.file,
                id: seed.object.id,
                frame: &seed.frame,
            },
            pin: SelectedControl {
                file: &c.pin.object.file,
                id: c.pin.object.id,
                frame: &c.pin.frame,
            },
            head: SelectedControl {
                file: &c.head.object.file,
                id: c.head.object.id,
                frame: &c.head.frame,
            },
            expected: &c.resources.body.expected,
        })
    }
}
