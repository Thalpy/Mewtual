//! Actual host-issued SDK graph fees from the SAME original canonical hot pool.
//! Only accounting/caps/metadata are retained: no Store, key, source, provider or live sender.
use super::*;
use catcoms_mls::persist::{
    OwnerSdkGraphReservation, OwnerSdkRestoreAdmission, OwnerSdkRestoreLimits,
};

struct GraphReservation {
    // Strong original account deliberately survives Store Drop; it contains no vault keys/session.
    pool: Arc<Mutex<Runtime>>,
    bytes: usize,
    limits: OwnerSdkRestoreLimits,
}
impl OwnerSdkGraphReservation for GraphReservation {
    fn limits(&self) -> OwnerSdkRestoreLimits {
        self.limits
    }
}
impl Drop for GraphReservation {
    fn drop(&mut self) {
        if let Ok(mut pool) = self.pool.lock() {
            if let Some(bytes) = pool.hot_bytes.checked_sub(self.bytes) {
                pool.hot_bytes = bytes;
            } else {
                pool.sdk_unavailable = true;
            } // Never refund/guess after accounting corruption.
        }
        // Poison permanently refuses future admissions at lock(), without recovering a counter.
    }
}
impl OwnerEvidenceBudget {
    /// Opt into the exact pinned initial SDK graph decoder. These immutable negative caps are
    /// not admission/payment: the original source consumer still acquires actual fees below.
    /// Two graphs may coexist during child/parent validation; require that potential capacity.
    /// Default checkpoints keep their explicit legacy typed-heap limitation unchanged.
    pub fn with_closed_sdk_restore(
        mut self,
        max_graph_bytes: usize,
        max_decode_work: usize,
    ) -> Result<Self, AppError> {
        if max_graph_bytes == 0 || max_graph_bytes > 16 * 1024 * 1024 || max_decode_work == 0 {
            return Err(refused());
        }
        let overlap = max_graph_bytes
            .checked_mul(2)
            .and_then(|n| n.checked_add(self.buffer_fee().ok()?))
            .ok_or_else(refused)?;
        if overlap > self.hot_bytes {
            return Err(refused());
        }
        self.sdk = Some(OwnerSdkRestoreLimits {
            max_typed_bytes: max_graph_bytes,
            max_work: max_decode_work,
        });
        Ok(self)
    }
}
impl ServerStore {
    /// Called ONLY after the actual original source validated its mount/pin/account and outside
    /// admission/head/pool writers. Return non-Clone independently debited fees before SDK entry;
    /// the parent flag is derived from that actual authenticated retained source, never a reader.
    pub(in crate::store::owner_evidence::canonical) fn reserve_owner_sdk_graphs(
        &self,
        needs_parent: bool,
    ) -> Result<
        (
            Option<OwnerSdkRestoreAdmission>,
            Option<OwnerSdkRestoreAdmission>,
        ),
        AppError,
    > {
        let pool = self.owner_evidence.canonical.clone();
        let reservations = {
            let mut runtime = pool.lock().map_err(|_| refused())?;
            if runtime.sdk_unavailable {
                return Err(refused());
            }
            let budget = runtime.budget.ok_or_else(refused)?;
            let Some(limits) = budget.sdk else {
                return Ok((None, None));
            };
            let count = if needs_parent { 2 } else { 1 };
            let fee = limits.max_typed_bytes;
            let total = fee.checked_mul(count).ok_or_else(refused)?;
            let hot = runtime
                .hot_bytes
                .checked_add(total)
                .filter(|n| *n <= budget.hot_bytes)
                .ok_or_else(refused)?;
            runtime.hot_bytes = hot;
            // No fallible/allocating call between the debit and ownership of both fees.
            (
                GraphReservation {
                    pool: pool.clone(),
                    bytes: fee,
                    limits,
                },
                needs_parent.then(|| GraphReservation {
                    pool: pool.clone(),
                    bytes: fee,
                    limits,
                }),
            )
        };
        // Guard is gone BEFORE trait/cached-cap construction or any possible fee.Drop callback.
        let (child, parent) = reservations;
        let child = OwnerSdkRestoreAdmission::new(child)?;
        let parent = parent.map(OwnerSdkRestoreAdmission::new).transpose()?;
        Ok((Some(child), parent))
    }
}

#[cfg(test)]
mod tests;
