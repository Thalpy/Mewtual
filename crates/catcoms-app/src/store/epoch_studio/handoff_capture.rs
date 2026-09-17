//! Staged automatic handoff (Flow H), stages H1 and H2.
//!
//! H1 runs under custody and is deliberately cheap: bounded authenticated reads, a structural
//! decode for classification, and the short live-authority mint. H2 runs on a blocking worker and
//! is where the expensive work lives: the full `decode_vault` of the retained branch, the private
//! successor reconstruction, and the per-operation change set.
//!
//! **No live capability crosses the boundary.** The worker receives captured *public* context
//! only, through `StudioEpoch::prepare_vault_source`: the group id, the designated-owner
//! `DeviceId`, the target and the actor. No `ServerGroup`, `MlsDevice`, MLS secret, store handle
//! or writer is moved into it. `copy_handoff_source(group)` is deliberately not used here; it is
//! the synchronous compatibility helper, and reaching for it would put live membership state in a
//! detached worker for no reason.
//!
//! Capturing the group id and the owner does not make them continuing authority. The captured
//! `StudioHandoffAuthority` is public verification context, and `sign_next` rechecks device,
//! membership, MLS epoch, observed tenure and the current-owner receipt before **every** single
//! signature. So if the owner, membership or MLS epoch changes while H2 is reconstructing, H2 can
//! at worst produce a stale proposal that H3 then refuses to sign. A detached result is never
//! authority.
use super::super::epoch_intents::{self, EpochIntentState};
use super::*;
use catcoms_crypto::DeviceId;
use catcoms_replication::studio::{StudioHandoffAuthority, StudioHandoffSigning};

/// Record identity and public live context, captured under custody. It carries no key, no store
/// handle, no `Server` and no budget, so it is safe to hold across a detach.
pub(crate) struct StudioHandoffStamp {
    mount: Arc<()>,
    pub(super) server: u64,
    pub(super) document: LogicalDocument,
    pub(super) target: StudioTarget,
    actor: DeviceId,
    actor_key: Vec<u8>,
    owner: DeviceId,
    mls: u64,
    /// The exact authenticated records H2 reconstructs from. H5 requires both to be unchanged
    /// before anything durable happens, because a plan derived from superseded bytes is a stale
    /// proposal however well formed it is.
    intent: (blake3::Hash, u64),
    source: (blake3::Hash, u64),
}

impl std::fmt::Debug for StudioHandoffStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioHandoffStamp { .. }")
    }
}

/// Authenticated plaintext plus the public restore context H2 needs. H1 produces exactly one of
/// these and nothing else durable.
pub(crate) struct StudioHandoffCapture {
    stamp: StudioHandoffStamp,
    group: Vec<u8>,
    basis: [u8; 32],
    authority: StudioHandoffAuthority,
    intent_bytes: Zeroizing<Vec<u8>>,
    source_bytes: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for StudioHandoffCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioHandoffCapture { .. }")
    }
}

/// A proposal carrying the private signing batch. It becomes durable only after a custody visit
/// reauthenticates the exact records it came from and signs every operation.
pub(crate) struct StudioHandoffPlan {
    pub(super) stamp: StudioHandoffStamp,
    pub(super) basis: [u8; 32],
    pub(super) signing: StudioHandoffSigning,
}

impl std::fmt::Debug for StudioHandoffPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioHandoffPlan { .. }")
    }
}

impl StudioHandoffCapture {
    /// H2, on a blocking worker and off custody. This is the expensive stage: the full
    /// `decode_vault` of any retained branch, the private successor reconstruction, and the
    /// per-operation change set. It signs nothing and writes nothing.
    pub(crate) fn prepare(self) -> Result<StudioHandoffPlan, AppError> {
        let scope = epoch_intents::scope_bytes(self.stamp.server, &self.stamp.document)?;
        let state = EpochIntentState::decode(&self.intent_bytes, &scope, &self.stamp.document)?;
        let metadata = state
            .handoff_metadata()
            .ok_or_else(|| invalid("overlay metadata missing"))?
            .clone();
        let source_scope = scope_bytes(self.stamp.server, &self.stamp.document)?;
        let (target, snapshot) =
            decode_record(&self.source_bytes, &source_scope, &self.stamp.document)?;
        if target != self.stamp.target {
            return Err(invalid("prepared source channel changed"));
        }
        // Captured public context only. See the module comment for why this is not
        // `copy_handoff_source(group)`.
        let source = StudioEpoch::prepare_vault_source(
            snapshot,
            &self.group,
            target,
            self.stamp.actor,
            self.stamp.owner,
        )
        .map_err(invalid)?;
        // The freshly decoded metadata must agree with the authority H1 minted. If the record
        // changed under the capture this refuses here, before any signature exists.
        let signing = metadata
            .prepare_handoff_detached(source, state.ledger.clone(), self.authority)
            .map_err(invalid)?;
        Ok(StudioHandoffPlan {
            stamp: self.stamp,
            basis: self.basis,
            signing,
        })
    }
}

impl ServerStore {
    /// Reacquired custody. Compares mount identity, numeric server, complete target, document,
    /// actor and key, designated owner, MLS epoch, and both authenticated record digests with
    /// their physical sizes. It decodes nothing.
    pub(super) fn studio_handoff_is_current(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        stamp: &StudioHandoffStamp,
    ) -> Result<bool, AppError> {
        if !Arc::ptr_eq(&stamp.mount, &self.registry_mount())
            || stamp.document.server_id != group.group_id()
            || stamp.actor != device.device_id()
            || stamp.actor_key != device.public_key_bytes()
            || Some(stamp.owner) != group.designated_committer()
            || stamp.mls != group.epoch()
        {
            return Ok(false);
        }
        let intent_scope = epoch_intents::scope_bytes(stamp.server, &stamp.document)?;
        let Some(intent) = self.read_scoped_intent_plain(&intent_scope)? else {
            return Ok(false);
        };
        if (blake3::hash(&intent.plain), intent.physical_bytes) != stamp.intent {
            return Ok(false);
        }
        let source_scope = scope_bytes(stamp.server, &stamp.document)?;
        let Some(source) =
            self.read_studio_record_bounded(&source_scope, source::MAX_RETAINED_BYTES as usize)?
        else {
            return Ok(false);
        };
        Ok((blake3::hash(&source.plain), source.physical_bytes) == stamp.source)
    }

    /// The H1 capture itself, after classification and authorization have both passed.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn capture_studio_handoff(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        document: &LogicalDocument,
        basis: [u8; 32],
        authority: StudioHandoffAuthority,
    ) -> Result<StudioHandoffCapture, AppError> {
        let intent_scope = epoch_intents::scope_bytes(server, document)?;
        let intent = self
            .read_scoped_intent_plain(&intent_scope)?
            .ok_or_else(|| invalid("overlay is missing"))?;
        let source_scope = scope_bytes(server, document)?;
        let source = self
            .read_studio_record_bounded(&source_scope, source::MAX_RETAINED_BYTES as usize)?
            .ok_or_else(|| invalid("overlay destination source missing"))?;
        self.check_studio_intent_link(server, document, &source_scope, &source.plain)?;
        Ok(StudioHandoffCapture {
            stamp: StudioHandoffStamp {
                mount: self.registry_mount(),
                server,
                document: document.clone(),
                target,
                actor: device.device_id(),
                actor_key: device.public_key_bytes(),
                owner: group
                    .designated_committer()
                    .ok_or_else(|| invalid("no current owner"))?,
                mls: group.epoch(),
                intent: (blake3::hash(&intent.plain), intent.physical_bytes),
                source: (blake3::hash(&source.plain), source.physical_bytes),
            },
            group: group.group_id(),
            basis,
            authority,
            intent_bytes: intent.plain,
            source_bytes: source.plain,
        })
    }
}
