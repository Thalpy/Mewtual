//! Hold and revalidate the fixed governed reservation under actual Current.
//! The read path authenticates an existing file. It cannot adopt an owner,
//! complete a partial write, enter MLS, or return a covering save receipt.
use super::*;
use crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::persistence::v2_text::capture::SelectedV2TextHead;
use crate::store::persistence::v2_text::io::{
    fixed_child, read_control, require_named, OpenedTextControl, MAX_TEXT_PATH,
};
use crate::store::persistence::v2_text::V2TextReadResources;
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::ServerStore;
use catcoms_storage::VaultPhysicalWorkLease;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const STAGE: &str = "text.wal.stage.v1";

/// An actual bounded, read-only File capture, tied to the exact selected
/// parent through its read-resource association. All file and path custody
/// leaves before the physical lease and the dedicated buffer reservation.
pub(in crate::store::persistence::v2_text) struct HeldGovernedAddFence {
    path: PathBuf,
    original: Option<OpenedTextControl>,
    expected: GovernedAddFenceData,
    authenticated: bool,
    resources: V2TextReadResources,
    // The independent buffer payment includes the bounded path and file read,
    // copied AEAD input/plaintext and exact same-handle EOF comparison.
    _lease: VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

const FENCE_READ_BYTES: usize =
    MAX_TEXT_PATH + 6 * FRAME_BYTES + 4 * std::mem::size_of::<HeldGovernedAddFence>() + 1024;

impl HeldGovernedAddFence {
    /// Prepare outside Current. Keep this whole receiver outside the writers
    /// through capture and any late refusal, so its file and lease drain there.
    pub(in crate::store::persistence::v2_text) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        expected: GovernedAddFenceData,
    ) -> Result<Self, AppError> {
        expected.require_parent(
            selected.head_data().context.id_data()?,
            selected.head_id_data(),
        )?;
        let resources = selected.resources().clone();
        resources.require_bound_original_negative(store, source)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(FENCE_READ_BYTES)?;
        fee.validate(pool, FENCE_READ_BYTES)?;
        let lease = store._session.retain_for_physical_work();
        let path = fixed_child(selected.physical_query_data().target, STAGE)?;
        resources.require_bound_original_negative(store, source)?;
        Ok(Self {
            path,
            original: None,
            expected,
            authenticated: false,
            resources,
            _lease: lease,
            _fee: fee,
        })
    }

    pub(in crate::store::persistence::v2_text) fn capture(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if self.original.is_some() {
            return Err(refused());
        }
        self.require_parent(guard, selected)?;
        self.original = Some(read_control(&self.path, FRAME_BYTES)?);
        guard.validate_source_only()
    }

    /// Outside Current. MAC success becomes usable only after a fresh check
    /// of the same original, named bytes and selected parent.
    pub(in crate::store::persistence::v2_text) fn authenticate(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.resources
            .require_bound_original_negative(store, source)?;
        let original = self.original.as_ref().ok_or_else(refused)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let data = GovernedAddFenceData::open_data(&key, &original.frame)?;
        if data != self.expected {
            return Err(refused());
        }
        self.resources
            .require_bound_original_negative(store, source)?;
        self.authenticated = true;
        Ok(())
    }

    pub(in crate::store::persistence::v2_text) fn require_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if !self.authenticated {
            return Err(refused());
        }
        self.require_parent(guard, selected)?;
        require_exact_held(&self.path, self.original.as_mut().ok_or_else(refused)?)?;
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()
    }

    fn require_parent(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        let (target, _, _) = guard.target_directory_data();
        if self.path.parent() != Some(target) {
            return Err(refused());
        }
        self.resources.require_same_original(selected.resources())?;
        self.expected.require_parent(
            selected.head_data().context.id_data()?,
            selected.head_id_data(),
        )?;
        selected.validate_under_current_guard(guard)?;
        guard.require_governed_native_invite_parent()
    }
}

fn require_exact_held(path: &Path, original: &mut OpenedTextControl) -> Result<(), AppError> {
    require_named(path, original)?;
    let file = &mut original.file;
    file.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut frame = [0; FRAME_BYTES];
    file.read_exact(&mut frame).map_err(io)?;
    if frame.as_slice() != original.frame.as_slice() {
        return Err(refused());
    }
    let mut trailer = [0];
    loop {
        match file.read(&mut trailer) {
            Ok(0) => break,
            Ok(_) => return Err(refused()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(io(error)),
        }
    }
    require_named(path, original)
}

fn io(error: std::io::Error) -> AppError {
    AppError::Io(error.to_string())
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests;
