//! Exact negative name profile for complete journal/no row: no pin/head/stage is allowed.
//! This reuses the audited native walker, not the later complete-Present inventory role.
use super::*;

pub(super) fn require_pre_initializing_inventory(
    group: (&Path, &File, catcoms_storage::kept::SessionId),
    target: (&Path, &File, catcoms_storage::kept::SessionId),
) -> Result<(), AppError> {
    #[cfg(not(windows))]
    {
        let _ = (group, target);
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
        super::super::genesis_inventory::require_exact_names(group.0, &[SEED])?;
        super::super::genesis_inventory::require_exact_names(
            target.0,
            &[
                "origin",
                "source.seed",
                "source.bin",
                "records.bin",
                "manifest.bin",
            ],
        )?;
        require_directory_name(group.0, group.1, group.2)?;
        require_directory_name(target.0, target.1, target.2)
    }
}
