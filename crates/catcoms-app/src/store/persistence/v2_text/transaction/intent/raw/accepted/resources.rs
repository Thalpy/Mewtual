//! NEW independently funded B metadata2MiB class (actual named invoice1,232,709B). This is ADDITIONAL to the original
//! A/I/S transaction1.25MiB and active16MiB payload/work, not unused credit in either.
//! Constructor waits happen outside writers and BEFORE any B copy/Box/allocation.
use super::*;
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};

#[cfg(all(test, windows))]
thread_local! {static NEXT_B_FEE_NOTICE:std::cell::RefCell<Option<Arc<std::sync::atomic::AtomicBool>>>=const{std::cell::RefCell::new(None)};}
pub(super) const ACCEPTED_METADATA_FEE: usize = 2 * 1024 * 1024;
mod retirement;
pub(in crate::store) use retirement::V2TextRetirementRootsClaim;
pub(super) const ACCEPTED_PAYMENT_LAYOUT_BYTES: usize =
    4 * std::mem::size_of::<AcceptedMetadataPayment>()
        + 4 * std::mem::size_of::<V2TextRetirementRootsClaim>()
        + 4 * 32;
struct AcceptedMetadataPayment {
    retirement_roots_busy: std::sync::atomic::AtomicBool,
    pool: CriticalCatalogReadPool,
    // LAST: observers cannot refund the actual new debit before their own cleanup.
    fee: CatalogReadReservation,
}
#[derive(Clone)]
pub(super) struct AcceptedMetadataResources {
    body: Arc<AcceptedMetadataPayment>,
}
impl AcceptedMetadataResources {
    pub(super) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        signature: &PreparedV2TextRawSignature,
    ) -> Result<Self, AppError> {
        signature.intent.retained.custody.validate_accounts(store)?;
        signature
            .intent
            .unsigned
            .validate_original(store, source, &signature.intent.retained)?;
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool
            .clone();
        let fee = pool.try_reserve(ACCEPTED_METADATA_FEE)?;
        fee.validate(&pool, ACCEPTED_METADATA_FEE)?;
        #[cfg(all(test, windows))]
        let fee = NEXT_B_FEE_NOTICE.with(|slot| {
            let mut fee = fee;
            if let Some(notice) = slot.borrow_mut().take() {
                fee.observe_drop_for_test(notice);
            }
            fee
        });
        // Waiting for new B credit cannot bless a now-closed original S/SDK/Source.
        signature
            .intent
            .unsigned
            .validate_original(store, source, &signature.intent.retained)?;
        signature.require_original_raw_data()?;
        Ok(Self {
            body: Arc::new(AcceptedMetadataPayment {
                retirement_roots_busy: std::sync::atomic::AtomicBool::new(false),
                pool,
                fee,
            }),
        })
    }
    pub(super) fn validate_prepaid_account(&self) -> Result<(), AppError> {
        self.body
            .fee
            .validate(&self.body.pool, ACCEPTED_METADATA_FEE)
    }
    pub(super) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        if Arc::ptr_eq(&self.body, &other.body) {
            Ok(())
        } else {
            Err(refused())
        }
    }
}

#[cfg(all(test, windows))]
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn with_metadata_fee_drop_notice_for_test<T>(
        notice: Arc<std::sync::atomic::AtomicBool>,
        prepare: impl FnOnce() -> T,
    ) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                NEXT_B_FEE_NOTICE.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        NEXT_B_FEE_NOTICE.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some(notice);
        });
        let _reset = Reset;
        let result = prepare();
        NEXT_B_FEE_NOTICE.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "fixture did not reserve actual new B metadata"
            )
        });
        result
    }
}
