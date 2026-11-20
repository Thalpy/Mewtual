//! Stable persistent labels, separate from live mount/read/SDK/writer/UI observations.
use super::common::*;
use crate::store::critical_namespaces::archive_bootstrap::founder_codec::{
    IntentId, TargetMarkerId,
};
use crate::store::critical_namespaces::archive_current_codec::V2GroupPinId;
use crate::store::critical_namespaces::archive_current_namespace_codec::CurrentLayoutData;
use crate::AppError;
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::owner_header::{OwnerBodyId, OwnerTenureId};
use catcoms_sync::owner_v2::BootstrapSourceId;
use catcoms_wire::Encoder;
use zeroize::Zeroizing;

const TAG: &[u8] = b"catcoms/private-chat-text-context/v1\0";
const MAX_GROUP: usize = 256;
/// Exact maximum canonical context: tag/version43, thirteen b32 fields, group260, revision/epoch16.
pub(in crate::store) const MAX_CONTEXT_PLAIN: usize = 787;
const _: () = assert!(4 + TAG.len() + 2 + 13 * 36 + 4 + MAX_GROUP + 16 == MAX_CONTEXT_PLAIN);

/// Labels only. Root must derive these from actual source/pin/scope originals before admission.
/// In particular layout.root is the serialized Archive2/3 root, never an OS directory ID.
#[derive(Clone, Copy)]
pub(in crate::store) struct TextContextSpecData<'a> {
    pub(in crate::store) layout: CurrentLayoutData,
    pub(in crate::store) marker: TargetMarkerId,
    pub(in crate::store) terminal: IntentId,
    pub(in crate::store) source: BootstrapSourceId,
    pub(in crate::store) source_context_digest: [u8; 32],
    pub(in crate::store) profile: OwnerFactId,
    pub(in crate::store) admission: OwnerBodyId,
    pub(in crate::store) signer: [u8; 32],
    pub(in crate::store) nonce: [u8; 32],
    pub(in crate::store) pin: V2GroupPinId,
    pub(in crate::store) tenure: OwnerTenureId,
    pub(in crate::store) genesis_parent: TextGenesisParentIdData,
    pub(in crate::store) group: &'a [u8],
}
impl std::fmt::Debug for TextContextSpecData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextContextSpecData")
            .finish_non_exhaustive()
    }
}

/// Owned bounded DATA; no resource, pointer, native ID, source owner, sender or key is retained.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct TextContextData {
    group: Vec<u8>,
    // The substituted group is always static empty; spec_data lends the actual bounded group.
    fixed: TextContextSpecData<'static>,
}
impl std::fmt::Debug for TextContextData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextContextData").finish_non_exhaustive()
    }
}
impl PartialEq for TextContextSpecData<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.layout == other.layout
            && self.marker == other.marker
            && self.terminal == other.terminal
            && self.source == other.source
            && self.source_context_digest == other.source_context_digest
            && self.profile == other.profile
            && self.admission == other.admission
            && self.signer == other.signer
            && self.nonce == other.nonce
            && self.pin == other.pin
            && self.tenure == other.tenure
            && self.genesis_parent == other.genesis_parent
            && self.group == other.group
    }
}
impl Eq for TextContextSpecData<'_> {}

impl TextContextData {
    pub(in crate::store) fn from_data(spec: TextContextSpecData<'_>) -> Result<Self, AppError> {
        if spec.group.is_empty() || spec.group.len() > MAX_GROUP {
            return Err(refused());
        }
        Ok(Self {
            group: spec.group.to_vec(),
            fixed: TextContextSpecData {
                layout: spec.layout,
                marker: spec.marker,
                terminal: spec.terminal,
                source: spec.source,
                source_context_digest: spec.source_context_digest,
                profile: spec.profile,
                admission: spec.admission,
                signer: spec.signer,
                nonce: spec.nonce,
                pin: spec.pin,
                tenure: spec.tenure,
                genesis_parent: spec.genesis_parent,
                group: &[],
            },
        })
    }

    pub(in crate::store) fn spec_data(&self) -> TextContextSpecData<'_> {
        TextContextSpecData {
            group: &self.group,
            ..self.fixed
        }
    }

    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let s = self.spec_data();
        let mut e = Encoder::with_capacity(MAX_CONTEXT_PLAIN);
        tag(&mut e, TAG)?;
        for value in [
            &s.layout.vault,
            &s.layout.root,
            s.marker.as_bytes(),
            s.terminal.as_bytes(),
            s.source.as_bytes(),
            &s.source_context_digest,
            s.profile.as_bytes(),
            s.admission.as_bytes(),
            &s.signer,
            &s.nonce,
            s.pin.as_bytes(),
            s.tenure.as_bytes(),
            s.genesis_parent.as_bytes(),
        ] {
            put(&mut e, value)?;
        }
        put(&mut e, s.group)?;
        e.put_u64(2).put_u64(0);
        finish(e, MAX_CONTEXT_PLAIN)
    }

    pub(in crate::store) fn decode_data(raw: &[u8]) -> Result<Self, AppError> {
        let mut d = input(raw, MAX_CONTEXT_PLAIN, TAG)?;
        let layout = CurrentLayoutData {
            vault: b32(&mut d)?,
            root: b32(&mut d)?,
        };
        let marker = TargetMarkerId::from_bytes(b32(&mut d)?);
        let terminal = IntentId::from_bytes(b32(&mut d)?);
        let source = BootstrapSourceId::from_bytes(b32(&mut d)?);
        let source_context_digest = b32(&mut d)?;
        let profile = OwnerFactId::from_bytes(b32(&mut d)?);
        let admission = OwnerBodyId::from_bytes(b32(&mut d)?);
        let signer = b32(&mut d)?;
        let nonce = b32(&mut d)?;
        let pin = V2GroupPinId::from_bytes(b32(&mut d)?);
        let tenure = OwnerTenureId::from_bytes(b32(&mut d)?);
        let genesis_parent = TextGenesisParentIdData::from_bytes(b32(&mut d)?);
        let group = field(&mut d, MAX_GROUP)?;
        if d.get_u64().map_err(|_| refused())? != 2 || d.get_u64().map_err(|_| refused())? != 0 {
            return Err(refused());
        }
        d.finish().map_err(|_| refused())?;
        // Complete borrowed framing/EOF and the profile constants precede the sole group copy.
        Self::from_data(TextContextSpecData {
            layout,
            marker,
            terminal,
            source,
            source_context_digest,
            profile,
            admission,
            signer,
            nonce,
            pin,
            tenure,
            genesis_parent,
            group,
        })
    }

    pub(in crate::store) fn id_data(&self) -> Result<TextContextIdData, AppError> {
        Ok(TextContextIdData::of_data(&self.encode_data()?))
    }
}

#[cfg(test)]
mod tests;
