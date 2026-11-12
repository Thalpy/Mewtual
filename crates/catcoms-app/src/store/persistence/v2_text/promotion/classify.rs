//! Closed read-only recovery dispatch DATA. Never choose another lane because an arbitrary
//! IO/MAC/parser error occurred. Each returned case still needs its own NEW actual admission.
use super::super::control::{inspect_seed_stage_mac_data, TextControlMacInspectionData};
use super::recovery::{self, HeldRecoveryControl};
use super::*;
use std::fs::{self, File};
use std::path::Path;

/// A present complete stage and a torn stage are deliberately different. Complete stage
/// bytes must resume unchanged, while the distinct scratch handler may dispose only its
/// proven nonauthoritative purpose/version workspace before any dependency exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum V2TextPromotionRecoveryClassData {
    CompleteMain,
    CompleteSeedStage,
    FirstSeedScratch,
    CleanUnstarted,
}

const PROBE_LAYOUT_BYTES: usize = 24 * (MAX_TEXT_PATH + 256)
    + 16 * (32768 + 40)
    + 16 * std::mem::size_of::<HeldRecoveryControl>()
    + 128 * 1024;
const _: () = assert!(V2_TEXT_PROMOTION_LAYOUT_BYTES + PROBE_LAYOUT_BYTES <= TEXT_WORK_BUFFER_FEE);

fn optional_control(path: &Path, maximum: usize) -> Result<Option<HeldRecoveryControl>, AppError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(recovery::io(error)),
        Ok(_) => {
            let mut held = HeldRecoveryControl::empty();
            held.capture(path, maximum)?;
            Ok(Some(held))
        }
    }
}

fn require_optional_name(path: &Path, held: &Option<HeldRecoveryControl>) -> Result<(), AppError> {
    match held {
        Some(control) => recovery::require_control_name(path, control),
        None => recovery::require_absent(path),
    }
}

struct ProbeChildren {
    pages: Option<File>,
    pages_id: Option<SessionId>,
    archive: Option<HeldRecoveryControl>,
    directory: Option<HeldRecoveryControl>,
    head_stage: Option<HeldRecoveryControl>,
}

impl PreparedV2TextPromotion {
    /// This bounded synchronous DATA probe uses the already-paid ORIGINAL16MiB scratch
    /// class; it allocates no4MiB recovery plan and grants no IO, scratch or selector right.
    /// Unknown/foreign/wrong-MAC MAIN, unexpected dependencies and prior missing file facts
    /// refuse explicitly. Actual source/lease/Current proofs are repeated by the chosen lane.
    pub(in crate::store) fn classify_recovery_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<V2TextPromotionRecoveryClassData, AppError> {
        self.work.validate_prepaid_account()?;
        self.resources.require_same_original(guard.resources())?;
        self.resources
            .validate_original(guard.store(), guard.source())?;
        if self.saved.is_some() {
            return Err(refused());
        }
        let (target, target_file, target_id) = guard.target_directory_data();
        if target != self.captured.target || target_id != self.captured.target_id {
            return Err(refused());
        }
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        if original_context(guard, &key)? != self.captured.context {
            return Err(refused());
        }
        let head_path = fixed_child(target, "head.bin")?;
        let seed_path = fixed_child(target, "text.seed")?;
        let seed_stage_path = fixed_child(target, "text.seed.stage.v1")?;
        let head_stage_path = fixed_child(target, "head.stage")?;
        let pages_path = fixed_child(target, "text.pages")?;
        let wal_path = fixed_child(target, "text.wal")?;
        let wal_stage_path = fixed_child(target, "text.wal.stage.v1")?;
        let archive_frame = self.archive.as_ref().ok_or_else(refused)?;
        let directory_frame = self.directory.as_ref().ok_or_else(refused)?;
        let archive_path = archive_child(&pages_path, archive_frame.id_data().as_bytes())?;
        let directory_path = work::page_child(&pages_path, directory_frame.id().as_bytes())?;
        let head = read_control(&head_path, 4136)?;
        if head.frame != self.captured.original_head {
            return Err(refused());
        }
        recovery::require_absent(&wal_path)?;
        recovery::require_absent(&wal_stage_path)?;
        let main = optional_control(&seed_path, 32768 + 40)?;
        let stage = optional_control(&seed_stage_path, 32768 + 40)?;
        if main.is_some() && stage.is_some() {
            return Err(refused());
        }
        let mut children = ProbeChildren {
            pages: None,
            pages_id: None,
            archive: None,
            directory: None,
            head_stage: None,
        };
        let need = if let Some(main) = &main {
            recovery::verify_original_seed_data(self, guard, main.raw()?)?;
            match fs::symlink_metadata(&pages_path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(recovery::io(error)),
                Ok(_) => {
                    let pages = crate::store::persistence::open_archive_directory(&pages_path)?;
                    children.pages_id =
                        Some(catcoms_storage::kept::session_id(&pages).map_err(recovery::io)?);
                    children.pages = Some(pages);
                    children.archive =
                        optional_control(&archive_path, archive_frame.frame_data().len())?;
                    children.directory =
                        optional_control(&directory_path, directory_frame.as_bytes().len())?;
                    if let Some(archive) = &children.archive {
                        if !archive_frame.frame_data().starts_with(archive.raw()?) {
                            return Err(refused());
                        }
                    }
                    if let Some(directory) = &children.directory {
                        if !directory_frame.as_bytes().starts_with(directory.raw()?) {
                            return Err(refused());
                        }
                    }
                }
            }
            children.head_stage = optional_control(
                &head_stage_path,
                self.head.as_ref().ok_or_else(refused)?.frame_data().len(),
            )?;
            if let Some(h) = &children.head_stage {
                if !self
                    .head
                    .as_ref()
                    .ok_or_else(refused)?
                    .frame_data()
                    .starts_with(h.raw()?)
                {
                    return Err(refused());
                }
            }
            // Missing listed NONSELECTED children are exactly what COMPLETE MAIN authorizes
            // the later SeedExactRepair lane to reconstruct. No missing history is inferred.
            V2TextPromotionRecoveryClassData::CompleteMain
        } else {
            recovery::require_absent(&pages_path)?;
            recovery::require_absent(&head_stage_path)?;
            if !self.slots.permits_first_seed_scratch_data() {
                return Err(refused());
            }
            if let Some(stage) = &stage {
                let raw = stage.raw()?;
                if raw == self.seed.as_ref().ok_or_else(refused)?.frame_data() {
                    recovery::verify_original_seed_data(self, guard, raw)?;
                    if self.slots.seed_native_data() != Some(stage.id()?) {
                        return Err(refused());
                    }
                    V2TextPromotionRecoveryClassData::CompleteSeedStage
                } else {
                    match inspect_seed_stage_mac_data(&key, raw)? {
                        TextControlMacInspectionData::Unauthenticated => {
                            V2TextPromotionRecoveryClassData::FirstSeedScratch
                        }
                        TextControlMacInspectionData::Authenticated => return Err(refused()),
                    }
                }
            } else {
                if !self.slots.is_unstarted_data() {
                    return Err(refused());
                }
                V2TextPromotionRecoveryClassData::CleanUnstarted
            }
        };
        let mut names = [""; 10];
        names[..6].copy_from_slice(&[
            "origin",
            "source.seed",
            "source.bin",
            "records.bin",
            "manifest.bin",
            "head.bin",
        ]);
        let mut count = 6;
        if main.is_some() {
            names[count] = "text.seed";
            count += 1;
        }
        if stage.is_some() {
            names[count] = "text.seed.stage.v1";
            count += 1;
        }
        if children.pages.is_some() {
            names[count] = "text.pages";
            count += 1;
        }
        if children.head_stage.is_some() {
            names[count] = "head.stage";
            count += 1;
        }
        work::require_exact_inventory(guard.directory_data().0, &["pin.bin"], false)?;
        work::require_exact_inventory(target, &names[..count], true)?;
        guard.validate_original_namespace()?;
        require_named(&head_path, &head)?;
        require_optional_name(&seed_path, &main)?;
        require_optional_name(&seed_stage_path, &stage)?;
        if let Some(pages) = &children.pages {
            crate::store::persistence::v2_genesis_activation::require_directory_name(
                &pages_path,
                pages,
                children.pages_id.ok_or_else(refused)?,
            )?;
            require_optional_name(&archive_path, &children.archive)?;
            require_optional_name(&directory_path, &children.directory)?;
        } else {
            recovery::require_absent(&pages_path)?;
        }
        require_optional_name(&head_stage_path, &children.head_stage)?;
        recovery::require_absent(&wal_path)?;
        recovery::require_absent(&wal_stage_path)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target,
            target_file,
            target_id,
        )?;
        guard.validate_source_only()?;
        Ok(need)
    }
}
