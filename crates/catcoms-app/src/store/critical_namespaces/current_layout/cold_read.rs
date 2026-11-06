//! Task-free historical group reads. These guards issue no task/owner, perform no directory
//! effects and cannot enter warm Seed/Head writers. Root separately authenticates its NEW
//! complete cold DATA receipt using the exact same resource loan and actual scope basis.
use super::*;
use crate::store::chat_admission::v2_current::ColdV2InitializingReadAdmission;
use crate::store::chat_admission::v2_current::{
    ColdV2CurrentReadAdmission, ColdV2GenesisScopeBasis,
};
use crate::store::owner_evidence::canonical::current_vacancy::cold_read as owners;
use crate::store::persistence::{
    v2_genesis_activation::ColdV2GenesisReadResources, GenesisSourceDataRef,
    OpenedColdV2GroupDirectory, COLD_V2_GROUP_DIRECTORY_READ_BYTES,
};
use std::fs::File;
use std::sync::MutexGuard;

/// Complete Factory-side simultaneous requested-buffer inventory. Root's independent cold
/// account adds this to its own retained closure/control/copy layouts before reserving its
/// cold-only class. Four old selectors include the filename association's held-name probes;
/// the group path and native directory conversions are independently named by persistence.
/// The concrete guards embed canonical role layouts, so no generic parser-size assumption is
/// used. This covers named buffers/copies, not allocator/RSS or process-global admission.
pub(in crate::store) const COLD_V2_GROUP_READ_GUARD_BYTES: usize =
    COLD_V2_GROUP_DIRECTORY_READ_BYTES
        + 4 * std::mem::size_of::<ColdV2CurrentGroupReadGuard<'static, 'static>>()
        + 4 * (MAX_OPEN_BASE_PATH + 256)
        + 2 * std::mem::size_of::<String>()
        + 64
        + 128;

// Both historical roles are consumed. Preserve the conservative four-Current layout
// invoice, with the actual equal-layout assertion below pinning its two+two coverage.
#[cfg(test)]
const _: () = {
    assert!(
        std::mem::size_of::<ColdV2InitializingGroupReadGuard<'static, 'static>>()
            == std::mem::size_of::<ColdV2CurrentGroupReadGuard<'static, 'static>>()
    );
};

/// Existing unowned Initializing1 read only. In particular this has neither a scope1 storage
/// permit nor the original writer/task identities required to write a head.
pub(in crate::store) struct ColdV2InitializingGroupReadGuard<'reader, 'writer> {
    // Directory/path drains before Root/canonical/pins. The borrowed loan was created before
    // admission and cannot become the last paid owner during any guard's destruction.
    directory: OpenedColdV2GroupDirectory<'reader>,
    root: MutexGuard<'reader, Option<super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::ColdV2InitializingOwnerReadNamespace<'reader, 'writer>,
    resources: &'reader ColdV2GenesisReadResources,
}
/// Existing unowned Current2 read only. It cannot reserve an initializer or renew a writer.
pub(in crate::store) struct ColdV2CurrentGroupReadGuard<'reader, 'writer> {
    directory: OpenedColdV2GroupDirectory<'reader>,
    root: MutexGuard<'reader, Option<super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::ColdV2CurrentOwnerReadNamespace<'reader, 'writer>,
    resources: &'reader ColdV2GenesisReadResources,
}

macro_rules! read_guard {
    ($guard:ident, $admission:ident, $method:ident, $owner_method:ident) => {
        impl $guard<'_, '_> {
            pub(in crate::store) fn store(&self) -> &ServerStore {
                self.owner.store()
            }
            pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
                self.owner.source()
            }
            pub(in crate::store) fn archive_root(
                &self,
            ) -> &super::super::archive_bootstrap::MountedRoot {
                self.root
                    .as_ref()
                    .expect("Root checked by private cold read constructor")
            }
            pub(in crate::store) fn directory_data(
                &self,
            ) -> (&std::path::Path, &File, catcoms_storage::kept::SessionId) {
                self.directory.directory_data()
            }
            pub(in crate::store) fn resources(&self) -> &ColdV2GenesisReadResources {
                self.resources
            }
            pub(in crate::store) fn scope_basis_data(&self) -> &ColdV2GenesisScopeBasis {
                self.owner.scope_basis_data()
            }
            /// No locks/I/O/account/SDK/encoding or allocation. Full physical proof belongs
            /// to the separate exact-object method and Root's actual artifact capture.
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.owner.validate_source_only()?;
                self.resources
                    .require_original_negative(self.store(), self.source())?;
                let root = self.archive_root();
                let context = self.source().data().context.spec_data();
                if root.negative.is_cancelled()
                    || !std::sync::Arc::ptr_eq(&root.mount, &self.store().registry_mount())
                    || root.descriptor.vault != context.vault
                {
                    return Err(refused());
                }
                Ok(())
            }
            fn validate_original_directory(&self) -> Result<(), AppError> {
                // Account checks may wait: do them BEFORE every full source/ID3 proof and
                // before the final named-directory comparison. No callback follows it.
                self.resources
                    .validate_original(self.store(), self.source())?;
                self.store()
                    .validate_mounted_current_layout_under_archive_root(self.archive_root())?;
                self.source()
                    .source()
                    .validate_under_archive_root(self.store(), self.archive_root())?;
                self.owner.validate_old_files()?;
                #[cfg(test)]
                run_before_final_directory_for_test();
                // Full source-control and old-name comparers can wait after the earlier
                // marker proof. Repeat ID3 now; only the last group proof and cached
                // original negatives may follow, never another large source read.
                self.store()
                    .validate_mounted_current_layout_under_archive_root(self.archive_root())?;
                self.directory.require_original_directory()?;
                self.validate_source_only()
            }
        }
        impl ServerStore {
            pub(in crate::store) fn $method<'reader, 'writer>(
                &'reader self,
                admission: &'reader $admission<'writer>,
                resources: &'reader ColdV2GenesisReadResources,
            ) -> Result<$guard<'reader, 'writer>, AppError> {
                if !std::ptr::eq(self, admission.store()) {
                    return Err(refused());
                }
                // The SAME loan is acquired and held outside Runtime by the real capture.
                // Validation precedes old selector allocation as well as group path/open.
                resources.validate_original(self, admission.source())?;
                admission.validate_source_only()?;
                let owner = self.$owner_method(admission)?;
                let root = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
                let mounted = root.as_ref().ok_or_else(refused)?;
                owner.validate_old_files()?;
                let directory =
                    self.open_cold_v2_group_directory(admission.source(), mounted, resources)?;
                let guard = $guard {
                    directory,
                    root,
                    owner,
                    resources,
                };
                guard.validate_original_directory()?;
                Ok(guard)
            }
        }
    };
}
read_guard!(
    ColdV2InitializingGroupReadGuard,
    ColdV2InitializingReadAdmission,
    lock_cold_initializing_genesis_group,
    lock_cold_initializing_v2_owner_namespace
);
read_guard!(
    ColdV2CurrentGroupReadGuard,
    ColdV2CurrentReadAdmission,
    lock_cold_current_genesis_group,
    lock_cold_current_v2_owner_namespace
);

// One-shot, thread-local harness seam after real comparer/account waits, before the final
// repeated ID3 and directory proofs. It cannot affect another parallel test/thread and takes
// its callback out of RefCell before invoking it; production has neither storage nor branch.
#[cfg(test)]
std::thread_local! {
    static BEFORE_FINAL_DIRECTORY: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn run_before_final_directory_for_test() {
    let hook = BEFORE_FINAL_DIRECTORY.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
pub(in crate::store) mod tests;
