//! Canonical restore classification BEFORE constructing any transport or runnable owner.
//!
//! These constructors accept only the actual Store's opaque bounded canonical input. Numeric
//! mirrors, decoded scope labels and old no-tail snapshots have no positive adapter here. The
//! final canonical file/root/parent durability barrier is separate from authenticated decoding.
use super::*;
use crate::store::owner_evidence::canonical::{
    OwnerHeadWriteGuard, SavedOwnerRestore, ValidatedOwnerSnapshot,
};
use crate::Server;
use catcoms_rt::{Clock, MeshTransport};

/// Closed external-lineage classification. Retired is decided without constructing T or MLS.
/// Initializing retains its original reservation; it cannot be treated as a new founding/join.
pub(crate) enum CanonicalOwnerRestore {
    Current(PrivateManagedRestoreAttempt),
    Initializing(PrivateManagedRestoreAttempt),
    Retired(super::retired::OfflineRetired),
}
impl std::fmt::Debug for CanonicalOwnerRestore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Current(_) => "CanonicalOwnerRestore::Current",
            Self::Initializing(_) => "CanonicalOwnerRestore::Initializing",
            Self::Retired(_) => "CanonicalOwnerRestore::Retired",
        };
        f.debug_struct(name).finish_non_exhaustive()
    }
}

/// Owns the original mount/pin/head/scope source before the caller constructs transport.
pub(crate) struct PrivateManagedRestoreAttempt {
    source: ValidatedOwnerSnapshot,
}
impl std::fmt::Debug for PrivateManagedRestoreAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateManagedRestoreAttempt")
            .finish_non_exhaustive()
    }
}

/// Exact private decode of ONE original canonical source. No mutable Server borrow is exposed:
/// re-encoding an OpenMLS provider after load changes HashMap order and is not authority to
/// replace the original canonical digest. Repairs that change saved state need their own
/// covering successor transaction; this first exact-restore path cannot acknowledge them.
pub(crate) struct PrivateManagedRestoredServer<T: MeshTransport, R: CryptoRngCore> {
    server: Option<Server<T, R>>,
    source: ValidatedOwnerSnapshot,
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for PrivateManagedRestoredServer<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateManagedRestoredServer")
            .finish_non_exhaustive()
    }
}

impl ServerStore {
    /// Apply original external lineage before any subscribe, dial, Actor or T construction.
    /// Authenticated canonical bytes alone do not prove durability, active membership or the
    /// current writer. A retired/older nonce preserves the exact capsule without creating MLS.
    pub(crate) fn classify_owner_canonical_restore(
        &self,
        source: ValidatedOwnerSnapshot,
    ) -> Result<CanonicalOwnerRestore, AppError> {
        source.validate_original(self)?;
        match source.admission() {
            ChatAdmissionResolution::Retired => Ok(CanonicalOwnerRestore::Retired(
                super::retired::OfflineRetired::from_validated_source(source),
            )),
            ChatAdmissionResolution::Current(binding) => {
                if binding.key.group() != source.group()
                    || binding.key.signer() != source.signer()
                    || binding.domain != source.domain()
                    || binding.nonce != source.nonce()
                    || binding.revision != source.revision()
                    || !binding.is_live()
                {
                    return Err(refused());
                }
                Ok(CanonicalOwnerRestore::Current(
                    PrivateManagedRestoreAttempt { source },
                ))
            }
            ChatAdmissionResolution::Initializing(preparation) => {
                if preparation.key.group() != source.group()
                    || preparation.key.signer() != source.signer()
                    || preparation.domain != source.domain()
                    || preparation.record.nonce != source.nonce()
                    || preparation.record.revision != source.revision()
                    || preparation.mount.is_cancelled()
                    || preparation.admission.is_cancelled()
                {
                    return Err(refused());
                }
                Ok(CanonicalOwnerRestore::Initializing(
                    PrivateManagedRestoreAttempt { source },
                ))
            }
        }
    }
}

impl PrivateManagedRestoreAttempt {
    /// Negative limits captured with this original source's actual read reservation. Inspecting
    /// them cannot choose new caps, construct a source or authorize a runtime owner.
    pub(crate) fn restore_limits(&self) -> catcoms_sync::OwnerInitializationRestoreLimits {
        self.source.restore_limits()
    }

    /// Decode synchronously and privately under the original charged source. This performs no
    /// networking/owner run. Actual self-removal is NEVER bypassed to obtain a runnable Server:
    /// retired sources were classified above, and ordinary positive MLS restore stays strict.
    pub(crate) fn restore_private<T: MeshTransport, R: CryptoRngCore>(
        self,
        original_store: &ServerStore,
        transport: T,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
    ) -> Result<PrivateManagedRestoredServer<T, R>, AppError> {
        self.source.validate_original(original_store)?;
        // Classification may have been retained while Leave/removal revoked the original
        // admission. Refuse that stale preparation BEFORE decoding private MLS/provider state;
        // the later attachment/final permit checks remain necessary for revocation during decode.
        match self.source.admission() {
            ChatAdmissionResolution::Current(binding) if binding.is_live() => {}
            ChatAdmissionResolution::Initializing(preparation)
                if !preparation.mount.is_cancelled() && !preparation.admission.is_cancelled() => {}
            _ => return Err(refused()),
        }
        let limits = self.restore_limits();
        let mut server = if self.source.is_operation_checkpoint() {
            Server::restore_owner_operation_checkpoint(
                self.source.snapshot(),
                transport,
                rng,
                clock,
                display_name,
                limits,
            )?
        } else {
            Server::restore_owner_initialization(
                self.source.snapshot(),
                transport,
                rng,
                clock,
                display_name,
                limits,
            )?
        };
        // No await, callback, network entry or Server borrow occurs before mandatory managed
        // classification, including if future decoders encounter a captured legacy no-tail.
        server.sync.require_managed_chat_scope();
        if server.group_id() != self.source.group()
            || server.sync.my_public_key().as_slice() != self.source.signer()
            || server.sync.fresh_chat_admission().is_some()
            || server.sync.owner_evidence_profile_id() != Some(self.source.profile())
            || server.sync.chat_scope_label().is_none_or(|label| {
                label.domain() != self.source.domain()
                    || label.nonce() != self.source.nonce()
                    || label.retired()
                    || label.initializing()
            })
        {
            return Err(refused());
        }
        // Reservation attachment checks the actual ACTIVE own leaf/full signer, not merely
        // roster presence, and supplies ONLY original negative custody. No context/publication
        // binding or stronger canonical owner runtime is installed by this private decode.
        match self.source.admission() {
            ChatAdmissionResolution::Current(binding) => {
                server.sync.attach_managed_chat_reservation(binding)?;
            }
            ChatAdmissionResolution::Initializing(preparation) => {
                server.sync.attach_managed_chat_reservation(preparation)?;
            }
            ChatAdmissionResolution::Retired => return Err(refused()),
        }
        // This check has no positive runtime side effect. In particular, a stage must not be
        // promoted or turn its independent row Current before its original externally pinned
        // chain and actual own credential have been verified. Final bind checks them again.
        server.sync.validate_owner_initialization_chain(
            self.source.profile(),
            self.source.founder(),
            self.source.signer(),
        )?;
        if let Some(parent) = self.source.operation_parent_snapshot() {
            server
                .sync
                .validate_owner_operation_parent(parent, limits, self.source.founder())?;
        }
        self.source.validate_original(original_store)?;
        Ok(PrivateManagedRestoredServer {
            server: Some(server),
            source: self.source,
        })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PrivateManagedRestoredServer<T, R> {
    pub(crate) fn source(&self) -> &ValidatedOwnerSnapshot {
        &self.source
    }

    /// Read-only private inspection; no mutable, generic snapshot or bare-Server extraction.
    pub(crate) fn server(&self) -> Result<&Server<T, R>, AppError> {
        self.server.as_ref().ok_or_else(refused)
    }

    /// Consume only after the actual SAME canonical file/root/parent barrier, Current inventory
    /// and original writer/source CAS. Initializing recovery uses its real reservation, never a
    /// fabricated fresh completion. Bindings are runtime-only: no loaded provider is re-encoded.
    pub(crate) fn take_ready(
        &mut self,
        permit: &ChatAdmissionStoragePermit<'_>,
        binding: &ChatAdmissionBinding,
        saved: &SavedOwnerRestore,
        head: &OwnerHeadWriteGuard<'_>,
    ) -> Result<(Server<T, R>, ChatAdmissionBinding), AppError> {
        if permit.mode() != ChatAdmissionStorageMode::Current
            || binding.key.group() != self.source.group()
            || binding.key.signer() != self.source.signer()
            || binding.domain != self.source.domain()
            || binding.nonce != self.source.nonce()
            || binding.revision != self.source.revision()
            || permit.domain() != binding.domain
            || permit.nonce() != binding.nonce
            || permit.revision() != binding.revision
            || !Arc::ptr_eq(permit.owner_identity(), &binding.owner_identity)
            || binding.owner_writer_identity().is_none()
        {
            return Err(refused());
        }
        permit.validate_original(permit.store(), self.source.group(), self.source.signer())?;
        saved.validate_private_candidate(head, permit, binding, &self.source)?;
        let server = self.server.as_mut().ok_or_else(refused)?;
        server.sync.bind_managed_chat_scope(binding)?;
        saved.bind_private_runtime(head, permit, binding, &self.source, &mut server.sync)?;
        // Keep source/read-fee custody in this now-empty private wrapper until the caller has
        // returned from its final admission/head callback. Dropping its pool-backed fee while
        // either writer is held would introduce an unnecessary inverse lock dependency.
        Ok((self.server.take().ok_or_else(refused)?, binding.clone()))
    }
}

#[cfg(test)]
mod tests;
