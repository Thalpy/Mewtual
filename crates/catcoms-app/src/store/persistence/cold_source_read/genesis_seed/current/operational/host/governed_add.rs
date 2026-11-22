//! Payments for the original native governed Add candidate. The candidate
//! keeps this debit for as long as its SDK graph or pending output owns it.
use super::*;
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::ServerStore;
use catcoms_mls::persist::text_current_add::{
    TextCurrentAddAdmission, TextCurrentAddLimits, TextCurrentAddReservation,
};

impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn retire_add_for_closed_ui(
        &mut self,
        closed: &[Option<&catcoms_rt::NativeChatKeyDrain>],
    ) -> Result<(), AppError> {
        self.owner.retire_add_for_closed_ui(closed)
    }

    /// Whole native Source retirement has exclusive custody of this idle host.
    /// It must retire existing Add Ends before waiting for drained keys.
    pub(in crate::store) fn discard_add_for_closed_source(
        &mut self,
        closed: &catcoms_rt::NativeChatKeyDrain,
    ) -> Result<(), AppError> {
        self.owner.discard_add_for_closed_source(closed)
    }
}

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

/// Must run outside all writers, before the planned reservation can commit.
/// It acquires candidate credit from this Store's actual mounted account.
pub(super) fn prepare_sdk_admission(
    store: &ServerStore,
) -> Result<TextCurrentAddAdmission, AppError> {
    let policy = store
        .archive_bootstrap_policy
        .as_ref()
        .ok_or_else(refused)?;
    GovernedAddSdkReservation::prepare(&policy.pending.pool)
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests;
