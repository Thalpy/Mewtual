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

#[cfg(test)]
#[cfg(any(windows, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn actual_initializing_inventory_requires_each_exact_pin_and_head_role() {
        for pin_present in [false, true] {
            for head in [
                GenesisRepairHeadInventory::Absent,
                GenesisRepairHeadInventory::Stage,
                GenesisRepairHeadInventory::Current,
            ] {
                let temp = tempfile::tempdir().unwrap();
                let group = temp.path().join("group");
                let target = temp.path().join("target");
                fs::create_dir(&group).unwrap();
                fs::create_dir(&target).unwrap();
                fs::write(group.join(SEED), b"original activation").unwrap();
                if pin_present {
                    fs::write(group.join(PIN), b"original pin").unwrap();
                }
                for name in [
                    "origin",
                    "source.seed",
                    "source.bin",
                    "records.bin",
                    "manifest.bin",
                ] {
                    fs::write(target.join(name), b"immutable source").unwrap();
                }
                match head {
                    GenesisRepairHeadInventory::Absent => {}
                    GenesisRepairHeadInventory::Stage => {
                        fs::write(target.join(STAGE), b"staged").unwrap()
                    }
                    GenesisRepairHeadInventory::Current => {
                        fs::write(target.join(HEAD), b"selected").unwrap()
                    }
                }
                let group_file = crate::store::persistence::open_archive_directory(&group).unwrap();
                let target_file =
                    crate::store::persistence::open_archive_directory(&target).unwrap();
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
                require_fixed_initializing_inventory(group_data, target_data, pin_present, head)
                    .unwrap();
                let wrong_heads = match head {
                    GenesisRepairHeadInventory::Absent => [
                        GenesisRepairHeadInventory::Stage,
                        GenesisRepairHeadInventory::Current,
                    ],
                    GenesisRepairHeadInventory::Stage => [
                        GenesisRepairHeadInventory::Absent,
                        GenesisRepairHeadInventory::Current,
                    ],
                    GenesisRepairHeadInventory::Current => [
                        GenesisRepairHeadInventory::Absent,
                        GenesisRepairHeadInventory::Stage,
                    ],
                };
                for wrong in wrong_heads {
                    assert!(require_fixed_initializing_inventory(
                        group_data,
                        target_data,
                        pin_present,
                        wrong
                    )
                    .is_err());
                }
                assert!(require_fixed_initializing_inventory(
                    group_data,
                    target_data,
                    !pin_present,
                    head
                )
                .is_err());
                let private = target.join("private");
                fs::write(&private, b"preserve").unwrap();
                assert!(require_fixed_initializing_inventory(
                    group_data,
                    target_data,
                    pin_present,
                    head
                )
                .is_err());
                assert_eq!(fs::read(private).unwrap(), b"preserve");
                assert_eq!(fs::read(group.join(SEED)).unwrap(), b"original activation");
            }
        }
    }
}

pub(super) fn require_fixed_initializing_inventory(
    group: (&Path, &File, catcoms_storage::kept::SessionId),
    target: (&Path, &File, catcoms_storage::kept::SessionId),
    pin_present: bool,
    head: GenesisRepairHeadInventory,
) -> Result<(), AppError> {
    // The sibling's GENESIS_FIXED_INVENTORY_BYTES pays the SAME audited Rust1.94 native
    // iterator/entry/UTF16/name/path overlap on Windows, and the fixed getdents
    // buffer on Linux. Other platforms refuse before starting a reader.
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (group, target, pin_present, head);
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
        let names: &[&str] = if pin_present { &[SEED, PIN] } else { &[SEED] };
        super::super::genesis_inventory::require_exact_names(group, names)?;
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
        super::super::genesis_inventory::require_exact_names(target, names)?;
        require_directory_name(group.0, group.1, group.2)?;
        require_directory_name(target.0, target.1, target.2)
    }
}
