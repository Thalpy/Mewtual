//! Isolated actual direct Add preparation; no network, live-field swap or save-success API.
//!
//! The covering App host uses this private provider through explicit snapshot refs, exact
//! successor/current-head custody and post-save install. This CLOSED first founder/member
//! checkpoint refuses real history and additional operations. Production activation still needs
//! latest receive-union/key repair, retained client init custody and the scalable v2 profile.
use super::operation_tail::{request_binding, OwnerOperationPredecessor, OwnerOperationTail};
use super::*;
use catcoms_mls::LocalMembershipChange;

const MAX_INPUT: usize = 8 * 1024;
const MAX_PACKAGE: usize = 32 * 1024;
const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const MAX_LEDGER_ROWS: usize = 4096;
const MAX_LEDGER_REPORTED_CAPACITY: usize = 8192;
const MAX_PROVIDER: usize = 8 * 1024 * 1024;

/// One actual Add on an isolated provider, never a portable authority or covering-save receipt.
/// No Clone/serde, response byte getter, transport, publisher or temporary live-field replacement.
pub struct PreparedOwnerMembership {
    pub(super) device: MlsDevice,
    pub(super) group: ServerGroup,
    pub(super) tenure: owner_tenure::OwnerTenure,
    pub(super) ledger: InviteLedger,
    pub(super) commit: CommitRecord,
    pub(super) evidence: InitialOwnerEvidence,
    pub(super) operation: OwnerOperationTail,
    pub(super) original_governance: [u8; 32],
    original_group: Vec<u8>,
    original_signer: [u8; 32],
    original_authenticator: [u8; 32],
    original_authority: RequestCancellation,
    profile: OwnerFactId,
}
impl std::fmt::Debug for PreparedOwnerMembership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedOwnerMembership")
            .finish_non_exhaustive()
    }
}
impl PreparedOwnerMembership {
    /// Inert exact lookup label, not a result byte getter or covering-save witness.
    pub fn request_binding(&self) -> [u8; 32] {
        self.operation.binding
    }
    /// Negative equality/lifetime check only. The App must additionally validate its ORIGINAL
    /// governing revision/head/source and held Current permit; this cannot adopt a newer writer.
    pub fn matches_original<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> bool {
        !self.original_authority.is_cancelled()
            && !node.owner_evidence_unbound()
            && node.group.group_id() == self.original_group
            && node.device.public_key_bytes().as_slice() == self.original_signer
            && node.group.epoch_authenticator_id() == self.original_authenticator
            && node.owner_evidence_profile_id() == Some(self.profile)
            && node.chat_scope_label().is_some_and(|label| {
                label.domain() == self.operation.predecessor.domain
                    && label.nonce() == self.operation.predecessor.nonce
                    && !label.retired()
                    && !label.initializing()
            })
    }
}

fn refusal() -> SyncError {
    SyncError::ChatScopeUnavailable
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Build one private actual Add after the host reserves complete source/candidate/response
    /// capacity and claims its original governing guard. Current metadata alone is insufficient:
    /// require the actual canonical owner runtime already bound to this source. No save, reply,
    /// topic, ledger, source provider, old-key window or governing field is installed here.
    /// The dedicated owner host must retain this object across every I/O error, never call the
    /// legacy admit_now path first, and save latest committed graph union through explicit refs.
    pub fn prepare_owner_direct_add_checkpoint(
        &mut self,
        invite: &InviteToken,
        package: &[u8],
        channel: u128,
        canonical_predecessor: OwnerOperationPredecessor,
        max_mls_bytes: usize,
        max_mls_backing_bytes: usize,
    ) -> Result<PreparedOwnerMembership, SyncError> {
        // Shared final-freeze guard also bounds metadata that a source may have accumulated
        // before this request. Unsupported retained work is preserved, never truncated to fit.
        let original_governance = membership_snapshot::governing_fingerprint(self)?;
        if self.owner_evidence_unbound()
            || self.initial_owner_evidence.is_none()
            // The first covering consumer has a CLOSED zero-history grammar. These O(1)
            // negative gates are required before a map scan, SDK copy or candidate allocation;
            // a later history-bearing transaction must supply its own complete receive/key
            // union view rather than deleting unsupported records to satisfy this predicate.
            || self.group.member_count() != 1
            || !self.docs.is_empty()
            || !self.commit_log.is_empty()
            || !self.past_keys.is_empty()
            || !self.pending_commits.is_empty()
            || !self.ledger.is_empty()
            || !self.direct_admit_results.is_empty()
            || !self.outbox.is_empty()
            || !self.managed_chat_outbox.is_empty()
            || !self.durable_chat.initialization_empty()
            || canonical_predecessor.generation != 1
            || self.chat_scope_label().is_none_or(|label| {
                label.domain() != canonical_predecessor.domain
                    || label.nonce() != canonical_predecessor.nonce
                    || label.retired()
                    || label.initializing()
            })
            || self.control_topic.as_bytes().is_empty()
            || self.control_topic.as_bytes().len() > operation_tail::MAX_TOPIC
            || !self.local_chat_scope.live()
            || !self.is_designated_committer()
            || self.config.max_committer_rank != 0
            || self.pending.is_some()
            || !self.add_request_queue.is_empty()
            || !self.admit_results.is_empty()
            || !self.admit_result_outbox.is_empty()
            || !self.outgoing_add_requests.is_empty()
            || !self.device_add_queue.is_empty()
            || !self.device_admit_results.is_empty()
            || !self.device_admit_outbox.is_empty()
            || !self.outgoing_device_adds.is_empty()
            || !self.companion_devices.is_empty()
            || !self.revoked_devices.is_empty()
            || !self.admitted_devices.is_empty()
            || !self.welcome_outbox.is_empty()
            || package.is_empty()
            || package.len() > MAX_PACKAGE
            || max_mls_bytes == 0
            || max_mls_bytes > MAX_PROVIDER
            || invite.group_id.len() > 256
            || invite.inviter_public_key.len() != 32
            || invite
                .bootstrap
                .len()
                .checked_add(invite.rendezvous.len())
                .is_none_or(|n| n > 8)
            || invite
                .bootstrap
                .iter()
                .chain(&invite.rendezvous)
                .any(|v| v.len() > 1024)
        {
            return Err(refusal());
        }
        self.policy_admission_ready()?;
        let group_id = self.group.group_id();
        let owner: [u8; 32] = self
            .device
            .public_key_bytes()
            .try_into()
            .map_err(|_| refusal())?;
        if invite.group_id != group_id
            || invite.inviter_device_id != self.device.device_id()
            || invite.inviter_public_key.as_slice() != owner
            || !invite.verify_self()
            || !self.invite_policy_matches(invite)
        {
            return Err(refusal());
        }
        let now = self.clock.now_ms();
        self.ledger.check(invite, now).map_err(MlsError::from)?;
        let evidence = self.initial_owner_evidence.as_ref().ok_or_else(refusal)?;
        let profile = evidence.original_profile().ok_or_else(refusal)?;
        if profile.issuer() != &owner {
            return Err(refusal());
        } // fixed-founder checkpoint only
        let mut facts = evidence.founder_chain(channel, &owner)?;
        let profile_id = profile.id();
        let predecessor = facts
            .values()
            .find_map(|fact| fact.basis_id())
            .ok_or_else(refusal)?;
        // The complete variable input was count/field-bounded above; the encoded <=8KiB gate
        // additionally limits the retained exact original. No arbitrary input clone precedes it.
        let invite_bytes = invite.encode();
        if invite_bytes.len() > MAX_INPUT {
            return Err(refusal());
        }
        let request = request_binding(
            &group_id,
            &owner,
            profile_id,
            channel,
            &invite_bytes,
            package,
        );
        // Existing negative rows and allocation/scan footprint are paid before duplication.
        // The host ALSO prepays one new consumed row and possible table growth (not this fee).
        let mut ledger = self
            .ledger
            .duplicate_bounded(MAX_LEDGER_ROWS - 1, MAX_LEDGER_REPORTED_CAPACITY)
            .map_err(MlsError::from)?;
        let original_authenticator = self.group.epoch_authenticator_id();
        let original_authority = self.local_chat_scope.authority().ok_or_else(refusal)?;
        let mls = catcoms_mls::persist::snapshot_server_bounded(
            &self.device,
            &self.group,
            max_mls_bytes,
            max_mls_backing_bytes,
        )?;
        // The provider's actual table/payload/output invoices are prepaid. Typed SDK JSON/group
        // heap expansion is a separate open gate; this gives an isolated provider without
        // swapping any live state or transport, not a whole-process resource proof.
        let (device, mut group) = catcoms_mls::persist::restore_server_provider_bounded(
            &mls,
            max_mls_bytes,
            max_mls_backing_bytes,
        )?;
        if group.group_id() != group_id
            || !group.has_local_signer(&owner)
            || !group.is_active()
            || group.epoch_authenticator_id() != original_authenticator
            || group.member_count() >= 64
        {
            return Err(refusal());
        }
        let key_package = device.parse_admitted_key_package(package, now)?;
        if serialize_key_package(&key_package)?.as_slice() != package {
            return Err(refusal());
        }
        group.validate_invite_binding(&key_package, invite)?;
        let before = owner_tenure::Position::of(&group);
        let mut tenure =
            owner_tenure::OwnerTenure::decode(&self.owner_tenure.encode(&self.group)?, &group)?;
        let (outcome, observation): (_, LocalMembershipChange) =
            group.add_member_observed(&device, key_package)?;
        ledger
            .consume(invite.invite_nonce)
            .map_err(MlsError::from)?;
        tenure.applied(before, &group); // pure private transition; no live reconciliation callback
        if !observation.matches_current(&group, &device)
            || group.designated_committer() != Some(device.device_id())
            || outcome.welcome.len() > MAX_RESPONSE - 1024
            || outcome.commit.len() > MAX_RESPONSE - 1024
        {
            return Err(refusal());
        }
        let reference = observation.key_package_reference().ok_or_else(refusal)?;
        let subject = *observation.subject();
        let policy = self
            .group_policy
            .as_ref()
            .map(|pin| pin.endorse(&group, &device))
            .transpose()?;
        let key = group.routing_transfer_key(&device)?;
        let secrets: Vec<_> = self
            .routing_secrets
            .iter()
            .map(|(label, secret)| (*label, **secret))
            .collect();
        let plaintext = Zeroizing::new(encode_join_transfer(
            self.routing_label,
            &secrets,
            &self.file_wrap_key,
            policy.as_ref(),
        ));
        // ONLY node-RNG use: consume original stream once, retain exact bytes, NEVER install a
        // candidate RNG or reseal on retry. SDK entropy above is its existing sanctioned source.
        let sealed_routing = encode_sealed(&seal(&key, &plaintext, &mut self.rng)?);
        let owner_sig = device.sign(&join_transcript(
            &group_id,
            &invite.invite_nonce,
            &outcome.welcome,
            &sealed_routing,
        ))?;
        let commit_auth = commit_auth_transcript(
            &group_id,
            outcome.commit_epoch,
            &original_authenticator,
            device.device_id().as_bytes(),
            &outcome.commit,
        );
        let commit = CommitRecord {
            group_id: group_id.clone(),
            commit_epoch: outcome.commit_epoch,
            committer_device: *device.device_id().as_bytes(),
            mls_commit: outcome.commit,
            base_authenticator: original_authenticator,
            committer_sig: device.sign(&commit_auth)?,
        };
        let sign = |body| {
            SignedOwnerFact::sign_using(
                OwnerFactSpec {
                    group: group_id.clone(),
                    profile: Some(profile_id),
                    body,
                },
                owner,
                |bytes| device.sign(bytes).map_err(|_| ChatError::Signature),
            )
            .map_err(|_| refusal())
        };
        let admission = sign(OwnerFactBody::Admission {
            subject,
            predecessor: profile_id,
            origin: AdmissionOrigin::Add {
                commit: observation.commit_digest(),
                before_epoch: observation.before_epoch(),
                after_epoch: observation.after_epoch(),
                key_package: reference,
                credential: observation.credential_digest().ok_or_else(refusal)?,
            },
        })?;
        let grant = sign(OwnerFactBody::RealmGrant {
            channel,
            admission: admission.id(),
            subject,
            rights: Rights::MEMBER,
        })?;
        let basis = sign(OwnerFactBody::Basis {
            channel,
            admission: admission.id(),
            subject,
            grant: grant.grant_id().ok_or_else(refusal)?,
            covered: vec![predecessor],
        })?;
        let bootstrap = sign(OwnerFactBody::JoinBootstrap {
            key_package: reference,
            subject,
            welcome: *blake3::hash(&outcome.welcome).as_bytes(),
            expected_founder: owner,
        })?;
        for fact in [admission, grant, basis, bootstrap] {
            if facts.insert(fact.id(), fact).is_some() {
                return Err(refusal());
            }
        }
        // Retain ONE encoded result. In particular do not also retain CachedAdmit's complete
        // Welcome and routing buffers: that would double the durable response without a fee.
        // The private operation allowance covers the temporary SDK Welcome and routing bytes
        // while this single exact response is encoded. No public getter exists before save.
        let response_len = 1usize
            .checked_add(12 + owner_sig.len())
            .and_then(|n| n.checked_add(outcome.welcome.len()))
            .and_then(|n| n.checked_add(sealed_routing.len()))
            .filter(|n| *n <= MAX_RESPONSE)
            .ok_or_else(refusal)?;
        let mut response = Encoder::with_capacity(response_len);
        response.put_u8(JOIN_READY);
        response
            .put_bytes(&outcome.welcome)
            .map_err(|_| refusal())?;
        response.put_bytes(&owner_sig).map_err(|_| refusal())?;
        response.put_bytes(&sealed_routing).map_err(|_| refusal())?;
        let response = response.finish();
        if response.len() > MAX_RESPONSE || original_authority.is_cancelled() {
            return Err(refusal());
        }
        let evidence = self
            .initial_owner_evidence
            .as_ref()
            .ok_or_else(refusal)?
            .after_first_add(facts)?;
        let operation = OwnerOperationTail {
            group: group_id.clone(),
            owner,
            profile: profile_id,
            predecessor: canonical_predecessor,
            channel,
            nonce: invite.invite_nonce,
            subject,
            reference,
            binding: request,
            invite: invite_bytes,
            package: package.to_vec(),
            response,
            topic: self.control_topic.as_bytes().to_vec(),
            commit: commit.commit_id(),
            before: original_authenticator,
            after: group.epoch_authenticator_id(),
        };
        Ok(PreparedOwnerMembership {
            device,
            group,
            tenure,
            ledger,
            commit,
            evidence,
            operation,
            original_governance,
            original_group: group_id,
            original_signer: owner,
            original_authenticator,
            original_authority,
            profile: profile_id,
        })
    }
}
