//! Closed fixed-name read custody for the version2 -> version3 upgrade.
//! This non-writing preparation does not reuse the old upgrader's same-schema
//! limits, recovery classifications or writing roles.
use super::*;

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
