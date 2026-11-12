//! Independently paid negative origin for V2 projection/source use.
//!
//! A token may outlive its host. Retain only tiny original identities and the already
//! funded closed cancellation chains, never the SDK, ReadCapture, Store, node, sender,
//! selected-head chain or physical session. This is NOT Current or a drain acknowledgement.
use super::owner::V2TextOwnerRef;
use crate::store::chat_admission::v2_current::{V2TextCurrentAdmission, V2TextScopeNegative};
use crate::store::critical_namespaces::current_layout::OriginalV2GenesisGroupObservation;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::critical_namespaces::refused;
use crate::store::owner_evidence::canonical::v2_registration::V2WriterObservation;
use crate::store::ServerStore;
use crate::AppError;
use catcoms_replication::chat::V2OriginNegative;
use catcoms_rt::RequestCancellation;
use std::sync::{Arc, OnceLock, Weak};

struct SourceNegativeBody {
    read_identity: Weak<()>,
    mount: RequestCancellation,
    root: RequestCancellation,
    task: OriginalV2GenesisGroupObservation,
    writer: V2WriterObservation,
    scope: OnceLock<V2TextScopeNegative>,
    // LAST: even when source-heavy owners have dropped, this body/control block, exact
    // Scope weak identity and tiny read/task/watch tails retain their OWN actual payment.
    _fee: CatalogReadReservation,
}

/// Prepare allocation/payment outside every host writer. The incomplete body is deliberately
/// not live and cannot be supplied as a positive source to Core before actual Scope capture.
pub(in crate::store) struct PreparedV2TextSourceObserver {
    body: Arc<SourceNegativeBody>,
}

#[derive(Clone)]
pub(in crate::store) struct V2TextSourceNegativeObserver {
    body: Arc<SourceNegativeBody>,
}

/// Exact requested metadata plus conservative tiny Arc/weak/callback wrapper allowance.
/// This is a NEW independent debit, not old source/SDK or transaction-work overlap credit.
/// Producer/view/window payments remain separate; this makes no OS/RSS claim.
pub(in crate::store) const V2_TEXT_SOURCE_OBSERVER_BYTES: usize =
    std::mem::size_of::<SourceNegativeBody>()
        + std::mem::size_of::<PreparedV2TextSourceObserver>()
        + std::mem::size_of::<V2TextSourceNegativeObserver>()
        + crate::store::chat_admission::v2_current::V2_TEXT_SCOPE_METADATA_BYTES
        + crate::store::persistence::ORIGINAL_BOOTSTRAP_READ_OBSERVATION_BYTES
        + 4 * 64;

impl PreparedV2TextSourceObserver {
    /// Observe only the exact independently paid observer debit before any body alias.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn observe_metadata_drop_for_test(
        &mut self,
        notice: Arc<std::sync::atomic::AtomicBool>,
    ) {
        Arc::get_mut(&mut self.body)
            .expect("test installs notice before any origin capture/alias")
            ._fee
            .observe_drop_for_test(notice);
    }
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
    ) -> Result<Self, AppError> {
        let writer = owner.observe_writer();
        owner.require_original_negative(store, &writer)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(V2_TEXT_SOURCE_OBSERVER_BYTES)?;
        fee.validate(pool, V2_TEXT_SOURCE_OBSERVER_BYTES)?;
        // The account wait may have outlived the original host. Repeat cached negatives
        // before any new control-block allocation. No borrowed SourceRef escapes setup.
        owner.require_original_negative(store, &writer)?;
        let capture = &owner.source().source().capture;
        Ok(Self {
            body: Arc::new(SourceNegativeBody {
                read_identity: Arc::downgrade(&capture.identity),
                mount: capture.negative.clone(),
                root: capture.root_negative.clone(),
                task: owner.task().observe_original(),
                writer,
                scope: OnceLock::new(),
                _fee: fee,
            }),
        })
    }

    /// Cache the SAME actual Scope once; retry may only reobserve it. All allocations are
    /// already paid/prepared, and a final failure leaves this original body in caller custody.
    pub(in crate::store) fn capture(
        &self,
        admission: &V2TextCurrentAdmission<'_>,
    ) -> Result<V2TextSourceNegativeObserver, AppError> {
        let pool = &admission
            .store()
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.body
            ._fee
            .validate(pool, V2_TEXT_SOURCE_OBSERVER_BYTES)?;
        admission.validate_source_only()?;
        self.body.task.require_same_original(admission.task())?;
        admission
            .owner_ref()
            .require_original_negative(admission.store(), &self.body.writer)?;
        if !self.body.read_identity.ptr_eq(&Arc::downgrade(
            &admission.source().source().capture.identity,
        )) {
            return Err(refused());
        }
        if let Some(scope) = self.body.scope.get() {
            scope.require_same_original(admission)?;
        } else {
            let scope = admission.observe_scope_negative()?;
            self.body.scope.set(scope).map_err(|_| refused())?;
        }
        // Scope capture is negative DATA only. It cannot create permission if cancellation
        // wins here; the body stays permanently tied to the first original owner identity.
        if !self.body.is_live() {
            return Err(refused());
        }
        admission.validate_source_only()?;
        Ok(V2TextSourceNegativeObserver {
            body: self.body.clone(),
        })
    }
}

impl V2TextSourceNegativeObserver {
    /// Exact cached original association, safe under the actual Scope/canonical writer.
    /// No pool/FS/SDK operation or positive-source reconstruction is performed.
    pub(in crate::store) fn require_same_original(
        &self,
        admission: &V2TextCurrentAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        self.body.task.require_same_original(admission.task())?;
        admission
            .owner_ref()
            .require_original_negative(admission.store(), &self.body.writer)?;
        self.body
            .scope
            .get()
            .ok_or_else(refused)?
            .require_same_original(admission)?;
        if !self.is_live()
            || !self.body.read_identity.ptr_eq(&Arc::downgrade(
                &admission.source().source().capture.identity,
            ))
        {
            return Err(refused());
        }
        Ok(())
    }
    /// Arc coercion/clone only: Core retains the SAME already-paid immutable origin body.
    /// There is no second allocation, callback registration, source capture or renewal.
    pub(in crate::store) fn origin_negative(&self) -> Arc<dyn V2OriginNegative> {
        self.body.clone()
    }

    pub(in crate::store) fn is_live(&self) -> bool {
        self.body.is_live()
    }
}

impl V2OriginNegative for SourceNegativeBody {
    fn is_live(&self) -> bool {
        self.read_identity.strong_count() != 0
            && !self.mount.is_cancelled()
            && !self.root.is_cancelled()
            && !self.task.is_cancelled()
            && !self.writer.is_cancelled()
            && self.scope.get().is_some_and(|scope| !scope.is_cancelled())
    }
}

impl std::fmt::Debug for SourceNegativeBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextSourceNegativeBody")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for PreparedV2TextSourceObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2TextSourceObserver")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for V2TextSourceNegativeObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextSourceNegativeObserver")
            .finish_non_exhaustive()
    }
}
