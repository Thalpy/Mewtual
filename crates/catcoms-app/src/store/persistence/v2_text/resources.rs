//! Actual resource custody for the paired physical capture/work cohort.
//!
//! The actual capture's full layout invoice and Current guard consume these payments.
//! A resource loan is not a source, selected-head, SDK operation, work or commit permit.
use super::common::refused;
use crate::store::chat_admission::v2_current::{
    V2TextCurrentAdmission, V2TextScopeNegative, V2_TEXT_SCOPE_METADATA_BYTES,
};
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::persistence::cold_source_read::OriginalBootstrapReadObservation;
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::ServerStore;
use crate::AppError;
use catcoms_rt::RequestCancellation;
use catcoms_sync::owner_v2::{
    BootstrapSourceContextData, BootstrapSourceId, OriginalV2GenesisSeedObservation,
};
use std::sync::{Arc, OnceLock};

#[cfg(all(test, windows))]
thread_local! {
    // Test-only exact-debit observation, consumed before the paid body is first shared.
    // A scoped reset prevents an aborted fixture leaking its notice into another prepare.
    static NEXT_READ_FEE_NOTICE: std::cell::RefCell<Option<Arc<std::sync::atomic::AtomicBool>>> = const { std::cell::RefCell::new(None) };
}

pub(in crate::store) const TEXT_READ_METADATA_FEE: usize = 1024 * 1024;
pub(in crate::store) const TEXT_WORK_BUFFER_FEE: usize = 16 * 1024 * 1024;

struct ReadResourceBody {
    original_read: OriginalBootstrapReadObservation,
    original_sdk: OriginalV2GenesisSeedObservation,
    context: BootstrapSourceContextData,
    source: BootstrapSourceId,
    mount: Arc<()>,
    negative: RequestCancellation,
    // Bound once ONLY by the genuine Current guard constructor. This is a tiny negative
    // observation, not a live owner/SDK or a permission. Source-only preparation starts
    // unbound; positive post-capture DATA cuts must require this actual Scope binding.
    scope: OnceLock<V2TextScopeNegative>,
    pool: CriticalCatalogReadPool,
    // LAST: contexts, weak-control tails and all observations precede the actual refund.
    fee: CatalogReadReservation,
}
pub(in crate::store) const TEXT_READ_RESOURCE_LAYOUT_BYTES: usize =
    std::mem::size_of::<ReadResourceBody>()
        + std::mem::size_of::<V2TextReadResources>()
        + V2_TEXT_SCOPE_METADATA_BYTES
        + 256
        + 4 * 64;

/// SAME paid small body. It contains no source/SDK owner, graph, Store, key or OS session.
/// Long-lived saved/negative tails may retain this1MiB loan, never the16MiB work payment.
#[derive(Clone)]
pub(in crate::store) struct V2TextReadResources {
    body: Arc<ReadResourceBody>,
}
impl std::fmt::Debug for V2TextReadResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextReadResources")
            .finish_non_exhaustive()
    }
}
impl V2TextReadResources {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn with_fee_drop_notice_for_test<T>(
        notice: Arc<std::sync::atomic::AtomicBool>,
        prepare: impl FnOnce() -> T,
    ) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                NEXT_READ_FEE_NOTICE.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        NEXT_READ_FEE_NOTICE.with(|slot| {
            assert!(slot.borrow().is_none(), "nested exact-read-fee observation");
            *slot.borrow_mut() = Some(notice);
        });
        let _reset = Reset;
        let output = prepare();
        NEXT_READ_FEE_NOTICE.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "fixture did not acquire its actual read debit"
            );
        });
        output
    }
    /// Physical-family only. Every concrete capture first checks its COMPLETE simultaneous
    /// layouts/path/control-buffer invoice against this class, then calls here OUTSIDE all
    /// writers. Source-only preparation works before a cold task/Scope owner exists.
    pub(super) fn prepare_paid(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<Self, AppError> {
        source.validate_original(store)?;
        store.validate_mounted_current_layout()?;
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool
            .clone();
        let fee = pool.try_reserve(TEXT_READ_METADATA_FEE)?;
        #[cfg(all(test, windows))]
        let fee = {
            let mut fee = fee;
            NEXT_READ_FEE_NOTICE.with(|slot| {
                if let Some(notice) = slot.borrow_mut().take() {
                    fee.observe_drop_for_test(notice);
                }
            });
            fee
        };
        fee.validate(&pool, TEXT_READ_METADATA_FEE)?;
        source.validate_original(store)?;
        store.validate_mounted_current_layout()?;
        let resources = Self {
            body: Arc::new(ReadResourceBody {
                original_read: source.source().observe_original(),
                original_sdk: source.seed().observe_original_seed_data(),
                context: source.data().context.clone(),
                source: source.data().source,
                mount: store.registry_mount(),
                negative: store.chat_mount.observer(),
                scope: OnceLock::new(),
                pool,
                fee,
            }),
        };
        resources.require_original_negative(store, source)?;
        Ok(resources)
    }
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.body, &other.body) {
            return Err(refused());
        }
        Ok(())
    }
    /// Called ONLY while borrowing an actual Current admission, before physical read
    /// permission. Cache the first exact Scope; a later owner cannot rebind equal labels.
    /// This may validate the prepaid account before the final source/name proof, but
    /// allocates no body and acquires no Source or Current authority from these resources.
    pub(in crate::store) fn bind_scope_negative(
        &self,
        admission: &V2TextCurrentAdmission<'_>,
    ) -> Result<(), AppError> {
        self.validate_original(admission.store(), admission.source())?;
        admission.validate_source_only()?;
        if let Some(scope) = self.body.scope.get() {
            scope.require_same_original(admission)?;
        } else {
            let scope = admission.observe_scope_negative()?;
            self.body.scope.set(scope).map_err(|_| refused())?;
        }
        self.body
            .scope
            .get()
            .ok_or_else(refused)?
            .require_same_original(admission)?;
        self.require_bound_original_negative(admission.store(), admission.source())
    }
    /// Cached negative boundary AFTER genuine Current capture. Source/SDK identity alone
    /// does not observe Scope revocation; data producers must use this after every RNG cut.
    /// No account/FS/SDK operation occurs, and a true result is not a Current permission.
    pub(in crate::store) fn require_bound_original_negative(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(store, source)?;
        if self.body.scope.get().ok_or_else(refused)?.is_cancelled() {
            return Err(refused());
        }
        Ok(())
    }
    /// Closed cached observations only: no account/host lock, FS, SDK, encode or allocation.
    /// This is deliberately weaker than an original physical named-source/Current proof.
    pub(in crate::store) fn require_original_negative(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        source.require_original_negative(store)?;
        self.body
            .original_read
            .require_same_original(source.source())?;
        self.body
            .original_sdk
            .require_same_seed_data(source.seed())?;
        if self.body.negative.is_cancelled()
            || self
                .body
                .scope
                .get()
                .is_some_and(V2TextScopeNegative::is_cancelled)
            || !Arc::ptr_eq(&self.body.mount, &store.registry_mount())
            || self.body.context != *source.data().context
            || self.body.source != source.data().source
        {
            return Err(refused());
        }
        Ok(())
    }
    /// May WAIT on the original account counter. Real source/ID3/held-name proofs follow
    /// this validation after every wait; it acquires no new credit and gives no permission.
    pub(in crate::store) fn validate_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.require_original_negative(store, source)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.body.fee.validate(pool, TEXT_READ_METADATA_FEE)?;
        self.require_original_negative(store, source)
    }
    fn validate_account_only(&self) -> Result<(), AppError> {
        self.body
            .fee
            .validate(&self.body.pool, TEXT_READ_METADATA_FEE)
    }
}

/// Temporary actual work payment. Do not inherit this in Scope/Current/observer bodies.
/// The concrete prepared-work owner may Arc-share THIS same object with its immutable
/// payload/results while their combined frozen footprint remains under the single invoice.
/// Files/results/large buffers must precede it in every owning struct's drop order.
pub(in crate::store) struct V2TextWorkResources {
    read: V2TextReadResources,
    fee: CatalogReadReservation,
}
impl std::fmt::Debug for V2TextWorkResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextWorkResources")
            .finish_non_exhaustive()
    }
}
impl V2TextWorkResources {
    /// The SAME small read body, not a new credit, source proof or effect admission.
    pub(super) fn read_resources(&self) -> &V2TextReadResources {
        &self.read
    }
    /// Repeat BOTH the actual prepaid work account and the SAME original read/SDK
    /// association before a producer copies or seals data outside host writers.
    pub(super) fn validate_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.validate_prepaid_account()?;
        self.read.validate_original(store, source)
    }
    /// The real prepared worker checks the FULL buffer/capacity/phase-overlap invoice first.
    /// No SourceSaved grant or decoder/SDK/read fee is reused as this independent credit.
    pub(super) fn prepare_paid(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        read: &V2TextReadResources,
    ) -> Result<Self, AppError> {
        read.validate_original(store, source)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(TEXT_WORK_BUFFER_FEE)?;
        fee.validate(pool, TEXT_WORK_BUFFER_FEE)?;
        read.validate_original(store, source)?;
        Ok(Self {
            read: read.clone(),
            fee,
        })
    }
    pub(in crate::store) fn require_same_read_resources(
        &self,
        read: &V2TextReadResources,
    ) -> Result<(), AppError> {
        self.read.require_same_original(read)
    }
    /// Funded account ONLY, suitable during already-admitted owner-dead immutable DATA
    /// drain. Source cancellation cannot renew a key/Current/commit permission from this.
    /// The separate affine running custody retains the original OS-session lock through IO.
    pub(in crate::store) fn validate_prepaid_account(&self) -> Result<(), AppError> {
        self.read.validate_account_only()?;
        self.fee
            .validate(&self.read.body.pool, TEXT_WORK_BUFFER_FEE)
    }
}

#[cfg(all(test, windows))]
mod observations;
