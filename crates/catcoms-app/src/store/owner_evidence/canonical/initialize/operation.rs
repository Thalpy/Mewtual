//! Real existing direct request -> private actual Add -> covering successor -> install ->
//! consuming original response. This remains the CLOSED no-history checkpoint host, not the
//! native production actor/typed rollout. Never return only an error that consumes this owner.
use super::*;
use catcoms_mls::InviteToken;
use catcoms_sync::owner_evidence::{
    ManagedOwnerOperationBinding, OwnerDirectRequest, OwnerOperationPredecessor,
    PreparedOwnerMembership,
};

#[cfg(test)]
mod tests;

/// Drop payloads FIRST and the allowance LAST. The responder's lower bytes can independently
/// retain Arc custody after this transaction is cleared; a timed-out waiter cannot refund them.
pub(super) struct PendingOwnerAdd {
    prepared: Option<PreparedOwnerMembership>,
    frozen: Zeroizing<Vec<u8>>,
    request: Option<OwnerDirectRequest>,
    predecessor: head::Meta,
    request_id: [u8; 32],
    effect: RequestCancellation,
    reservation: OwnerOperationReservation,
}

struct InstallBinding<'a> {
    original: &'a OwnerCanonicalBinding,
    predecessor: OwnerOperationPredecessor,
    request: [u8; 32],
}
impl ManagedChatScopeReservation for InstallBinding<'_> {
    fn group(&self) -> &[u8] {
        self.original.group()
    }
    fn signer(&self) -> &[u8; 32] {
        self.original.signer()
    }
    fn domain(&self) -> [u8; 32] {
        self.original.domain()
    }
    fn nonce(&self) -> [u8; 32] {
        self.original.nonce()
    }
    fn owner_identity(&self) -> Arc<()> {
        self.original.owner_identity()
    }
    fn reservation_observer(&self) -> RequestCancellation {
        self.original.reservation_observer()
    }
    fn revocation(&self) -> Arc<dyn ManagedChatScopeRevocation> {
        self.original.revocation()
    }
}
impl ManagedChatScopeBinding for InstallBinding<'_> {
    fn publication_authority(&self) -> RequestCancellation {
        self.original.publication_authority()
    }
}
impl ManagedOwnerEvidenceBinding for InstallBinding<'_> {
    fn profile_id(&self) -> OwnerFactId {
        self.original.profile_id()
    }
    fn owner_founder(&self) -> &[u8; 32] {
        self.original.owner_founder()
    }
}
impl ManagedOwnerOperationBinding for InstallBinding<'_> {
    fn predecessor(&self) -> OwnerOperationPredecessor {
        self.predecessor
    }
    fn saved_request_binding(&self) -> [u8; 32] {
        self.request
    }
}

impl<T: MeshTransport + 'static, R: CryptoRngCore> OwnedOwnerServer<T, R> {
    /// Actual original saved/current/head source; destination already holds its independent
    /// before-await pin+private creation allowance. No destination Store lock is held here.
    pub(in crate::store::owner_evidence) fn verify_checkpoint_member<
        U: MeshTransport,
        S: CryptoRngCore,
    >(
        &self,
        store: &ServerStore,
        member: &catcoms_sync::ChannelSync<U, S>,
        expected: &catcoms_replication::chat::owner_facts::OwnerProfileExpectation,
        channel: u128,
    ) -> Result<catcoms_sync::owner_evidence::VerifiedJoinedOwnerEvidence, AppError> {
        if self.pending_add.is_some() || self.current_head.generation != 2 {
            return Err(refused());
        }
        store.with_chat_admission_storage_permit(&self.binding.scope, |permit| {
            self.writer.with_saved_current_head(
                &permit,
                &self.binding.scope,
                &self.current_head,
                || {
                    self.server
                        .sync
                        .verify_joined_from_current_owner_checkpoint(member, expected, channel)
                        .map_err(|_| refused())
                },
            )?
        })
    }

    fn predecessor(&self) -> OwnerOperationPredecessor {
        OwnerOperationPredecessor {
            domain: self.binding.domain(),
            nonce: self.binding.nonce(),
            head: self.current_head.head,
            generation: self.current_head.generation,
            snapshot: self.current_head.snapshot,
        }
    }

    /// An invite from the actual original saved/current/head founder. It conveys no accepted
    /// Add result. The checkpoint refuses a second operation rather than evicting the first.
    pub fn mint_owner_checkpoint_invite(
        &self,
        store: &ServerStore,
        nonce: [u8; 16],
        expires: u64,
        bootstrap: Vec<String>,
    ) -> Result<InviteToken, AppError> {
        if self.pending_add.is_some() || self.current_head.generation != 1 {
            return Err(refused());
        }
        store.with_chat_admission_storage_permit(&self.binding.scope, |permit| {
            self.writer.with_saved_current_head(
                &permit,
                &self.binding.scope,
                &self.current_head,
                || {
                    self.server
                        .sync
                        .mint_invite(nonce, expires, bootstrap)
                        .map_err(AppError::from)
                },
            )?
        })
    }

    /// A single real transport tick. Capture THIS invocation's original native authority/head
    /// BEFORE await. A retained uncertain candidate is repaired BEFORE any new tick/mutation.
    /// Ordinary history receive during repair is not yet supported: this host cannot activate
    /// production Chat V2 until the reviewed latest-union/key transaction replaces this gate.
    pub async fn run_owner_checkpoint_once(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<bool, AppError> {
        if self.pending_add.is_some() {
            let (effect, service) = self.retry_pending_add(store, rng)?;
            let _ = self
                .server
                .sync
                .publish_saved_owner_operation_checkpoint(effect, service)
                .await;
            return Ok(true);
        }
        let effect = store.with_chat_admission_storage_permit(&self.binding.scope, |permit| {
            self.writer.with_saved_current_head(
                &permit,
                &self.binding.scope,
                &self.current_head,
                || self.writer.publication_authority(),
            )
        })?;
        if self.current_head.generation == 2 && self.server.sync.owner_commit_submission_owed() {
            // Best effort under separate paid lower custody. Capacity/network refusal leaves
            // the immutable debt intact and cannot turn an accepted result into a new Add.
            if let Ok(service) =
                store.with_chat_admission_storage_permit(&self.binding.scope, |permit| {
                    store.reserve_owner_response(
                        &permit,
                        &self.writer,
                        &self.binding.scope,
                        &self.current_head,
                    )
                })
            {
                let _ = self
                    .server
                    .sync
                    .publish_saved_owner_operation_checkpoint(effect.clone(), service)
                    .await;
            }
        }
        let progress = self.server.sync.run_once().await?;
        let Some(request) = self.server.sync.take_owner_direct_request() else {
            return Ok(progress);
        };
        if effect.is_cancelled() || !request.is_current() {
            return Ok(true);
        }
        if self.current_head.generation == 2 {
            // Exact durable lookup precedes fresh invite/KP/role/lifetime policy. A changed
            // request gets no result and can never mint another Add under its spent nonce.
            let service =
                store.with_chat_admission_storage_permit(&self.binding.scope, |permit| {
                    store.reserve_owner_response(
                        &permit,
                        &self.writer,
                        &self.binding.scope,
                        &self.current_head,
                    )
                })?;
            let _ = self
                .server
                .sync
                .respond_saved_owner_operation_checkpoint(request, effect, service);
            return Ok(true);
        }
        let reservation =
            store.with_chat_admission_storage_permit(&self.binding.scope, |permit| {
                store.reserve_owner_operation(
                    &permit,
                    &self.writer,
                    &self.binding.scope,
                    &self.current_head,
                )
            })?;
        let limits = reservation.limits();
        let (invite, package) = match request.original_parts() {
            Ok(parts) => parts,
            Err(_) => return Ok(true),
        };
        let channel = self.server.sync.owner_checkpoint_channel()?;
        let predecessor = self.predecessor();
        let prepared = match self.server.sync.prepare_owner_direct_add_checkpoint(
            &invite,
            package,
            channel,
            predecessor,
            limits.max_mls_bytes,
            limits.max_mls_backing_bytes,
        ) {
            Ok(prepared) => prepared,
            Err(_) => return Ok(true),
        };
        let frozen = self.server.sync.snapshot_owner_direct_add_checkpoint(
            &prepared,
            limits.max_snapshot_bytes,
            limits.max_mls_bytes,
            limits.max_mls_backing_bytes,
        )?;
        let request_id = prepared.request_binding();
        self.pending_add = Some(PendingOwnerAdd {
            prepared: Some(prepared),
            frozen,
            request: Some(request),
            predecessor: self.current_head.clone(),
            request_id,
            effect,
            reservation,
        });
        let (effect, service) = self.retry_pending_add(store, rng)?;
        let _ = self
            .server
            .sync
            .publish_saved_owner_operation_checkpoint(effect, service)
            .await;
        Ok(true)
    }

    fn retry_pending_add(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(RequestCancellation, catcoms_rt::SharedRequestKeepalive), AppError> {
        let pending = self.pending_add.as_mut().ok_or_else(refused)?;
        if pending.effect.is_cancelled() {
            return Err(refused());
        }
        let context = OwnerOperationPredecessor {
            domain: self.binding.domain(),
            nonce: self.binding.nonce(),
            head: pending.predecessor.head,
            generation: pending.predecessor.generation,
            snapshot: pending.predecessor.snapshot,
        };
        let digest = *blake3::hash(&pending.frozen).as_bytes();
        let meta = store.with_chat_admission_storage_permit(&self.binding.scope, |permit| {
            self.writer.with_first_add_successor(
                &permit,
                head::OwnerSuccessorInput {
                    binding: &self.binding.scope,
                    expected: &pending.predecessor,
                    reservation: &pending.reservation,
                    attempt: pending.request_id,
                    frozen: &pending.frozen,
                },
                rng,
                |head, saved| {
                    saved.validate_install(
                        head,
                        &permit,
                        &self.binding.scope,
                        &self.writer,
                        &pending.predecessor,
                        digest,
                    )?;
                    let binding = InstallBinding {
                        original: &self.binding,
                        predecessor: context,
                        request: pending.request_id,
                    };
                    self.server
                        .sync
                        .install_owner_direct_add_checkpoint(&mut pending.prepared, &binding)?;
                    Ok(saved.metadata())
                },
            )
        })?;
        self.current_head = meta;
        // Both admission/head callbacks returned. Payload/frozen buffers can be dropped before
        // the phase fee; actual response/lower clones retain their independent shared custody.
        let mut completed = self.pending_add.take().ok_or_else(refused)?;
        let request = completed.request.take();
        let effect = completed.effect.clone();
        // Preserve the struct's payload-before-fee unwind order while clearing its fields.
        // Only after their ACTUAL Drop may the original private allowance be reduced.
        drop(completed.prepared.take());
        drop(std::mem::take(&mut completed.frozen));
        let PendingOwnerAdd { reservation, .. } = completed;
        let service = reservation.into_response_custody()?;
        if let Some(request) = request {
            let _ = self.server.sync.respond_saved_owner_operation_checkpoint(
                request,
                effect.clone(),
                service.clone(),
            );
        }
        Ok((effect, service))
    }
}
