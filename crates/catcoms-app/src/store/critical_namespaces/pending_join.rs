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
    pub(in crate::store) bytes: Zeroizing<Vec<u8>>,
    pub(in crate::store) next: codec::Summary,
    pub(in crate::store) expected: codec::Summary,
    pub(in crate::store) original: Arc<Original>,
    // Actual process-owned handle, never reconstructed from a decoded "owned" status. Keeping
    // the object alive prevents its in-session file number being recycled after a short write.
    pub(in crate::store) created: Option<CreatedStage>,
    pub(in crate::store) _fee: CatalogReadReservation,
}
pub(in crate::store) struct CreatedStage {
    pub(in crate::store) file: File,
    pub(in crate::store) identity: Option<catcoms_storage::kept::SessionId>,
}
pub(in crate::store) struct Mounted {
    pub(in crate::store) current: codec::Summary,
    pub(in crate::store) pending: Option<PendingCandidate>,
    _metadata_fee: CatalogReadReservation,
}
pub(in crate::store) type SharedMounted = Mutex<Option<Mounted>>;
mod producer;
pub use producer::{
    ActivatedCheckpointJoin, JournalledCheckpointJoinPreparation, ReturnedCheckpointJoinActivation,
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
        self.settle_pending_join_catalog(&opened)?;
        opened.validate_original(self)?;
        let mut next = Some(Mounted {
            current: opened.selected.clone(),
            pending: None,
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
        let domain = self.chat_vault_domain()?;
        let current = self.read_pending_join_catalog_slot(Slot::Current, &domain, &read)?;
        let stage = self.read_pending_join_catalog_slot(Slot::Stage, &domain, &read)?;
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
        domain: &[u8; 32],
        read: &CatalogReadReservation,
    ) -> Result<Option<(File, codec::Summary)>, AppError> {
        let Some((file, raw)) = self.open_pending_join_catalog_slot(slot, read)? else {
            return Ok(None);
        };
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&raw)?)?);
        let parsed = codec::parse(&plain, domain)?;
        let summary = parsed.summary(*blake3::hash(&raw).as_bytes());
        // All borrowed/plain/raw/cipher input is released on return BEFORE the next source.
        Ok(Some((file, summary)))
    }
}
