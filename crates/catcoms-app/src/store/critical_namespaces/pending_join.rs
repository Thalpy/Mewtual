//! Explicit private checkpoint policy, known Pending-only catalog opening and original custody.
//! This is not the normal Store opener, a native feature switch, or join/ready publication.
use super::pending_join_codec as codec;
use super::*;
use crate::store::chat_admission::ChatAdmissionCapture;
use crate::store::owner_evidence::CapturedOwnerProfilePin;
use std::sync::Mutex;

const U: usize = codec::MAX_CATALOG_PLAIN + 16;
// ONE raw source/unframe/decrypt inventory plus current/stage fixed summaries and one parser.
// Sources are classified sequentially; no whole current plaintext survives the stage read.
pub(in crate::store) const READ_FEE: usize = codec::MAX_CATALOG_FRAMED
    + 1
    + U
    + U
    + 2 * std::mem::size_of::<codec::Summary>()
    + std::mem::size_of::<codec::Parsed<'static>>()
    + super::catalog_envelope::OUTER_METADATA_BYTES;
const METADATA_FEE: usize =
    std::mem::size_of::<Mounted>() + 2 * std::mem::size_of::<codec::Summary>();

/// Trusted local configuration for the inactive Pending-only handler. Supply clones of ONE
/// shared pool for read/write/retained copies. Saved sizes or IPC may not create this policy.
/// It disables the old unjournalled checkpoint join API on that physical mount from the outset.
#[derive(Clone)]
pub struct PendingJoinCatalogPolicy {
    pub(in crate::store) pool: CriticalCatalogReadPool,
    persistent_capacity: usize,
    pub(super) execution: Option<execution_policy::Settings>,
}
impl std::fmt::Debug for PendingJoinCatalogPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingJoinCatalogPolicy")
            .finish_non_exhaustive()
    }
}
impl PendingJoinCatalogPolicy {
    /// The two fixed files may BOTH retain a maximum frame after a crash. This checks a trusted
    /// logical persistent allowance; it is not filesystem free-space reservation or a global
    /// disk quota. Pool capacity is independently checked at each actual allocation admission.
    pub fn new(
        pool: CriticalCatalogReadPool,
        persistent_capacity: usize,
    ) -> Result<Self, AppError> {
        if persistent_capacity < 2 * codec::MAX_CATALOG_FRAMED {
            return Err(refused());
        }
        Ok(Self {
            pool,
            persistent_capacity,
            execution: None,
        })
    }
}

pub(in crate::store) struct Original {
    pub(in crate::store) capture: ChatAdmissionCapture,
    pub(in crate::store) pin: CapturedOwnerProfilePin,
    // Shared originals may outlive the caller when an uncertain output is retained by Store.
    // Their bounded capture/pin copies therefore keep an independent ORIGINAL strong fee.
    _fee: CatalogReadReservation,
}
impl Original {
    fn new(
        store: &ServerStore,
        capture: ChatAdmissionCapture,
        pin: CapturedOwnerProfilePin,
    ) -> Result<Arc<Self>, AppError> {
        let policy = store.pending_join_policy.as_ref().ok_or_else(refused)?;
        let fee = policy
            .pool
            .try_reserve(std::mem::size_of::<Self>() + 2 * 256)?;
        Ok(Arc::new(Self {
            capture,
            pin,
            _fee: fee,
        }))
    }
}

/// Exact pending output lives HERE through ambiguous I/O, independently of a caller dropping
/// its preparation. It cannot be overtaken or replaced by a freshly sealed source. Byte owners
/// precede their actual strong fee, so quota cannot be refunded while those bytes survive.
pub(in crate::store) struct PendingCandidate {
    // Original runtime identity, never encoded or reconstructed by a cold stage parser.
    identity: Arc<()>,
    pub(in crate::store) bytes: Zeroizing<Vec<u8>>,
    pub(in crate::store) next: codec::Summary,
    pub(in crate::store) expected: codec::Summary,
    pub(in crate::store) original: Arc<Original>,
    // Actual process-owned handle, never reconstructed from a decoded "owned" status. Keeping
    // the object alive prevents its in-session file number being recycled after a short write.
    pub(in crate::store) created: Option<CreatedStage>,
    // Present only BEFORE first actual counter admission. Consumed under original writers
    // immediately before Store takes the exact candidate; subsequent repair is DATA only.
    pub(in crate::store) execution_task:
        Option<crate::store::chat_admission::predecessor::OriginalJoinExecutionObservation>,
    // Genuine Sync retained signed DATA, never reconstructed from an on-disk input summary.
    // Presence selects the separate physical DATA permit; the full exact Input-only transition
    // is rechecked under both serializers on first admission AND every admitted repair.
    pub(in crate::store) received_input: Option<Arc<catcoms_sync::SealedOriginalJoinInput>>,
    // Only the special original RetryGrant admission can reserve this genuine owner. DATA
    // repair marks SavedData after real barriers; it never activates or owns the task sender.
    pub(in crate::store) continuation_outcome:
        Option<Arc<crate::store::chat_admission::predecessor::JoinContinuationOutcome>>,
    pub(in crate::store) _fee: CatalogReadReservation,
}
/// Inert snapshot from the ACTUAL Store-owned pending object. A final private repair permit
/// must match this same identity AND byte commitments under admission->catalog; parsed labels
/// cannot reconstruct it. It contains no task sender, source/master, scope or live authority.
pub(in crate::store) struct RetainedJoinCandidateIdentity {
    identity: Arc<()>,
    expected_frame: [u8; 32],
    next_frame: [u8; 32],
    expected_digest: [u8; 32],
    next_digest: [u8; 32],
}
impl PendingCandidate {
    pub(in crate::store) fn retained_identity(&self) -> RetainedJoinCandidateIdentity {
        RetainedJoinCandidateIdentity {
            identity: self.identity.clone(),
            expected_frame: self.expected.framed,
            next_frame: self.next.framed,
            expected_digest: self.expected.digest,
            next_digest: self.next.digest,
        }
    }
}
impl RetainedJoinCandidateIdentity {
    pub(in crate::store) fn validate_candidate(
        &self,
        candidate: &PendingCandidate,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.identity, &candidate.identity)
            || self.expected_frame != candidate.expected.framed
            || self.next_frame != candidate.next.framed
            || self.expected_digest != candidate.expected.digest
            || self.next_digest != candidate.next.digest
            || candidate.execution_task.is_some()
        {
            return Err(refused());
        }
        codec::successor(&candidate.expected, &candidate.next)?;
        if candidate.received_input.is_some() {
            candidate.validate_received_input_change()?;
        }
        Ok(())
    }
}
pub(in crate::store) struct CreatedStage {
    pub(in crate::store) file: File,
    pub(in crate::store) identity: Option<catcoms_storage::kept::SessionId>,
}
pub(in crate::store) struct Mounted {
    pub(in crate::store) current: codec::Summary,
    pub(in crate::store) pending: Option<PendingCandidate>,
    pub(in crate::store) networks: [Option<Arc<producer::NetworkShared>>; codec::MAX_ROWS],
    _metadata_fee: CatalogReadReservation,
}
pub(in crate::store) type SharedMounted = Mutex<Option<Mounted>>;
pub(super) mod execution_payload;
pub(super) mod execution_policy;
pub(super) mod execution_state;
pub use execution_policy::CheckpointJoinExecutionPolicy;
mod producer;
mod source_exclusion;
pub use producer::{
    ActivatedCheckpointJoin, CheckpointJoinNetwork, CheckpointJoinSubmissionStatus,
    JournalledCheckpointJoinPreparation, ReturnedCheckpointJoinActivation,
};

#[derive(Clone, Copy)]
pub(in crate::store) enum Settle {
    None,
    DuplicateStage,
    PromoteStage,
}
pub(in crate::store) struct Opened {
    mount: Arc<()>,
    negative: RequestCancellation,
    pub(in crate::store) current: Option<File>,
    pub(in crate::store) stage: Option<File>,
    pub(in crate::store) selected: codec::Summary,
    pub(in crate::store) settle: Settle,
    // Every file and metadata copy drops before this original read fee.
    _read: CatalogReadReservation,
}
impl Opened {
    pub(in crate::store) fn validate_original(&self, store: &ServerStore) -> Result<(), AppError> {
        if self.negative.is_cancelled() || !Arc::ptr_eq(&self.mount, &store.registry_mount()) {
            return Err(refused());
        }
        Ok(())
    }
}

impl ServerStore {
    /// Fixed policy metadata only. It neither issues resources nor grants a journal worker.
    pub(in crate::store) fn pending_join_policy_enabled(&self) -> bool {
        self.pending_join_policy.is_some()
    }

    pub(in crate::store) fn open_pending_join_catalog_gate(
        &self,
        actual_format: VaultFormat,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        // Explicit opt-in requires a previously activated actual v6 vault. This constructor
        // cannot infer upgrade/absence authority from a nonempty stage or an old wrapper.
        if actual_format != VaultFormat::CriticalNamespaces {
            return Err(refused());
        }
        let policy = self.pending_join_policy.as_ref().ok_or_else(refused)?;
        if policy.persistent_capacity < 2 * codec::MAX_CATALOG_FRAMED {
            return Err(refused());
        }
        let metadata_fee = policy.pool.try_reserve(METADATA_FEE)?;
        let opened = self.inspect_pending_join_catalog()?;
        // BOTH full candidates have authenticated and their exact known relation is fixed
        // before any older format gate or cleanup can run. Missing old inventories refuse.
        if !self.has_owner_operations_intent()? {
            return Err(refused());
        }
        self.open_owner_operations_gate(actual_format, passphrase, rng)?;
        self.install_opened_pending_catalog(opened, metadata_fee)
    }

    /// Only the owned initial native factory reaches this edge after the SAME
    /// Store activated its Empty critical catalog. Existing lower inventories
    /// are already mounted; replaying their constructor would discard/refuse
    /// original negatives. Revalidate, never clear or reopen them.
    pub(in crate::store::critical_namespaces) fn attach_created_empty_catalog(
        &self,
    ) -> Result<(), AppError> {
        if self.chat_mount.observer().is_cancelled()
            || self.chat_vault_format()? != VaultFormat::CriticalNamespaces
            || !self.has_owner_operations_intent()?
            || self
                .pending_join_catalog
                .lock()
                .map_err(|_| refused())?
                .is_some()
            || self.current_layout.lock().map_err(|_| refused())?.is_some()
            || self
                .archive_bootstrap_root
                .lock()
                .map_err(|_| refused())?
                .is_some()
            || self
                .member_outbox_layout
                .lock()
                .map_err(|_| refused())?
                .is_some()
        {
            return Err(refused());
        }
        super::refuse_unimplemented_archive_storage(&self.dir)?;
        super::governance_layout::refuse_unsupported_storage(&self.dir)?;
        super::member_outbox_layout::refuse_unsupported_storage(&self.dir)?;
        crate::store::persistence::native_registry::refuse_unsupported_storage(&self.dir)?;
        let empty = self.inspect_empty_critical_catalog()?.ok_or_else(refused)?;
        empty.validate_original(self)?;
        let policy = self.pending_join_policy.as_ref().ok_or_else(refused)?;
        if policy.persistent_capacity < 2 * codec::MAX_CATALOG_FRAMED {
            return Err(refused());
        }
        let metadata_fee = policy.pool.try_reserve(METADATA_FEE)?;
        let opened = self.inspect_pending_join_catalog()?;
        if opened.selected.generation != 1
            || opened.selected.count != 0
            || opened.selected.archive.is_some()
            || opened.selected.current_layout.is_some()
            || opened.selected.member_outbox.is_some()
        {
            return Err(refused());
        }
        self.install_opened_pending_catalog(opened, metadata_fee)
    }

    fn install_opened_pending_catalog(
        &self,
        opened: Opened,
        metadata_fee: CatalogReadReservation,
    ) -> Result<(), AppError> {
        self.settle_pending_join_catalog(&opened)?;
        opened.validate_original(self)?;
        let mut next = Some(Mounted {
            current: opened.selected.clone(),
            pending: None,
            networks: std::array::from_fn(|_| None),
            _metadata_fee: metadata_fee,
        });
        let installed = {
            let mut mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            if mounted.is_none() {
                *mounted = next.take();
                true
            } else {
                false
            }
        };
        // On refusal, any unused byte/fee owner drops after the catalog guard above. This
        // constructor is private until completion and never starts an active source worker.
        if !installed {
            return Err(refused());
        }
        Ok(())
    }

    pub(in crate::store) fn inspect_pending_join_catalog(&self) -> Result<Opened, AppError> {
        let policy = self.pending_join_policy.as_ref().ok_or_else(refused)?;
        let read = policy.pool.try_reserve(READ_FEE)?;
        let current = self.read_pending_join_catalog_slot(Slot::Current, &read)?;
        let stage = self.read_pending_join_catalog_slot(Slot::Stage, &read)?;
        let (selected, settle) = match (&current, &stage) {
            (Some((_, current)), None) => (current.clone(), Settle::None),
            (Some((_, current)), Some((_, stage))) if current.digest == stage.digest => {
                (current.clone(), Settle::DuplicateStage)
            }
            (Some((_, current)), Some((_, stage))) => {
                codec::successor(current, stage)?;
                (stage.clone(), Settle::PromoteStage)
            }
            (None, Some((_, stage))) if stage.generation == 1 && stage.count == 0 => {
                (stage.clone(), Settle::PromoteStage)
            }
            // A nonempty stage without its actual parent is not Empty or a fresh source.
            _ => return Err(refused()),
        };
        Ok(Opened {
            mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            current: current.map(|(file, _)| file),
            stage: stage.map(|(file, _)| file),
            selected,
            settle,
            _read: read,
        })
    }

    fn read_pending_join_catalog_slot(
        &self,
        slot: Slot,
        read: &CatalogReadReservation,
    ) -> Result<Option<(File, codec::Summary)>, AppError> {
        let Some((file, raw)) = self.open_pending_join_catalog_slot(slot, read)? else {
            return Ok(None);
        };
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&raw)?)?);
        let parsed = self.parse_configured_join_catalog(&plain)?;
        let summary = parsed.summary(*blake3::hash(&raw).as_bytes());
        // All borrowed/plain/raw/cipher input is released on return BEFORE the next source.
        Ok(Some((file, summary)))
    }
}
