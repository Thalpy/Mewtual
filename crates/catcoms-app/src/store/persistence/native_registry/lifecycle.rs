//! Closed fixed-name read custody for the version2 -> version3 upgrade.
//! This non-writing preparation does not reuse the old upgrader's same-schema
//! limits, recovery classifications or writing roles.
use super::*;
pub(in crate::store) mod upgrade;

// Closed version3 physical bounds, checked against the semantic codec by Root.
// The original version2 capture and its byte ceiling remain unchanged.
pub(in crate::store) const MAX_INDEX_FRAME: usize =
    native::MAX_INDEX_FRAME + 45 + 9 * 1024 + 36 + 468 * 1024;
pub(in crate::store) const MAX_UPGRADE_SEED_FRAME: usize = 2 * native::MAX_CATALOG_FRAME
    + native::MAX_INDEX_FRAME
    + MAX_INDEX_FRAME
    + 2 * native::MAX_MARKER_FRAME
    + 4096;
pub(in crate::store) const EXPECTED_CURRENT_FEE: usize = 12
    * (critical::archive_bootstrap::MAX_MARKER_FRAMED
        + critical::archive_current_namespace_codec::MAX_MARKER_FRAME)
    + 16 * (native::MAX_BASE_PATH + 256)
    + 64 * 1024;

pub(in crate::store) const UPGRADE_SEED_NAME: &str = "chat-critical-namespaces.native-index3.seed";
pub(in crate::store) const UPGRADE_INDEX_NAME: &str = "native-index3.upgrade-index";
pub(in crate::store) const UPGRADE_MARKER_NAME: &str = "native-index3.upgrade-marker";

/// A reader without a version3 recovery handler must preserve every role,
/// including empty files and directories. Presence never means disposable.
pub(in crate::store) fn refuse_upgrade_debt(store: &ServerStore) -> Result<(), AppError> {
    for (root, name) in [
        (false, UPGRADE_SEED_NAME),
        (true, UPGRADE_INDEX_NAME),
        (true, UPGRADE_MARKER_NAME),
    ] {
        absent(&path(store, root, name)?)?;
    }
    Ok(())
}

/// Continuously holds the actual original index2 tuple. Callers supply no
/// path, remembered framed hash, generation assertion or readiness flag.
pub(in crate::store) struct UpgradeSnapshot {
    original: operational::IndexSnapshot,
}
impl UpgradeSnapshot {
    pub(in crate::store) fn capture(store: &ServerStore) -> Result<Self, AppError> {
        refuse_upgrade_debt(store)?;
        let value = Self {
            original: operational::IndexSnapshot::capture(store)?,
        };
        if value.original.stage().is_some() {
            return Err(denied());
        }
        value.require(store)?;
        Ok(value)
    }
    pub(in crate::store) fn catalog(&self) -> &[u8] {
        self.original.catalog()
    }
    pub(in crate::store) fn index(&self) -> &[u8] {
        self.original.index()
    }
    pub(in crate::store) fn marker(&self) -> &[u8] {
        self.original.marker()
    }
    pub(in crate::store) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        refuse_upgrade_debt(store)?;
        self.original.require_selected(store)?;
        refuse_upgrade_debt(store)
    }
}

/// Independent fixed-marker expectation before Current/JOIN cleaners. Root's
/// owning handler retains these real inputs and their full original payment;
/// no field/extractor exposes a mounted root or a source/SDK credential.
pub(in crate::store) struct ExpectedCurrentInputs {
    mount: crate::store::chat_admission::PhysicalMountNegative,
    current: HeldNativeInput,
    archive: HeldNativeInput,
    root: HeldDirectory,
    vault: HeldDirectory,
}
impl ExpectedCurrentInputs {
    pub(in crate::store) fn capture(
        store: &ServerStore,
        debit: &CatalogReadReservation,
        pool: &critical::read_pool::CriticalCatalogReadPool,
    ) -> Result<Self, AppError> {
        debit.validate(pool, EXPECTED_CURRENT_FEE)?;
        let value = Self {
            mount: store.capture_physical_mount_negative(),
            vault: HeldDirectory::capture(&path(store, false, "")?)?,
            root: HeldDirectory::capture(&path(store, true, "")?)?,
            archive: HeldNativeInput::capture(
                &path(store, true, critical::archive_bootstrap::ROOT_MARKER)?,
                critical::archive_bootstrap::MAX_MARKER_FRAMED,
            )?
            .ok_or_else(denied)?,
            current: HeldNativeInput::capture(
                &path(store, true, critical::current_layout::CURRENT_MARKER)?,
                critical::archive_current_namespace_codec::MAX_MARKER_FRAME,
            )?
            .ok_or_else(denied)?,
        };
        value.require(store)?;
        Ok(value)
    }
    pub(in crate::store) fn authenticate(
        &self,
        store: &ServerStore,
    ) -> Result<critical::archive_current_namespace_codec::CurrentLayoutData, AppError> {
        self.require(store)?;
        let key = zeroize::Zeroizing::new(store.keys.db_key()?);
        let archive = critical::archive_bootstrap::read_marker_descriptor_data(
            &key,
            &store.chat_vault_domain()?,
            self.archive.raw(),
        )?;
        let layout = critical::archive_current_namespace_codec::CurrentLayoutData {
            vault: archive.vault,
            root: archive.root,
        };
        critical::archive_current_namespace_codec::open_marker_data(
            &key,
            layout,
            self.current.raw(),
        )?;
        self.require(store)?;
        Ok(layout)
    }
    pub(in crate::store) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_original_physical_mount(&self.mount)?;
        self.vault.require(&path(store, false, "")?)?;
        self.root.require(&path(store, true, "")?)?;
        self.archive.require(&path(
            store,
            true,
            critical::archive_bootstrap::ROOT_MARKER,
        )?)?;
        self.current.require(&path(
            store,
            true,
            critical::current_layout::CURRENT_MARKER,
        )?)?;
        self.vault.require(&path(store, false, "")?)?;
        self.root.require(&path(store, true, "")?)?;
        store.require_original_physical_mount(&self.mount)?;
        Ok(())
    }
}
