//! Explicit borrowed governing view for the covering first-Add consumer. The common serializer
//! substitutes these references while it continues to borrow all other live fields. It never
//! installs a temporary MLS/group/ledger, swaps an actor or constructs another transport owner.
use super::membership_candidate::PreparedOwnerMembership;
use super::operation_tail::OwnerOperationTail;
use super::*;

pub(crate) struct OwnerMembershipSnapshot<'a> {
    pub(crate) group: &'a ServerGroup,
    pub(crate) ledger: &'a InviteLedger,
    pub(crate) tenure: &'a owner_tenure::OwnerTenure,
    pub(crate) evidence: &'a InitialOwnerEvidence,
    pub(crate) commit: &'a CommitRecord,
    pub(crate) operation: &'a OwnerOperationTail,
}

/// Negative closed-state guard. It must stay O(1) for ordinary/history-bearing collections;
/// no scan, truncation, hash-table compaction or snapshot is a substitute for these refusals.
pub(super) fn closed_source<T: MeshTransport, R: CryptoRngCore>(
    node: &ChannelSync<T, R>,
) -> Result<(), SyncError> {
    if node.owner_evidence_unbound()
        || !node.local_chat_scope.live()
        || node.group.member_count() != 1
        || node.group.epoch() != 0
        || !node.docs.is_empty()
        || !node.commit_log.is_empty()
        || !node.pending_commits.is_empty()
        || !node.past_keys.is_empty()
        || !node.ledger.is_empty()
        || !node.direct_admit_results.is_empty()
        || !node.durable_chat.initialization_empty()
        || !node.peer_records.is_empty()
        || !node.outbox.is_empty()
        || !node.managed_chat_outbox.is_empty()
        || node.pending.is_some()
        || !node.add_request_queue.is_empty()
        || !node.admit_results.is_empty()
        || !node.admit_result_outbox.is_empty()
        || !node.outgoing_add_requests.is_empty()
        || !node.device_add_queue.is_empty()
        || !node.device_admit_results.is_empty()
        || !node.device_admit_outbox.is_empty()
        || !node.outgoing_device_adds.is_empty()
        || !node.companion_devices.is_empty()
        || !node.revoked_devices.is_empty()
        || !node.admitted_devices.is_empty()
        || !node.welcome_outbox.is_empty()
        || !node.admin_roster.is_empty()
        || node.roster_gen != 0
        || node.config.max_committer_rank != 0
        || !node.is_designated_committer()
        || node.routing_secrets.len() > MAX_ROUTING_SECRETS
        || node.rendezvous_nodes.len() > initialization_snapshot::MAX_INITIAL_RENDEZVOUS
        || node
            .rendezvous_nodes
            .iter()
            .any(|(address, peer)| address.len() > 1024 || peer.len() > 128)
        || node.member_finalization_pending.len()
            > initialization_snapshot::MAX_INITIAL_FINALIZATIONS
        || node.member_finalization_pending.capacity()
            > initialization_snapshot::MAX_INITIAL_FINALIZATION_CAPACITY
    {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    Ok(())
}

/// Complete bounded governing metadata comparison for this closed source. Receive-side graph
/// concurrency is deliberately unsupported here (closed_source refuses real history); production
/// needs the reviewed latest-union/key view and common governing guard before activation.
pub(super) fn governing_fingerprint<T: MeshTransport, R: CryptoRngCore>(
    node: &ChannelSync<T, R>,
) -> Result<[u8; 32], SyncError> {
    closed_source(node)?;
    let bad = || SyncError::ChatScopeUnavailable;
    let evidence = node.initial_owner_evidence.as_ref().ok_or_else(bad)?;
    let profile = evidence.original_profile().ok_or_else(bad)?;
    let policy = group_policy::encode_pin(node.group_policy.as_ref());
    let tenure = node.owner_tenure.encode(&node.group)?;
    let durable = node.durable_chat.encode()?;
    if policy.len() > 4101 || tenure.len() > 256 || durable.len() > 256 {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    let label = node.chat_scope_label().ok_or_else(bad)?;
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/owner-first-add-governance/v1\0");
    hash.update(&node.group.epoch_authenticator_id());
    hash.update(profile.id().as_bytes());
    hash.update(&label.domain());
    hash.update(&label.nonce());
    hash.update(&node.routing_label.to_be_bytes());
    for (slot, secret) in &node.routing_secrets {
        hash.update(&slot.to_be_bytes());
        hash.update(secret.as_slice());
    }
    hash.update(&node.file_wrap_key[..]);
    hash.update(&(policy.len() as u32).to_be_bytes());
    hash.update(&policy);
    hash.update(&(tenure.len() as u32).to_be_bytes());
    hash.update(&tenure);
    hash.update(&(durable.len() as u32).to_be_bytes());
    hash.update(&durable);
    hash.update(&node.group_policy_revision.to_be_bytes());
    hash.update(&[u8::from(node.group_policy_active)]);
    Ok(*hash.finalize().as_bytes())
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Encode the same original private Add under prepaid source/candidate/snapshot limits.
    /// The returned sealed-store INPUT is not a Saved/current/head witness or permission to
    /// deliver its response. The actual App successor writer owns the storage/installation edge.
    pub fn snapshot_owner_direct_add_checkpoint(
        &mut self,
        prepared: &PreparedOwnerMembership,
        max_snapshot_bytes: usize,
        max_mls_bytes: usize,
        max_mls_backing_bytes: usize,
    ) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        if !prepared.matches_original(self)
            || governing_fingerprint(self)? != prepared.original_governance
            || max_mls_bytes == 0
            || max_mls_backing_bytes == 0
        {
            return Err(SyncError::ChatScopeUnavailable);
        }
        let original_founder = self
            .initial_owner_evidence
            .as_ref()
            .and_then(InitialOwnerEvidence::original_profile)
            .ok_or(SyncError::ChatScopeUnavailable)?
            .issuer();
        if prepared.operation.topic.as_slice() != self.control_topic.as_bytes() {
            return Err(SyncError::ChatScopeUnavailable);
        }
        // Validate the complete original chain/result BEFORE any canonical write can latch it.
        // The expectation comes from this already bound ORIGINAL root, never the candidate's
        // self-selected bytes. Install and cold restore repeat the same negative verification.
        prepared.evidence.verify_direct_operation(
            &prepared.operation,
            &prepared.commit,
            &prepared.group,
            &prepared.device,
            original_founder,
        )?;
        let metadata = operation_tail::MAX_BYTES
            .checked_add(operation_tail::MAX_COMMIT)
            .and_then(|n| n.checked_add(initialization_snapshot::METADATA_FEE))
            .ok_or(SyncError::Malformed)?;
        if max_mls_bytes
            .checked_add(metadata)
            .is_none_or(|n| n > max_snapshot_bytes)
        {
            return Err(SyncError::OwnerInitializationUnsupported);
        }
        let mls = catcoms_mls::persist::snapshot_server_bounded(
            &prepared.device,
            &prepared.group,
            max_mls_bytes,
            max_mls_backing_bytes,
        )?;
        let capacity = mls
            .len()
            .checked_add(metadata)
            .ok_or(SyncError::Malformed)?;
        let view = OwnerMembershipSnapshot {
            group: &prepared.group,
            ledger: &prepared.ledger,
            tenure: &prepared.tenure,
            evidence: &prepared.evidence,
            commit: &prepared.commit,
            operation: &prepared.operation,
        };
        let bytes = self.snapshot_with_governance(&mls, Some(capacity), Some(&view))?;
        if bytes.len() > max_snapshot_bytes || !prepared.matches_original(self) {
            return Err(SyncError::ChatScopeUnavailable);
        }
        Ok(bytes)
    }
}
