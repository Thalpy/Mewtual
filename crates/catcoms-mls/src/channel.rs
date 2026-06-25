//! Per-document key derivation from the group's epoch exporter secret.
//!
//! Channels, the wiki, status feed and calendar are not separate MLS groups —
//! each derives its own key from the group exporter secret plus a canonical,
//! **injective** `(doc_type, doc_id)` context (see `catcoms-wire`). Because the
//! context encoding is collision-free, distinct documents get independent keys
//! "for free". Re-deriving every epoch means these keys inherit the group's
//! forward secrecy and post-compromise security.
//!
//! Content keys use [`CHANNEL_EXPORTER_LABEL`]; network-visible identifiers use a
//! distinct [`METADATA_EXPORTER_LABEL`], so learning a content key never reveals
//! metadata keys and vice versa.

use catcoms_wire::{
    exporter_context, DocType, CHANNEL_EXPORTER_LABEL, MEDIA_EXPORTER_LABEL,
    METADATA_EXPORTER_LABEL,
};

use crate::device::MlsDevice;
use crate::group::ServerGroup;
use crate::MlsError;

/// Length of a derived key.
pub const KEY_LEN: usize = 32;

impl ServerGroup {
    /// Derive the 32-byte content base secret for a document at the current
    /// epoch. Distinct `(doc_type, doc_id)` pairs yield independent secrets.
    pub fn channel_secret(
        &self,
        device: &MlsDevice,
        doc_type: DocType,
        doc_id: u128,
    ) -> Result<[u8; KEY_LEN], MlsError> {
        let context = exporter_context(doc_type, doc_id);
        let secret = self.export_secret(device, CHANNEL_EXPORTER_LABEL, &context, KEY_LEN)?;
        secret
            .try_into()
            .map_err(|_| MlsError::Internal("unexpected exported secret length"))
    }

    /// Derive the 32-byte **media** secret for a call at the current epoch — the base key for E2E
    /// real-time media (voice/video frames). All members derive it identically from the group
    /// exporter; distinct `call_id`s yield independent keys. Domain-separated from content + metadata
    /// keys by [`MEDIA_EXPORTER_LABEL`]. The key is never sent on the wire.
    pub fn media_secret(
        &self,
        device: &MlsDevice,
        call_id: u128,
    ) -> Result<[u8; KEY_LEN], MlsError> {
        let context = call_id.to_be_bytes();
        let secret = self.export_secret(device, MEDIA_EXPORTER_LABEL, &context, KEY_LEN)?;
        secret
            .try_into()
            .map_err(|_| MlsError::Internal("unexpected exported secret length"))
    }

    /// Derive the 32-byte network-metadata secret for a document at the current
    /// epoch (used later for blinded gossipsub topics / rendezvous namespaces).
    /// Domain-separated from [`ServerGroup::channel_secret`] by a distinct label.
    pub fn metadata_secret(
        &self,
        device: &MlsDevice,
        doc_type: DocType,
        doc_id: u128,
    ) -> Result<[u8; KEY_LEN], MlsError> {
        let context = exporter_context(doc_type, doc_id);
        let secret = self.export_secret(device, METADATA_EXPORTER_LABEL, &context, KEY_LEN)?;
        secret
            .try_into()
            .map_err(|_| MlsError::Internal("unexpected exported secret length"))
    }

    /// Derive the 32-byte **routing** secret for the current epoch — the single
    /// secret from which the per-removal routing label (`ns_secret_L`: blinded
    /// gossipsub topics + rendezvous namespaces) is keyed. This is just the
    /// metadata secret under the dedicated [`DocType::Routing`] context, so it is
    /// domain-separated from every per-document metadata secret.
    ///
    /// openmls only exports the *current* epoch's secret; the routing layer
    /// snapshots this value at each member **removal** to build the rotation
    /// history (a removed member cannot export the post-removal epoch's secret).
    pub fn routing_metadata_secret(&self, device: &MlsDevice) -> Result<[u8; KEY_LEN], MlsError> {
        self.metadata_secret(device, DocType::Routing, 0)
    }

    /// Derive the 32-byte **routing-transfer wrap key** for the current epoch — the
    /// symmetric key under which a member seals the routing state (`L` + the
    /// `ns_secret_L` history) for a peer joining at this same epoch. Both the
    /// admitting member (after merging the Add) and the joiner (after processing the
    /// Welcome) are at the same epoch and so derive the identical key; an outsider
    /// (relay/observer), lacking the group secrets, cannot. Domain-separated from
    /// [`ServerGroup::routing_metadata_secret`] by a distinct context id, so it is
    /// never equal to any `ns_secret_L`.
    pub fn routing_transfer_key(&self, device: &MlsDevice) -> Result<[u8; KEY_LEN], MlsError> {
        self.metadata_secret(device, DocType::Routing, 1)
    }
}
