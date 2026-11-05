//! Actual private founder -> paid freeze -> immutable pin -> canonical save -> Current handoff.
use super::*;
use crate::store::chat_admission::fresh::PrivateManagedServer;
use crate::Server;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Clock, MeshTransport};
use catcoms_sync::local_chat_scope::{ManagedChatScopeBinding, ManagedChatScopeRevocation};
use catcoms_sync::owner_evidence::{ManagedOwnerEvidenceBinding, PreparedOwnerEvidence};

/// Owns the whole original creation and exact retry custody. The Private field is FIRST so its
/// frozen plaintext is zeroized BEFORE the following reservation releases its buffer fee.
/// Never add a consuming convenience retry returning only Error: an error must retain this
/// object, including possibly installed head, exact facts, nonce and publication prohibition.
pub struct OwnerFounderInitialization<T: MeshTransport, R: CryptoRngCore> {
    private: PrivateManagedServer<T, R>,
    prepared: Option<PreparedOwnerEvidence>,
    creation: OwnerPendingCreation,
    reservation: Option<OwnerInitializationReservation>,
    saved: Option<SavedOwnerInitialization>,
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for OwnerFounderInitialization<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerFounderInitialization")
            .finish_non_exhaustive()
    }
}

/// The actual canonical writer lifetime owns the never-exposed Server. There is deliberately
/// no public bare-Server extraction, actor spawn or typed-dispatch activation in CHECKPOINT-1.
/// Later production adapters must move this owner into the actual task, preserving original
/// writer/admission cancellation. Dropping it retires the writer BEFORE dropping the Server.
pub struct OwnedOwnerServer<T: MeshTransport, R: CryptoRngCore> {
    writer: OwnerWriterRegistration,
    server: Server<T, R>,
    profile: OwnerFactId,
    founder: [u8; 32],
    current_head: head::Meta,
    binding: OwnerCanonicalBinding,
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for OwnedOwnerServer<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedOwnerServer").finish_non_exhaustive()
    }
}
impl<T: MeshTransport, R: CryptoRngCore> OwnedOwnerServer<T, R> {
    // Only canonical restore's actual Saved/head callback can call this crate-private adapter.
    pub(super) fn from_restored(
        server: Server<T, R>,
        writer: OwnerWriterRegistration,
        scope: ChatAdmissionBinding,
        profile: OwnerFactId,
        founder: [u8; 32],
        current_head: head::Meta,
    ) -> Self {
        Self {
            writer,
            server,
            profile,
            founder,
            current_head,
            binding: OwnerCanonicalBinding {
                scope,
                profile,
                founder,
            },
        }
    }

    /// Capture ONLY inert expected-root provenance under the actual SOURCE Current/head.
    /// Return after releasing those locks; a destination capture must not acquire another
    /// Store's pin/admission while source writers are held (crossed A->B/B->A deadlock).
    pub(in crate::store::owner_evidence) fn capture_ready_profile_origin(
        &self,
        source_store: &ServerStore,
    ) -> Result<(Vec<u8>, OwnerFactId, [u8; 32], RequestCancellation), AppError> {
        source_store.with_chat_admission_storage_permit(&self.binding.scope, |permit| {
            self.writer.with_saved_current_head(
                &permit,
                &self.binding.scope,
                &self.current_head,
                || {
                    (
                        self.server.group_id(),
                        self.profile,
                        self.founder,
                        self.publication_authority(),
                    )
                },
            )
        })
    }

    /// Read-only labels, not proof bytes or a runnable Server escape.
    pub fn group(&self) -> Vec<u8> {
        self.server.group_id()
    }
    pub fn profile(&self) -> OwnerFactId {
        self.profile
    }
    pub fn founder(&self) -> &[u8; 32] {
        &self.founder
    }
    /// Original negative-only publication observation for the actual task/driver handoff.
    /// This does not retain the writer, renew a dropped task or authorize another server.
    pub(crate) fn publication_authority(&self) -> RequestCancellation {
        self.binding.publication_authority()
    }

    /// Retire this exact LOCAL scope before Leave acknowledgement. Original signed/canonical
    /// history stays retained; this is not an MLS Remove or a history deletion operation.
    pub fn retire_local_owner(
        &self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        store.retire_chat_admission(&self.binding.scope, rng)
    }

    pub fn is_live(&self) -> bool {
        !self.writer.publication_authority().is_cancelled()
            && !self.publication_authority().is_cancelled()
    }
}

impl ServerStore {
    /// Perform actual synchronous founding under the ORIGINAL managed physical mount. No
    /// subscription, self advertisement, registration or MLS response is released. The returned
    /// owner must remain retained across retry errors; no call here accepts a restored Server.
    pub fn prepare_owner_evidence_founder<T: MeshTransport, R: CryptoRngCore>(
        &self,
        transport: T,
        device: MlsDevice,
        rng: R,
        clock: Box<dyn Clock + Send>,
        display_name: impl Into<String>,
        channel: u128,
    ) -> Result<OwnerFounderInitialization<T, R>, AppError> {
        if self.chat_vault_format()? != catcoms_storage::VaultFormat::OwnerEvidence {
            return Err(refused());
        }
        let creation = self.reserve_pending_owner_creation()?;
        let private = self.private_managed_founder(transport, device, rng, clock, display_name)?;
        let prepared = private
            .server()?
            .sync
            .prepare_owner_evidence_checkpoint(channel)
            .map_err(|_| refused())?;
        Ok(OwnerFounderInitialization {
            private,
            prepared: Some(prepared),
            creation,
            reservation: None,
            saved: None,
        })
    }
}
impl<T: MeshTransport, R: CryptoRngCore> OwnerFounderInitialization<T, R> {
    /// Read-only original creation labels; no proof bytes or publication capability.
    pub fn group(&self) -> Result<&[u8], AppError> {
        Ok(self.prepared.as_ref().ok_or_else(refused)?.group())
    }
    pub fn founder(&self) -> Result<&[u8; 32], AppError> {
        Ok(self.prepared.as_ref().ok_or_else(refused)?.founder())
    }
    pub fn profile(&self) -> Result<OwnerFactId, AppError> {
        Ok(self.prepared.as_ref().ok_or_else(refused)?.profile_id())
    }

    /// Save and hand off ONLY this exact candidate. Pre/post-rename failures retain everything
    /// in self for automatic exact retry. A visible file is never interpreted as durable success.
    /// Production activation, scalable archive/membership publication and native adapter remain
    /// separate gates; this first consumed transaction is not the completed Chat V2 product.
    pub fn retry(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<OwnedOwnerServer<T, R>, AppError> {
        let prepared = self.prepared.as_ref().ok_or_else(refused)?;
        let scope = self.private.reserve_scope(store, rng)?;
        if self.reservation.is_none() {
            let attempt = self
                .private
                .server()?
                .sync
                .fresh_chat_admission()
                .ok_or_else(refused)?
                .attempt_id();
            self.reservation =
                Some(store.reserve_owner_initialization(&scope, attempt, &mut self.creation)?);
        }
        // Acquire quota BEFORE signing/encoding retained facts or freezing provider/server state.
        let reservation = self.reservation.as_ref().ok_or_else(refused)?;
        if self.private.frozen_snapshot().is_err() {
            let sync = &mut self.private.server_mut_for_owner_initialization()?.sync;
            sync.stage_managed_chat_scope(scope.domain(), scope.nonce())?;
            sync.stage_owner_evidence_checkpoint(prepared)?;
        }
        store.install_private_founder_pin(&self.private, prepared, rng)?;
        // Confirm the installed pin through the mounted immutable source, not snapshot labels.
        let pinned = store.capture_mounted_owner_profile(prepared.group())?;
        if pinned.profile() != prepared.profile_id() || pinned.founder() != prepared.founder() {
            return Err(refused());
        }
        self.private.freeze_for_owner_initialization(reservation)?;
        {
            let frozen = self.private.frozen_snapshot()?;
            let saved = &mut self.saved;
            store.finish_chat_admission_with_storage(&scope, rng, |permit, rng| {
                let mut head = reservation.cell.lock()?;
                *saved = Some(head.save_initialization(
                    &permit,
                    reservation,
                    frozen,
                    prepared.profile_id(),
                    rng,
                )?);
                Ok(())
            })?;
        }
        // AlreadyCurrent cannot rerun an old covering callback. It may hand off only this
        // original retained saved result if the actual canonical current head is still exact.
        let binding = match store.resolve_chat_admission(
            &crate::store::ChatAdmissionKey::new(prepared.group(), *prepared.founder())?,
            scope.nonce(),
        )? {
            crate::store::ChatAdmissionResolution::Current(binding) => binding,
            _ => return Err(refused()),
        };
        let saved = self.saved.as_ref().ok_or_else(refused)?;
        let writer = reservation.writer.as_ref().ok_or_else(refused)?;
        let (server, scope_binding) =
            store.with_chat_admission_storage_permit(&binding, |permit| {
                let head = reservation.cell.lock()?;
                let strengthened = binding.with_owner_writer(&permit, &head, writer)?;
                self.private
                    .take_ready(&permit, &strengthened, saved, &head)
            })?;
        let binding = OwnerCanonicalBinding {
            scope: scope_binding,
            profile: prepared.profile_id(),
            founder: *prepared.founder(),
        };
        // Only this exact Saved+Current+head path constructs the stronger trusted-host adapter.
        // Runtime attachment changes no saved facts/snapshot bytes. Plain scope binding cannot
        // authorize an owner-tail restore, and original writer cancellation reaches both gates.

        let writer = self
            .reservation
            .as_mut()
            .ok_or_else(refused)?
            .writer
            .take()
            .ok_or_else(refused)?;
        let prepared = self.prepared.take().ok_or_else(refused)?;
        let profile = prepared.profile_id();
        let founder = *prepared.founder();
        // Frozen was already zeroized by take_ready. Release hot fee only after that transfer.
        self.reservation = None;
        Ok(OwnedOwnerServer {
            writer,
            server,
            profile,
            founder,
            current_head: self.saved.as_ref().ok_or_else(refused)?.metadata(),
            binding,
        })
    }
}

pub(super) struct OwnerCanonicalBinding {
    scope: ChatAdmissionBinding,
    profile: OwnerFactId,
    founder: [u8; 32],
}
impl ManagedChatScopeReservation for OwnerCanonicalBinding {
    fn group(&self) -> &[u8] {
        self.scope.group()
    }
    fn signer(&self) -> &[u8; 32] {
        self.scope.signer()
    }
    fn domain(&self) -> [u8; 32] {
        self.scope.domain()
    }
    fn nonce(&self) -> [u8; 32] {
        self.scope.nonce()
    }
    fn owner_identity(&self) -> Arc<()> {
        self.scope.owner_identity()
    }
    fn reservation_observer(&self) -> RequestCancellation {
        self.scope.reservation_observer()
    }
    fn revocation(&self) -> Arc<dyn ManagedChatScopeRevocation> {
        self.scope.revocation()
    }
}
impl ManagedChatScopeBinding for OwnerCanonicalBinding {
    fn publication_authority(&self) -> RequestCancellation {
        self.scope.publication_authority()
    }
}
impl ManagedOwnerEvidenceBinding for OwnerCanonicalBinding {
    fn profile_id(&self) -> OwnerFactId {
        self.profile
    }
    fn owner_founder(&self) -> &[u8; 32] {
        &self.founder
    }
}

/// Only SavedOwnerInitialization calls this after its exact private frozen-candidate/head check.
/// The unbound tail decoder and plain Current scope have no route to this constructor.
pub(super) fn bind_saved_runtime<T: MeshTransport, R: CryptoRngCore>(
    sync: &mut catcoms_sync::ChannelSync<T, R>,
    scope: &ChatAdmissionBinding,
    profile: OwnerFactId,
    founder: [u8; 32],
) -> Result<(), AppError> {
    sync.bind_managed_owner_evidence(&OwnerCanonicalBinding {
        scope: scope.clone(),
        profile,
        founder,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod unexposed_tests;
