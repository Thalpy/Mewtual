//! Exact encrypted receipt for a selected public Approve. Data in this
//! file needs both the physical receipt selection and current chat proofs.
use super::{header::ReceiverKey, *};
use crate::store::persistence::v2_text::private_index::{
    ApprovedReviewIndexValueData, ReceivedReviewIndexValueData,
};
use catcoms_replication::chat::{
    owner_bodies::{
        ReviewDecision, ReviewStreamEntry, ReviewStreamRecord, MAX_REVIEW_STREAM_RECORD_BYTES,
    },
    FounderGenesisAnchorData,
};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

const TAG: &[u8] = b"catcoms/private-chat-reviewed-original-receipt/v1\0";
pub(super) const MAX_APPROVAL_FRAME: usize =
    40 + 2 * 4 + TAG.len() + MAX_REVIEW_STREAM_RECORD_BYTES;

pub(super) fn cover(
    key: &ReceiverKey,
    pin: &FounderGenesisAnchorData,
    selected: &ReviewStreamEntry,
    raw: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let record = ReviewStreamRecord::decode_public_approval(pin, selected.id(), raw)
        .map_err(|_| denied())?;
    if record.entry_data() != selected {
        return Err(denied());
    }
    let mut frame = Encoder::with_capacity(2 * 4 + TAG.len() + raw.len());
    frame.put_bytes(TAG).map_err(|_| denied())?;
    frame.put_bytes(raw).map_err(|_| denied())?;
    key.seal(frame, rng)
}

pub(super) fn open(
    key: &ReceiverKey,
    pin: &FounderGenesisAnchorData,
    receipt: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let plain = key.open(receipt, MAX_APPROVAL_FRAME)?;
    let mut input = Decoder::new(&plain);
    if input.get_bytes().map_err(|_| denied())? != TAG {
        return Err(denied());
    }
    let raw = input.get_bytes().map_err(|_| denied())?;
    input.finish().map_err(|_| denied())?;
    let record = ReviewStreamRecord::decode(pin, raw).map_err(|_| denied())?;
    if record
        .entry_data()
        .case_step_data()
        .is_none_or(|step| step.choice_data() != ReviewDecision::Approve)
    {
        return Err(denied());
    }
    Ok(Zeroizing::new(raw.to_vec()))
}

pub(super) fn row(
    header: ReceivedReviewIndexValueData,
    record: &ReviewStreamRecord<'_>,
    encrypted: &[u8],
) -> Result<ApprovedReviewIndexValueData, AppError> {
    let entry = record.entry_data();
    let step = entry.case_step_data().ok_or_else(denied)?;
    if header.entry != *entry.id().as_bytes()
        || header.sequence != entry.position_data().sequence
        || header.case != step.case_data()
        || step.choice_data() != ReviewDecision::Approve
    {
        return Err(denied());
    }
    Ok(ApprovedReviewIndexValueData {
        action: record.held_data().target_data().spec().action,
        header,
        receipt: digest(encrypted),
    })
}

pub(super) fn digest(frame: &[u8]) -> [u8; 32] {
    blake3::derive_key(
        "catcoms/private-chat-reviewed-original-receipt-frame/v1",
        frame,
    )
}
