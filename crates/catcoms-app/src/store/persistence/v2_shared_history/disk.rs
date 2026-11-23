//! Exact selected history writes under an already held private directory.
//!
//! The mounted caller supplies the original vault session and archive account.
//! A fully authenticated WAL can select only its own exact original. No cold
//! code signs, mints an Admission, or activates a historical projection.
use super::*;
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::persistence::v2_member_outbox::fs::{
    bounded_entries, bounded_file_length, ensure_exact_held, finish_exact_write, observe,
    open_exact_write, read_bounded_held, remove_listed, require_exact_held, require_same_name,
    HeldDirectory, Presence, Removal, MAX_FILE,
};
use crate::store::{frame, unframe};
use catcoms_storage::kept::SessionId;
use catcoms_storage::VaultSessionGuard;
use std::fs::File;
use std::sync::Mutex;

const SELECTOR_TAG: &[u8] = b"catcoms/private-chat-v2-shared-history-selector/v1\0";
const SELECTOR_KEY: &[u8] = b"chat-v2/shared-history/selector";
const LOCATOR_KEY: &[u8] = b"chat-v2/shared-history/locator";
const SELECTOR_BYTES: usize = 4 + SELECTOR_TAG.len() + 2 + 3 * 36 + 40;
const WAL_TAG: &[u8] = b"catcoms/private-chat-v2-shared-history-wal/v1\0";
const WAL_NAME: &str = "history.wal";
const WAL_BYTES: usize = 4 + WAL_TAG.len() + 2 + 4 + SELECTOR_BYTES + 4 + MAX_SHARED_HISTORY_BUNDLE;
const _: () = assert!(WAL_BYTES <= MAX_FILE);
const MAX_RECORDS_PER_DIRECTORY: usize = 2048;
// Account for the full bounded native inventory, including the offending entry.
const INVENTORY_FEE: usize = (2 * MAX_RECORDS_PER_DIRECTORY + 2) * 1024;

pub(in crate::store::persistence) const DISK_FEE: usize = SHARED_HISTORY_RECORD_FEE
    + 3 * WAL_BYTES
    + MAX_SHARED_HISTORY_BUNDLE
    + INVENTORY_FEE
    + 16 * 1024;

#[derive(Clone, Copy)]
pub(in crate::store::persistence) struct SharedHistoryDiskQuota {
    records: usize,
    bytes: usize,
}

impl SharedHistoryDiskQuota {
    pub(in crate::store::persistence) const fn native() -> Self {
        Self {
            records: MAX_RECORDS_PER_DIRECTORY,
            bytes: 256 * 1024 * 1024,
        }
    }

    pub(super) fn new(records: usize, bytes: usize) -> Result<Self, AppError> {
        if !(1..=MAX_RECORDS_PER_DIRECTORY).contains(&records) || bytes <= SELECTOR_BYTES {
            return Err(refused());
        }
        Ok(Self { records, bytes })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::store::persistence) struct SelectedHistoryRecord {
    pub(in crate::store::persistence) action: ActionId,
    pub(in crate::store::persistence) manifest: PrivateChatPageId,
}

pub(in crate::store::persistence) struct SharedHistoryDisk {
    directory: HeldDirectory,
    scope: SharedHistoryScope,
    quota: SharedHistoryDiskQuota,
    gate: Mutex<()>,
    pool: CriticalCatalogReadPool,
    key_binding: [u8; 32],
}

impl SharedHistoryDisk {
    /// The physical mount installs exactly one owner for this held directory
    /// and never lends this constructor to the network or a caller's DTO.
    pub(in crate::store::persistence) fn from_held(
        directory: HeldDirectory,
        scope: SharedHistoryScope,
        quota: SharedHistoryDiskQuota,
        pool: CriticalCatalogReadPool,
        key: &[u8; 32],
    ) -> Result<Self, AppError> {
        directory.require()?;
        let key_binding = Self::bind_key(&scope, key);
        Ok(Self {
            directory,
            scope,
            quota,
            gate: Mutex::new(()),
            pool,
            key_binding,
        })
    }

    /// Seal the WAL once so a short write can resume on the same held file.
    pub(in crate::store::persistence) fn prepare_save<'a>(
        &'a self,
        key: &[u8; 32],
        expected: SelectedHistoryRecord,
        bundle: &[u8],
        rng: &mut impl CryptoRngCore,
        session: &'a VaultSessionGuard,
        payment: CatalogReadReservation,
    ) -> Result<PreparedSharedHistorySave<'a>, AppError> {
        payment.validate(&self.pool, DISK_FEE)?;
        self.require_key(key)?;
        let (admission, original) = open_shared_history_bundle(
            &self.scope,
            key,
            (expected.action, expected.manifest),
            bundle,
        )?;
        let selector = self.seal_selector(key, expected, rng)?;
        Ok(PreparedSharedHistorySave {
            disk: self,
            expected,
            admission,
            original,
            session,
            wal: encode_wal(&selector, bundle)?,
            attempt: None,
            _payment: payment,
        })
    }

    /// Recover a completely authenticated admitted frame under the original
    /// held directory. A partial or foreign frame is preserved for repair.
    pub(in crate::store::persistence) fn recover(
        &self,
        key: &[u8; 32],
        session: &VaultSessionGuard,
        payment: CatalogReadReservation,
    ) -> Result<Option<SelectedHistoryRecord>, AppError> {
        payment.validate(&self.pool, DISK_FEE)?;
        self.require_key(key)?;
        let _lease = session.retain_for_physical_work();
        let _gate = self.gate.try_lock().map_err(|_| refused())?;
        self.directory.require()?;
        self.recover_locked(key)
    }

    pub(in crate::store::persistence) fn read(
        &self,
        key: &[u8; 32],
        action: ActionId,
        session: &VaultSessionGuard,
        payment: CatalogReadReservation,
    ) -> Result<Option<SavedSharedOriginal>, AppError> {
        payment.validate(&self.pool, DISK_FEE)?;
        self.require_key(key)?;
        let _lease = session.retain_for_physical_work();
        let _gate = self.gate.try_lock().map_err(|_| refused())?;
        self.directory.require()?;
        if observe(&self.directory.child(WAL_NAME))? == Presence::Present {
            return Err(refused());
        }
        let Some((selected, admission, original)) = self.read_selected(key, action)? else {
            self.directory.require()?;
            return Ok(None);
        };
        self.directory.require()?;
        Ok(Some(SavedSharedOriginal {
            selected,
            admission,
            original,
            _payment: payment,
        }))
    }

    pub(in crate::store::persistence) fn require_scope(
        &self,
        scope: SharedHistoryScope,
    ) -> Result<(), AppError> {
        if self.scope != scope {
            return Err(refused());
        }
        self.directory.require()
    }

    fn recover_locked(&self, key: &[u8; 32]) -> Result<Option<SelectedHistoryRecord>, AppError> {
        let path = self.directory.child(WAL_NAME);
        if observe(&path)? == Presence::Absent {
            return Ok(None);
        }
        let (file, identity, wal) = read_bounded_held(&path, WAL_BYTES, false)?;
        // Rebarrier the authenticated exact WAL before finishing its selection.
        self.finish_wal(key, &wal, &mut (file, identity)).map(Some)
    }

    fn finish_wal(
        &self,
        key: &[u8; 32],
        wal: &[u8],
        original: &mut (File, SessionId),
    ) -> Result<SelectedHistoryRecord, AppError> {
        let (selector, bundle) = decode_wal(wal)?;
        let selected = self.open_selector(key, selector)?;
        open_shared_history_bundle(
            &self.scope,
            key,
            (selected.action, selected.manifest),
            bundle,
        )?;
        self.directory.require()?;
        let wal_path = self.directory.child(WAL_NAME);
        require_exact_held(&wal_path, original, wal)?;
        self.require_archive_capacity(key, selected, bundle.len())?;
        let mut barrier = open_exact_write(&wal_path, false)?;
        if barrier.1 != original.1 {
            return Err(refused());
        }
        finish_exact_write(&wal_path, &mut barrier, wal)?;
        drop(barrier);
        self.directory.sync()?;
        require_same_name(&wal_path, &original.0, original.1)?;
        let bundle_path = self.bundle_path(selected.manifest);
        let mut bundle_file = ensure_exact_held(&bundle_path, bundle)?;
        self.directory.sync()?;
        require_exact_held(&wal_path, original, wal)?;
        require_exact_held(&bundle_path, &mut bundle_file, bundle)?;
        let selector_path = self.selector_path(key, selected.action);
        let mut selector_file = ensure_exact_held(&selector_path, selector)?;
        self.directory.sync()?;
        // Exact authenticated WAL may be retired only after both selected
        // objects and their containing directory have passed barriers.
        require_exact_held(&selector_path, &mut selector_file, selector)?;
        require_exact_held(&bundle_path, &mut bundle_file, bundle)?;
        require_exact_held(&wal_path, original, wal)?;
        if remove_listed(&wal_path)? != Removal::Removed {
            return Err(refused());
        }
        self.directory.sync()?;
        Ok(selected)
    }

    fn bind_key(scope: &SharedHistoryScope, key: &[u8; 32]) -> [u8; 32] {
        *blake3::keyed_hash(key, &scope.id()).as_bytes()
    }

    fn require_key(&self, key: &[u8; 32]) -> Result<(), AppError> {
        if Self::bind_key(&self.scope, key) != self.key_binding {
            return Err(refused());
        }
        Ok(())
    }

    /// Size the candidate from what is physically present under this owner.
    /// The two outputs of a partially installed WAL each count once at their
    /// full candidate size. One bounded WAL has its own allowance.
    fn require_archive_capacity(
        &self,
        key: &[u8; 32],
        selected: SelectedHistoryRecord,
        bundle_bytes: usize,
    ) -> Result<(), AppError> {
        let limit = 2 * self.quota.records + 1;
        let names = bounded_entries(&self.directory, limit)?;
        if names.len() > limit {
            return Err(refused());
        }
        let target_selector = self.selector_path(key, selected.action);
        let target_bundle = self.bundle_path(selected.manifest);
        let mut selectors = 1;
        let mut bundles = 1;
        let mut used = SELECTOR_BYTES + bundle_bytes;
        for name in names {
            let leaf = name.to_str().ok_or_else(refused)?;
            let path = self.directory.child(leaf);
            if leaf == WAL_NAME {
                bounded_file_length(&path, WAL_BYTES)?;
                continue;
            }
            let (prefix, digest) = leaf.split_once('-').ok_or_else(refused)?;
            let digits = digest.strip_suffix(".bin").ok_or_else(refused)?;
            if digits.len() != 64
                || !digits
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(refused());
            }
            let (maximum, candidate_bytes, target, count) = match prefix {
                "s" => (
                    SELECTOR_BYTES,
                    SELECTOR_BYTES,
                    &target_selector,
                    &mut selectors,
                ),
                "b" => (
                    MAX_SHARED_HISTORY_BUNDLE,
                    bundle_bytes,
                    &target_bundle,
                    &mut bundles,
                ),
                _ => return Err(refused()),
            };
            let length = bounded_file_length(&path, maximum)?;
            if &path == target {
                if length > candidate_bytes {
                    return Err(refused());
                }
            } else {
                *count += 1;
                used = used.checked_add(length).ok_or_else(refused)?;
            }
        }
        if selectors > self.quota.records || bundles > self.quota.records || used > self.quota.bytes
        {
            return Err(refused());
        }
        self.directory.require()
    }

    fn read_selected(
        &self,
        key: &[u8; 32],
        action: ActionId,
    ) -> Result<Option<(SelectedHistoryRecord, OwnerFactId, AssembledActionOriginal)>, AppError>
    {
        let path = self.selector_path(key, action);
        if observe(&path)? == Presence::Absent {
            return Ok(None);
        }
        let (selector_file, selector_id, selector) =
            read_bounded_held(&path, SELECTOR_BYTES, false)?;
        let selected = self.open_selector(key, &selector)?;
        if selected.action != action {
            return Err(refused());
        }
        let bundle_path = self.bundle_path(selected.manifest);
        let (bundle_file, bundle_id, bytes) =
            read_bounded_held(&bundle_path, MAX_SHARED_HISTORY_BUNDLE, false)?;
        let (admission, original) = open_shared_history_bundle(
            &self.scope,
            key,
            (selected.action, selected.manifest),
            &bytes,
        )?;
        require_exact_held(&path, &mut (selector_file, selector_id), &selector)?;
        require_exact_held(&bundle_path, &mut (bundle_file, bundle_id), &bytes)?;
        self.directory.require()?;
        Ok(Some((selected, admission, original)))
    }

    fn selector_path(&self, key: &[u8; 32], action: ActionId) -> std::path::PathBuf {
        let context = self.scope.key(key);
        let locator_key = Zeroizing::new(*blake3::keyed_hash(&context, LOCATOR_KEY).as_bytes());
        let locator = blake3::keyed_hash(&locator_key, action.as_bytes());
        self.directory.child(&format!("s-{}.bin", locator.to_hex()))
    }

    fn bundle_path(&self, manifest: PrivateChatPageId) -> std::path::PathBuf {
        self.directory
            .child(&format!("b-{}.bin", hex::encode(manifest.as_bytes())))
    }

    fn seal_selector(
        &self,
        key: &[u8; 32],
        data: SelectedHistoryRecord,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let context = self.scope.key(key);
        let key = Zeroizing::new(*blake3::keyed_hash(&context, SELECTOR_KEY).as_bytes());
        let mut plain = Encoder::with_capacity(SELECTOR_BYTES - 40);
        wire(plain.put_bytes(SELECTOR_TAG))?.put_u16(VERSION);
        wire(plain.put_bytes(&self.scope.id()))?;
        wire(plain.put_bytes(data.action.as_bytes()))?;
        wire(plain.put_bytes(data.manifest.as_bytes()))?;
        let sealed = catcoms_crypto::seal(&key, &Zeroizing::new(plain.finish()), rng)?;
        Ok(Zeroizing::new(frame(&sealed)))
    }

    fn open_selector(&self, key: &[u8; 32], raw: &[u8]) -> Result<SelectedHistoryRecord, AppError> {
        if raw.len() != SELECTOR_BYTES {
            return Err(refused());
        }
        let context = self.scope.key(key);
        let key = Zeroizing::new(*blake3::keyed_hash(&context, SELECTOR_KEY).as_bytes());
        let plain = Zeroizing::new(catcoms_crypto::unseal(&key, &unframe(raw)?)?);
        let mut input = Decoder::new(&plain);
        if wire(input.get_bytes())? != SELECTOR_TAG
            || wire(input.get_u16())? != VERSION
            || digest(&mut input)? != self.scope.id()
        {
            return Err(refused());
        }
        let action = ActionId::from_bytes(digest(&mut input)?);
        let manifest = PrivateChatPageId::from_bytes(digest(&mut input)?);
        wire(input.finish())?;
        Ok(SelectedHistoryRecord { action, manifest })
    }
}

pub(in crate::store::persistence) struct PreparedSharedHistorySave<'disk> {
    disk: &'disk SharedHistoryDisk,
    expected: SelectedHistoryRecord,
    admission: OwnerFactId,
    original: AssembledActionOriginal,
    wal: Zeroizing<Vec<u8>>,
    attempt: Option<(File, SessionId)>,
    session: &'disk VaultSessionGuard,
    _payment: CatalogReadReservation,
}

impl PreparedSharedHistorySave<'_> {
    pub(in crate::store::persistence) fn commit(
        &mut self,
        key: &[u8; 32],
    ) -> Result<SelectedHistoryRecord, AppError> {
        let disk = self.disk;
        disk.require_key(key)?;
        let _lease = self.session.retain_for_physical_work();
        let _gate = disk.gate.try_lock().map_err(|_| refused())?;
        disk.directory.require()?;
        let path = disk.directory.child(WAL_NAME);
        if self.attempt.is_none() {
            disk.recover_locked(key)?;
            if let Some(selected) = self.prior_selection(key)? {
                return Ok(selected);
            }
            let (_, bundle) = decode_wal(&self.wal)?;
            disk.require_archive_capacity(key, self.expected, bundle.len())?;
            self.attempt = Some(open_exact_write(&path, true)?);
        } else if observe(&path)? == Presence::Absent {
            // The selection may already be durable when the final cleanup
            // barrier fails. Recheck the original before acknowledging it.
            let selected = self.prior_selection(key)?.ok_or_else(refused)?;
            self.attempt = None;
            return Ok(selected);
        }
        let held = self.attempt.as_mut().ok_or_else(refused)?;
        finish_exact_write(&path, held, &self.wal)?;
        disk.directory.sync()?;
        let selected = disk.finish_wal(key, &self.wal, held)?;
        self.attempt = None;
        Ok(selected)
    }

    fn prior_selection(&self, key: &[u8; 32]) -> Result<Option<SelectedHistoryRecord>, AppError> {
        let Some((selected, admission, original)) =
            self.disk.read_selected(key, self.expected.action)?
        else {
            self.disk.directory.require()?;
            return Ok(None);
        };
        if admission != self.admission
            || original.original_bytes() != self.original.original_bytes()
        {
            return Err(refused());
        }
        self.disk.directory.sync()?;
        Ok(Some(selected))
    }
}

pub(in crate::store) struct SavedSharedOriginal {
    pub(in crate::store::persistence) selected: SelectedHistoryRecord,
    pub(in crate::store) admission: OwnerFactId,
    pub(in crate::store) original: AssembledActionOriginal,
    _payment: CatalogReadReservation,
}

fn encode_wal(selector: &[u8], bundle: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if selector.len() != SELECTOR_BYTES || bundle.len() > MAX_SHARED_HISTORY_BUNDLE {
        return Err(refused());
    }
    let mut out = Encoder::with_capacity(WAL_BYTES);
    wire(out.put_bytes(WAL_TAG))?.put_u16(VERSION);
    wire(out.put_bytes(selector))?;
    wire(out.put_bytes(bundle))?;
    Ok(Zeroizing::new(out.finish()))
}

fn decode_wal(wal: &[u8]) -> Result<(&[u8], &[u8]), AppError> {
    if wal.len() > WAL_BYTES {
        return Err(refused());
    }
    let mut input = Decoder::new(wal);
    if wire(input.get_bytes())? != WAL_TAG || wire(input.get_u16())? != VERSION {
        return Err(refused());
    }
    let selector = wire(input.get_bytes())?;
    let bundle = wire(input.get_bytes())?;
    wire(input.finish())?;
    if selector.len() != SELECTOR_BYTES || bundle.len() > MAX_SHARED_HISTORY_BUNDLE {
        return Err(refused());
    }
    Ok((selector, bundle))
}

#[cfg(test)]
mod tests;
