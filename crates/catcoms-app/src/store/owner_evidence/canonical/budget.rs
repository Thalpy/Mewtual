//! Explicit persistent replacement and hot-buffer capacity, acquired BEFORE private freeze.
use super::*;

mod operation;
mod sdk;
pub(crate) use operation::OwnerOperationReservation;

/// Host configuration for the inactive whole-state checkpoint. These are local resource limits,
/// not a signed public profile or authority. Finite retention is honest: no proof/history is
/// evicted to fit. Production uses the separately reviewed scalable cold-page successor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerEvidenceBudget {
    pub(super) max_heads: usize,
    pub(super) persistent_bytes: u64,
    pub(super) hot_bytes: usize,
    pub(super) snapshot: usize,
    pub(super) mls: usize,
    pub(super) mls_backing: usize,
    pub(super) sdk: Option<catcoms_mls::persist::OwnerSdkRestoreLimits>,
}
impl OwnerEvidenceBudget {
    /// Require an explicit whole-current + replacement allowance for EACH admitted head and
    /// four simultaneous initialization buffers, plus two explicit provider backing/payload fees.
    /// Vec payload capacities are independently checked against twice max_mls_bytes. Zero,
    /// overflow and unsupported maxima refuse; this does not close the separate SDK-heap gate.
    pub fn checkpoint(
        max_heads: usize,
        persistent_bytes: u64,
        hot_bytes: usize,
        max_snapshot_bytes: usize,
        max_mls_bytes: usize,
        max_mls_backing_bytes: usize,
    ) -> Result<Self, AppError> {
        if max_heads == 0
            || max_heads > MAX_HEADS
            || max_snapshot_bytes > MAX_SNAPSHOT_BYTES
            || max_mls_bytes == 0
            || max_mls_bytes > MAX_MLS_BYTES
            || max_mls_backing_bytes == 0
            || max_mls_backing_bytes > MAX_MLS_BACKING_BYTES
            || max_mls_bytes
                .checked_add(64 * 1024)
                .is_none_or(|fee| fee > max_snapshot_bytes)
        {
            return Err(refused());
        }
        let value = Self {
            max_heads,
            persistent_bytes,
            hot_bytes,
            snapshot: max_snapshot_bytes,
            mls: max_mls_bytes,
            mls_backing: max_mls_backing_bytes,
            sdk: None,
        };
        if value.head_fee()? > persistent_bytes || value.buffer_fee()? > hot_bytes {
            return Err(refused());
        }
        Ok(value)
    }
    pub(super) fn head_fee(&self) -> Result<u64, AppError> {
        self.snapshot
            .checked_add(WRAPPER_FEE + SEALED_FEE)
            .and_then(|fee| fee.checked_mul(2))
            .and_then(|fee| u64::try_from(fee).ok())
            .ok_or_else(refused)
    }
    pub(super) fn buffer_fee(&self) -> Result<usize, AppError> {
        let buffers = self
            .snapshot
            .checked_add(WRAPPER_FEE + SEALED_FEE + 1)
            .and_then(|fee| fee.checked_mul(4))
            .ok_or_else(refused)?;
        // Provider table backing is independent of encoded payload/output. Retain capacity for
        // the original private provider and one validation/candidate provider at the same time;
        // key/value Vec capacities are separately capped and paid at TWICE the encoded allowance
        // for each. This is a checked local policy, not an SDK allocation-size assumption. These
        // fees do not assert that arbitrary SDK JSON/group heaps are bounded by byte framing:
        // that restore preflight remains an explicit activation gate until its consumer exists.
        // They cover private initialization/restore overlap only. Transfer releases this fee;
        // active OwnedOwnerServer memory needs independent host admission before activation.
        let providers = self
            .mls
            .checked_mul(2)
            .and_then(|payload| payload.checked_add(self.mls_backing))
            .and_then(|fee| fee.checked_mul(2))
            .ok_or_else(refused)?;
        buffers.checked_add(providers).ok_or_else(refused)
    }
}

/// Prepaid bounded creation slot acquired BEFORE actual Server::found/Profile preparation.
/// It is transferred to the known-head reservation, never released beside retained frozen data.
pub(crate) struct OwnerPendingCreation {
    runtime: Weak<Mutex<Runtime>>,
    mount: Arc<()>,
    negative: RequestCancellation,
    limits: OwnerInitializationSnapshotLimits,
    hot_fee: usize,
    active: bool,
}
impl OwnerPendingCreation {
    /// Actual original paid limits; the private join retains this SAME owner through master,
    /// request/candidate overlap and later canonical initialization. Metadata is not payment.
    pub(crate) fn snapshot_limits(
        &self,
        store: &ServerStore,
    ) -> Result<OwnerInitializationSnapshotLimits, AppError> {
        if !self.active
            || self.negative.is_cancelled()
            || !Arc::ptr_eq(&self.mount, &store.registry_mount())
            || self
                .runtime
                .upgrade()
                .is_none_or(|runtime| !Arc::ptr_eq(&runtime, &store.owner_evidence.canonical))
        {
            return Err(refused());
        }
        Ok(self.limits)
    }
}
impl Drop for OwnerPendingCreation {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Some(runtime) = self.runtime.upgrade() {
            if let Ok(mut runtime) = runtime.lock() {
                if let (Some(bytes), Some(count)) = (
                    runtime.hot_bytes.checked_sub(self.hot_fee),
                    runtime.pending_creations.checked_sub(1),
                ) {
                    runtime.hot_bytes = bytes;
                    runtime.pending_creations = count;
                }
            }
        }
    }
}

/// Closed limits carried by an actual original Store capacity reservation. Labels alone never
/// authorize a save; final storage still needs an opaque admission permit and original head CAS.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OwnerInitializationSnapshotLimits {
    pub(crate) max_snapshot_bytes: usize,
    pub(crate) max_mls_bytes: usize,
    pub(crate) max_mls_backing_bytes: usize,
}

/// One paid original initialization. Non-Clone/nonserialized; retaining it keeps its exact
/// writer alive, but only weakly observes the mounted Store/admission. Dropping cannot keep the
/// vault OS session or keys alive, and it cannot release a retained canonical history allowance.
pub(crate) struct OwnerInitializationReservation {
    pub(super) runtime: Weak<Mutex<Runtime>>,
    pub(super) cell: Arc<head::HeadCell>,
    pub(super) writer: Option<OwnerWriterRegistration>,
    limits: OwnerInitializationSnapshotLimits,
    hot_fee: usize,
    attempt: [u8; 32],
    admission: ChatAdmissionPreparation,
}
impl std::fmt::Debug for OwnerInitializationReservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerInitializationReservation")
            .finish_non_exhaustive()
    }
}
impl OwnerInitializationReservation {
    pub(crate) fn validate_private_source(
        &self,
        group: &[u8],
        signer: &[u8; 32],
        attempt: [u8; 32],
        preparation: &ChatAdmissionPreparation,
    ) -> Result<(), AppError> {
        if group != self.admission.group()
            || signer != self.admission.signer()
            || attempt != self.attempt
            || preparation.group() != group
            || preparation.signer() != signer
            || preparation.nonce() != self.admission.nonce()
            || preparation.domain() != self.admission.domain()
            || !Arc::ptr_eq(
                &preparation.owner_identity(),
                &self.admission.owner_identity(),
            )
            || self.admission.reservation_observer().is_cancelled()
            || self.runtime.upgrade().is_none()
            || self
                .writer
                .as_ref()
                .is_none_or(|writer| writer.publication_authority().is_cancelled())
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn snapshot_limits(&self) -> OwnerInitializationSnapshotLimits {
        self.limits
    }
    pub(super) fn attempt(&self) -> [u8; 32] {
        self.attempt
    }
}
impl Drop for OwnerInitializationReservation {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.upgrade() {
            if let Ok(mut runtime) = runtime.lock() {
                // The same pool issued this exact fee. Poison/underflow fails closed rather
                // than granting extra capacity; retained per-head persistent charge is sticky.
                if let Some(remaining) = runtime.hot_bytes.checked_sub(self.hot_fee) {
                    runtime.hot_bytes = remaining;
                }
            }
        }
    }
}

impl ServerStore {
    /// Factory-module regressions use the SAME real configured pool reservation; no literal
    /// paid flag/scalar limit or fabricated source is accepted. No extra production issuer.
    #[cfg(test)]
    pub(crate) fn reserve_private_join_creation_for_test(
        &self,
    ) -> Result<OwnerPendingCreation, AppError> {
        self.reserve_pending_owner_creation()
    }

    /// Configure the whole-state checkpoint once per physical mount, BEFORE allocating any
    /// head/candidate. Repeating identical configuration is harmless; changing limits while
    /// obligations exist refuses. Startup disk inventory must complete before reservation.
    pub fn configure_owner_evidence_budget(
        &self,
        budget: OwnerEvidenceBudget,
    ) -> Result<(), AppError> {
        if !self.chat_vault_format()?.requires_owner_evidence() {
            return Err(refused());
        }
        let mut runtime = self
            .owner_evidence
            .canonical
            .lock()
            .map_err(|_| refused())?;
        if let Some(original) = runtime.budget {
            return if original == budget {
                Ok(())
            } else {
                Err(refused())
            };
        }
        // The physical exclusive mount is already held. Exact canonical inventory/restore is
        // the only permitted source for existing files; first creation cannot ignore them.
        self.inventory_owner_heads(&mut runtime, budget)?;
        runtime.budget = Some(budget);
        Ok(())
    }

    pub(super) fn reserve_pending_owner_creation(&self) -> Result<OwnerPendingCreation, AppError> {
        if !self.chat_vault_format()?.requires_owner_evidence() {
            return Err(refused());
        }
        let mut runtime = self
            .owner_evidence
            .canonical
            .lock()
            .map_err(|_| refused())?;
        let budget = runtime.budget.ok_or_else(refused)?;
        let count = runtime
            .pending_creations
            .checked_add(1)
            .ok_or_else(refused)?;
        let all = runtime.heads.len().checked_add(count).ok_or_else(refused)?;
        let fee = budget.buffer_fee()?;
        let hot = runtime
            .hot_bytes
            .checked_add(fee)
            .filter(|bytes| *bytes <= budget.hot_bytes)
            .ok_or_else(refused)?;
        if all > budget.max_heads
            || (all as u64)
                .checked_mul(budget.head_fee()?)
                .is_none_or(|bytes| bytes > budget.persistent_bytes)
        {
            return Err(refused());
        }
        runtime.pending_creations = count;
        runtime.hot_bytes = hot;
        Ok(OwnerPendingCreation {
            runtime: Arc::downgrade(&self.owner_evidence.canonical),
            mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            limits: OwnerInitializationSnapshotLimits {
                max_snapshot_bytes: budget.snapshot,
                max_mls_bytes: budget.mls,
                max_mls_backing_bytes: budget.mls_backing,
            },
            hot_fee: fee,
            active: true,
        })
    }

    pub(super) fn reserve_owner_initialization(
        &self,
        preparation: &ChatAdmissionPreparation,
        attempt: [u8; 32],
        creation: &mut OwnerPendingCreation,
    ) -> Result<OwnerInitializationReservation, AppError> {
        if !self.chat_vault_format()?.requires_owner_evidence()
            || preparation.reservation_observer().is_cancelled()
        {
            return Err(refused());
        }
        if !creation.active
            || creation.negative.is_cancelled()
            || !Arc::ptr_eq(&creation.mount, &self.registry_mount())
            || !Weak::ptr_eq(
                &creation.runtime,
                &Arc::downgrade(&self.owner_evidence.canonical),
            )
        {
            return Err(refused());
        }
        #[cfg(test)]
        {
            // Scheduling seam only: the actual original admission/creation was already checked.
            // Release the seam mutex before the callback; the test races real negative recovery
            // against the SAME subsequent pool admission, not a mocked successful authority.
            let before_pool = self
                .owner_evidence
                .before_owner_initialization_pool
                .lock()
                .map_err(|_| refused())?
                .take();
            if let Some(before_pool) = before_pool {
                before_pool();
            }
        }
        let mut runtime = self
            .owner_evidence
            .canonical
            .lock()
            .map_err(|_| refused())?;
        let budget = runtime.budget.ok_or_else(refused)?;
        let identifier = key(preparation.group(), preparation.signer());
        // Negative recovery can retire this exact Initializing row while this reservation waits
        // for the pool mutex. Never allocate/install a head from a source sampled before that
        // retirement; the observer is irreversible and must be checked under this actual guard.
        if preparation.reservation_observer().is_cancelled() || creation.negative.is_cancelled() {
            return Err(refused());
        }
        // No caller-controlled numeric IDs can grow an unbounded mutex map. Existing source
        // refuses first initialization; supported restore/rejoin must use its exact known head.
        if runtime.heads.contains_key(&identifier)
            || runtime.heads.len() >= budget.max_heads
            || ((runtime.heads.len() as u64) + 1)
                .checked_mul(budget.head_fee()?)
                .is_none_or(|fee| fee > budget.persistent_bytes)
        {
            return Err(refused());
        }
        let hot_fee = creation.hot_fee;
        if hot_fee != budget.buffer_fee()? || runtime.pending_creations == 0 {
            return Err(refused());
        }
        let cell = Arc::new(head::HeadCell::new(identifier));
        let writer = OwnerWriterRegistration::initial(self, &cell, preparation)?;
        runtime.heads.insert(identifier, cell.clone());
        runtime.pending_creations -= 1;
        creation.active = false;
        Ok(OwnerInitializationReservation {
            runtime: Arc::downgrade(&self.owner_evidence.canonical),
            cell,
            writer: Some(writer),
            limits: OwnerInitializationSnapshotLimits {
                max_snapshot_bytes: budget.snapshot,
                max_mls_bytes: budget.mls,
                max_mls_backing_bytes: budget.mls_backing,
            },
            hot_fee,
            attempt,
            admission: preparation.clone(),
        })
    }
}
