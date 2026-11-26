//! Local-only index of selected archive originals. Cold initialization pays
//! for one bounded directory inventory and checks only a page of exact
//! bundles per turn. Existing archive formats are left untouched.
use super::*;
use std::collections::{BTreeMap, VecDeque};

const SELECTORS_PER_TURN: usize = 8;
const SELECTOR_LOCATOR_BYTES: usize = "s-".len() + 64 + ".bin".len();
// Include locator and B-tree node allocations at peak while a cold build
// interleaves with live selected writes.
pub(in crate::store::persistence) const RELAY_INDEX_FEE: usize =
    MAX_RECORDS_PER_DIRECTORY * 384 + 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct ArchiveRelayEntry {
    pub(in crate::store) action: ActionId,
    pub(in crate::store) admission: OwnerFactId,
}

/// There is one paid instance per held archive. Locators are deliberately
/// opaque outside this type. An installed index cannot authorize a wire ID
/// without a fresh scope and author check at the serving edge.
pub(in crate::store::persistence) struct SharedHistoryRelayIndex {
    remaining: Vec<[u8; SELECTOR_LOCATOR_BYTES]>,
    unreadable: VecDeque<[u8; SELECTOR_LOCATOR_BYTES]>,
    entries: BTreeMap<ActionId, OwnerFactId>,
    _payment: CatalogReadReservation,
}

impl SharedHistoryRelayIndex {
    pub(in crate::store::persistence) fn start(
        disk: &SharedHistoryDisk,
        session: &VaultSessionGuard,
        payment: CatalogReadReservation,
        scratch: &CatalogReadReservation,
    ) -> Result<Self, AppError> {
        payment.validate(&disk.pool, RELAY_INDEX_FEE)?;
        scratch.validate(&disk.pool, DISK_FEE)?;
        let _lease = session.retain_for_physical_work();
        let _gate = disk.gate.try_lock().map_err(|_| refused())?;
        disk.require_clean_relay()?;
        let maximum = 2 * disk.quota.records;
        let names = bounded_entries(&disk.directory, maximum)?;
        if names.len() > maximum {
            return Err(refused());
        }
        let mut remaining = Vec::with_capacity(disk.quota.records);
        for name in names {
            let (leaf, role) = history_leaf(&name)?;
            match role {
                HistoryLeaf::Wal => return Err(refused()),
                HistoryLeaf::Bundle(_) => {}
                HistoryLeaf::Selector => {
                    if remaining.len() == disk.quota.records {
                        return Err(refused());
                    }
                    remaining.push(leaf.as_bytes().try_into().map_err(|_| refused())?);
                }
            }
        }
        disk.require_clean_relay()?;
        Ok(Self {
            remaining,
            unreadable: VecDeque::new(),
            entries: BTreeMap::new(),
            _payment: payment,
        })
    }

    pub(in crate::store::persistence) fn advance(
        &mut self,
        disk: &SharedHistoryDisk,
        key: &[u8; 32],
        session: &VaultSessionGuard,
        scratch: &CatalogReadReservation,
    ) -> Result<bool, AppError> {
        scratch.validate(&disk.pool, DISK_FEE)?;
        disk.require_key(key)?;
        let _lease = session.retain_for_physical_work();
        let _gate = disk.gate.try_lock().map_err(|_| refused())?;
        let cold = !self.remaining.is_empty();
        let count = if cold {
            self.remaining.len()
        } else {
            self.unreadable.len()
        };
        for _ in 0..count.min(SELECTORS_PER_TURN) {
            disk.require_clean_relay()?;
            let leaf = if cold {
                *self.remaining.last().ok_or_else(refused)?
            } else {
                *self.unreadable.front().ok_or_else(refused)?
            };
            let selected = disk.read_relay_locator(key, &leaf);
            disk.require_clean_relay()?;
            if cold {
                self.remaining.pop();
            } else {
                self.unreadable.pop_front();
            }
            // Preserve an unreadable file and make progress on independent
            // originals. Later local turns retry the affected exact locator.
            match selected.and_then(|entry| self.note_selected(entry, disk.quota)) {
                Ok(()) => {}
                Err(_) => self.unreadable.push_back(leaf),
            }
        }
        disk.require_clean_relay()?;
        Ok(self.remaining.is_empty())
    }

    /// Only the admitted selected write or an exact checked cold bundle can
    /// extend the index. A duplicate must agree with the retained Admission.
    pub(in crate::store::persistence) fn note_selected(
        &mut self,
        selected: ArchiveRelayEntry,
        quota: SharedHistoryDiskQuota,
    ) -> Result<(), AppError> {
        if let Some(admission) = self.entries.get(&selected.action) {
            return if *admission == selected.admission {
                Ok(())
            } else {
                Err(refused())
            };
        }
        if self.entries.len() == quota.records {
            return Err(refused());
        }
        self.entries.insert(selected.action, selected.admission);
        Ok(())
    }

    pub(in crate::store::persistence) fn entries(
        &self,
    ) -> Option<impl ExactSizeIterator<Item = ArchiveRelayEntry> + '_> {
        self.remaining.is_empty().then(|| {
            self.entries
                .iter()
                .map(|(&action, &admission)| ArchiveRelayEntry { action, admission })
        })
    }
}

impl SharedHistoryDisk {
    fn read_relay_locator(
        &self,
        key: &[u8; 32],
        leaf: &[u8; SELECTOR_LOCATOR_BYTES],
    ) -> Result<ArchiveRelayEntry, AppError> {
        let path = self
            .directory
            .child(std::str::from_utf8(leaf).map_err(|_| refused())?);
        let (file, identity, raw) = read_bounded_held(&path, SELECTOR_BYTES, false)?;
        let selector = self.open_selector(key, &raw)?;
        let action = selector.record.action;
        if self.selector_path(key, action) != path {
            return Err(refused());
        }
        let (admission, _) = self.open_selected_bundle(key, &selector)?;
        require_exact_held(&path, &mut (file, identity), &raw)?;
        Ok(ArchiveRelayEntry { action, admission })
    }

    /// Visit only exact readable records in this paid page. A dirty archive
    /// or lost directory refuses the whole request. An unreadable exact
    /// record under a clean archive cannot hide other selected originals.
    pub(in crate::store::persistence) fn visit_relay_page(
        &self,
        key: &[u8; 32],
        session: &VaultSessionGuard,
        scratch: &CatalogReadReservation,
        entries: &[ArchiveRelayEntry],
        require_current: impl Fn() -> Result<(), AppError>,
        mut visit: impl FnMut(ArchiveRelayEntry, &SignedAction) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        scratch.validate(&self.pool, DISK_FEE)?;
        self.require_key(key)?;
        if entries.len() > SELECTORS_PER_TURN {
            return Err(refused());
        }
        let _lease = session.retain_for_physical_work();
        let _gate = self.gate.try_lock().map_err(|_| refused())?;
        self.require_clean_relay()?;
        for entry in entries {
            require_current()?;
            let selected = self
                .read_selected(key, entry.action)
                .ok()
                .flatten()
                .filter(|(_, admission, _)| *admission == entry.admission);
            if let Some((_, _, original)) = selected {
                visit(*entry, original.action())?;
            }
            self.require_clean_relay()?;
        }
        Ok(())
    }

    pub(in crate::store::persistence) fn require_clean_relay(&self) -> Result<(), AppError> {
        self.directory.require()?;
        if observe(&self.directory.child(WAL_NAME))? == Presence::Present {
            return Err(refused());
        }
        Ok(())
    }

    pub(in crate::store::persistence) fn relay_quota(&self) -> SharedHistoryDiskQuota {
        self.quota
    }
}

impl PreparedSharedHistorySave {
    pub(in crate::store::persistence) fn relay_entry(&self) -> ArchiveRelayEntry {
        ArchiveRelayEntry {
            action: self.expected.action,
            admission: self.admission,
        }
    }
}
