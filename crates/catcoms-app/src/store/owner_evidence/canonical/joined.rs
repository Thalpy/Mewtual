//! Actual captured-root fresh join -> exact certificate verifier -> canonical member save.
//!
//! This CHECKPOINT host handoff is not production root distribution. The real owner-side Add
//! must cover its MLS/ledger/public facts/exact Welcome before response; this client cannot
//! prove remote fsync. Production v2 uses the separately signed invite/public-header closure.
use super::*;
use crate::store::chat_admission::fresh::{PrivateManagedJoinAttempt, PrivateManagedServer};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_rt::{Clock, MeshTransport, PeerId};
use catcoms_sync::owner_evidence::VerifiedJoinedOwnerEvidence;

/// Acquired original destination creation fee and immutable root BEFORE first network await.
/// No raw group/profile/founder tuple or downloaded self-signed Profile constructs this object.
pub struct OwnerCheckpointJoinAttempt {
    attempt: PrivateManagedJoinAttempt,
    creation: OwnerPendingCreation,
}
impl std::fmt::Debug for OwnerCheckpointJoinAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerCheckpointJoinAttempt")
            .finish_non_exhaustive()
    }
}
pub struct OwnerJoinedInitialization<T: MeshTransport, R: CryptoRngCore> {
    private: PrivateManagedServer<T, R>, // frozen/provider drops before following quota guards
    creation: OwnerPendingCreation,
    evidence: Option<VerifiedJoinedOwnerEvidence>,
    reservation: Option<OwnerInitializationReservation>,
    saved: Option<SavedOwnerInitialization>,
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for OwnerJoinedInitialization<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerJoinedInitialization")
            .finish_non_exhaustive()
    }
}
impl ServerStore {
    /// Explicit trusted-process CHECKPOINT path from an actual opaque ready source. Source
    /// Current/head is captured then released before destination pin checks; no cross-Store
    /// writer is held across handshake. Remote production input needs the signed v2 invite.
    pub fn prepare_owner_evidence_checkpoint_join<T: MeshTransport, R: CryptoRngCore>(
        &self,
        source_store: &ServerStore,
        source: &OwnedOwnerServer<T, R>,
        device: MlsDevice,
        invite: InviteToken,
    ) -> Result<OwnerCheckpointJoinAttempt, AppError> {
        let creation = self.reserve_pending_owner_creation()?;
        let pin = self.capture_ready_owner_profile(source_store, source)?;
        let attempt = self.private_managed_join_attempt(device, invite, pin)?;
        Ok(OwnerCheckpointJoinAttempt { attempt, creation })
    }
}
impl OwnerCheckpointJoinAttempt {
    /// Actual exact-KP join. Successful completion keeps its private source; a failed initial
    /// handshake changes no local canonical head. Joiner crash/reply-loss recovery additionally
    /// needs retained private KP/init-material custody and is NOT claimed by this checkpoint.
    pub async fn finish_direct<T: MeshTransport, R: CryptoRngCore>(
        self,
        transport: T,
        rng: R,
        clock: Box<dyn Clock + Send>,
        name: impl Into<String>,
        inviter: PeerId,
    ) -> Result<OwnerJoinedInitialization<T, R>, AppError> {
        let private = self
            .attempt
            .finish_direct(transport, rng, clock, name, inviter)
            .await?;
        Ok(OwnerJoinedInitialization {
            private,
            creation: self.creation,
            evidence: None,
            reservation: None,
            saved: None,
        })
    }
}
impl<T: MeshTransport, R: CryptoRngCore> OwnerJoinedInitialization<T, R> {
    /// Supply original bounded owner-signed facts. Expected root comes ONLY from the same
    /// pre-await capture INSIDE private join custody. Later caller expectations/rosters cannot
    /// substitute. Bad/foreign proof never mutates or clears already retained good evidence.
    pub fn supply_evidence(
        &mut self,
        store: &ServerStore,
        frames: &[&[u8]],
        channel: u128,
    ) -> Result<(), AppError> {
        let pin = self.private.captured_profile().ok_or_else(refused)?;
        pin.validate_original(store, pin.group())?;
        let expected =
            catcoms_replication::chat::owner_facts::OwnerProfileExpectation::from_trusted_host(
                pin.group(),
                *pin.founder(),
                pin.profile(),
            )
            .map_err(|_| refused())?;
        let verified = self
            .private
            .server()?
            .sync
            .verify_joined_owner_evidence(&expected, frames, channel)
            .map_err(|_| refused())?;
        if let Some(existing) = &self.evidence {
            if !existing.equivalent_proof(&verified) {
                return Err(refused());
            }
        } else {
            self.evidence = Some(verified);
        }
        Ok(())
    }
    /// Exact original certificate/nonce/head retry, retained across both rename outcomes.
    pub fn retry(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<OwnedOwnerServer<T, R>, AppError> {
        let verified = self.evidence.as_ref().ok_or_else(refused)?;
        let pin = self.private.captured_profile().ok_or_else(refused)?;
        pin.validate_original(store, verified.group())?;
        let profile = pin.profile();
        let founder = *pin.founder();
        if profile != verified.profile_id()
            || !verified.matches_original(&self.private.server()?.sync)
        {
            return Err(refused());
        }
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
        let reservation = self.reservation.as_ref().ok_or_else(refused)?;
        if self.private.frozen_snapshot().is_err() {
            let sync = &mut self.private.server_mut_for_owner_initialization()?.sync;
            sync.stage_managed_chat_scope(scope.domain(), scope.nonce())?;
            sync.stage_joined_owner_evidence_checkpoint(verified)?;
        }
        store.install_joined_owner_pin(&self.private, verified, rng)?;
        self.private.freeze_for_owner_initialization(reservation)?;
        {
            let frozen = self.private.frozen_snapshot()?;
            let saved = &mut self.saved;
            store.finish_chat_admission_with_storage(&scope, rng, |permit, rng| {
                let mut head = reservation.cell.lock()?;
                *saved =
                    Some(head.save_initialization(&permit, reservation, frozen, profile, rng)?);
                Ok(())
            })?;
        }
        let binding = match store.resolve_chat_admission(
            &crate::store::ChatAdmissionKey::new(verified.group(), *verified.subject())?,
            scope.nonce(),
        )? {
            crate::store::ChatAdmissionResolution::Current(binding) => binding,
            _ => return Err(refused()),
        };
        let saved = self.saved.as_ref().ok_or_else(refused)?;
        let writer = reservation.writer.as_ref().ok_or_else(refused)?;
        let (server, binding) = store.with_chat_admission_storage_permit(&binding, |permit| {
            let head = reservation.cell.lock()?;
            let binding = binding.with_owner_writer(&permit, &head, writer)?;
            self.private.take_ready(&permit, &binding, saved, &head)
        })?;
        let writer = self
            .reservation
            .as_mut()
            .ok_or_else(refused)?
            .writer
            .take()
            .ok_or_else(refused)?;
        let head = self.saved.as_ref().ok_or_else(refused)?.metadata();
        // The ready Server/canonical tail retains every signed original. The completed outer
        // object must release its now-unused verifier copy BEFORE returning the paid hot fee.
        // Both callbacks have returned, so no pool-drop inverse lock can occur here.
        self.evidence = None;
        self.reservation = None; // private Frozen was already zeroized by take_ready
        Ok(OwnedOwnerServer::from_restored(
            server, writer, binding, profile, founder, head,
        ))
    }
}
