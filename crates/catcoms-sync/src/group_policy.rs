//! Authenticated communication policy. This pin is independent of document contents and roles.

use super::*;

const PIN_VERSION: u8 = 1;

pub(super) fn encode_pin(policy: Option<&GroupPolicy>) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u8(PIN_VERSION);
    e.put_bytes(&policy.map(GroupPolicy::encode).unwrap_or_default())
        .expect("bounded group policy");
    e.finish()
}

pub(super) fn decode_pin(bytes: &[u8]) -> Result<Option<GroupPolicy>, SyncError> {
    let mut d = Decoder::new(bytes);
    if d.get_u8().map_err(|_| SyncError::Malformed)? != PIN_VERSION {
        return Err(PolicyError::Malformed.into());
    }
    let encoded = d.get_bytes().map_err(|_| SyncError::Malformed)?;
    let policy = if encoded.is_empty() {
        None
    } else {
        Some(GroupPolicy::decode(encoded)?)
    };
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(policy)
}

/// Dedicated has a reserved authenticated wire value, but is not usable until its serving and
/// discovery restrictions exist. Recognizing a mode must never silently enable ordinary P2P.
pub(super) fn require_supported(policy: &GroupPolicy) -> Result<(), PolicyError> {
    if policy.mode() != GroupMode::PeerToPeer {
        return Err(PolicyError::Malformed);
    }
    Ok(())
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub fn group_mode(&self) -> GroupMode {
        if !self.group_policy_active {
            return GroupMode::LegacyUnverified;
        }
        self.group_policy
            .as_ref()
            .map_or(GroupMode::LegacyUnverified, GroupPolicy::mode)
    }

    pub fn group_policy(&self) -> Option<&GroupPolicy> {
        self.group_policy.as_ref()
    }

    pub fn group_policy_digest(&self) -> Option<[u8; 32]> {
        self.group_policy.as_ref().map(GroupPolicy::digest)
    }

    /// O(1) persistence invalidation, including an unchanged body re-endorsed by a new owner.
    pub fn group_policy_revision(&self) -> u64 {
        self.group_policy_revision
    }

    /// New member-mesh permissions require explicit authenticated P2P policy. Legacy behavior
    /// is not evidence of permission for new durable route or serving capabilities.
    pub fn policy_allows_member_mesh(&self) -> bool {
        self.group_mode() == GroupMode::PeerToPeer
    }

    pub fn policy_allows_service(&self) -> bool {
        self.policy_allows_member_mesh()
    }

    pub fn dedicated_service(&self) -> Option<DeviceId> {
        self.group_policy
            .as_ref()
            .and_then(GroupPolicy::dedicated_service)
    }

    /// Explicit creation/migration by the actual MLS owner. A pin can never change mode. This
    /// does not publish: native callers must save their sealed snapshot before advertising the
    /// decision with `publish_group_policy`, so a crash cannot forget an announced owner pin.
    pub fn initialize_group_policy(&mut self, mode: GroupMode) -> Result<(), SyncError> {
        if mode != GroupMode::PeerToPeer {
            return Err(PolicyError::Malformed.into());
        }
        if !self.is_designated_committer() {
            return Err(PolicyError::Unauthorized.into());
        }
        if let Some(policy) = &self.group_policy {
            return if policy.mode() == mode {
                Ok(())
            } else {
                Err(PolicyError::Conflict.into())
            };
        }
        self.group_policy = Some(GroupPolicy::issue(&self.group, &self.device, mode)?);
        self.group_policy_revision = self.group_policy_revision.wrapping_add(1);
        Ok(())
    }

    /// Fresh construction only. Native must save before granting continuing route authority;
    /// legacy migration uses initialize -> persist -> publish and stays inactive meanwhile.
    pub fn initialize_new_group_policy(&mut self) -> Result<(), SyncError> {
        if self.group.epoch() != 0 || self.group.member_count() != 1 {
            return Err(PolicyError::Unauthorized.into());
        }
        self.initialize_group_policy(GroupMode::PeerToPeer)?;
        self.group_policy_active = true;
        Ok(())
    }

    /// Current-owner endorsement for admission. The digest does not change when ownership does.
    pub(super) fn admission_policy(&self) -> Result<Option<GroupPolicy>, SyncError> {
        let Some(policy) = &self.group_policy else {
            return Ok(None);
        };
        require_supported(policy)?;
        if self.is_designated_committer() {
            Ok(Some(policy.endorse(&self.group, &self.device)?))
        } else {
            policy.verify_current_owner(&self.group)?;
            Ok(Some(policy.clone()))
        }
    }

    pub(super) fn invite_policy_matches(&self, invite: &InviteToken) -> bool {
        match (&self.group_policy, &invite.policy) {
            (None, None) => true,
            (Some(pin), Some(offered)) => {
                require_supported(offered).is_ok()
                    && offered.verify_pin(&self.group).is_ok()
                    && pin.digest() == offered.digest()
            }
            _ => false,
        }
    }

    /// An Add into a vacated lower leaf makes the joining device the new MLS owner. The old
    /// owner cannot attest policy under that future authority. Until an authenticated succession
    /// proof is carried by the protocol, refuse before committing/consuming the invitation.
    pub(super) fn policy_admission_ready(&self) -> Result<(), SyncError> {
        if self.group_policy.is_some() && !self.group_policy_active {
            return Err(PolicyError::PendingPersistence.into());
        }
        if self.group_policy.is_some() && self.group.designated_committer_index() != Some(0) {
            return Err(PolicyError::AdmissionAuthorityUnavailable.into());
        }
        Ok(())
    }

    /// Advertise an already persisted owner decision. Publication is queued through the normal
    /// bounded control path; receiving members independently check owner authority and pin equality.
    pub fn publish_group_policy(&mut self) -> Result<(), SyncError> {
        if !self.is_designated_committer() {
            return Err(PolicyError::Unauthorized.into());
        }
        let policy = self.admission_policy()?.ok_or(PolicyError::Invalid)?;
        let mut frame = vec![CTRL_GROUP_POLICY];
        frame.extend_from_slice(&policy.encode());
        // The outbox also carries document operations, whose first byte is not a control tag.
        // Remove only our previous control-topic frame, including after that topic rotated out
        // of the accepted window. One remembered topic suffices because we enqueue at most one.
        let previous_topic = self.group_policy_last_topic.as_ref();
        self.outbox.retain(|(topic, bytes)| {
            !(Some(topic) == previous_topic && bytes.first() == Some(&CTRL_GROUP_POLICY))
        });
        // A policy retry must not evict locally accepted chat or bypass the shared queue bound.
        // The armed discovery retry will submit it after the queue has room again.
        if self.outbox.len() < self.config.max_outbox {
            self.outbox.push((self.control_topic.clone(), frame));
            self.group_policy_last_topic = Some(self.control_topic.clone());
        }
        self.group_policy_publish_ready = true;
        self.group_policy_active = true;
        Ok(())
    }

    /// Native's bounded periodic discovery pass retries only an already durable owner pin.
    pub fn republish_group_policy_if_ready(&mut self) {
        if self.group_policy_publish_ready && self.is_designated_committer() {
            if let Err(error) = self.publish_group_policy() {
                tracing::debug!(%error, "group policy refresh unavailable");
            }
        }
    }

    pub(super) fn on_group_policy(&mut self, bytes: &[u8]) {
        let accepted = (|| -> Result<GroupPolicy, SyncError> {
            let policy = GroupPolicy::decode(bytes)?;
            require_supported(&policy)?;
            policy.verify_current_owner(&self.group)?;
            if self
                .group_policy
                .as_ref()
                .is_some_and(|pin| pin.digest() != policy.digest())
            {
                return Err(PolicyError::Conflict.into());
            }
            if self
                .group_policy
                .as_ref()
                .is_some_and(|pin| pin.issued_epoch() > policy.issued_epoch())
            {
                return Err(PolicyError::Invalid.into());
            }
            Ok(policy)
        })();
        match accepted {
            Ok(policy) => {
                let pending_local = self.group_policy.is_some()
                    && !self.group_policy_active
                    && self.is_designated_committer();
                if self.group_policy.as_ref() != Some(&policy) {
                    self.group_policy_revision = self.group_policy_revision.wrapping_add(1);
                    self.group_policy = Some(policy);
                }
                if !pending_local {
                    self.group_policy_active = true;
                }
            }
            Err(error) => tracing::debug!(%error, "rejected group policy control message"),
        }
    }
}

#[cfg(test)]
mod tests;
