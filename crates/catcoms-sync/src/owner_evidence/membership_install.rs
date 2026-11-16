//! The trusted host's covering-save -> synchronous governed install boundary. This is a Rust
//! host contract, not a peer/renderer interface or a way to deserialize saved=true authority.
use super::membership_candidate::PreparedOwnerMembership;
use super::operation_tail::OwnerOperationPredecessor;
use super::*;

/// Implemented by the actual canonical App successor result under its still-held original
/// Current/head custody. Arbitrary Rust implementations are outside Sync's trust boundary,
/// just as for ManagedChatScopeBinding. Never implement it for a decoded label or a raw nonce.
pub trait ManagedOwnerOperationBinding: ManagedOwnerEvidenceBinding {
    fn predecessor(&self) -> OwnerOperationPredecessor;
    fn saved_request_binding(&self) -> [u8; 32];
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Move the actual prepared governing state only AFTER the original mounted successor's
    /// real file+parent barrier. All fallible consistency/chain checks precede `take`; failure
    /// therefore retains the exact candidate and its caller-owned frozen uncertainty/fee.
    /// No callback, await, transport, publication or application success event occurs here.
    pub fn install_owner_direct_add_checkpoint(
        &mut self,
        prepared: &mut Option<PreparedOwnerMembership>,
        binding: &impl ManagedOwnerOperationBinding,
    ) -> Result<(), SyncError> {
        let bad = || SyncError::ChatScopeUnavailable;
        let candidate = prepared.as_ref().ok_or_else(bad)?;
        if !candidate.matches_original(self)
            || membership_snapshot::governing_fingerprint(self)? != candidate.original_governance
            || binding.publication_authority().is_cancelled()
            || binding.group() != candidate.operation.group
            || binding.signer() != &candidate.operation.owner
            || binding.profile_id() != candidate.operation.profile
            || binding.domain() != candidate.operation.predecessor.domain
            || binding.nonce() != candidate.operation.predecessor.nonce
            || binding.predecessor() != candidate.operation.predecessor
            || binding.saved_request_binding() != candidate.operation.binding
        {
            return Err(bad());
        }
        candidate.evidence.verify_direct_operation(
            &candidate.operation,
            &candidate.commit,
            &candidate.group,
            &candidate.device,
            binding.owner_founder(),
        )?;
        // Repeat the ORIGINAL negative after all verification. This is still the same actual
        // host custody; an old task cannot acquire a later admission/writer as a substitute.
        if binding.publication_authority().is_cancelled() || !candidate.matches_original(self) {
            return Err(bad());
        }
        let candidate = prepared.take().ok_or_else(bad)?;
        self.group = candidate.group;
        self.device = candidate.device;
        self.ledger = candidate.ledger;
        self.owner_tenure = candidate.tenure;
        self.initial_owner_evidence = Some(candidate.evidence);
        self.commit_log.push_back(candidate.commit);
        self.owner_operation = Some(Arc::new(candidate.operation));
        self.owner_commit_submitted = false;
        // Do not call with_observed_mls_transition: the private tenure already applied the
        // actual transition once, and that helper would apply it again. Preserve its negative
        // reconciliation order explicitly before the caller can admit response/commit debt.
        self.observe_chat_scope_membership();
        self.admin_roster
            .retain(|device| self.group.contains_device(device));
        self.reconcile_voice_actor();
        Ok(())
    }
}
