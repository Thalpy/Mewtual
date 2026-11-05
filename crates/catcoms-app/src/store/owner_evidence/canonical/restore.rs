//! Actual canonical restart/Initializing recovery, with no fresh proof fabrication.
use super::*;
use crate::store::chat_admission::restore::{
    CanonicalOwnerRestore, PrivateManagedRestoreAttempt, PrivateManagedRestoredServer,
};
use crate::store::chat_admission::retired::OfflineRetired;
pub use crate::store::chat_admission::{OwnerUnexposedInitialization, OwnerUnexposedRetirement};
use crate::store::ChatAdmissionResolution;
use catcoms_rt::{Clock, MeshTransport};

mod recovery;
pub(in crate::store) use recovery::UnexposedHeadAction;

/// Classify external lineage BEFORE accepting/constructing any transport. Removed/old nonce
/// source remains an offline evidence capsule, never an inactive runnable MLS actor.
#[derive(Debug)]
pub enum OwnerRestoreClassification {
    Current(OwnerRestorePreparation),
    Initializing(OwnerRestorePreparation),
    Retired(OfflineOwnerEvidence),
    /// No canonical or staged source exists and no live creator owns this first reservation.
    /// Host recovery retries this negative closure automatically; it cannot construct transport.
    UnexposedInitialization(OwnerUnexposedInitialization),
}
#[derive(Debug)]
pub struct OwnerRestorePreparation {
    attempt: PrivateManagedRestoreAttempt,
}
/// Retained canonical history only. No transport, Server or unprivileged body reader escapes.
#[derive(Debug)]
pub struct OfflineOwnerEvidence {
    offline: OfflineRetired,
}
impl OfflineOwnerEvidence {
    pub fn group(&self) -> &[u8] {
        self.offline.source().group()
    }
    pub fn signer(&self) -> &[u8; 32] {
        self.offline.source().signer()
    }
    pub fn profile(&self) -> OwnerFactId {
        self.offline.source().profile()
    }
}

/// Retains its exact private decode, charged source, actual barrier and original new writer
/// across every error. No generic restored Server or mutable candidate accessor is exposed.
pub struct OwnerRestoreInitialization<T: MeshTransport, R: CryptoRngCore> {
    private: Option<PrivateManagedRestoredServer<T, R>>,
    saved: Option<SavedOwnerRestore>,
    writer: Option<OwnerWriterRegistration>,
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for OwnerRestoreInitialization<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerRestoreInitialization")
            .finish_non_exhaustive()
    }
}
impl ServerStore {
    /// Load ONLY this mounted immutable group's canonical source. An exact never-exposed first
    /// Initializing reservation with no source has a separate negative recovery classification;
    /// no missing file can bootstrap captured no-tail numeric history or choose an alias.
    pub fn prepare_owner_evidence_restore(
        &self,
        group: &[u8],
        signer: [u8; 32],
    ) -> Result<OwnerRestoreClassification, AppError> {
        let identity = crate::store::ChatAdmissionKey::new(group, signer)?;
        if let Some(unexposed) = self.try_prepare_unexposed_owner_initialization(&identity)? {
            return Ok(OwnerRestoreClassification::UnexposedInitialization(
                unexposed,
            ));
        }
        let pin = self.capture_mounted_owner_profile(group)?;
        let source = self.prepare_owner_canonical_restore(pin, signer)?;
        Ok(match self.classify_owner_canonical_restore(source)? {
            CanonicalOwnerRestore::Current(attempt) => {
                OwnerRestoreClassification::Current(OwnerRestorePreparation { attempt })
            }
            CanonicalOwnerRestore::Initializing(attempt) => {
                OwnerRestoreClassification::Initializing(OwnerRestorePreparation { attempt })
            }
            CanonicalOwnerRestore::Retired(offline) => {
                OwnerRestoreClassification::Retired(OfflineOwnerEvidence { offline })
            }
        })
    }
}
impl OwnerRestorePreparation {
    /// Consume a read preparation to build its exact private decode. Decode failure changes no
    /// signed state/current head; canonical bytes remain on disk for bounded automatic retry.
    pub fn restore_private<T: MeshTransport, R: CryptoRngCore>(
        self,
        store: &ServerStore,
        transport: T,
        rng: R,
        clock: Box<dyn Clock + Send>,
        name: impl Into<String>,
    ) -> Result<OwnerRestoreInitialization<T, R>, AppError> {
        let private = self
            .attempt
            .restore_private(store, transport, rng, clock, name)?;
        Ok(OwnerRestoreInitialization {
            private: Some(private),
            saved: None,
            writer: None,
        })
    }
}
impl<T: MeshTransport, R: CryptoRngCore> OwnerRestoreInitialization<T, R> {
    /// Cross the real canonical restart barrier, SAME Current scope and source/head CAS before
    /// transferring. A saved Active tail with external Initializing uses its exact reservation;
    /// neither phase infers founding, new nonce, historical acceptance or lost proof success.
    pub fn retry(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<OwnedOwnerServer<T, R>, AppError> {
        let private = self.private.as_mut().ok_or_else(refused)?;
        if private.server()?.sync.owner_evidence_profile_id() != Some(private.source().profile()) {
            return Err(refused());
        }
        let original = private.source();
        original.validate_original(store)?;
        let group = original.group().to_vec();
        let signer = *original.signer();
        let nonce = original.nonce();
        let profile = original.profile();
        let founder = *original.founder();
        // The original read object owns its fee through this entire callback. It is not dropped
        // while admission/head are held, and no re-encoding compares SDK HashMap byte order.
        let cell = original.cell.clone();
        if let ChatAdmissionResolution::Initializing(preparation) = original.admission() {
            let saved = &mut self.saved;
            store.finish_chat_admission_with_storage(preparation, rng, |permit, _| {
                let mut head = cell.lock()?;
                *saved = Some(head.establish_restore_barrier(&permit, original)?);
                Ok(())
            })?;
        }
        let binding = match original.admission() {
            ChatAdmissionResolution::Current(binding) => binding.clone(),
            ChatAdmissionResolution::Initializing(_) => match store.resolve_chat_admission(
                &crate::store::ChatAdmissionKey::new(&group, signer)?,
                nonce,
            )? {
                ChatAdmissionResolution::Current(binding) => binding,
                _ => return Err(refused()),
            },
            ChatAdmissionResolution::Retired => return Err(refused()),
        };
        let (server, binding) = store.with_chat_admission_storage_permit(&binding, |permit| {
            let source = private.source();
            let mut head = cell.lock()?;
            if self.saved.is_none() {
                self.saved = Some(head.establish_restore_barrier(&permit, source)?);
            }
            let saved = self.saved.as_ref().ok_or_else(refused)?;
            if self.writer.is_none() {
                self.writer = Some(saved.register_writer(&mut head, &permit, source, &binding)?);
            }
            let writer = self.writer.as_ref().ok_or_else(refused)?;
            let strengthened = binding.with_owner_writer(&permit, &head, writer)?;
            private.take_ready(&permit, &strengthened, saved, &head)
        })?;
        let writer = self.writer.take().ok_or_else(refused)?;
        // Both storage callbacks returned: zeroize/release the now-empty private source and
        // refund its read fee OUTSIDE admission/head. Retaining this completed outer object
        // cannot retain another whole plaintext or strand paid hot capacity beside the Actor.
        self.private = None;
        Ok(OwnedOwnerServer::from_restored(
            server,
            writer,
            binding,
            profile,
            founder,
            self.saved.as_ref().ok_or_else(refused)?.metadata(),
        ))
    }
}

#[cfg(test)]
mod tests;
