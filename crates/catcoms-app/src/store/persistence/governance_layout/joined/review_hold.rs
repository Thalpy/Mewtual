//! The native founder's selected removed-post review directory.
//! Capturing authenticated storage grants neither review admission nor
//! an admin read. The original founder owner lends the actual scope later.
use super::*;

mod declaration;
use crate::store::chat_admission::PhysicalMountNegative;
use crate::store::critical_namespaces::review_hold_namespace;
use crate::store::persistence::v2_member_outbox::fs::{
    bounded_entries, create_namespace, observe_directory, HeldDirectory, Presence,
};
use crate::store::persistence::v2_review_hold::{
    disk::{HeldReviewQuery, HoldCandidate, HoldDisk, SelectedHeldWindow, WORK},
    HeldCase, HoldScope,
};
use crate::store::persistence::v2_shared_history::{disk::DISK_FEE, SharedHistoryScope};
use catcoms_replication::chat::{
    SignedAction, V2GeneralPrefixVerifier, V2RemovedCreateEvidence, V2RemovedCreateRecord,
};
pub(super) use declaration::{recover, require_no_scratch};
use std::collections::BTreeMap;

const MAX_SCOPES: usize = 8;

pub(in crate::store) const DIRECTORY_NAME: &str = "chat-v2-removed-holds";
pub(in crate::store) const MARKER_NAME: &str = "chat-critical-namespaces.review-hold.marker";
const WITNESS_DIRECTORY: &str = "chat-v2-removed-hold-receipts";

pub(in crate::store) struct SelectedReviewHold {
    marker: OpenedArchiveObject,
    directory: HeldDirectory,
    partitions: BTreeMap<[u8; 32], HeldDirectory>,
    witnesses: Option<HeldDirectory>,
    mount: PhysicalMountNegative,
    _fee: CatalogReadReservation,
}

impl SelectedReviewHold {
    pub(super) fn capture(
        store: &ServerStore,
        key: &[u8; 32],
        descriptor: &[u8],
    ) -> Result<Self, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        // Keep the hold root, up to eight scoped names, the optional
        // witness directory and this operation's path copies paid.
        let paths = fixed(store, false, DIRECTORY_NAME)?
            .as_os_str()
            .len()
            .checked_add(80)
            .and_then(|bytes| bytes.checked_mul(2 * (MAX_SCOPES + 4)))
            .and_then(|bytes| bytes.checked_add(16 * 1024))
            .ok_or_else(denied)?;
        let fee = policy.current.archive.pending.pool.try_reserve(paths)?;
        let marker = read(
            &fixed(store, false, MARKER_NAME)?,
            review_hold_namespace::MARKER_LEN,
        )?
        .ok_or_else(denied)?;
        let expected = review_hold_namespace::marker_data(key, descriptor)?;
        if marker.1.as_slice() != expected {
            return Err(denied());
        }
        let selected = Self {
            marker,
            directory: HeldDirectory::open(fixed(store, false, DIRECTORY_NAME)?)?,
            partitions: BTreeMap::new(),
            witnesses: None,
            mount: store.capture_physical_mount_negative(),
            _fee: fee,
        };
        selected.require(store)?;
        Ok(selected)
    }

    pub(super) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_original_physical_mount(&self.mount)?;
        self.directory.require()?;
        let (file, frame) = &self.marker;
        exact_held(
            &fixed(store, false, MARKER_NAME)?,
            file,
            session_id(file).map_err(io)?,
            frame,
        )?;
        self.directory.require()?;
        if store.has_rejoin_evidence()? {
            return Err(denied());
        }
        Ok(())
    }
}

impl ServerStore {
    /// Read only the selected scoped directory and authenticated covering
    /// witnesses. An unrelated archive WAL is never opened by this read.
    pub(in crate::store) fn read_selected_review_window(
        &self,
        scope: SharedHistoryScope,
        query: HeldReviewQuery,
        mut require_current: impl FnMut() -> Result<(), AppError>,
        verify: impl FnMut(&V2RemovedCreateRecord<'_>) -> Result<SignedAction, AppError>,
    ) -> Result<Option<SelectedHeldWindow>, AppError> {
        if scope.vault != self.chat_vault_domain()? || !(1..=8).contains(&query.limit) {
            return Err(denied());
        }
        self.validate_mounted_governance_layout()?;
        let _lease = self._session.retain_for_physical_work();
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let pool = &policy.current.archive.pending.pool;
        let _work = pool.try_reserve(16 * 1024)?;
        let mount = self.review_hold_mount.try_lock().map_err(|_| denied())?;
        let Some(owner) = mount.as_ref() else {
            return Ok(None);
        };
        require_current()?;
        owner.require(self)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let label = HoldScope::new(scope).partition_label(&key);
        let partition_path = owner.directory.child(&hex::encode(label));
        let directory = if let Some(directory) = owner.partitions.get(&label) {
            directory.try_clone()?
        } else if observe_directory(&partition_path)? == Presence::Present {
            HeldDirectory::open(partition_path)?
        } else {
            owner.require(self)?;
            require_current()?;
            return Ok(Some(SelectedHeldWindow::empty(pool.try_reserve(1024)?)));
        };
        let mut disk = HoldDisk::from_held(directory, scope, &key, pool.clone())?;
        if let Ok(Some(witnesses)) = owner.read_witness_directory(self) {
            let marker = owner.marker.1.as_slice().try_into().map_err(|_| denied())?;
            disk = disk.with_selection_witnesses(witnesses, marker);
        }
        disk.selected_review_window(
            &key,
            query,
            &self._session,
            || {
                require_current()?;
                owner.require(self)
            },
            verify,
        )
        .map(Some)
    }

    #[cfg(test)]
    pub(in crate::store) fn review_hold_partition_for_test(
        &self,
        scope: SharedHistoryScope,
    ) -> Result<PathBuf, AppError> {
        let key = Zeroizing::new(self.keys.db_key()?);
        let mut mount = self.review_hold_mount.try_lock().map_err(|_| denied())?;
        let owner = mount.as_mut().ok_or_else(denied)?;
        let partition = owner.partition(self, scope, &key, &mut || Ok(()))?;
        partition.require()?;
        Ok(owner
            .directory
            .child(&hex::encode(HoldScope::new(scope).partition_label(&key))))
    }

    /// Cover one verified late original under the selected founder's
    /// exclusive source operation. Hold the ID8 owner through absence,
    /// ID10 selection and receipt so ordinary selection cannot overtake it.
    /// The caller retains the original carrier and native work permit.
    /// Rederivation uses already checked prefixes and dependencies. It
    /// cannot enter any of the ordinary storage read APIs recursively.
    pub(in crate::store) fn retain_removed_create(
        &self,
        scope: SharedHistoryScope,
        (verifier, evidence, original): (&V2GeneralPrefixVerifier, &V2RemovedCreateEvidence, &[u8]),
        mut require_current: impl FnMut() -> Result<(), AppError>,
        rederive: impl FnOnce(&[u8]) -> Result<V2RemovedCreateEvidence, AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Option<HeldCase>, AppError> {
        verifier
            .require_current_removed_create(evidence)
            .map_err(|_| denied())?;
        evidence
            .require_original_data(original)
            .map_err(|_| denied())?;
        require_current()?;
        let target = evidence.target_data().spec();
        if target.group.as_slice() != scope.group
            || target.channel != scope.channel
            || target.context.profile != scope.profile
        {
            return Err(denied());
        }

        self.with_selected_shared_history(scope, None, |ordinary, _, key, pool| {
            // A missing, busy, corrupt or uncertain archive is not an
            // absence proof. A strictly selected matching original wins.
            if let Some(saved) = ordinary.read(
                key,
                target.action,
                &self._session,
                pool.try_reserve(DISK_FEE)?,
            )? {
                // Both signatures may be valid for the same ActionId.
                // The existing selected exact bytes continue to win.
                if saved.admission != target.context.admission {
                    return Err(denied());
                }
                require_current()?;
                return Ok(None);
            }
            let fee = pool.try_reserve(WORK)?;
            let mut mount = self.review_hold_mount.try_lock().map_err(|_| denied())?;
            let owner = mount.as_mut().ok_or_else(denied)?;
            require_current()?;
            owner.require(self)?;
            let partition = owner.partition(self, scope, key, &mut require_current)?;
            let witnesses = owner.prepare_witness_directory(self, &mut require_current);
            // Failure to prepare optional audit evidence cannot strand
            // the exact body. The original authority still has to be live.
            require_current()?;
            owner.require(self)?;
            let mut disk = HoldDisk::from_held(partition, scope, key, pool.clone())?;
            if let Ok(witnesses) = witnesses {
                let marker = owner.marker.1.as_slice().try_into().map_err(|_| denied())?;
                disk = disk.with_selection_witnesses(witnesses, marker);
            }
            let resulting = disk.retain(
                key,
                HoldCandidate {
                    verifier,
                    evidence,
                    original,
                },
                (&self._session, fee),
                |_| {
                    require_current()?;
                    owner.require(self)
                },
                rederive,
                rng,
            )?;
            Ok(Some(resulting))
        })
    }
}

impl SelectedReviewHold {
    fn read_witness_directory(
        &self,
        store: &ServerStore,
    ) -> Result<Option<HeldDirectory>, AppError> {
        if let Some(held) = &self.witnesses {
            return held.try_clone().map(Some);
        }
        let path = fixed(store, false, WITNESS_DIRECTORY)?;
        if observe_directory(&path)? == Presence::Absent {
            return Ok(None);
        }
        HeldDirectory::open(path).map(Some)
    }

    // Keep this optional sibling outside the legacy ID10 scope inventory.
    // An older writer can still add exact bodies without audit witnesses.
    fn prepare_witness_directory(
        &mut self,
        store: &ServerStore,
        require_current: &mut impl FnMut() -> Result<(), AppError>,
    ) -> Result<HeldDirectory, AppError> {
        self.require(store)?;
        if let Some(directory) = &self.witnesses {
            return directory.try_clone();
        }
        let path = fixed(store, false, WITNESS_DIRECTORY)?;
        if observe_directory(&path)? == Presence::Absent {
            require_current()?;
            create_namespace(&path)?;
        }
        let directory = HeldDirectory::open(path)?;
        // Cover the sibling's name before any receipt can name a held body.
        sync_directory(&store.dir).map_err(io)?;
        self.require(store)?;
        require_current()?;
        let original = directory.try_clone()?;
        self.witnesses = Some(directory);
        Ok(original)
    }

    fn partition(
        &mut self,
        store: &ServerStore,
        scope: SharedHistoryScope,
        key: &[u8; 32],
        require_current: &mut impl FnMut() -> Result<(), AppError>,
    ) -> Result<HeldDirectory, AppError> {
        self.require(store)?;
        let label = HoldScope::new(scope).partition_label(key);
        if let Some(owned) = self.partitions.get(&label) {
            return owned.try_clone();
        }
        if self.partitions.len() >= MAX_SCOPES {
            return Err(denied());
        }
        let path = self.directory.child(&hex::encode(label));
        if observe_directory(&path)? == Presence::Absent {
            let names = bounded_entries(&self.directory, MAX_SCOPES)?;
            if names.len() >= MAX_SCOPES {
                return Err(denied());
            }
            self.require(store)?;
            require_current()?;
            create_namespace(&path)?;
        }
        let partition = HeldDirectory::open(path)?;
        self.directory.sync()?;
        self.require(store)?;
        partition.require()?;
        let original = partition.try_clone()?;
        self.partitions.insert(label, original);
        Ok(partition)
    }
}

pub(super) fn require_absent(store: &ServerStore) -> Result<(), AppError> {
    absent(&fixed(store, false, MARKER_NAME)?)?;
    absent(&fixed(store, false, DIRECTORY_NAME)?)
}
