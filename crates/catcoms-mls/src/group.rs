//! The [`ServerGroup`] wrapper over an openmls `MlsGroup`.
//!
//! Each operation takes the [`MlsDevice`] that owns this group's state (the
//! device that created or joined it); openmls reads its keys from that device's
//! provider.

use core::fmt;

use catcoms_crypto::DeviceId;
use openmls::prelude::*;
use tls_codec::{Deserialize as _, Serialize as _};

use crate::config::{create_config, join_config};
use crate::device::MlsDevice;
use crate::invite::{membership_from_key_package, InviteError, InviteLedger, InviteToken};
use crate::{proto, MlsError};

/// The result of adding a member: the joiner's Welcome, the Commit to fan out to
/// existing members, and the epoch the commit was built at.
#[derive(Debug, Clone)]
pub struct AddOutcome {
    /// Serialized Welcome for the joining device.
    pub welcome: Vec<u8>,
    /// Serialized Commit message for existing members to apply.
    pub commit: Vec<u8>,
    /// The epoch the commit was built at (advances the group to `commit_epoch + 1`).
    pub commit_epoch: u64,
}

/// The result of *staging* a membership commit without merging it (see
/// [`ServerGroup::stage_add`]). The group is left with a pending commit at
/// `commit_epoch`; `base_authenticator` is the epoch-state fingerprint it was
/// built on (the fork-vs-lag binding).
#[derive(Debug, Clone)]
pub struct StagedOutcome {
    /// Serialized Commit message for existing members to apply.
    pub commit: Vec<u8>,
    /// Serialized Welcome for a joining device (present for Adds, absent for Removes).
    pub welcome: Option<Vec<u8>>,
    /// The epoch the commit was built at (it advances the group to `commit_epoch + 1`).
    pub commit_epoch: u64,
    /// The committer's epoch-state fingerprint *before* this commit.
    pub base_authenticator: [u8; 32],
}

/// The result of processing an inbound MLS message.
#[derive(Debug)]
pub enum Incoming {
    /// A decrypted application-message payload.
    Application(Vec<u8>),
    /// A commit was processed and merged (group state advanced). `removed` is true
    /// iff the commit contained at least one Remove proposal; the signal the
    /// routing layer uses to rotate the per-removal metadata secret (`ns_secret_L`)
    /// identically on every member, not just the local committer.
    CommitApplied {
        /// Whether this commit removed at least one member.
        removed: bool,
    },
    /// A proposal or other control message was processed (no payload).
    Other,
}

/// One server/connection: a wrapper over an MLS group.
pub struct ServerGroup {
    group: MlsGroup,
}

impl ServerGroup {
    /// Found a new group with `device` as the only member.
    pub fn create(device: &MlsDevice) -> Result<Self, MlsError> {
        let group = MlsGroup::new(
            device.provider(),
            device.signer(),
            &create_config(),
            device.credential(),
        )
        .map_err(proto)?;
        Ok(Self { group })
    }

    /// Join an existing group from a serialized Welcome (as produced by
    /// [`ServerGroup::add_member`]).
    pub fn join(device: &MlsDevice, welcome_bytes: &[u8]) -> Result<Self, MlsError> {
        let msg_in = MlsMessageIn::tls_deserialize(&mut &welcome_bytes[..]).map_err(proto)?;
        let welcome = match msg_in.extract() {
            MlsMessageBodyIn::Welcome(w) => w,
            _ => return Err(MlsError::WrongMessageType),
        };
        let group =
            StagedWelcome::new_from_welcome(device.provider(), &join_config(), welcome, None)
                .map_err(proto)?
                .into_group(device.provider())
                .map_err(proto)?;
        Ok(Self { group })
    }

    /// Reconstruct a group from a `device` whose provider storage was **restored** from a
    /// snapshot (Phase 9c). `group_id` is the value from [`ServerGroup::group_id`]. See
    /// [`crate::persist`].
    pub(crate) fn load(device: &MlsDevice, group_id: &[u8]) -> Result<Self, MlsError> {
        let gid = GroupId::from_slice(group_id);
        let group = MlsGroup::load(device.provider().storage(), &gid)
            .map_err(|e| MlsError::Protocol(format!("{e:?}")))?
            .ok_or(MlsError::Internal("group missing from restored storage"))?;
        Ok(Self { group })
    }

    /// Add `key_package`'s device and merge the commit. Returns the [`AddOutcome`]:
    /// the Welcome for the joiner, **and** the serialized Commit (which previously
    /// was discarded) so it can be fanned out to existing members, plus the epoch
    /// the commit was built at (it advances the group from `commit_epoch` to
    /// `commit_epoch + 1`).
    pub fn add_member(
        &mut self,
        device: &MlsDevice,
        key_package: KeyPackage,
    ) -> Result<AddOutcome, MlsError> {
        let (commit, welcome, _group_info) = self
            .group
            .add_members(device.provider(), device.signer(), &[key_package])
            .map_err(proto)?;
        let commit_epoch = self.epoch();
        self.group
            .merge_pending_commit(device.provider())
            .map_err(proto)?;
        Ok(AddOutcome {
            welcome: welcome.tls_serialize_detached().map_err(proto)?,
            commit: commit.tls_serialize_detached().map_err(proto)?,
            commit_epoch,
        })
    }

    /// The device id of the **designated committer**; the member with the lowest
    /// leaf index (the only roster value every member derives identically from the
    /// ratchet tree). In the single-committer model this member is the only one
    /// permitted to produce commits, which prevents concurrent commits from
    /// forking the epoch chain.
    pub fn designated_committer(&self) -> Option<DeviceId> {
        self.group
            .members()
            .min_by_key(|m| m.index.u32())
            .map(|m| DeviceId::from_public_key_bytes(&m.signature_key))
    }

    /// Whether `device` is the designated committer.
    pub fn is_designated_committer(&self, device: &MlsDevice) -> bool {
        self.designated_committer() == Some(device.device_id())
    }

    /// The leaf index of the designated committer (the lowest occupied index).
    pub fn designated_committer_index(&self) -> Option<u32> {
        self.group.members().map(|m| m.index.u32()).min()
    }

    /// The designated committer's leaf index and a digest over its leaf **identity**:
    /// `blake3(index, signature_key, credential bytes)`.
    ///
    /// This exists so an observer can tell an ordinary same-owner commit from a remove-and-re-add of
    /// the committer. Comparing `DeviceId` and epoch cannot: a device removed and re-added in one
    /// commit keeps its `DeviceId`, so a witness preserves the old tenure start while the rejoining
    /// device computes a new one, and the two disagree about who may issue a receipt.
    ///
    /// The HPKE `encryption_key` is **deliberately excluded**. An ordinary self-update rotates that
    /// key while keeping the credential, and a self-update is not a discontinuity: including it would
    /// make every key rotation look like a new tenure and destroy the knowledge this value exists to
    /// preserve. The credential is the right discriminator because a joiner's KeyPackage credential is
    /// bound to `(this group, invite_nonce)`, so a genuine rejoin always presents a different one
    /// while an update never changes it.
    pub fn designated_committer_leaf(&self) -> Option<(u32, [u8; 32])> {
        let member = self.group.members().min_by_key(|m| m.index.u32())?;
        let mut hash = blake3::Hasher::new_derive_key("catcoms/mls-committer-leaf/v1");
        hash.update(&member.index.u32().to_be_bytes());
        hash.update(&member.signature_key);
        hash.update(member.credential.serialized_content());
        Some((member.index.u32(), *hash.finalize().as_bytes()))
    }

    /// The leaf index of a current member, by device id.
    pub fn member_leaf_index(&self, device_id: &DeviceId) -> Option<u32> {
        self.group
            .members()
            .find(|m| DeviceId::from_public_key_bytes(&m.signature_key) == *device_id)
            .map(|m| m.index.u32())
    }

    /// The raw Ed25519 signature public key of a current member, by device id.
    /// Used to verify a committer's per-commit signature by **roster lookup**
    /// (a `DeviceId` is a one-way hash of the key, so the verifier looks the key
    /// up rather than recovering it from the id).
    pub fn member_signature_key(&self, device_id: &DeviceId) -> Option<Vec<u8>> {
        self.group
            .members()
            .find(|m| DeviceId::from_public_key_bytes(&m.signature_key) == *device_id)
            .map(|m| m.signature_key)
    }

    /// A 32-byte fingerprint of this group's current epoch state; `BLAKE3` of the
    /// MLS `epoch_authenticator` (a members-only value every member derives
    /// identically). Two records built against the same fingerprint are a genuine
    /// same-base fork (resolvable by tie-break); different fingerprints at the same
    /// epoch number mean the branches diverged earlier (a deep fork we refuse to
    /// silently merge). Hashed so the raw epoch secret never leaves the device.
    pub fn epoch_authenticator_id(&self) -> [u8; 32] {
        *blake3::hash(self.group.epoch_authenticator().as_slice()).as_bytes()
    }

    /// Mint a single-use, device-bound invite to this group, signed by `inviter`
    /// (who must be a current member). `invite_nonce` must be unique per invite.
    /// Carries no rendezvous infra addresses (`bootstrap`-only); see
    /// [`ServerGroup::mint_invite_with_rendezvous`] for the discovery-enabled form.
    pub fn mint_invite(
        &self,
        inviter: &MlsDevice,
        invite_nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
    ) -> Result<InviteToken, MlsError> {
        self.mint_invite_with_rendezvous(
            inviter,
            invite_nonce,
            expires_at_ms,
            bootstrap,
            Vec::new(),
        )
    }

    /// Mint an invite that also carries zero-knowledge **rendezvous** infra addresses
    /// (6e-3d-9), so a joiner can discover the inviter under the pre-join `join_ns`
    /// without a hard-coded server address. The rendezvous set is bound into the
    /// inviter signature (a relay cannot strip or substitute it).
    ///
    /// The set is signed **verbatim**, so the caller should validate it first with
    /// `catcoms_net::validate_rendezvous_addrs` (reject `/p2p-circuit`, require a
    /// `/p2p/` id, distinct PeerIds); that lives in `catcoms-net` where multiaddrs
    /// parse, and an invalid set minted here would otherwise fail only at the joiner.
    pub fn mint_invite_with_rendezvous(
        &self,
        inviter: &MlsDevice,
        invite_nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
        rendezvous: Vec<String>,
    ) -> Result<InviteToken, MlsError> {
        let inviter_public_key = inviter.public_key_bytes();
        let payload = InviteToken::signing_payload(
            &self.group_id(),
            &inviter.device_id(),
            &inviter_public_key,
            &invite_nonce,
            expires_at_ms,
            &bootstrap,
            &rendezvous,
        );
        let signature = inviter.sign_raw(&payload)?;
        Ok(InviteToken {
            group_id: self.group_id(),
            inviter_device_id: inviter.device_id(),
            inviter_public_key,
            invite_nonce,
            expires_at_ms,
            bootstrap,
            rendezvous,
            policy: None,
            signature,
        })
    }

    /// Admit a device using a single-use invite. Validates, in order: the token
    /// targets this group; the inviter is a current member and signed the token;
    /// the invite is fresh (not expired/revoked/used); and the joiner's KeyPackage
    /// credential is bound to exactly `(this group, invite_nonce)`. On success the
    /// nonce is consumed and the [`AddOutcome`] (Welcome + Commit) is returned.
    pub fn add_member_via_invite(
        &mut self,
        inviter: &MlsDevice,
        key_package: KeyPackage,
        token: &InviteToken,
        ledger: &mut InviteLedger,
        now_ms: u64,
    ) -> Result<AddOutcome, MlsError> {
        let group_id = self.group_id();
        if token.group_id != group_id {
            return Err(InviteError::WrongGroup.into());
        }
        ledger.check(token, now_ms)?;

        // The inviter must be a current member; verify the token under their key.
        let inviter_pk = self
            .member_signature_key(&token.inviter_device_id)
            .ok_or(InviteError::InviterNotMember)?;
        if !token.verify(&inviter_pk) {
            return Err(InviteError::BadSignature.into());
        }

        // The joiner's KeyPackage credential must bind to this group + nonce, and
        // its device id must content-address its own leaf signature key.
        let membership = membership_from_key_package(&key_package)?;
        let leaf_pk = key_package.leaf_node().signature_key().as_slice();
        if membership.group_id != group_id
            || membership.invite_nonce != token.invite_nonce
            || DeviceId::from_public_key_bytes(leaf_pk) != membership.device_id
        {
            return Err(InviteError::CredentialMismatch.into());
        }

        let outcome = self.add_member(inviter, key_package)?;
        ledger.consume(token.invite_nonce)?;
        Ok(outcome)
    }

    /// Validate that `key_package` is admissible under `token` **without** adding it
    /// or consuming the invite; the binding checks `add_member_via_invite` runs
    /// before the Add, factored out so a *staged* (fork-resolvable) admission can
    /// validate up front and consume the invite only once its commit merges.
    /// Invite freshness (the ledger) is checked separately by the caller.
    pub fn validate_invite_binding(
        &self,
        key_package: &KeyPackage,
        token: &InviteToken,
    ) -> Result<(), MlsError> {
        let group_id = self.group_id();
        if token.group_id != group_id {
            return Err(InviteError::WrongGroup.into());
        }
        let inviter_pk = self
            .member_signature_key(&token.inviter_device_id)
            .ok_or(InviteError::InviterNotMember)?;
        if !token.verify(&inviter_pk) {
            return Err(InviteError::BadSignature.into());
        }
        let membership = membership_from_key_package(key_package)?;
        let leaf_pk = key_package.leaf_node().signature_key().as_slice();
        if membership.group_id != group_id
            || membership.invite_nonce != token.invite_nonce
            || DeviceId::from_public_key_bytes(leaf_pk) != membership.device_id
        {
            return Err(InviteError::CredentialMismatch.into());
        }
        Ok(())
    }

    /// Validate that `key_package` is admissible as `expected_device`'s leaf, bound to
    /// `(this group, bind_nonce)`; the **certificate-bound** analogue of
    /// [`ServerGroup::validate_invite_binding`], for the multi-device companion admission
    /// (`docs/design-multi-device.md` M3), which carries a device certificate instead of an
    /// invite token.
    ///
    /// `bind_nonce` is derived deterministically from the certificate by the admitting layer,
    /// so a KeyPackage minted against one certificate can never be relayed into an admission
    /// for another; the same non-replayability the invite nonce gives the invite path, and
    /// the same leaf-credential shape every member re-checks in
    /// [`ServerGroup::process_incoming`].
    pub fn validate_device_binding(
        &self,
        key_package: &KeyPackage,
        expected_device: &DeviceId,
        bind_nonce: &[u8; 16],
    ) -> Result<(), MlsError> {
        let membership = membership_from_key_package(key_package)?;
        let leaf_pk = key_package.leaf_node().signature_key().as_slice();
        if membership.group_id != self.group_id()
            || membership.invite_nonce != *bind_nonce
            || membership.device_id != *expected_device
            || DeviceId::from_public_key_bytes(leaf_pk) != membership.device_id
        {
            return Err(InviteError::CredentialMismatch.into());
        }
        Ok(())
    }

    /// Remove the member with `target` device id and merge the commit (this
    /// advances the epoch, healing forward secrecy / post-compromise security).
    pub fn remove_member(&mut self, device: &MlsDevice, target: &DeviceId) -> Result<(), MlsError> {
        let index = self
            .group
            .members()
            .find(|m| DeviceId::from_public_key_bytes(&m.signature_key) == *target)
            .map(|m| m.index)
            .ok_or(MlsError::MemberNotFound)?;
        self.group
            .remove_members(device.provider(), device.signer(), &[index])
            .map_err(proto)?;
        self.group
            .merge_pending_commit(device.provider())
            .map_err(proto)?;
        Ok(())
    }

    /// Stage an Add **without merging it**: produce the commit + Welcome but leave
    /// the group with a pending commit at the current epoch. Call
    /// [`ServerGroup::merge_staged_self`] to adopt it (advancing the epoch) or
    /// [`ServerGroup::abort_staged`] to discard it (restoring the pre-stage state,
    /// epoch secrets intact). This is the producer side of fork resolution: a
    /// committer stages, broadcasts, and only merges once it knows it won.
    pub fn stage_add(
        &mut self,
        device: &MlsDevice,
        key_package: KeyPackage,
    ) -> Result<StagedOutcome, MlsError> {
        let base_authenticator = self.epoch_authenticator_id();
        let commit_epoch = self.epoch();
        let (commit, welcome, _group_info) = self
            .group
            .add_members(device.provider(), device.signer(), &[key_package])
            .map_err(proto)?;
        Ok(StagedOutcome {
            commit: commit.tls_serialize_detached().map_err(proto)?,
            welcome: Some(welcome.tls_serialize_detached().map_err(proto)?),
            commit_epoch,
            base_authenticator,
        })
    }

    /// Stage a Remove without merging it (see [`ServerGroup::stage_add`]).
    pub fn stage_remove(
        &mut self,
        device: &MlsDevice,
        target: &DeviceId,
    ) -> Result<StagedOutcome, MlsError> {
        let index = self
            .group
            .members()
            .find(|m| DeviceId::from_public_key_bytes(&m.signature_key) == *target)
            .map(|m| m.index)
            .ok_or(MlsError::MemberNotFound)?;
        let base_authenticator = self.epoch_authenticator_id();
        let commit_epoch = self.epoch();
        let (commit, welcome, _group_info) = self
            .group
            .remove_members(device.provider(), device.signer(), &[index])
            .map_err(proto)?;
        Ok(StagedOutcome {
            commit: commit.tls_serialize_detached().map_err(proto)?,
            welcome: welcome
                .map(|w| w.tls_serialize_detached())
                .transpose()
                .map_err(proto)?,
            commit_epoch,
            base_authenticator,
        })
    }

    /// Adopt this group's own staged commit (advances the epoch). The inverse of
    /// [`ServerGroup::abort_staged`].
    pub fn merge_staged_self(&mut self, device: &MlsDevice) -> Result<(), MlsError> {
        self.group
            .merge_pending_commit(device.provider())
            .map_err(proto)
    }

    /// Discard this group's own staged commit, restoring the pre-stage state with
    /// epoch secrets intact (openmls `clear_pending_commit` only flips the group
    /// state back to Operational). The fork **loser**'s primitive.
    pub fn abort_staged(&mut self, device: &MlsDevice) -> Result<(), MlsError> {
        self.group
            .clear_pending_commit(device.provider().storage())
            .map_err(proto)
    }

    /// Encrypt an application message, returning the serialized MLS message.
    pub fn create_application_message(
        &mut self,
        device: &MlsDevice,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, MlsError> {
        let out = self
            .group
            .create_message(device.provider(), device.signer(), plaintext)
            .map_err(proto)?;
        out.tls_serialize_detached().map_err(proto)
    }

    /// Process a serialized inbound MLS message (application message or commit).
    pub fn process_incoming(
        &mut self,
        device: &MlsDevice,
        bytes: &[u8],
    ) -> Result<Incoming, MlsError> {
        let msg_in = MlsMessageIn::tls_deserialize(&mut &bytes[..]).map_err(proto)?;
        let protocol = msg_in
            .try_into_protocol_message()
            .map_err(|_| MlsError::WrongMessageType)?;
        let processed = self
            .group
            .process_message(device.provider(), protocol)
            .map_err(proto)?;
        // M-1's other half: refuse an EXTERNAL commit outright.
        //
        // An external (resync) commit can remove a member and seat its sender at the vacated leaf
        // through the commit's own path leaf, with **no Add proposal at all**. Both the
        // credential-binding loop and M-1 below walk `add_proposals()`, so neither would see it: an
        // external commit is a way to produce exactly the remove-and-re-add shape M-1 exists to
        // forbid, while stepping around the check. MLS also exempts external senders from the
        // wire-format rule, so the ciphertext-only policy does not block it either.
        //
        // Refusing is safe because this product has no external-join flow: every member arrives by
        // Welcome after an Add. If one is ever introduced, the joiner's path leaf has to be bound and
        // checked the way an Add's credential already is, and M-1 restated over it.
        if matches!(processed.sender(), Sender::NewMemberCommit) {
            return Err(InviteError::CredentialMismatch.into());
        }
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(app) => {
                Ok(Incoming::Application(app.into_bytes()))
            }
            ProcessedMessageContent::StagedCommitMessage(staged) => {
                // Defense in depth: every member independently validates that any
                // Add in this commit carries a credential bound to THIS group and
                // content-addressing its own leaf key; so a malicious committer
                // cannot inject an unbound or cross-group device. (Single-use nonce
                // enforcement stays with the admitting committer's ledger; this is
                // the binding check every applier can make without the invite token.)
                let group_id = self.group_id();
                for add in staged.add_proposals() {
                    let key_package = add.add_proposal().key_package();
                    let membership = membership_from_key_package(key_package)?;
                    let leaf_pk = key_package.leaf_node().signature_key().as_slice();
                    if membership.group_id != group_id
                        || DeviceId::from_public_key_bytes(leaf_pk) != membership.device_id
                    {
                        return Err(InviteError::CredentialMismatch.into());
                    }
                }
                // M-1. A single commit must not both remove the PRE-COMMIT designated committer and
                // add the same `DeviceId`.
                //
                // This is the one commit shape that leaves members provably unable to agree. A
                // remove-and-re-add in one commit keeps the device's `DeviceId`, so a witness sees
                // an ordinary same-owner step and preserves the old tenure start, while the rejoining
                // device knows its membership restarted and computes a new one. They then disagree
                // about who may issue a receipt, and the disagreement does not self-correct.
                //
                // Enforced on the RECEIVE side, on every staged commit, not in the committer's invite
                // ledger. The ledger is local to the admitting party: every other member can check
                // only that an Add's credential names this group and matches its leaf key, so a
                // malicious, modified or merely buggy committer could build this shape and honest
                // witnesses would merge it. Refusing before the merge means no member ever reaches
                // the ambiguous position.
                //
                // Stated over `DeviceId` rather than leaf index, so it does not depend on whether
                // OpenMLS happens to recycle the same leaf. What it does NOT forbid: a device
                // rotating to a new identity (remove A, add A' with a different `DeviceId`), or a
                // genuine rejoin in a LATER commit. Only the ambiguous shape is excluded.
                //
                // **Reachable, and this rule is the ONLY thing that stops it.** This crate's own
                // builders each commit a single inline proposal, which was once mistaken for proof
                // the shape could not arrive. It can: an MLS commit carries its proposals by value,
                // so any existing member can send one commit with an inline Remove of the designated
                // committer and an inline Add of the same `DeviceId`. That sender is not the removed
                // member, so it is not the self-removal case, and it is an ordinary member commit,
                // so the external-commit refusal above does not apply either.
                //
                // `m1_tests::a_witness_refuses_one_commit_that_removes_the_committer_and_re_adds_its_device_id`
                // builds exactly that commit with OpenMLS's own commit builder and feeds it to a
                // witness. With this rule deleted the witness MERGES it (`CommitApplied`): OpenMLS
                // performs no validation that rejects a re-add of a just-removed signature key, and
                // the credential-binding loop above passes because the Add's key package is
                // correctly bound. Nothing upstream makes this rule redundant.
                if let Some(committer) = self.designated_committer() {
                    let removes_committer = staged.remove_proposals().any(|remove| {
                        self.group
                            .members()
                            .find(|m| m.index == remove.remove_proposal().removed())
                            .is_some_and(|m| {
                                DeviceId::from_public_key_bytes(&m.signature_key) == committer
                            })
                    });
                    if removes_committer {
                        let re_adds_committer = staged.add_proposals().any(|add| {
                            let leaf_pk = add
                                .add_proposal()
                                .key_package()
                                .leaf_node()
                                .signature_key()
                                .as_slice();
                            DeviceId::from_public_key_bytes(leaf_pk) == committer
                        });
                        if re_adds_committer {
                            return Err(InviteError::CommitterReAdded.into());
                        }
                    }
                }
                // Inspect the staged commit for Remove proposals *before* the merge
                // consumes it; every member uses this to rotate `ns_secret_L`.
                let removed = staged.remove_proposals().next().is_some();
                self.group
                    .merge_staged_commit(device.provider(), *staged)
                    .map_err(proto)?;
                Ok(Incoming::CommitApplied { removed })
            }
            _ => Ok(Incoming::Other),
        }
    }

    /// Export `length` bytes of secret keyed to this group's current epoch.
    pub(crate) fn export_secret(
        &self,
        device: &MlsDevice,
        label: &str,
        context: &[u8],
        length: usize,
    ) -> Result<Vec<u8>, MlsError> {
        self.group
            .export_secret(device.provider().crypto(), label, context, length)
            .map_err(proto)
    }

    /// The current epoch number.
    pub fn epoch(&self) -> u64 {
        self.group.epoch().as_u64()
    }

    /// This group's id (openmls' random id, 16 bytes, chosen at creation).
    pub fn group_id(&self) -> Vec<u8> {
        self.group.group_id().as_slice().to_vec()
    }

    /// The number of current members.
    pub fn member_count(&self) -> usize {
        self.group.members().count()
    }

    /// The device ids of all current members.
    pub fn member_device_ids(&self) -> Vec<DeviceId> {
        self.group
            .members()
            .map(|m| DeviceId::from_public_key_bytes(&m.signature_key))
            .collect()
    }

    /// Whether `id` is a current member.
    pub fn contains_device(&self, id: &DeviceId) -> bool {
        self.member_device_ids().contains(id)
    }

    /// Whether this local MLS instance still belongs to the group after applied removals.
    pub fn is_active(&self) -> bool {
        self.group.is_active()
    }
}

impl fmt::Debug for ServerGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerGroup")
            .field("epoch", &self.epoch())
            .field("members", &self.member_count())
            .finish()
    }
}

#[cfg(test)]
mod m1_tests {
    use super::*;

    /// The leaf identity excludes the HPKE encryption key, so a key rotation is not a discontinuity.
    ///
    /// Without that exclusion every self-update would look like a new tenure and destroy the very
    /// knowledge the value exists to preserve. Asserted on the digest's inputs rather than by
    /// performing an update, because no self-update path is exposed here: the digest must be stable
    /// across two groups whose committer has the same index, signature key and credential.
    #[test]
    fn the_committer_leaf_digest_is_stable_for_one_identity() {
        let alice = MlsDevice::generate().unwrap();
        let one = ServerGroup::create(&alice).unwrap();
        let two = ServerGroup::create(&alice).unwrap();
        assert_eq!(
            one.designated_committer_leaf(),
            two.designated_committer_leaf(),
            "the same identity at the same index must hash the same, whatever else differs"
        );

        let bob = MlsDevice::generate().unwrap();
        let other = ServerGroup::create(&bob).unwrap();
        assert_ne!(
            one.designated_committer_leaf(),
            other.designated_committer_leaf(),
            "a different identity must not"
        );
    }

    /// M-1 on the RECEIVE path, against the shape a hostile or modified member can actually send.
    ///
    /// This crate's own builders each commit one inline proposal, so none of them can produce a
    /// remove-and-re-add, and that was once mistaken for evidence the rule was unreachable. An MLS
    /// commit carries its proposals by value, though, so any existing member can send ONE commit with
    /// an inline Remove of the designated committer and an inline Add of the same `DeviceId`. Built
    /// here with OpenMLS's own commit builder, by Bob, who is neither the committer nor the member
    /// being removed - so it is not the self-removal case - and it is an ordinary member commit, so
    /// the external-commit refusal does not apply. Carol is the witness.
    ///
    /// Every Add uses an invite-bound key package, so the credential-binding loop that runs before
    /// M-1 passes. The refusal can therefore only be M-1's, and Carol's group must be left exactly
    /// where it was.
    #[test]
    fn a_witness_refuses_one_commit_that_removes_the_committer_and_re_adds_its_device_id() {
        let alice = MlsDevice::generate().unwrap();
        let bob = MlsDevice::generate().unwrap();
        let carol = MlsDevice::generate().unwrap();
        let mut alice_group = ServerGroup::create(&alice).unwrap();
        let group_id = alice_group.group_id();

        let bob_added = alice_group
            .add_member(
                &alice,
                bob.key_package_for_invite(&group_id, [1; 16]).unwrap(),
            )
            .unwrap();
        let mut bob_group = ServerGroup::join(&bob, &bob_added.welcome).unwrap();
        let carol_added = alice_group
            .add_member(
                &alice,
                carol.key_package_for_invite(&group_id, [2; 16]).unwrap(),
            )
            .unwrap();
        bob_group
            .process_incoming(&bob, &carol_added.commit)
            .expect("Bob must follow Carol's admission");
        let mut carol_group = ServerGroup::join(&carol, &carol_added.welcome).unwrap();

        assert_eq!(carol_group.designated_committer(), Some(alice.device_id()));
        let alice_leaf = carol_group.member_leaf_index(&alice.device_id()).unwrap();
        let epoch = carol_group.epoch();
        let members = carol_group.member_device_ids();

        // Bob's hostile commit: remove Alice and re-add Alice's own identity, inline, in one commit.
        let returning = alice.key_package_for_invite(&group_id, [3; 16]).unwrap();
        let bundle = bob_group
            .group
            .commit_builder()
            .propose_removals([LeafNodeIndex::new(alice_leaf)])
            .propose_adds([returning])
            .load_psks(bob.provider().storage())
            .unwrap()
            .build(
                bob.provider().rand(),
                bob.provider().crypto(),
                bob.signer(),
                |_| true,
            )
            .expect("OpenMLS must let Bob build this commit, or the test is not reaching a witness")
            .stage_commit(bob.provider())
            .unwrap();
        let commit = bundle.into_commit().tls_serialize_detached().unwrap();

        // M-1's own error, distinct from the credential-binding refusal that runs just before it, so
        // this assertion alone shows which rule refused.
        let refused = carol_group.process_incoming(&carol, &commit);
        assert!(
            matches!(
                refused,
                Err(MlsError::Invite(InviteError::CommitterReAdded))
            ),
            "a witness must refuse a commit that removes the committer and re-adds its DeviceId; \
             got {refused:?}"
        );
        assert_eq!(carol_group.epoch(), epoch, "the witness must not advance");
        assert_eq!(
            carol_group.member_device_ids(),
            members,
            "the witness's roster must be unchanged"
        );
        assert_eq!(carol_group.designated_committer(), Some(alice.device_id()));
    }
}
