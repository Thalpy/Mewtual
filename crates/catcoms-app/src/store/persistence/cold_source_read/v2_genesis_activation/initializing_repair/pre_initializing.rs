//! Private family bridge from the actual durable-born row1 role. No raw basis/Record or
//! public file constructor exists. The new body owns ONLY its second independent payment.
use super::super::pre_initializing::PreInitializingFileLoan;
use super::*;
use crate::store::critical_namespaces::current_layout::DurableBornV2GenesisRow1GroupGuard;

impl ColdV2GenesisRepairSeed {
    pub(in super::super) fn require_born_successor_invoice() -> Result<(), AppError> {
        validate_repair_invoice()
    }

    pub(in super::super) fn from_durable_born(
        guard: &DurableBornV2GenesisRow1GroupGuard<'_, '_>,
    ) -> Result<Arc<Self>, AppError> {
        guard
            .resources()
            .validate_original(guard.store(), guard.source())?;
        guard.validate_source_only()?;
        let original = guard.pre_initializing_seed().original_files()?;
        // Clone original handles and COPY existing identity values. A new path open or
        // session_id query here could silently select/adopt a different original object.
        let files = RepairFiles {
            basis: guard.scope_basis_data().clone(),
            group_path: cold_capture::owned_path(original.group_path)?,
            seed_path: cold_capture::owned_path(original.seed_path)?,
            pin_path: cold_capture::owned_path(original.pin_path)?,
            head_path: cold_capture::owned_path(original.head_path)?,
            stage_path: cold_capture::owned_path(original.stage_path)?,
            group_directory: original.group_directory.try_clone().map_err(io)?,
            group_id: original.group_id,
            target_directory: original.target_directory.try_clone().map_err(io)?,
            target_id: original.target_id,
            seed: ReadControl {
                object: ReadFile {
                    file: original.seed.object.file.try_clone().map_err(io)?,
                    id: original.seed.object.id,
                    len: original.seed.object.len,
                },
                frame: Zeroizing::new(original.seed.frame.to_vec()),
            },
            pin: OnceLock::new(),
            head: OnceLock::new(),
            candidates: OnceLock::new(),
            // IMPORTANT: this is the BornCapture's new loan. Never clone the old pre-seed
            // resource body: it would keep the old1MiB alive throughout normal Current.
            resources: guard.resources().clone(),
        };
        let candidates = RepairCandidates {
            pin: Zeroizing::new(original.pin_frame.to_vec()),
            head: Zeroizing::new(original.head_frame.to_vec()),
        };
        files
            .candidates
            .set(candidates)
            .map_err(|_| critical_namespaces::refused())?;
        // Return the independent complete candidate to BornCapture for caching BEFORE any
        // later comparer/barrier proof. It remains DATA until the concrete role validates it.
        Ok(Arc::new(Self {
            files: Arc::new(files),
        }))
    }

    pub(in super::super) fn born_metadata_bytes_data() -> usize {
        2 * std::mem::size_of::<RepairFiles>()
            + 2 * std::mem::size_of::<ColdV2GenesisRepairSeed>()
            + 4 * std::mem::size_of::<SelectedTarget>()
            + std::mem::size_of::<PreInitializingFileLoan<'static>>()
            + 8 * 64
    }
}
