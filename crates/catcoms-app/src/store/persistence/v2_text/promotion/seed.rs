//! Complete immutable promotion journal DATA, before any dependent page/head-stage effect.
//!
//! It carries the exact ORIGINAL randomized genesis frame, so a new cold mount never has to
//! recreate its ciphertext identity after O_G replaced head.bin. Its parser alone grants no
//! source/Current/scratch-disposal/repair authority; cold must prove the independently paid
//! source/pin and authenticate each inner frame against the outer expected context.
use super::*;
use crate::store::persistence::v2_text::common::{
    field, finish, input, put, tag, TextGenesisParentIdData,
};
use crate::store::persistence::v2_text::context::MAX_CONTEXT_PLAIN;
use crate::store::persistence::v2_text::control::TextControlKindData;
use catcoms_wire::Encoder;

const TAG: &[u8] = b"catcoms/private-chat-text-seed/v1\0";
pub(super) const MAX_PROMOTION_SEED_PLAIN: usize = 24_733;
const _: () = assert!(
    4 + TAG.len() + 2 + 4 + MAX_CONTEXT_PLAIN + 36 + 4 + 4136 + 4 + 1519 + 4 + 1725 + 4 + 16470
        == MAX_PROMOTION_SEED_PLAIN
);

pub(super) fn prepare(
    promotion: &PreparedV2TextPromotion,
    key: &[u8; 32],
    rng: &mut dyn CryptoRngCore,
) -> Result<PreparedTextControlFrameData, AppError> {
    let context = promotion.captured.context.encode_data()?;
    let archive = promotion.archive.as_ref().ok_or_else(refused)?;
    let head = promotion.head.as_ref().ok_or_else(refused)?;
    let directory = promotion.directory.as_ref().ok_or_else(refused)?;
    let mut e = Encoder::with_capacity(MAX_PROMOTION_SEED_PLAIN);
    tag(&mut e, TAG)?;
    put(&mut e, &context)?;
    put(
        &mut e,
        promotion
            .captured
            .context
            .spec_data()
            .genesis_parent
            .as_bytes(),
    )?;
    for bytes in [
        promotion.captured.original_head.as_slice(),
        archive.frame_data(),
        head.frame_data(),
        directory.as_bytes(),
    ] {
        put(&mut e, bytes)?;
    }
    let plain = finish(e, MAX_PROMOTION_SEED_PLAIN)?;
    // Borrowed full cap/framing/identity check also pins the actual encoder's own shape.
    let parsed = parse_data(&plain)?;
    if parsed.context != promotion.captured.context
        || parsed.original_head != promotion.captured.original_head.as_slice()
        || parsed.archive_frame != archive.frame_data()
        || parsed.head_frame != head.frame_data()
        || parsed.directory_frame != directory.as_bytes()
    {
        return Err(refused());
    }
    PreparedTextControlFrameData::prepare_data(TextControlKindData::Seed, key, &plain, rng)
}

pub(in crate::store::persistence::v2_text) struct ParsedPromotionSeedData<'a> {
    pub(in crate::store::persistence::v2_text) context: TextContextData,
    pub(in crate::store::persistence::v2_text) original_head: &'a [u8],
    pub(in crate::store::persistence::v2_text) archive_frame: &'a [u8],
    pub(in crate::store::persistence::v2_text) head_frame: &'a [u8],
    pub(in crate::store::persistence::v2_text) directory_frame: &'a [u8],
}

pub(in crate::store::persistence::v2_text) fn parse_data(
    raw: &[u8],
) -> Result<ParsedPromotionSeedData<'_>, AppError> {
    let mut d = input(raw, MAX_PROMOTION_SEED_PLAIN, TAG)?;
    let context_frame = field(&mut d, MAX_CONTEXT_PLAIN)?;
    let original_id = super::super::common::b32(&mut d)?;
    let original_head = field(&mut d, 4136)?;
    let archive_frame = field(&mut d, 1519)?;
    let head_frame = field(&mut d, 1725)?;
    let directory_frame = field(&mut d, 16470)?;
    d.finish().map_err(|_| refused())?;
    if original_head.len() <= 40
        || archive_frame.len() <= 40
        || head_frame.len() <= 40
        || directory_frame.len() <= 86
    {
        return Err(refused());
    }
    let context = TextContextData::decode_data(context_frame)?;
    if *TextGenesisParentIdData::of_data(original_head).as_bytes() != original_id
        || context.spec_data().genesis_parent.as_bytes() != &original_id
    {
        return Err(refused());
    }
    Ok(ParsedPromotionSeedData {
        context,
        original_head,
        archive_frame,
        head_frame,
        directory_frame,
    })
}
