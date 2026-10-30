//! Narrow nonserialized observations of successful LOCAL MLS operations. A public owner
//! certificate may attest these facts under its independently pinned trust profile; this
//! token itself is neither portable historical proof nor permission to publish before save.
use super::*;

/// Which actual operation produced this observation. Unsupported staged/remote operations
/// do not acquire a local owner observation merely because today's roster looks plausible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalMembershipChangeKind {
    Add,
    Remove,
}

/// Opaque result of one successful Add/Remove. There is no Clone, decoder or public field
/// constructor. Restoring a group or comparing epochs cannot manufacture this observation.
pub struct LocalMembershipChange {
    kind: LocalMembershipChangeKind,
    group: Vec<u8>,
    author: [u8; 32],
    subject: [u8; 32],
    before_epoch: u64,
    after_epoch: u64,
    state: [u8; 32],
    commit: Vec<u8>,
    key_package: Option<[u8; 32]>,
    credential: Option<[u8; 32]>,
}
impl fmt::Debug for LocalMembershipChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalMembershipChange")
            .field("kind", &self.kind)
            .field("before_epoch", &self.before_epoch)
            .field("after_epoch", &self.after_epoch)
            .finish_non_exhaustive()
    }
}
impl LocalMembershipChange {
    pub fn kind(&self) -> LocalMembershipChangeKind {
        self.kind
    }
    pub fn group(&self) -> &[u8] {
        &self.group
    }
    pub fn author(&self) -> &[u8; 32] {
        &self.author
    }
    pub fn subject(&self) -> &[u8; 32] {
        &self.subject
    }
    pub fn before_epoch(&self) -> u64 {
        self.before_epoch
    }
    pub fn after_epoch(&self) -> u64 {
        self.after_epoch
    }
    /// Exact already-produced MLS bytes. The caller must retain them with its candidate
    /// publication obligation; obtaining this token is not a durable-export permit.
    pub fn commit(&self) -> &[u8] {
        &self.commit
    }
    pub fn commit_digest(&self) -> [u8; 32] {
        *blake3::hash(&self.commit).as_bytes()
    }
    pub fn key_package_reference(&self) -> Option<[u8; 32]> {
        self.key_package
    }
    pub fn credential_digest(&self) -> Option<[u8; 32]> {
        self.credential
    }

    /// Exact local binding check, not historical authority. A producer must additionally
    /// check its pinned profile, original admission and actual current owner/lease custody.
    pub fn matches_current(&self, group: &ServerGroup, device: &MlsDevice) -> bool {
        self.group == group.group_id()
            && group.is_active()
            && group.has_local_signer(&self.author)
            && self.author.as_slice() == device.public_key_bytes()
            && self.after_epoch == group.epoch()
            && self.state == group.epoch_authenticator_id()
            && self.before_epoch.checked_add(1) == Some(self.after_epoch)
            && match self.kind {
                LocalMembershipChangeKind::Add => {
                    group
                        .member_signature_key(&catcoms_crypto::DeviceId::from_public_key_bytes(
                            &self.subject,
                        ))
                        .as_deref()
                        == Some(self.subject.as_slice())
                }
                LocalMembershipChangeKind::Remove => !group.contains_device(
                    &catcoms_crypto::DeviceId::from_public_key_bytes(&self.subject),
                ),
            }
    }
}

impl ServerGroup {
    /// Actual Add plus an opaque exact-KP observation for the gated owner-fact producer.
    /// This does not replace invite/role policy checks: perform them before this mutation.
    /// Construct candidates on an isolated provider and save them before exporting Welcome.
    pub fn add_member_observed(
        &mut self,
        device: &MlsDevice,
        key_package: KeyPackage,
    ) -> Result<(AddOutcome, LocalMembershipChange), MlsError> {
        require_local_device(self, device)?;
        let subject = crate::key_package_signature_key(&key_package)
            .try_into()
            .map_err(|_| MlsError::Internal("non-Ed25519 member key"))?;
        let key_package_reference = device.key_package_reference(&key_package)?;
        let credential = key_package.leaf_node().credential();
        if credential.serialized_content().len() > MAX_MEMBER_CATALOG_CREDENTIAL_BYTES {
            return Err(MlsError::Internal(
                "owner observation credential exceeds bound",
            ));
        }
        // Bind the credential TYPE and TLS framing too. The prior raw-content bound makes
        // this allocation bounded before serialization, not just checked afterwards.
        let credential = credential.tls_serialize_detached().map_err(proto)?;
        if credential.len() > MAX_MEMBER_CATALOG_CREDENTIAL_BYTES + 8 {
            return Err(MlsError::Internal(
                "owner credential TLS frame exceeds bound",
            ));
        }
        let credential = credential_digest(&credential);
        let group = self.group_id();
        let author = local_key(device)?;
        let before_epoch = self.epoch();
        before_epoch
            .checked_add(1)
            .ok_or(MlsError::Internal("MLS epoch exhausted"))?;
        let outcome = self.add_member(device, key_package)?;
        let observation = LocalMembershipChange {
            kind: LocalMembershipChangeKind::Add,
            group,
            author,
            subject,
            before_epoch,
            after_epoch: self.epoch(),
            commit: outcome.commit.clone(),
            state: self.epoch_authenticator_id(),
            key_package: Some(key_package_reference),
            credential: Some(credential),
        };
        if !observation.matches_current(self, device) {
            return Err(MlsError::Internal(
                "Add observation did not match merged group",
            ));
        }
        Ok((outcome, observation))
    }

    /// Actual Remove plus the exact serialized Commit, which the legacy convenience Remove
    /// discards. No observation is available on failure; callers cannot reuse a prior token
    /// as a new operation or silently substitute a same-number/key re-admission.
    pub fn remove_member_observed(
        &mut self,
        device: &MlsDevice,
        target: &catcoms_crypto::DeviceId,
    ) -> Result<LocalMembershipChange, MlsError> {
        require_local_device(self, device)?;
        if target == &device.device_id() {
            return Err(MlsError::Internal(
                "observed local self-retirement is unsupported",
            ));
        }
        let member = self
            .group
            .members()
            .find(|member| {
                catcoms_crypto::DeviceId::from_public_key_bytes(&member.signature_key) == *target
            })
            .ok_or(MlsError::MemberNotFound)?;
        let subject = member
            .signature_key
            .as_slice()
            .try_into()
            .map_err(|_| MlsError::Internal("non-Ed25519 member key"))?;
        let index = member.index;
        let group = self.group_id();
        let author = local_key(device)?;
        let before_epoch = self.epoch();
        before_epoch
            .checked_add(1)
            .ok_or(MlsError::Internal("MLS epoch exhausted"))?;
        let (commit, _, _) = self
            .group
            .remove_members(device.provider(), device.signer(), &[index])
            .map_err(proto)?;
        // Serialize before merge. A failure leaves only an isolated candidate pending; no
        // caller may expose it or claim the transition was durably installed.
        let commit = commit.tls_serialize_detached().map_err(proto)?;
        self.group
            .merge_pending_commit(device.provider())
            .map_err(proto)?;
        let observation = LocalMembershipChange {
            kind: LocalMembershipChangeKind::Remove,
            group,
            author,
            subject,
            before_epoch,
            after_epoch: self.epoch(),
            commit,
            key_package: None,
            credential: None,
            state: self.epoch_authenticator_id(),
        };
        if !observation.matches_current(self, device) {
            return Err(MlsError::Internal(
                "Remove observation did not match merged group",
            ));
        }
        Ok(observation)
    }
}

fn local_key(device: &MlsDevice) -> Result<[u8; 32], MlsError> {
    device
        .public_key_bytes()
        .as_slice()
        .try_into()
        .map_err(|_| MlsError::Internal("non-Ed25519 local key"))
}
fn require_local_device(group: &ServerGroup, device: &MlsDevice) -> Result<(), MlsError> {
    if !group.is_active() || !group.has_local_signer(&device.public_key_bytes()) {
        return Err(MlsError::Internal(
            "owner observation needs active local device",
        ));
    }
    Ok(())
}
fn credential_digest(bytes: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"mewtual/chat/mls-credential/v1\0");
    h.update(bytes);
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests;
