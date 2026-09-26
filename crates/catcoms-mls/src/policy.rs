//! Immutable group communication policy, authenticated by the MLS designated committer.
//!
//! An envelope proves who endorsed a policy. First adoption requires the current MLS owner;
//! a sealed local pin subsequently survives owner changes. Renewing the endorsement never
//! changes the policy digest or the dedicated service identity.

use catcoms_crypto::{verify_with_public_bytes, DeviceId};
use catcoms_wire::{Decoder, Encoder};
use thiserror::Error;

use crate::{MlsDevice, MlsError, ServerGroup};

const POLICY_DOMAIN: &str = "catcoms/group-policy/v1";
const BODY_DOMAIN: &str = "catcoms/group-policy/body/v1";
pub const MAX_GROUP_POLICY_BYTES: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupMode {
    /// No authenticated policy was present in the legacy invite or sealed snapshot.
    #[default]
    LegacyUnverified,
    PeerToPeer,
    Dedicated,
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("unsupported or malformed group policy")]
    Malformed,
    #[error("group policy requires the current MLS designated committer")]
    Unauthorized,
    #[error("group policy signature or group binding is invalid")]
    Invalid,
    #[error("an authenticated group policy cannot be replaced")]
    Conflict,
    #[error(
        "policy-bound admission is unavailable after an owner transfer into a higher MLS leaf"
    )]
    AdmissionAuthorityUnavailable,
    #[error("group policy must be saved before it can be activated")]
    PendingPersistence,
    #[error(transparent)]
    Mls(#[from] MlsError),
}

/// A signed policy envelope. Fields are private so callers cannot accidentally edit a pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupPolicy {
    group_id: Vec<u8>,
    mode: GroupMode,
    dedicated_service: Option<DeviceId>,
    issued_epoch: u64,
    issuer_public_key: Vec<u8>,
    signature: [u8; 64],
}

impl GroupPolicy {
    pub fn issue(
        group: &ServerGroup,
        owner: &MlsDevice,
        mode: GroupMode,
    ) -> Result<Self, PolicyError> {
        if !group.is_designated_committer(owner) {
            return Err(PolicyError::Unauthorized);
        }
        if mode == GroupMode::LegacyUnverified {
            return Err(PolicyError::Malformed);
        }
        let mut policy = Self {
            group_id: group.group_id(),
            mode,
            dedicated_service: (mode == GroupMode::Dedicated).then_some(owner.device_id()),
            issued_epoch: group.epoch(),
            issuer_public_key: owner.public_key_bytes(),
            signature: [0; 64],
        };
        policy.signature = owner.sign_raw(&policy.signing_payload())?;
        Ok(policy)
    }

    /// Endorse an already pinned body under today's owner. This permits P2P joins after owner
    /// transfer without treating a former owner's signature as proof of current governance.
    pub fn endorse(&self, group: &ServerGroup, owner: &MlsDevice) -> Result<Self, PolicyError> {
        self.verify_pin(group)?;
        if !group.is_designated_committer(owner) {
            return Err(PolicyError::Unauthorized);
        }
        let mut endorsed = self.clone();
        endorsed.issued_epoch = group.epoch();
        endorsed.issuer_public_key = owner.public_key_bytes();
        endorsed.signature = owner.sign_raw(&endorsed.signing_payload())?;
        Ok(endorsed)
    }

    pub fn mode(&self) -> GroupMode {
        self.mode
    }
    pub fn version(&self) -> u8 {
        1
    }
    pub fn group_id(&self) -> &[u8] {
        &self.group_id
    }
    pub fn dedicated_service(&self) -> Option<DeviceId> {
        self.dedicated_service
    }
    pub fn issuer(&self) -> DeviceId {
        DeviceId::from_public_key_bytes(&self.issuer_public_key)
    }
    /// Envelope freshness only; this is not an independently observed owner-tenure proof.
    pub fn issued_epoch(&self) -> u64 {
        self.issued_epoch
    }

    fn body(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_str(BODY_DOMAIN).expect("domain fits");
        e.put_bytes(&self.group_id).expect("group id fits");
        e.put_u8(match self.mode {
            GroupMode::PeerToPeer => 1,
            GroupMode::Dedicated => 2,
            GroupMode::LegacyUnverified => 0,
        });
        e.put_bytes(
            self.dedicated_service
                .as_ref()
                .map_or(&[][..], |id| id.as_bytes()),
        )
        .expect("service fits");
        e.finish()
    }

    /// Immutable policy identity; renewed owner endorsements have the same digest.
    pub fn digest(&self) -> [u8; 32] {
        *blake3::hash(&self.body()).as_bytes()
    }

    fn signing_payload(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_str(POLICY_DOMAIN).expect("domain fits");
        e.put_bytes(&self.body()).expect("policy fits");
        e.put_u64(self.issued_epoch);
        e.put_bytes(&self.issuer_public_key).expect("key fits");
        e.finish()
    }

    pub fn verify_self(&self) -> bool {
        self.group_id.len() <= 256
            && !self.group_id.is_empty()
            && self.issuer_public_key.len() == 32
            && matches!(
                (self.mode, self.dedicated_service),
                (GroupMode::PeerToPeer, None) | (GroupMode::Dedicated, Some(_))
            )
            && verify_with_public_bytes(
                &self.issuer_public_key,
                &self.signing_payload(),
                &self.signature,
            )
    }

    /// Check a previously accepted, vault-sealed pin. Do not require its original issuer to
    /// remain the owner: doing so would erase a mode boundary on owner transfer/removal.
    pub fn verify_pin(&self, group: &ServerGroup) -> Result<(), PolicyError> {
        if !self.verify_self()
            || self.group_id != group.group_id()
            || self.issued_epoch > group.epoch()
        {
            return Err(PolicyError::Invalid);
        }
        Ok(())
    }

    /// Untrusted first adoption and join require the actual current designated committer.
    pub fn verify_current_owner(&self, group: &ServerGroup) -> Result<(), PolicyError> {
        self.verify_pin(group)?;
        if group.designated_committer() != Some(self.issuer()) {
            return Err(PolicyError::Unauthorized);
        }
        Ok(())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.signing_payload()).expect("policy fits");
        e.put_bytes(&self.signature).expect("signature fits");
        e.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyError> {
        if bytes.len() > MAX_GROUP_POLICY_BYTES {
            return Err(PolicyError::Malformed);
        }
        let bad = |_| PolicyError::Malformed;
        let mut outer = Decoder::new(bytes);
        let signed = outer.get_bytes().map_err(bad)?;
        let signature = outer
            .get_bytes()
            .map_err(bad)?
            .try_into()
            .map_err(|_| PolicyError::Malformed)?;
        outer.finish().map_err(bad)?;
        let mut d = Decoder::new(signed);
        if d.get_str().map_err(bad)? != POLICY_DOMAIN {
            return Err(PolicyError::Malformed);
        }
        let body = d.get_bytes().map_err(bad)?;
        let issued_epoch = d.get_u64().map_err(bad)?;
        let issuer_public_key = d.get_bytes().map_err(bad)?.to_vec();
        d.finish().map_err(bad)?;
        let mut b = Decoder::new(body);
        if b.get_str().map_err(bad)? != BODY_DOMAIN {
            return Err(PolicyError::Malformed);
        }
        let group_id = b.get_bytes().map_err(bad)?.to_vec();
        let mode = match b.get_u8().map_err(bad)? {
            1 => GroupMode::PeerToPeer,
            2 => GroupMode::Dedicated,
            _ => return Err(PolicyError::Malformed),
        };
        let service = b.get_bytes().map_err(bad)?;
        let dedicated_service = if service.is_empty() {
            None
        } else {
            Some(DeviceId::from_bytes(
                service.try_into().map_err(|_| PolicyError::Malformed)?,
            ))
        };
        b.finish().map_err(bad)?;
        let policy = Self {
            group_id,
            mode,
            dedicated_service,
            issued_epoch,
            issuer_public_key,
            signature,
        };
        if !policy.verify_self() {
            return Err(PolicyError::Invalid);
        }
        Ok(policy)
    }
}
