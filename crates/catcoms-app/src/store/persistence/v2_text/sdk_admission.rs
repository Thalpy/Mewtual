//! Candidate credit for an original mounted founder or existing member Add.
use super::common::refused;
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::ServerStore;
use crate::AppError;
use catcoms_mls::persist::text_current_add::{
    TextCurrentAddAdmission, TextCurrentAddLimits, TextCurrentAddReservation,
};

struct GovernedAddSdkReservation {
    _debit: CatalogReadReservation,
}

impl TextCurrentAddReservation for GovernedAddSdkReservation {
    fn limits(&self) -> TextCurrentAddLimits {
        TextCurrentAddLimits::governed_text_v2()
    }
}

impl GovernedAddSdkReservation {
    fn required_bytes() -> Result<usize, AppError> {
        TextCurrentAddLimits::governed_text_v2()
            .buffer_reservation_bytes()
            .and_then(|buffers| {
                buffers.checked_add(TextCurrentAddAdmission::metadata_layout_bytes::<Self>()?)
            })
            .ok_or_else(refused)
    }

    fn prepare(pool: &CriticalCatalogReadPool) -> Result<TextCurrentAddAdmission, AppError> {
        let bytes = Self::required_bytes()?;
        let debit = pool.try_reserve(bytes)?;
        debit.validate(pool, bytes)?;
        TextCurrentAddAdmission::new(Self { _debit: debit }).map_err(|_| refused())
    }
}

/// Reserve from the mounted account before entering the selected writer.
pub(in crate::store) fn prepare_sdk_admission(
    store: &ServerStore,
) -> Result<TextCurrentAddAdmission, AppError> {
    let policy = store
        .archive_bootstrap_policy
        .as_ref()
        .ok_or_else(refused)?;
    GovernedAddSdkReservation::prepare(&policy.pending.pool)
}

#[cfg(all(test, any(windows, target_os = "linux")))]
#[test]
fn governed_add_sdk_budget_keeps_the_exact_debit_in_the_admission() {
    let bytes = GovernedAddSdkReservation::required_bytes().unwrap();
    let short = CriticalCatalogReadPool::new(bytes - 1).unwrap();
    assert!(GovernedAddSdkReservation::prepare(&short).is_err());
    assert_eq!(short.charged_for_test(), 0);

    let exact = CriticalCatalogReadPool::new(bytes).unwrap();
    let admission = GovernedAddSdkReservation::prepare(&exact).unwrap();
    assert!(admission.supports_routing_data());
    assert_eq!(exact.charged_for_test(), bytes);
    assert!(GovernedAddSdkReservation::prepare(&exact.clone()).is_err());
    drop(admission);
    assert_eq!(exact.charged_for_test(), 0);
    assert!(GovernedAddSdkReservation::prepare(&exact).is_ok());
}
