//! Close an exact first initialization that never acquired a retained canonical source.
//!
//! Restore cannot invent the lost private MLS device/fresh proof after process loss. It can
//! safely retire this exact never-exposed local reservation, preserving its non-evicting record.
//! Existing source bytes and registered live canonical writers take precedence over absence.
//! Before writer registration, negative recovery may win; that original creator must then
//! fail its post-pool reservation check rather than revive the retired nonce.
use super::*;
use crate::store::owner_evidence::canonical::UnexposedHeadAction;

/// Original mounted reservation, with no Server, snapshot bytes, transport or positive binding.
/// Keep this object across negative save failures so retry cannot adopt another nonce/mount.
pub struct OwnerUnexposedInitialization {
    preparation: ChatAdmissionPreparation,
}
impl std::fmt::Debug for OwnerUnexposedInitialization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerUnexposedInitialization")
            .finish_non_exhaustive()
    }
}

/// Negative recovery metadata only. Retained source means use exact canonical/staged recovery;
/// it says nothing about that source's authenticity, committed state or completeness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerUnexposedRetirement {
    Retired,
    SourceRetained,
}

impl ServerStore {
    /// Actual restore entry only. An absent canonical file alone cannot authorize this result:
    /// require this mounted vault's exact Fresh+Initializing row and original no-owner/no-source
    /// namespace observation. Merely capturing the result does not revoke or save anything.
    pub(crate) fn try_prepare_unexposed_owner_initialization(
        &self,
        identity: &ChatAdmissionKey,
    ) -> Result<Option<OwnerUnexposedInitialization>, AppError> {
        let mut retired_custody = None;
        let mut runtime = self.chat_mount.locked()?;
        if !runtime.managed
            || runtime.dirty
            || runtime.format != Some(catcoms_storage::VaultFormat::OwnerEvidence)
        {
            return Err(refused());
        }
        let Some(record) = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(identity)
            .filter(|row| row.origin == Origin::Fresh && row.phase == Phase::Initializing)
            .cloned()
        else {
            return Ok(None);
        };
        self.with_unexposed_owner_head(identity, || {
            let preparation =
                self.reservation_locked(&mut runtime, identity, &record, &mut retired_custody)?;
            Ok(UnexposedHeadAction::Observe(OwnerUnexposedInitialization {
                preparation,
            }))
        })
    }
}

impl OwnerUnexposedInitialization {
    /// Repeat actual negative admission and covering save. A later Current promotion, admission,
    /// physical mount, live writer or newly retained source cannot be cancelled by this capture.
    /// An exact already-started Retired save may retry despite revoked publication observation;
    /// it never returns Current or consumes a new fresh provenance/nonce.
    pub fn retry(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<OwnerUnexposedRetirement, AppError> {
        let original = &self.preparation;
        let mut retired_custody = None;
        let mut runtime = store.chat_mount.locked()?;
        if !Arc::ptr_eq(&original.mount_identity, &store.chat_mount.identity)
            || original.mount.is_cancelled()
            || original.domain != store.chat_vault_domain()?
            || !runtime.managed
            || runtime.format != Some(catcoms_storage::VaultFormat::OwnerEvidence)
        {
            return Err(refused());
        }
        let row = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&original.key)
            .ok_or_else(refused)?;
        if row.origin != Origin::Fresh
            || row.nonce != original.record.nonce
            || row.provenance != original.record.provenance
            || match row.phase {
                Phase::Initializing => row.revision != original.record.revision || runtime.dirty,
                // Retirement already revoked its original observer. This exact negative debt
                // can still settle its parent barrier, including at revision exhaustion.
                Phase::Retired => row.revision != original.record.revision.saturating_add(1),
                Phase::Current => true,
            }
        {
            return Err(refused());
        }
        let retired = store.with_unexposed_owner_head(&original.key, || {
            store.retire_chat_row(
                &mut runtime,
                &original.key,
                original.record.nonce,
                original.record.revision,
                rng,
                &mut retired_custody,
            )?;
            Ok(UnexposedHeadAction::Retired(()))
        })?;
        Ok(if retired.is_some() {
            OwnerUnexposedRetirement::Retired
        } else {
            OwnerUnexposedRetirement::SourceRetained
        })
    }
}

#[cfg(test)]
mod tests;
