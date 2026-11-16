//! Settle an already-authorized dirty scope inventory, independently of any group task.
//! Cold reads can need this BEFORE a Head/task/owner exists; live source or owner checks
//! would strand that earlier phase. This returns no activation, source or binding authority.
use super::*;

impl ServerStore {
    /// One explicit paid maintenance attempt against this SAME loaded managed inventory.
    /// The caller acquires the actual temporary scope credit outside host writers. Only the
    /// existing flush may change dirty -> clean; no row, owner, nonce, phase, claim, receipt
    /// or source is created, replaced, reloaded or adopted. Ordinary scope/Current admission
    /// must still repeat all its own source, owner, group and artifact checks afterward.
    ///
    /// A cancelled source/owner may have previously authorized rows waiting for durability.
    /// Saving that debt does not revive its publisher. A cancelled MOUNT, unsupported format,
    /// lost original namespace or foreign/insufficient credit still refuses maintenance.
    pub(in crate::store) fn resync_v2_current_scope_inventory(
        &self,
        scope_buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        // Pay/check BEFORE waiting on admission or performing namespace I/O. The resource
        // loan is not a mount/source capability and is never acquired/refunded in this call.
        require_buffers(self, scope_buffers)?;
        let mut runtime = self.chat_mount.locked()?;
        require_buffers(self, scope_buffers)?;
        require_base(self, &runtime)?;
        // AD -> brief read-only Root, including the original OS/root and ID3 marker proof.
        // Its function-local Root/current guards have dropped before any scope write below.
        self.validate_mounted_current_layout()?;
        require_base(self, &runtime)?;
        if runtime.dirty {
            require_buffers(self, scope_buffers)?;
            // Account validation may wait; repeat the actual namespace and mount AFTER it.
            self.validate_mounted_current_layout()?;
            require_base(self, &runtime)?;
            // Serialize the SAME latest table under its original writer, preserving all
            // groups. Never restore a cached table image, infer per-row durable floors,
            // sample a nonce, reload disk or manufacture a replacement owner for this repair.
            self.flush_chat_admissions(&mut runtime, rng)?;
        }
        // Successful write/sync is not present-time mount/namespace evidence. Repeat the
        // actual physical proof; only the closed cached base/mount check follows it.
        self.validate_mounted_current_layout()?;
        require_base(self, &runtime)
    }
}

#[cfg(test)]
#[path = "inventory_resync_tests.rs"]
mod tests;
