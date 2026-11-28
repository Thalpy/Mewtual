//! Namespace6's joined anchor record. Authenticating these bytes is separate
//! from selecting the original pin file or accepting a remote genesis.
use super::{
    archive_current_codec::V2GroupPinId, archive_current_namespace_codec::CurrentLayoutData,
    refused,
};
use crate::store::{frame, unframe};
use crate::AppError;
use catcoms_crypto::{seal, unseal};
use catcoms_replication::chat::join_bindings::{
    GenesisAnchorBindingData, SignedInviteBindingData, MAX_JOIN_BINDING_BYTES,
};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

/// Negative profile check for optional original-founder identity memory.
/// ID9 declaration itself ends this A1-only interpretation, even before
/// an A2 has been selected. No parsed flag supplies route authority.
pub(in crate::store) fn require_a1_founder_route_profile(
    store: &crate::store::ServerStore,
) -> Result<(), AppError> {
    let declared = {
        let catalog = store
            .pending_join_catalog
            .try_lock()
            .map_err(|_| refused())?;
        catalog
            .as_ref()
            .ok_or_else(refused)?
            .current
            .rejoin_payload_digest
            .is_some()
    };
    if declared || store.has_rejoin_evidence()? {
        return Err(refused());
    }
    Ok(())
}

pub(in crate::store) mod declaration;
pub(in crate::store) mod index;
pub(super) mod namespace;
pub(in crate::store) mod pending;
pub(in crate::store) mod welcome;

const TAG: &[u8] = b"catcoms/private-chat-v2-joined-anchor-pin/v1\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-v2-joined-anchor-pin-key/v1";
const VERSION: u16 = 1;
const OVERHEAD: usize = 4 + TAG.len() + 2 + 3 * (4 + 32) + 4 + 40;
const MAX_BYTES: usize = OVERHEAD + MAX_JOIN_BINDING_BYTES;

pub(super) struct JoinedAnchorPin {
    accepted_invite: SignedInviteBindingData,
    original_pending: [u8; 32],
    layout: CurrentLayoutData,
    id: V2GroupPinId,
}

impl JoinedAnchorPin {
    pub(super) fn anchor_data(&self) -> &GenesisAnchorBindingData {
        &self.accepted_invite.spec_data().anchor
    }

    pub(super) fn pending_context_data(&self) -> catcoms_sync::V2PendingJoinContextData {
        catcoms_sync::V2PendingJoinContextData {
            vault: self.layout.vault,
            root: self.layout.root,
            original_pending: self.original_pending,
            pin: *self.id.as_bytes(),
        }
    }

    #[cfg(test)]
    pub(super) fn open_pending_source(
        &self,
        key: &[u8; 32],
        sealed: &catcoms_sync::SealedV2PendingJoinSource,
        limits: catcoms_sync::DirectJoinLimits,
    ) -> Result<catcoms_sync::RecoveredV2PendingJoinSource, AppError> {
        catcoms_sync::RecoveredV2PendingJoinSource::open(
            sealed,
            key,
            self.pending_context_data(),
            &self.accepted_invite,
            limits,
        )
        .map_err(|_| refused())
    }
}

fn require_accepted(
    invite: &SignedInviteBindingData,
    original: &GenesisAnchorBindingData,
) -> Result<(), AppError> {
    let spec = invite.spec_data();
    if spec.anchor != *original || spec.inviter != original.founder || !invite.verify() {
        return Err(refused());
    }
    Ok(())
}

pub(super) fn seal_initial(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    original_pending: [u8; 32],
    accepted_anchor: &GenesisAnchorBindingData,
    invite: &SignedInviteBindingData,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    require_accepted(invite, accepted_anchor)?;
    if original_pending == [0; 32] {
        return Err(refused());
    }
    let invite = invite.encode();
    if invite.len() > MAX_JOIN_BINDING_BYTES {
        return Err(refused());
    }
    let mut e = Encoder::with_capacity(OVERHEAD - 40 + invite.len());
    let failed = |_| refused();
    e.put_bytes(TAG).map_err(failed)?;
    e.put_u16(VERSION);
    for value in [&layout.vault, &layout.root, &original_pending] {
        e.put_bytes(value).map_err(failed)?;
    }
    e.put_bytes(&invite).map_err(failed)?;
    let plain = Zeroizing::new(e.finish());
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    Ok(Zeroizing::new(frame(&seal(&key, &plain, rng)?)))
}

/// Rejoin checks the exact selected ciphertext ID and the pinned anchor.
/// The original acceptance and pending join provenance remain unchanged.
#[cfg(test)]
pub(super) fn open_original(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    expected: V2GroupPinId,
    accepted_anchor: &GenesisAnchorBindingData,
    raw: &[u8],
) -> Result<JoinedAnchorPin, AppError> {
    let pin = decode_pin(key, layout, expected, raw)?;
    require_accepted(&pin.accepted_invite, accepted_anchor)?;
    Ok(pin)
}

fn decode_pin(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    expected: V2GroupPinId,
    raw: &[u8],
) -> Result<JoinedAnchorPin, AppError> {
    if !(OVERHEAD + 1..=MAX_BYTES).contains(&raw.len()) || V2GroupPinId::of(raw) != expected {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
    let mut d = Decoder::new(&plain);
    let failed = |_| refused();
    if d.get_bytes().map_err(failed)? != TAG
        || d.get_u16().map_err(failed)? != VERSION
        || d.get_bytes().map_err(failed)? != layout.vault
        || d.get_bytes().map_err(failed)? != layout.root
    {
        return Err(refused());
    }
    let original_pending: [u8; 32] = d
        .get_bytes()
        .map_err(failed)?
        .try_into()
        .map_err(|_| refused())?;
    let invite = d.get_bytes().map_err(failed)?;
    d.finish().map_err(failed)?;
    if original_pending == [0; 32] {
        return Err(refused());
    }
    let invite = SignedInviteBindingData::decode(invite).map_err(|_| refused())?;
    require_accepted(&invite, &invite.spec_data().anchor)?;
    Ok(JoinedAnchorPin {
        accepted_invite: invite,
        original_pending,
        layout,
        id: expected,
    })
}

#[cfg(test)]
mod tests;
