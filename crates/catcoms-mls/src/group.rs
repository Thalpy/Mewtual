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
}

impl fmt::Debug for ServerGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerGroup")
            .field("epoch", &self.epoch())
            .field("members", &self.member_count())
            .finish()
    }
}
