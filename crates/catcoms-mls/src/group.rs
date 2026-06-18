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
use crate::{proto, MlsError};

/// The result of processing an inbound MLS message.
#[derive(Debug)]
pub enum Incoming {
    /// A decrypted application-message payload.
    Application(Vec<u8>),
    /// A commit was processed and merged (group state advanced).
    CommitApplied,
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

    /// Add `key_package`'s device and merge the commit. Returns the serialized
    /// Welcome to deliver to the joiner.
    pub fn add_member(
        &mut self,
        device: &MlsDevice,
        key_package: KeyPackage,
    ) -> Result<Vec<u8>, MlsError> {
        let (_commit, welcome, _group_info) = self
            .group
            .add_members(device.provider(), device.signer(), &[key_package])
            .map_err(proto)?;
        self.group
            .merge_pending_commit(device.provider())
            .map_err(proto)?;
        welcome.tls_serialize_detached().map_err(proto)
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
                self.group
                    .merge_staged_commit(device.provider(), *staged)
                    .map_err(proto)?;
                Ok(Incoming::CommitApplied)
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

    /// This group's id (random 32 bytes chosen at creation).
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
