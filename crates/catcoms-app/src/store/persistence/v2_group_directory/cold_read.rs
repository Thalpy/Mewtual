//! Open-only cold directory DATA. A held directory is neither a warm SavedDirectory nor a
//! cold task claim; only Root's separate authenticated artifact capture can issue its receipt.
use super::*;
use crate::store::persistence::v2_genesis_activation::ColdV2GenesisReadResources;

/// One original named directory selected by the real mounted vault/root/group. All owners
/// remain inside a borrowed resource loan's lifetime; this type owns no payment or sender.
pub(in crate::store) struct OpenedColdV2GroupDirectory<'loan> {
    path: PathBuf,
    file: File,
    identity: SessionId,
    // The actual outside prepaid owner stays borrowed through File/path destruction.
    // A future Store-private caller cannot return unpaid directory DATA.
    _resources: &'loan ColdV2GenesisReadResources,
}
impl OpenedColdV2GroupDirectory<'_> {
    pub(in crate::store) fn directory_data(&self) -> (&Path, &File, SessionId) {
        (&self.path, &self.file, self.identity)
    }
    /// Actual original directory check only. Call after all account/source/namespace waits,
    /// and follow it only with cached original negatives. Never use the file-only SessionAt.
    pub(in crate::store) fn require_original_directory(&self) -> Result<(), AppError> {
        require_same_directory(&self.path, &self.file, self.identity)
    }
}

/// Actual requested fixed-path/File conversion inventory. The outer cold guard adds its
/// own concrete layouts and canonical selector inventory; Root adds the complete sum to
/// its independently paid cold capture BEFORE any allocation. No native/RSS claim follows.
pub(in crate::store) const COLD_V2_GROUP_DIRECTORY_READ_BYTES: usize = 2 * std::mem::size_of::<OpenedColdV2GroupDirectory<'static>>()
        + MAX_GROUP_PATH_BYTES
        + 2 * std::mem::size_of::<File>()
        + 2 * std::mem::size_of::<fs::Metadata>()
        + 32 * 1024 // bounded native directory path conversions, separate from Rust paths
        + std::mem::size_of::<blake3::Hasher>()
        + 2 * 66; // lower-hex suffix and literal g- selector

impl ServerStore {
    /// Opens the existing fixed directory without create/mkdir, adopting no warm task or
    /// barrier status. A failed read has no filesystem effects or persistent unit refund.
    pub(in crate::store) fn open_cold_v2_group_directory<'loan>(
        &self,
        source: &GenesisSourceDataRef<'_>,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
        resources: &'loan ColdV2GenesisReadResources,
    ) -> Result<OpenedColdV2GroupDirectory<'loan>, AppError> {
        resources.validate_original(self, source)?;
        self.validate_mounted_current_layout_under_archive_root(root)?;
        source.source().validate_under_archive_root(self, root)?;
        #[cfg(test)]
        OPEN_ENTRIES.with(|entries| entries.set(entries.get() + 1));
        let layout = CurrentLayoutData {
            vault: root.descriptor.vault,
            root: root.descriptor.root,
        };
        let path = original_group_path(self, layout, source.data().context.spec_data().group)?;
        let file = open_archive_directory(&path)?;
        let identity = catcoms_storage::kept::session_id(&file).map_err(directory_io)?;
        let directory = OpenedColdV2GroupDirectory {
            path,
            file,
            identity,
            _resources: resources,
        };
        directory.require_original_directory()?;
        resources.require_original_negative(self, source)?;
        Ok(directory)
    }
}

// Counts the real group path/open entry, not a substitute guard/receipt. Thread-locality
// keeps parallel fixtures independent; the variable does not exist in production.
#[cfg(test)]
std::thread_local! {
    static OPEN_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(in crate::store) fn cold_group_read_entries_for_test() -> usize {
    OPEN_ENTRIES.with(std::cell::Cell::get)
}
