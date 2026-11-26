//! Exact signed originals carried by an approved, connected archive holder.
//! The current exporter protects the reply; it cannot authorize its author.

use super::{data, ActionId, Decoder, Encoder, OwnerFactId, SyncError, Zeroizing};
use crate::owner_v2::member_envelope::{
    checked_action, OpenedV2MemberEnvelopeData, V2MemberEnvelopeScopeData,
};
use catcoms_replication::chat::MAX_ACTION_BYTES;
use catcoms_storage::pad::{self, OP_PAD_CEILING, OP_PAD_FLOOR, PAD_FOOTER_BYTES};

const MAX_PLAIN: usize = 2 * 4 + 32 + MAX_ACTION_BYTES;
pub(super) const MAX_PADDED_BYTES: usize = MAX_PLAIN.next_power_of_two() + PAD_FOOTER_BYTES;
const _: () = assert!(MAX_PLAIN < OP_PAD_CEILING);

pub(super) fn encode_original(
    scope: &V2MemberEnvelopeScopeData,
    action: ActionId,
    admission: OwnerFactId,
    original: &[u8],
) -> Result<Zeroizing<Vec<u8>>, SyncError> {
    if checked_action(original, scope, admission)?.id() != action {
        return Err(SyncError::Malformed);
    }
    let mut body = Encoder::with_capacity(MAX_PLAIN - MAX_ACTION_BYTES + original.len());
    data(body.put_bytes(admission.as_bytes()))?;
    data(body.put_bytes(original))?;
    let body = Zeroizing::new(body.finish());
    Ok(Zeroizing::new(pad::pad(
        &body,
        OP_PAD_FLOOR,
        OP_PAD_CEILING,
    )?))
}

pub(super) fn open_original(
    scope: &V2MemberEnvelopeScopeData,
    padded: &[u8],
) -> Result<OpenedV2MemberEnvelopeData, SyncError> {
    if padded.len() > MAX_PADDED_BYTES {
        return Err(SyncError::Malformed);
    }
    let plain = pad::unpad(padded, OP_PAD_FLOOR, OP_PAD_CEILING)?;
    if plain.len() > MAX_PLAIN {
        return Err(SyncError::Malformed);
    }
    let mut body = Decoder::new(plain);
    let admission = OwnerFactId::from_bytes(
        data(body.get_bytes())?
            .try_into()
            .map_err(|_| SyncError::Malformed)?,
    );
    let original = data(body.get_bytes())?;
    data(body.finish())?;
    OpenedV2MemberEnvelopeData::from_authenticated_original(scope, admission, original)
}
