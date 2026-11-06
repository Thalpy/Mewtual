//! Negative old pin-writer fence. This is not an admission/publication permission.
use super::*;

/// Keep the actual admission writer alive through an OLD immutable pin mutation. The caller
/// then takes pins and Root in that order. No Runtime field or Current constructor escapes.
pub(in crate::store) struct OldOwnerPinAdmission<'writer> {
    store: &'writer ServerStore,
    original: &'writer ChatAdmissionPreparation,
    runtime: std::sync::MutexGuard<'writer, Runtime>,
}
impl OldOwnerPinAdmission<'_> {
    /// The constructor alone may inspect the same original row while repairing an outstanding
    /// scope-inventory barrier. Dirty state never permits pin mutation or reaches the caller.
    fn validate_source_before_inventory_repair(&self) -> Result<(), AppError> {
        let original = self.original;
        if !self.runtime.managed
            || !Arc::ptr_eq(&original.mount_identity, &self.store.chat_mount.identity)
            || original.domain != self.store.chat_vault_domain()?
            || original.mount.is_cancelled()
            || original.admission.is_cancelled()
        {
            return Err(refused());
        }
        let row = self
            .runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&original.key)
            .ok_or_else(refused)?;
        let owner = self.runtime.owners.get(&original.key).ok_or_else(refused)?;
        if row.phase == Phase::Retired
            || row.nonce != original.record.nonce
            || row.revision != original.record.revision
            || row.origin != original.record.origin
            || row.provenance != original.record.provenance
            || owner.nonce != row.nonce
            || owner.revision != row.revision
            || !Arc::ptr_eq(&owner.life.identity(), &original.owner_identity)
        {
            return Err(refused());
        }
        self.store
            .require_no_v2_group_directory(original.key.group())
    }

    /// Recheck after EVERY subordinate wait and before pin insertion/flush. Holding admission
    /// prevents durable retirement, while an independent negative revoker remains observable.
    /// Exact Current is permitted only as this same original immutable-pin retry, never a new
    /// covering snapshot or scope permission. This takes only Root, never admission/pins again.
    pub(in crate::store) fn validate_original(&self) -> Result<(), AppError> {
        if self.runtime.dirty {
            return Err(refused());
        }
        self.validate_source_before_inventory_repair()
    }
}
impl ServerStore {
    pub(in crate::store) fn lock_old_owner_pin_admission<'writer>(
        &'writer self,
        original: &'writer ChatAdmissionPreparation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<OldOwnerPinAdmission<'writer>, AppError> {
        let runtime = self.chat_mount.locked()?;
        self.require_no_pending_v2_group(original.key.group())?;
        // Before pins/Root only: every real source admission uses this same admission writer,
        // so a matching JOIN cannot appear during the later synchronous pin transaction.
        self.require_no_pending_checkpoint_join_source(
            original.key.group(),
            original.key.signer(),
        )?;
        let mut guard = OldOwnerPinAdmission {
            store: self,
            original,
            runtime,
        };
        guard.validate_source_before_inventory_repair()?;
        if guard.runtime.dirty {
            // Existing initializers retry their exact scope barrier automatically. Pin writing
            // precedes finish_chat_admission in that flow, so refusing dirty here would strand
            // a genuine saved head after a Current parent-sync failure. Save the whole ORIGINAL
            // inventory first, before pins; no new row/owner/nonce or covering callback is made.
            self.flush_chat_admissions(&mut guard.runtime, rng)?;
        }
        guard.validate_original()?;
        Ok(guard)
    }

    /// Actual serialized negative observation for cross-module tests with a source but no row.
    /// It issues no writer/permit to the caller and performs no effects or fresh construction.
    #[cfg(test)]
    pub(in crate::store) fn check_join_group_exclusion_for_test(
        &self,
        group: &[u8],
        other_signer: &[u8; 32],
    ) -> Result<(bool, bool), AppError> {
        let _runtime = self.chat_mount.locked()?;
        Ok((
            self.require_no_pending_checkpoint_join_source(group, other_signer)
                .is_ok(),
            self.require_no_pending_checkpoint_join_group(group).is_ok(),
        ))
    }
}
