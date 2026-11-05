//! Private fresh-server custody for the canonical owner-state initialization transaction.
//!
//! This module must land with its actual owner-saver consumer. It deliberately has no public
//! generic `(Server, proof, capture)` constructor or snapshot/"fresh" boolean adapter. The fixed
//! factories below perform the real creation/join themselves and retain their original custody.
//! No caller can extract a runnable Server before the exact canonical head and durable Current
//! barriers succeed under the same original writer/admission custody.
use super::*;
use crate::store::owner_evidence::canonical::{
    OwnerHeadWriteGuard, OwnerInitializationReservation, SavedOwnerInitialization,
};
use crate::store::owner_evidence::CapturedOwnerProfilePin;
use crate::Server;
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_rt::{Clock, MeshTransport, PeerId};

/// A never-exposed server and its inseparable original local admission capture. Initialization
/// and uncertain save retries keep this entire object; dropping a failed callback must not
/// substitute an older snapshot or resample the scope nonce. No Clone/serde/Debug secrets.
pub(crate) struct PrivateManagedServer<T: MeshTransport, R: CryptoRngCore> {
    server: Option<Server<T, R>>,
    capture: ChatAdmissionCapture,
    // Fixed from this actual attached completion, never supplied by a caller. The opaque
    // non-Clone source stays INSIDE the private Server until successful ready extraction.
    attempt: [u8; 32],
    preparation: Option<ChatAdmissionPreparation>,
    frozen: Option<FrozenPrivateServerSnapshot>,
    // Welcome joins MUST retain their expected root from before the first handshake await.
    // None is possible only for an actual fresh founder, whose real profile is prepared next.
    expected_profile: Option<CapturedOwnerProfilePin>,
}

/// One exact secret candidate, retained through every pre/post-rename retry. The canonical
/// saver borrows these bytes; it never reconstructs them from a newer graph, signs again or
/// clones the complete snapshot to retry. A digest describes bytes, not storage authority.
pub(crate) struct FrozenPrivateServerSnapshot {
    bytes: Zeroizing<Vec<u8>>,
    digest: [u8; 32],
}
impl std::fmt::Debug for FrozenPrivateServerSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrozenPrivateServerSnapshot")
            .finish_non_exhaustive()
    }
}
impl FrozenPrivateServerSnapshot {
    /// Secret plaintext; the sole canonical saver seals it before any file I/O.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

#[cfg(test)]
mod tests;
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for PrivateManagedServer<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateManagedServer")
            .finish_non_exhaustive()
    }
}

/// Capture this BEFORE native bootstrap/dial or the first join await, then release the Store
/// mutex. The attempt owns the MLS device material required by that handshake, but retains
/// only weak mounted-vault custody: no Store, vault session owner, vault keys or writer guard.
/// Only its fixed actual handshake can produce a private completed Server.
pub(crate) struct PrivateManagedJoinAttempt {
    capture: ChatAdmissionCapture,
    device: MlsDevice,
    invite: InviteToken,
    expected_profile: CapturedOwnerProfilePin,
}
impl std::fmt::Debug for PrivateManagedJoinAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateManagedJoinAttempt")
            .finish_non_exhaustive()
    }
}

impl ServerStore {
    /// Actual synchronous creation, private until canonical owner initialization commits. The
    /// original physical mount is observed before MLS work; no await/publication occurs between
    /// creation and the exact logical capture. A revoked mount cannot be replaced afterward.
    pub(crate) fn private_managed_founder<T: MeshTransport, R: CryptoRngCore>(
        &self,
        transport: T,
        device: MlsDevice,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
    ) -> Result<PrivateManagedServer<T, R>, AppError> {
        {
            let runtime = self.chat_mount.locked()?;
            if !runtime.managed || runtime.dirty {
                return Err(refused());
            }
        }
        let mount = self.chat_mount.observer();
        let server = Server::found(transport, device, rng, clock, display_name)?;
        if mount.is_cancelled() {
            return Err(refused());
        }
        let identity = ChatAdmissionKey::new(
            &server.group_id(),
            server
                .sync
                .my_public_key()
                .try_into()
                .map_err(|_| refused())?,
        )?;
        let capture = self.capture_chat_admission(identity)?;
        PrivateManagedServer::complete(server, capture, None)
    }

    /// Reserve the ORIGINAL mount/lineage before any join network work. Metadata classification
    /// is used only to refuse unmanaged construction; it never constructs the fresh proof.
    /// The device and exact invite are owned here so completion cannot swap their full identity.
    pub(crate) fn private_managed_join_attempt(
        &self,
        device: MlsDevice,
        invite: InviteToken,
        expected_profile: CapturedOwnerProfilePin,
    ) -> Result<PrivateManagedJoinAttempt, AppError> {
        // Choosing a root is an explicit trusted invitation/configuration decision. A Profile
        // downloaded after Welcome, roster entry or self-signed body cannot replace this input.
        expected_profile.validate_original(self, &invite.group_id)?;
        let identity = ChatAdmissionKey::new(
            &invite.group_id,
            device
                .public_key_bytes()
                .try_into()
                .map_err(|_| refused())?,
        )?;
        let capture = self.capture_chat_admission(identity)?;
        Ok(PrivateManagedJoinAttempt {
            capture,
            device,
            invite,
            expected_profile,
        })
    }
}

impl PrivateManagedJoinAttempt {
    /// Only necessary authenticated admission traffic is allowed here. The existing real join
    /// mints its fresh KeyPackage and validates the ACTUALLY selected Welcome package/signature
    /// before new_joined attaches opaque completion. No subscribe, self advertisement or owner
    /// run is performed. Other helper/reply/grant funnels need their own fixed attempt methods;
    /// they must not fall back to a generic old-proof/new-capture completion API.
    pub(crate) async fn finish_direct<T: MeshTransport, R: CryptoRngCore>(
        self,
        transport: T,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
        inviter: PeerId,
    ) -> Result<PrivateManagedServer<T, R>, AppError> {
        if self.capture.mount.is_cancelled()
            || self.expected_profile.is_revoked()
            || self
                .capture
                .admission
                .as_ref()
                .is_some_and(RequestCancellation::is_cancelled)
        {
            return Err(refused());
        }
        let server = Server::join(
            transport,
            self.device,
            rng,
            clock,
            display_name,
            inviter,
            &self.invite,
        )
        .await?;
        PrivateManagedServer::complete(server, self.capture, Some(self.expected_profile))
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PrivateManagedServer<T, R> {
    // ONLY fixed actual factories in this module call this private constructor. A raw restored
    // server has no completion; even a genuine old completion may not borrow a later capture.
    fn complete(
        mut server: Server<T, R>,
        capture: ChatAdmissionCapture,
        expected_profile: Option<CapturedOwnerProfilePin>,
    ) -> Result<Self, AppError> {
        let source = server.sync.fresh_chat_admission().ok_or_else(refused)?;
        if source.group() != capture.key.group()
            || source.signer() != capture.key.signer()
            || server.group_id() != capture.key.group()
            || server.sync.my_public_key().as_slice() != capture.key.signer()
            || capture.mount.is_cancelled()
            || capture
                .admission
                .as_ref()
                .is_some_and(RequestCancellation::is_cancelled)
            || source.key_package_reference().is_some() != expected_profile.is_some()
            || expected_profile
                .as_ref()
                .is_some_and(|pin| pin.group() != capture.key.group() || pin.is_revoked())
        {
            return Err(refused());
        }
        server.sync.require_managed_chat_scope();
        let attempt = server
            .sync
            .fresh_chat_admission()
            .ok_or_else(refused)?
            .attempt_id();
        Ok(Self {
            server: Some(server),
            capture,
            attempt,
            preparation: None,
            frozen: None,
            expected_profile,
        })
    }

    /// Borrow the actual private source for original-founder profile preparation. Its opaque
    /// token stays inside this Server until ready extraction. This is not registration or save.
    pub(crate) fn server(&self) -> Result<&Server<T, R>, AppError> {
        self.server.as_ref().ok_or_else(refused)
    }

    /// Trusted owner initializer only: build the complete private candidate/archive and stage
    /// Active before canonical save. Never run, subscribe, publish or pass this borrow to a
    /// background writer. Actual-diff review must inspect the sole canonical saver consumer.
    pub(crate) fn server_mut_for_owner_initialization(
        &mut self,
    ) -> Result<&mut Server<T, R>, AppError> {
        if self.frozen.is_some() {
            // Once canonical I/O can see S0, the private initializer cannot turn it into S1
            // while later presenting S0's covering save/head as authority to expose S1.
            return Err(refused());
        }
        self.server.as_mut().ok_or_else(refused)
    }

    pub(crate) fn preparation(&self) -> Result<&ChatAdmissionPreparation, AppError> {
        self.preparation.as_ref().ok_or_else(refused)
    }

    /// An actual Welcome's immutable ORIGINAL expected root. There is intentionally no setter
    /// or post-join conversion from downloaded bytes. A fresh founder prepares its own profile
    /// from its actual unchanged founding state instead and therefore has no prior pin here.
    pub(crate) fn captured_profile(&self) -> Option<&CapturedOwnerProfilePin> {
        self.expected_profile.as_ref()
    }

    fn validate_source(&self) -> Result<(), AppError> {
        let server = self.server()?;
        let source = server.sync.fresh_chat_admission().ok_or_else(refused)?;
        if source.group() != self.capture.key.group()
            || source.signer() != self.capture.key.signer()
            || source.attempt_id() != self.attempt
            || server.group_id() != self.capture.key.group()
            || server.sync.my_public_key().as_slice() != self.capture.key.signer()
            || source.key_package_reference().is_some() != self.expected_profile.is_some()
            || self
                .expected_profile
                .as_ref()
                .is_some_and(|pin| pin.group() != self.capture.key.group() || pin.is_revoked())
        {
            // Even the trusted candidate builder cannot accidentally substitute a restored
            // Server or the same key's later genuine admission for this captured attempt.
            return Err(refused());
        }
        Ok(())
    }

    /// Retain the exact private issued candidate on failure. Preparation's own retry slot also
    /// survives a pre-rename inventory failure after its old owner was synchronously revoked.
    /// No user intent or old raw history is automatically promoted by this fresh-only path.
    pub(crate) fn reserve_scope(
        &mut self,
        original_store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<ChatAdmissionPreparation, AppError> {
        self.validate_source()?;
        if let Some(pin) = &self.expected_profile {
            pin.validate_original(original_store, self.capture.key.group())?;
        }
        if self.preparation.is_none() {
            self.preparation = Some(original_store.prepare_chat_admission(
                &self.capture,
                self.attempt,
                Origin::Fresh,
                rng,
            )?);
        }
        let preparation = self.preparation.as_ref().ok_or_else(refused)?;
        if !Arc::ptr_eq(
            &preparation.mount_identity,
            &original_store.chat_mount.identity,
        ) || preparation.domain != original_store.chat_vault_domain()?
            || preparation.mount.is_cancelled()
            || preparation.admission.is_cancelled()
        {
            return Err(refused());
        }
        if self.frozen.is_some() {
            // All persistable mutation is closed after freeze, even an ostensibly idempotent
            // staging call. The real canonical saver may retry only the retained exact bytes.
            return Ok(preparation.clone());
        }
        let server = self.server.as_mut().ok_or_else(refused)?;
        server
            .sync
            .reserve_managed_chat_scope(preparation.domain(), preparation.nonce())?;
        server.sync.attach_managed_chat_reservation(preparation)?;
        Ok(preparation.clone())
    }

    /// Freeze only AFTER the real canonical owner has reserved its configured persistent and
    /// buffer quota, built all facts and staged the exact Active tail/accepted-v1 aliases. The
    /// reservation is an opaque real Store result, not a caller-supplied byte cap or paid bool.
    /// This method runs outside final storage writers: bounded recovery/staging work must finish
    /// before entering the admission -> canonical-head save callback.
    pub(crate) fn freeze_for_owner_initialization(
        &mut self,
        reservation: &OwnerInitializationReservation,
    ) -> Result<&FrozenPrivateServerSnapshot, AppError> {
        self.validate_source()?;
        let preparation = self.preparation.as_ref().ok_or_else(refused)?;
        if preparation.mount.is_cancelled() || preparation.admission.is_cancelled() {
            return Err(refused());
        }
        reservation.validate_private_source(
            preparation.key.group(),
            preparation.key.signer(),
            self.attempt,
            preparation,
        )?;
        let server = self.server.as_mut().ok_or_else(refused)?;
        if server.sync.chat_scope_label().is_none_or(|label| {
            label.domain() != preparation.domain()
                || label.nonce() != preparation.nonce()
                || label.retired()
                || label.initializing()
        }) {
            return Err(refused());
        }
        if self.frozen.is_none() {
            let limits = reservation.snapshot_limits();
            if limits.max_snapshot_bytes == 0
                || limits.max_mls_bytes == 0
                || limits.max_mls_bytes > limits.max_snapshot_bytes
                || limits.max_mls_backing_bytes == 0
            {
                return Err(refused());
            }
            // The sync/MLS path preflights its closed private-candidate fields and SAME-guard
            // provider map before allocation. A post-encoding len check alone is not that cap.
            let bytes = server.snapshot_owner_initialization(
                limits.max_snapshot_bytes,
                limits.max_mls_bytes,
                limits.max_mls_backing_bytes,
            )?;
            reservation.validate_private_source(
                preparation.key.group(),
                preparation.key.signer(),
                self.attempt,
                preparation,
            )?;
            if preparation.mount.is_cancelled() || preparation.admission.is_cancelled() {
                return Err(refused());
            }
            let digest = *blake3::hash(&bytes).as_bytes();
            self.frozen = Some(FrozenPrivateServerSnapshot { bytes, digest });
        }
        self.frozen.as_ref().ok_or_else(refused)
    }

    /// Retained exact bytes for canonical retry/head comparison, never a proof of a covering save.
    pub(crate) fn frozen_snapshot(&self) -> Result<&FrozenPrivateServerSnapshot, AppError> {
        self.frozen.as_ref().ok_or_else(refused)
    }

    /// Called ONLY by canonical owner initialization after its opaque saved-head result and
    /// inventory Current barrier. The caller already holds admission -> head custody and supplies
    /// its Current permit; this method must NOT reacquire admission from inside that head writer.
    /// It checks the original Store and SAME runtime nonce owner again, binds the scope, and moves
    /// the private server exactly once. Failure retains the candidate for automatic repair/retry.
    pub(crate) fn take_ready(
        &mut self,
        permit: &ChatAdmissionStoragePermit<'_>,
        binding: &ChatAdmissionBinding,
        saved: &SavedOwnerInitialization,
        head: &OwnerHeadWriteGuard<'_>,
    ) -> Result<(Server<T, R>, ChatAdmissionBinding), AppError> {
        self.validate_source()?;
        let frozen_digest = self.frozen_snapshot()?.digest();
        let preparation = self.preparation.as_ref().ok_or_else(refused)?;
        if permit.mode() != ChatAdmissionStorageMode::Current
            || preparation.key != binding.key
            || preparation.record.nonce != binding.nonce
            || preparation.record.revision != binding.revision
            || preparation.domain != binding.domain
            || !Arc::ptr_eq(&preparation.owner_identity, &binding.owner_identity)
            || !Arc::ptr_eq(permit.owner_identity(), &binding.owner_identity)
            || permit.nonce() != binding.nonce
            || permit.revision() != binding.revision
            || permit.domain() != binding.domain
            || binding.owner_writer_identity().is_none()
        {
            return Err(refused());
        }
        permit.validate_original(permit.store(), binding.key.group(), binding.key.signer())?;
        // Current scope alone cannot release a stale private S0 after a canonical S1 save.
        // Only the actual covering saver can construct this saved-head result. Its final CAS
        // check uses the ALREADY held head writer and original mount/admission, without an
        // await, fresh lookup/adoption or recursively acquiring either writer.
        saved.validate_private_candidate(head, permit, binding, self.attempt, frozen_digest)?;
        let server = self.server.as_mut().ok_or_else(refused)?;
        server.sync.bind_managed_chat_scope(binding)?;
        saved.bind_private_runtime(head, permit, binding, &mut server.sync)?;
        // No await or source mutation occurs between validation and this one final consume.
        // The runnable Server must never later offer this old creation/Welcome as a new attempt.
        let _ = server.sync.take_fresh_chat_admission();
        let server = self.server.take().ok_or_else(refused)?;
        // The actual canonical bytes/head and Current barrier are now durable. Release the
        // one paid plaintext encoding buffer at this transfer; a retained empty private wrapper
        // must not keep the initialization fee's buffer alive beside the new runnable owner.
        self.frozen = None;
        Ok((server, binding.clone()))
    }
}
