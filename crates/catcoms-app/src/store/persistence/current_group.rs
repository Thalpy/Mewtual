//! Fixed group-wide NEGATIVE lookup. This opens no group and writes/adopts no Current state.
use super::*;

impl ServerStore {
    /// Called with the original Root held, after admission/pins where the consuming path owns
    /// those serializers. Do not call owner_canonical_directory or any creating path builder.
    /// Even an empty directory, dangling link, unreadable entry or ordinary file is RESERVED.
    pub(in crate::store) fn archive_group_absent_under_original_root(
        &self,
        mounted: &critical_namespaces::archive_bootstrap::MountedRoot,
        group: &[u8],
    ) -> Result<bool, AppError> {
        use critical_namespaces::{archive_bootstrap as archive, current_group, refused};
        let policy = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
        mounted
            ._fee
            .validate(&policy.pending.pool, archive::ROOT_METADATA_FEE)?;
        if mounted.negative.is_cancelled()
            || !std::sync::Arc::ptr_eq(&mounted.mount, &self.registry_mount())
        {
            return Err(refused());
        }
        // Both checked child paths fit the existing <=4096 physical-open profile and its
        // mounted metadata allowance. There is no directory scan or lifetime group allocation.
        let root = self.dir.join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT);
        validate_archive_root_session(&root, mounted)?;
        let suffix = current_group::group_suffix(
            &mounted.descriptor.vault,
            &mounted.descriptor.root,
            group,
        )?;
        let mut name = [0; 66];
        name[..2].copy_from_slice(b"g-");
        name[2..].copy_from_slice(&suffix);
        let name = std::str::from_utf8(&name).map_err(|_| refused())?;
        let absent = match fs::symlink_metadata(root.join(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Ok(_) => false,
            Err(_) => return Err(refused()),
        };
        // A replaced/missing parent must not turn its child into false NotFound/absence.
        validate_archive_root_session(&root, mounted)?;
        Ok(absent)
    }
}
