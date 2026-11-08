//! Existing Current2 borrowers for the actual operational host.
//!
//! Reuse the audited cached warm/cold/repaired association borrower, not its physical seed
//! cleanup validator or covering-write permission. The new opaque type cannot enter either
//! cleanup or Init1 writer APIs. Canonical proves the SAME installed writer separately, and
//! the physical text family proves the selected operational O/H after these cached checks.
use super::*;
use crate::store::chat_admission::current_lineage::CurrentFilenameAssociation;
use crate::store::persistence::V2TextOwnerRef;
use std::sync::Weak;

pub(in crate::store) struct V2TextCurrentAdmission<'writer> {
    owner: &'writer V2TextOwnerRef<'writer>,
    // Actual same held Runtime and closed original Scope family; no scope IO or seed read.
    read: GenesisCurrent2SettlementReadAdmission<'writer>,
}

/// A negative original-owner observation, not Current or selected-head proof. The closed
/// Scope authority retains its independently funded original metadata tails. A containing
/// text observer must additionally pay this wrapper/weak control block in its OWN invoice.
#[derive(Clone)]
pub(in crate::store) struct V2TextScopeNegative {
    identity: Weak<()>,
    observer: RequestCancellation,
}

pub(in crate::store) const V2_TEXT_SCOPE_METADATA_BYTES: usize =
    std::mem::size_of::<V2TextCurrentAdmission<'static>>()
        + std::mem::size_of::<V2TextScopeNegative>()
        + 64
        + std::mem::size_of::<CurrentFilenameAssociation<'static>>();

impl<'writer> V2TextCurrentAdmission<'writer> {
    /// Namespace family uses only this actual cached borrower. It carries neither a
    /// covering-write role nor permission to invoke the old seed physical validator.
    pub(in crate::store) fn original_read(
        &self,
    ) -> &GenesisCurrent2SettlementReadAdmission<'writer> {
        &self.read
    }
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.read.store()
    }
    pub(in crate::store) fn owner_ref(&self) -> &V2TextOwnerRef<'writer> {
        self.owner
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'writer> {
        self.owner.source()
    }
    pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
        self.owner.task()
    }
    pub(in crate::store) fn group(&self) -> &[u8] {
        self.read.group()
    }
    pub(in crate::store) fn signer(&self) -> &[u8; 32] {
        self.read.signer()
    }
    /// Stable row nonce from the SAME actual held Scope family, never a UI/mount/read ID.
    /// This is context DATA; the admission and actual source proof remain mandatory.
    pub(in crate::store) fn scope_nonce_data(&self) -> Result<[u8; 32], AppError> {
        self.validate_source_only()?;
        match (
            self.warm_admission(),
            self.cold_admission(),
            self.repaired_admission(),
        ) {
            (Some(a), None, None) => Ok(a.preparation.inner.nonce()),
            (None, Some(a), None) => Ok(a.nonce()),
            (None, None, Some(a)) => Ok(a.owner_admission().nonce()),
            _ => Err(refused()),
        }
    }
    pub(in crate::store) fn writer_observation(&self) -> Result<&V2WriterObservation, AppError> {
        self.read.writer_observation()
    }
    pub(in crate::store) fn warm_admission(&self) -> Option<&V2CurrentAdmission<'writer>> {
        self.read.warm_admission()
    }
    pub(in crate::store) fn cold_admission(&self) -> Option<&ColdOwnedCurrentAdmission<'writer>> {
        self.read.cold_admission()
    }
    pub(in crate::store) fn repaired_admission(
        &self,
    ) -> Option<&ColdRepairedGenesisCurrentAdmission<'writer>> {
        self.read.repaired_admission()
    }

    /// Called before and after subordinate waits. The reused borrower checks the actual
    /// clean loaded row2/nonce/owner/binding and all group lineage negatives. It performs
    /// no old mandatory-seed physical validation, pool wait, codec, SDK or filesystem work.
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.owner.require_closed_family()?;
        self.read.validate_source_only()?;
        self.owner
            .require_original_negative(self.store(), self.writer_observation()?)
    }

    pub(in crate::store) fn current_filename_association(
        &self,
    ) -> Result<CurrentFilenameAssociation<'_>, AppError> {
        self.validate_source_only()?;
        // Each loan comes from the SAME actual Runtime held by this callback. No startup
        // snapshot, cloned index or budget-None-as-empty shortcut may substitute for it.
        match (
            self.warm_admission(),
            self.cold_admission(),
            self.repaired_admission(),
        ) {
            (Some(a), None, None) => a.current_filename_association(),
            (None, Some(a), None) => a.current_filename_association(),
            (None, None, Some(a)) => a.current_filename_association(),
            _ => Err(refused()),
        }
    }

    fn binding(&self) -> Result<&V2CurrentBinding, AppError> {
        match (
            self.owner.warm_scope(),
            self.owner.cold_scope(),
            self.owner.repaired_scope(),
        ) {
            (Some(scope), None, None) => scope.current_binding(),
            (None, Some(scope), None) => scope.current_binding(),
            (None, None, Some(scope)) => scope.current_binding(),
            _ => Err(refused()),
        }
    }

    /// No allocation: clone already-funded exact negative controls only. It may be captured
    /// under AD after the containing consumer's metadata allowance was acquired outside it.
    /// This does not detach a current permit or retain a revoker/sender/Store/SDK graph.
    pub(in crate::store) fn observe_scope_negative(&self) -> Result<V2TextScopeNegative, AppError> {
        self.validate_source_only()?;
        let observer = match (
            self.warm_admission(),
            self.cold_admission(),
            self.repaired_admission(),
        ) {
            (Some(a), None, None) => a.preparation.inner.admission.clone(),
            (None, Some(a), None) => a.owner_observer().clone(),
            (None, None, Some(a)) => a.owner_admission().owner_observer().clone(),
            _ => return Err(refused()),
        };
        Ok(V2TextScopeNegative {
            identity: Arc::downgrade(&self.binding()?.inner.owner_identity),
            observer,
        })
    }
}

impl V2TextScopeNegative {
    /// Only the actual closed Scope authority is called. It uses weak/atomic original
    /// cancellation, with no Root/Runtime/pool/host lock or filesystem callback.
    pub(in crate::store) fn is_cancelled(&self) -> bool {
        self.identity.strong_count() == 0 || self.observer.is_cancelled()
    }
    pub(in crate::store) fn require_same_original(
        &self,
        admission: &V2TextCurrentAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        if self.is_cancelled()
            || !self
                .identity
                .ptr_eq(&Arc::downgrade(&admission.binding()?.inner.owner_identity))
        {
            return Err(refused());
        }
        Ok(())
    }
}

impl ServerStore {
    /// One read/effect-admission callback over the original clean Current2 owner. This
    /// writes no Scope and issues no physical selection permission by itself. Its HRTB
    /// prevents an admission/Runtime/namespace guard from escaping the held AD writer.
    /// Callbacks must BORROW outer retained candidate/fee slots: capturing their last
    /// owning resource by value and using `?` before returning it would still drop it
    /// inside the callback under AD. Transfer only after all fallible callback checks, or
    /// cache the real owning outcome externally before a later negative can return Err.
    pub(in crate::store) fn with_v2_text_current<T>(
        &self,
        owner: &V2TextOwnerRef<'_>,
        callback: impl for<'writer> FnOnce(&V2TextCurrentAdmission<'writer>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        owner.require_closed_family()?;
        // These owning results are declared BEFORE Runtime. On a late original-source
        // refusal, T may contain Files or the last metadata payment. It must drain only
        // after AD and all callback subordinate guards have actually been released.
        let outcome;
        let final_check;
        {
            let runtime = self.chat_mount.locked()?;
            let read = super::settlement::read(self, &runtime, owner.original_association())?;
            let admission = V2TextCurrentAdmission { owner, read };
            admission.validate_source_only()?;
            outcome = callback(&admission);
            final_check = admission.validate_source_only();
            // The block explicitly ends the real Runtime borrow BEFORE propagating a
            // late error or returning/destroying the callback's owning outcome.
        }
        final_check?;
        outcome
    }
}

impl std::fmt::Debug for V2TextCurrentAdmission<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextCurrentAdmission")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for V2TextScopeNegative {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextScopeNegative")
            .finish_non_exhaustive()
    }
}
