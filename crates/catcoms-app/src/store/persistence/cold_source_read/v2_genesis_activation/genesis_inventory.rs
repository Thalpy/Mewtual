//! Bounded NEGATIVE inventory of the two original genesis folders. No decoded role here
//! authorizes Current, settlement, source selection or omission of a real activation seed.
//! The caller owns the SAME admission/pins/canonical/Root serializers and an ACTUAL fee
//! covering GENESIS_FIXED_INVENTORY_BYTES before entry. Unknown private data is preserved.
use super::*;

/// Negative DATA selector only. The physical capture must independently prove genuine
/// original presence/NotFound and latch its own opaque role; this helper is not that issuer.
#[derive(Clone, Copy)]
pub(super) enum GenesisSeedInventoryRole {
    Present,
    Absent,
}

const MAX_INVENTORY_PATH: usize =
    crate::store::critical_namespaces::current_layout::MAX_OPEN_BASE_PATH + 1024;

/// Pinned Rust1.94 Windows source audit: sys/fs/windows.rs ReadDir holds root Arc<PathBuf>
/// and fixed find-data; readdir constructs root clone/wildcard/UTF16 paths; file_name owns
/// at most one260-wide name, and file_type reads fixed cached attributes. Charge their
/// simultaneous path conversion/reallocation copies, actual iterator/entry/name layouts,
/// plus separately named fixed native/name/error scratch. This is logical requested-buffer
/// accounting, not Windows kernel/native-heap/RSS or a portable std implementation claim.
/// https://raw.githubusercontent.com/rust-lang/rust/1.94.0/library/std/src/sys/fs/windows.rs
/// https://raw.githubusercontent.com/rust-lang/rust/1.94.0/library/std/src/sys/path/windows.rs
pub(super) const GENESIS_FIXED_INVENTORY_BYTES: usize = 24 * MAX_INVENTORY_PATH
    + std::mem::size_of::<std::fs::ReadDir>()
    + std::mem::size_of::<std::fs::DirEntry>()
    + std::mem::size_of::<std::ffi::OsString>()
    + 64 * 1024;

/// Complete paid traversal ONLY of these two bounded fixed folders. Never a lifetime or
/// root-wide scan and never a recursive cleaner. At most expected-count+1 entries per
/// folder are visited; no entry.path()/sort/name Vec or lossy Unicode normalization exists.
pub(super) fn require_fixed_genesis_inventory(
    group: (&Path, &File, catcoms_storage::kept::SessionId),
    target: (&Path, &File, catcoms_storage::kept::SessionId),
    role: GenesisSeedInventoryRole,
) -> Result<(), AppError> {
    // Unix std delegates opendir storage to libc; the std type's size alone cannot pay
    // that opaque allocation. Refuse before iterator allocation or any cleanup effect.
    // A separately audited native bound/reader is required to support another platform.
    #[cfg(not(windows))]
    {
        let _ = (group, target, role);
        Err(critical_namespaces::refused())
    }
    #[cfg(windows)]
    {
        for (path, _, _) in [group, target] {
            if !path.is_absolute() || path.as_os_str().len() > MAX_INVENTORY_PATH {
                return Err(critical_namespaces::refused());
            }
        }
        require_directory_name(group.0, group.1, group.2)?;
        require_directory_name(target.0, target.1, target.2)?;
        let group_names: &[&str] = match role {
            GenesisSeedInventoryRole::Present => &[PIN, SEED],
            GenesisSeedInventoryRole::Absent => &[PIN],
        };
        require_exact_names(group.0, group_names)?;
        require_exact_names(
            target.0,
            &[
                "origin",
                "source.seed",
                "source.bin",
                "records.bin",
                "manifest.bin",
                HEAD,
            ],
        )?;
        // The path-based walker grants no held-object identity. Repeat BOTH real original
        // directory Files/names afterward; only caller's cached negatives may follow its
        // final source/ID3 and named-object proof edge.
        require_directory_name(group.0, group.1, group.2)?;
        require_directory_name(target.0, target.1, target.2)
    }
}

#[cfg(windows)]
// Shared only with the closed Initializing1 inventory. This is a bounded negative name
// comparison; callers still prove/pay their own platform, directories and concrete role.
pub(super) fn require_exact_names(path: &Path, expected: &[&str]) -> Result<(), AppError> {
    if expected.len() > 8 {
        return Err(critical_namespaces::refused());
    }
    let mut entries = std::fs::read_dir(path).map_err(io)?;
    let mut seen = 0u16;
    // The +1 observation is deliberate: an extra/unsupported entry closes this profile
    // immediately, independent of directory enumeration order. Do not collect/skip it.
    for _ in 0..=expected.len() {
        let Some(entry) = entries.next() else {
            return if seen == (1u16 << expected.len()) - 1 {
                Ok(())
            } else {
                Err(critical_namespaces::refused())
            };
        };
        let entry = entry.map_err(io)?;
        if !entry.file_type().map_err(io)?.is_file() {
            return Err(critical_namespaces::refused());
        }
        // Native exact names, including case. No caller path, aliases or signature bytes
        // are logged, interpreted as a child selector, or normalized into a known slot.
        let name = entry.file_name();
        let index = expected
            .iter()
            .position(|expected| name == std::ffi::OsStr::new(expected))
            .ok_or_else(critical_namespaces::refused)?;
        let bit = 1u16 << index;
        if seen & bit != 0 {
            return Err(critical_namespaces::refused());
        }
        seen |= bit;
    }
    Err(critical_namespaces::refused())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct InventoryFixture {
        group: PathBuf,
        target: PathBuf,
        group_file: File,
        group_id: catcoms_storage::kept::SessionId,
        target_file: File,
        target_id: catcoms_storage::kept::SessionId,
        // Last: real directory handles must drain before TempDir removes their paths.
        _temp: tempfile::TempDir,
    }
    impl InventoryFixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let group = temp.path().join("group");
            let target = temp.path().join("target");
            fs::create_dir(&group).unwrap();
            fs::create_dir(&target).unwrap();
            for name in [PIN, SEED] {
                fs::write(group.join(name), [1]).unwrap();
            }
            for name in [
                "origin",
                "source.seed",
                "source.bin",
                "records.bin",
                "manifest.bin",
                HEAD,
            ] {
                fs::write(target.join(name), [2]).unwrap();
            }
            let group_file = crate::store::persistence::open_archive_directory(&group).unwrap();
            let target_file = crate::store::persistence::open_archive_directory(&target).unwrap();
            let group_id = catcoms_storage::kept::session_id(&group_file).unwrap();
            let target_id = catcoms_storage::kept::session_id(&target_file).unwrap();
            Self {
                group,
                target,
                group_file,
                group_id,
                target_file,
                target_id,
                _temp: temp,
            }
        }
        fn check(&self, role: GenesisSeedInventoryRole) -> Result<(), AppError> {
            require_fixed_genesis_inventory(
                (&self.group, &self.group_file, self.group_id),
                (&self.target, &self.target_file, self.target_id),
                role,
            )
        }
    }

    #[test]
    fn actual_inventory_preserves_immutable_source_seed_in_both_explicit_roles() {
        let f = InventoryFixture::new();
        #[cfg(windows)]
        {
            f.check(GenesisSeedInventoryRole::Present).unwrap();
            assert!(f.check(GenesisSeedInventoryRole::Absent).is_err());
            fs::remove_file(f.group.join(SEED)).unwrap();
            assert!(f.check(GenesisSeedInventoryRole::Present).is_err());
            f.check(GenesisSeedInventoryRole::Absent).unwrap();
            assert_eq!(fs::read(f.target.join("source.seed")).unwrap(), [2]);
        }
        #[cfg(not(windows))]
        {
            // Unsupported std/libc native allocation profile stays an explicit refusal;
            // this test is not skipped or reported as portable settlement coverage.
            assert!(f.check(GenesisSeedInventoryRole::Present).is_err());
            assert!(f.check(GenesisSeedInventoryRole::Absent).is_err());
            assert_eq!(fs::read(f.group.join(SEED)).unwrap(), [1]);
        }
    }

    #[test]
    fn actual_stage_or_unrecognized_private_data_is_preserved_and_refused() {
        for (group, name) in [
            (true, "private.bin"),
            (false, STAGE),
            (false, "next-private.bin"),
        ] {
            let f = InventoryFixture::new();
            let path = if group {
                f.group.join(name)
            } else {
                f.target.join(name)
            };
            fs::write(&path, b"possible later state").unwrap();
            assert!(f.check(GenesisSeedInventoryRole::Present).is_err());
            assert_eq!(fs::read(path).unwrap(), b"possible later state");
            assert_eq!(fs::read(f.group.join(SEED)).unwrap(), [1]);
        }
    }

    #[test]
    fn absent_essential_and_nonregular_known_slot_never_become_an_empty_profile() {
        for nonregular in [false, true] {
            let f = InventoryFixture::new();
            let path = f.target.join("source.bin");
            fs::remove_file(&path).unwrap();
            if nonregular {
                fs::create_dir(&path).unwrap();
            }
            assert!(f.check(GenesisSeedInventoryRole::Present).is_err());
            assert_eq!(fs::read(f.group.join(PIN)).unwrap(), [1]);
        }
    }

    #[test]
    fn many_extra_entries_cannot_hide_a_later_head_or_trigger_collection() {
        let f = InventoryFixture::new();
        for index in 0..32 {
            fs::write(f.target.join(format!("private-{index}")), [3]).unwrap();
        }
        assert!(f.check(GenesisSeedInventoryRole::Present).is_err());
        assert_eq!(fs::read(f.target.join("private-31")).unwrap(), [3]);
        assert_eq!(fs::read(f.target.join(HEAD)).unwrap(), [2]);
    }
}
