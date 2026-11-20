//! Exact negative name profile for complete journal/no row: no pin/head/stage is allowed.
//! This reuses the audited native walker, not the later complete-Present inventory role.
use super::*;

pub(super) fn require_pre_initializing_inventory(
    group: (&Path, &File, catcoms_storage::kept::SessionId),
    target: (&Path, &File, catcoms_storage::kept::SessionId),
) -> Result<(), AppError> {
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (group, target);
        Err(critical_namespaces::refused())
    }
    #[cfg(any(windows, target_os = "linux"))]
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
        super::super::genesis_inventory::require_exact_names(group, &[SEED])?;
        super::super::genesis_inventory::require_exact_names(
            target,
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

#[cfg(test)]
#[cfg(any(windows, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn actual_pre_initializing_inventory_never_accepts_a_pin_head_or_stage() {
        let temp = tempfile::tempdir().unwrap();
        let group = temp.path().join("group");
        let target = temp.path().join("target");
        fs::create_dir(&group).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(group.join(SEED), b"complete journal").unwrap();
        for name in [
            "origin",
            "source.seed",
            "source.bin",
            "records.bin",
            "manifest.bin",
        ] {
            fs::write(target.join(name), b"immutable source").unwrap();
        }
        let group_file = crate::store::persistence::open_archive_directory(&group).unwrap();
        let target_file = crate::store::persistence::open_archive_directory(&target).unwrap();
        let group_data = (
            &*group,
            &group_file,
            catcoms_storage::kept::session_id(&group_file).unwrap(),
        );
        let target_data = (
            &*target,
            &target_file,
            catcoms_storage::kept::session_id(&target_file).unwrap(),
        );
        require_pre_initializing_inventory(group_data, target_data).unwrap();
        for path in [group.join(PIN), target.join(HEAD), target.join(STAGE)] {
            fs::write(&path, b"later work").unwrap();
            assert!(require_pre_initializing_inventory(group_data, target_data).is_err());
            assert_eq!(fs::read(&path).unwrap(), b"later work");
            fs::remove_file(path).unwrap();
            require_pre_initializing_inventory(group_data, target_data).unwrap();
        }
        assert_eq!(fs::read(group.join(SEED)).unwrap(), b"complete journal");
    }
}
