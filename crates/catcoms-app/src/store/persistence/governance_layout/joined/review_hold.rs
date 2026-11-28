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
    disk::{
        HeldReviewQuery, HoldCandidate, HoldDisk, HoldRootRead, PreparedHeldWindow,
        SelectedReviewOriginal, WORK,
    },
    HeldAuditObservation, HeldCase, HoldScope, NativeTextHeldReviewContinuation,
};
use crate::store::persistence::v2_shared_history::SharedHistoryScope;
use catcoms_replication::chat::{
    SignedAction, V2GeneralPrefixVerifier, V2RemovedCreateEvidence, V2RemovedCreateRecord,
};
#[cfg(test)]
pub(in crate::store) use declaration::warm_fault::{with_warm_review_hook, WarmReviewCut};
pub(super) use declaration::{recover, require_no_scratch};
pub(in crate::store) use declaration::{WarmFounderReviewCurrent, WarmFounderReviewOrigin};
use std::{collections::BTreeMap, sync::Mutex};

const MAX_SCOPES: usize = 8;

pub(in crate::store) const DIRECTORY_NAME: &str = "chat-v2-removed-holds";
pub(in crate::store) const MARKER_NAME: &str = "chat-critical-namespaces.review-hold.marker";
const WITNESS_DIRECTORY: &str = "chat-v2-removed-hold-receipts";

pub(in crate::store) struct SelectedReviewHold {
    marker: OpenedArchiveObject,
    directory: HeldDirectory,
    partitions: BTreeMap<[u8; 32], HeldDirectory>,
    witnesses: Option<HeldDirectory>,
    audit_revision: Arc<Mutex<u64>>,
    mount: PhysicalMountNegative,
    _fee: CatalogReadReservation,
}

impl SelectedReviewHold {
    fn mount_bytes(store: &ServerStore) -> Result<usize, AppError> {
        // Keep the hold root, up to eight scoped names, the optional
        // witness directory and this operation's path copies paid.
        fixed(store, false, DIRECTORY_NAME)?
            .as_os_str()
            .len()
            .checked_add(80)
            .and_then(|bytes| bytes.checked_mul(2 * (MAX_SCOPES + 4)))
            .and_then(|bytes| bytes.checked_add(16 * 1024))
            .ok_or_else(denied)
    }

    fn reserve_mount(store: &ServerStore) -> Result<CatalogReadReservation, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        policy
            .current
            .archive
            .pending
            .pool
            .try_reserve_classified(Self::mount_bytes(store)?, || AppError::ChatV2ArchiveBusy)
    }

    pub(super) fn capture(
        store: &ServerStore,
        key: &[u8; 32],
        descriptor: &[u8],
    ) -> Result<Self, AppError> {
        Self::capture_prepaid(store, key, descriptor, Self::reserve_mount(store)?)
    }

    fn capture_prepaid(
        store: &ServerStore,
        key: &[u8; 32],
        descriptor: &[u8],
        fee: CatalogReadReservation,
    ) -> Result<Self, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        fee.validate(
            &policy.current.archive.pending.pool,
            Self::mount_bytes(store)?,
        )?;
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
            audit_revision: Arc::new(Mutex::new(0)),
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
    /// Hold the strict ID8 absence proof through decision admission. A
    /// history read error cannot become evidence of a missing original.
    pub(in crate::store) fn with_absent_ordinary_review_target<T>(
        &self,
        scope: SharedHistoryScope,
        case: HeldCase,
        review: impl FnOnce() -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.with_selected_shared_history(scope, None, |ordinary, _, key, pool| {
            let selected = ordinary.read(
                key,
                case.action,
                &self._session,
                pool.try_reserve(ordinary.disk_fee())?,
            )?;
            if selected.is_some() {
                return Err(denied());
            }
            review()
        })
    }

    /// Prepare the exact captured hold for reproof after the founder
    /// resolves ordinary dependencies. The hold mutex leaves before any
    /// ID8 or ID5 ancestry read to preserve their existing lock order.
    pub(in crate::store) fn prepare_held_review_original(
        &self,
        scope: SharedHistoryScope,
        expected: HeldCase,
        mut require_current: impl FnMut() -> Result<(), AppError>,
    ) -> Result<SelectedReviewOriginal, AppError> {
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let pool = &policy.current.archive.pending.pool;
        let key = Zeroizing::new(self.keys.db_key()?);
        let mut mount = self.review_hold_mount.try_lock().map_err(|_| denied())?;
        let owner = mount.as_mut().ok_or_else(denied)?;
        require_current()?;
        owner.require(self)?;
        let directory = owner.read_partition(scope, &key)?.ok_or_else(denied)?;
        let witnesses = owner.read_witness_directory(self)?.ok_or_else(denied)?;
        let marker = owner.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let root = owner.audit_root(self, None)?;
        let disk = HoldDisk::from_held(directory, scope, &key, pool.clone())?
            .with_selection_witnesses(witnesses, marker);
        let mut selected =
            disk.prepare_selected_review_original(&key, expected, root, &self._session)?;
        require_current()?;
        owner.require(self)?;
        selected.require_original()?;
        Ok(selected)
    }

    /// Read only the selected scoped directory and authenticated covering
    /// witnesses. An unrelated archive WAL is never opened by this read.
    pub(in crate::store) fn read_selected_review_window(
        &self,
        scope: SharedHistoryScope,
        continuation: Option<&NativeTextHeldReviewContinuation>,
        limit: usize,
        mut require_current: impl FnMut() -> Result<(), AppError>,
        verify: impl FnMut(&V2RemovedCreateRecord<'_>) -> Result<SignedAction, AppError>,
    ) -> Result<Option<(PreparedHeldWindow, HeldAuditObservation)>, AppError> {
        if scope.vault != self.chat_vault_domain()? || !(1..=8).contains(&limit) {
            return Err(denied());
        }
        self.validate_mounted_governance_layout()?;
        let lease = self._session.retain_for_physical_work();
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let pool = &policy.current.archive.pending.pool;
        let _work = pool.try_reserve(16 * 1024)?;
        let mut mount = self.review_hold_mount.try_lock().map_err(|_| denied())?;
        let Some(owner) = mount.as_mut() else {
            if continuation.is_some() {
                return Err(denied());
            }
            return Ok(None);
        };
        require_current()?;
        owner.require(self)?;
        let observation = HeldAuditObservation::capture(&owner.audit_revision)?;
        let after = continuation
            .map(|cursor| cursor.after_for(scope, &observation))
            .transpose()?;
        let query = HeldReviewQuery { after, limit };
        let key = Zeroizing::new(self.keys.db_key()?);
        let Some(directory) = owner.read_partition(scope, &key)? else {
            owner.require(self)?;
            require_current()?;
            let label = HoldScope::new(scope).partition_label(&key);
            let absent = owner.directory.child(&hex::encode(label));
            let root = owner.audit_root(self, Some(absent))?;
            let empty = PreparedHeldWindow::empty(pool.try_reserve(16 * 1024)?);
            return Ok(Some((empty.with_root(root, lease), observation)));
        };
        let mut disk = HoldDisk::from_held(directory, scope, &key, pool.clone())?;
        if let Ok(Some(witnesses)) = owner.read_witness_directory(self) {
            let marker = owner.marker.1.as_slice().try_into().map_err(|_| denied())?;
            disk = disk.with_selection_witnesses(witnesses, marker);
        }
        let root = owner.audit_root(self, None)?;
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
        .map(|page| Some((page.with_root(root, lease), observation)))
    }

    #[cfg(test)]
    pub(in crate::store) fn review_hold_partition_for_test(
        &self,
        scope: SharedHistoryScope,
        mut before_parent_barrier: impl FnMut() -> Result<(), AppError>,
    ) -> Result<PathBuf, AppError> {
        let key = Zeroizing::new(self.keys.db_key()?);
        let mut mount = self.review_hold_mount.try_lock().map_err(|_| denied())?;
        let owner = mount.as_mut().ok_or_else(denied)?;
        let partition = owner.partition(self, scope, &key, &mut before_parent_barrier)?;
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
                pool.try_reserve(ordinary.disk_fee())?,
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
            let original_revision = owner.audit_revision.clone();
            let mut revision = original_revision.try_lock().map_err(|_| denied())?;
            *revision = revision.checked_add(1).ok_or_else(denied)?;
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
    fn audit_root(
        &self,
        store: &ServerStore,
        absent_partition: Option<PathBuf>,
    ) -> Result<HoldRootRead, AppError> {
        self.require(store)?;
        let (marker, bytes) = &self.marker;
        Ok(HoldRootRead {
            root: self.directory.try_clone()?,
            marker: (
                marker.try_clone().map_err(io)?,
                session_id(marker).map_err(io)?,
            ),
            marker_path: fixed(store, false, MARKER_NAME)?,
            marker_bytes: bytes.as_slice().try_into().map_err(|_| denied())?,
            absent_partition,
        })
    }

    fn read_partition(
        &mut self,
        scope: SharedHistoryScope,
        key: &[u8; 32],
    ) -> Result<Option<HeldDirectory>, AppError> {
        let label = HoldScope::new(scope).partition_label(key);
        if let Some(directory) = self.partitions.get(&label) {
            return directory.try_clone().map(Some);
        }
        let path = self.directory.child(&hex::encode(label));
        if observe_directory(&path)? == Presence::Absent {
            return Ok(None);
        }
        if self.partitions.len() >= MAX_SCOPES {
            return Err(denied());
        }
        let directory = HeldDirectory::open(path)?;
        self.partitions.insert(label, directory.try_clone()?);
        Ok(Some(directory))
    }

    fn read_witness_directory(
        &mut self,
        store: &ServerStore,
    ) -> Result<Option<HeldDirectory>, AppError> {
        if let Some(held) = &self.witnesses {
            return held.try_clone().map(Some);
        }
        let path = fixed(store, false, WITNESS_DIRECTORY)?;
        if observe_directory(&path)? == Presence::Absent {
            return Ok(None);
        }
        let directory = HeldDirectory::open(path)?;
        self.witnesses = Some(directory.try_clone()?);
        Ok(Some(directory))
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
            let original = directory.try_clone()?;
            // A read can have captured a directory that survived an
            // interrupted first creation. Cover its parent on writer entry.
            require_current()?;
            sync_directory(&store.dir).map_err(io)?;
            self.require(store)?;
            original.require()?;
            return Ok(original);
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
            let original = owned.try_clone()?;
            // The same cache also accepts original handles from read-only
            // audit. Observation alone cannot cover a new directory name.
            require_current()?;
            self.directory.sync()?;
            self.require(store)?;
            original.require()?;
            return Ok(original);
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
