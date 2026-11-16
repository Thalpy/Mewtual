//! Negative bounded fixed-folder inventory for the actual Initializing1 repair closure.
//! Selection DATA below grants no File, scope, writer or repair authority. The actual paid
//! original receipt supplies both directories, and the concrete writer supplies permission.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum GenesisRepairHeadInventory {
    Absent,
    Stage,
    Current,
}

pub(super) fn require_fixed_initializing_inventory(
    group: (&Path, &File, catcoms_storage::kept::SessionId),
    target: (&Path, &File, catcoms_storage::kept::SessionId),
    pin_present: bool,
    head: GenesisRepairHeadInventory,
) -> Result<(), AppError> {
    // The sibling's GENESIS_FIXED_INVENTORY_BYTES pays the SAME audited Rust1.94 native
    // iterator/entry/UTF16/name/path overlap. Unsupported libc implementations refuse
    // before read_dir. Do not infer a portable bound from size_of::<ReadDir>().
    #[cfg(not(windows))]
    {
        let _ = (group, target, pin_present, head);
        Err(critical_namespaces::refused())
    }
    #[cfg(windows)]
    {
        const MAX: usize =
            crate::store::critical_namespaces::current_layout::MAX_OPEN_BASE_PATH + 1024;
        for (path, _, _) in [group, target] {
            if !path.is_absolute() || path.as_os_str().len() > MAX {
                return Err(critical_namespaces::refused());
            }
        }
        require_directory_name(group.0, group.1, group.2)?;
        require_directory_name(target.0, target.1, target.2)?;
        let names: &[&str] = if pin_present { &[SEED, PIN] } else { &[SEED] };
        super::super::genesis_inventory::require_exact_names(group.0, names)?;
        let names: &[&str] = match head {
            GenesisRepairHeadInventory::Absent => &[
                "origin",
                "source.seed",
                "source.bin",
                "records.bin",
                "manifest.bin",
            ],
            GenesisRepairHeadInventory::Stage => &[
                "origin",
                "source.seed",
                "source.bin",
                "records.bin",
                "manifest.bin",
                STAGE,
            ],
            GenesisRepairHeadInventory::Current => &[
                "origin",
                "source.seed",
                "source.bin",
                "records.bin",
                "manifest.bin",
                HEAD,
            ],
        };
        super::super::genesis_inventory::require_exact_names(target.0, names)?;
        require_directory_name(group.0, group.1, group.2)?;
        require_directory_name(target.0, target.1, target.2)
    }
}
