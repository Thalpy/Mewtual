//! The first governed Add reservation. It deliberately uses the existing WAL
//! key and outer tag with a new logical version. Old scratch readers can
//! authenticate it and must preserve it. The V1 WAL decoder still refuses it.
use super::super::common::{b32, put, TextContextIdData, TextHeadIdData};
use super::*;
use catcoms_sync::V2JoinRequestPacketData;
use catcoms_wire::Encoder;

mod archive;
pub(in crate::store) use archive::recovery::{
    HeldColdFounderAdd, HeldColdFounderGenesis, HeldGovernedArchiveRead,
};
mod governing_record;
mod header_signing;
pub(in crate::store::persistence::v2_text) mod held;
mod outcome;
mod request;
mod response;
mod sdk_entry;
mod selected_head;
mod welcome_intent;

const ROLE: &[u8] = b"catcoms/private-chat-text-governed-add-fence/v2\0";
const REQUEST_DOMAIN: &str = "catcoms/private-chat-text-governed-add-request/v2";
const FENCE_VERSION: u16 = 2;
const WAL_TAG: &[u8] = TextControlKindData::Wal.domains().0;
const PLAIN_BYTES: usize = 4 + WAL_TAG.len() + 2 + 4 + ROLE.len() + 4 * (4 + 32);
pub(in crate::store::persistence::v2_text) const FRAME_BYTES: usize = PLAIN_BYTES + 40;

/// Stable reservation labels only. File custody, source validity and native
/// SDK admission must all be established by the owning storage transaction.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::persistence::v2_text) struct GovernedAddFenceData {
    context: TextContextIdData,
    parent: TextHeadIdData,
    invite: [u8; 32],
    request: [u8; 32],
}

impl GovernedAddFenceData {
    pub(in crate::store::persistence::v2_text) fn for_request(
        context: TextContextIdData,
        parent: TextHeadIdData,
        packet: &V2JoinRequestPacketData<'_>,
    ) -> Self {
        Self {
            context,
            parent,
            invite: packet.founder_add_data().invite.id_data(),
            request: blake3::derive_key(REQUEST_DOMAIN, packet.packet_bytes_data()),
        }
    }

    pub(in crate::store::persistence::v2_text) fn seal_data(
        self,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(PLAIN_BYTES);
        put(&mut e, WAL_TAG)?;
        e.put_u16(FENCE_VERSION);
        put(&mut e, ROLE)?;
        for value in [
            self.context.as_bytes(),
            self.parent.as_bytes(),
            &self.invite,
            &self.request,
        ] {
            put(&mut e, value)?;
        }
        let plain = Zeroizing::new(e.finish());
        let key = Self::wal_key(key);
        let raw = Zeroizing::new(frame(&seal(&key, &plain, rng)?));
        if raw.len() != FRAME_BYTES {
            return Err(refused());
        }
        Ok(raw)
    }

    pub(in crate::store::persistence::v2_text) fn open_data(
        key: &[u8; 32],
        raw: &[u8],
    ) -> Result<Self, AppError> {
        if raw.len() != FRAME_BYTES {
            return Err(refused());
        }
        let key = Self::wal_key(key);
        let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
        let mut d = Decoder::new(&plain);
        if field(&mut d, WAL_TAG.len())? != WAL_TAG
            || d.get_u16().map_err(|_| refused())? != FENCE_VERSION
            || field(&mut d, ROLE.len())? != ROLE
        {
            return Err(refused());
        }
        let result = Self {
            context: TextContextIdData::from_bytes(b32(&mut d)?),
            parent: TextHeadIdData::from_bytes(b32(&mut d)?),
            invite: b32(&mut d)?,
            request: b32(&mut d)?,
        };
        d.finish().map_err(|_| refused())?;
        Ok(result)
    }

    fn require_parent(
        self,
        context: TextContextIdData,
        parent: TextHeadIdData,
    ) -> Result<(), AppError> {
        if self.context != context || self.parent != parent {
            return Err(refused());
        }
        Ok(())
    }

    fn require_packet(self, packet: &V2JoinRequestPacketData<'_>) -> Result<(), AppError> {
        if self != Self::for_request(self.context, self.parent, packet) {
            return Err(refused());
        }
        Ok(())
    }

    fn wal_key(key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(blake3::derive_key(
            TextControlKindData::Wal.domains().1,
            key,
        ))
    }
}

#[cfg(test)]
mod tests;
