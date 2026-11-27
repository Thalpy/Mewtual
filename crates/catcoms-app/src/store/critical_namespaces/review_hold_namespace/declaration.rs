//! Exact recovery data for the founder-only catalog declaration. A seed
//! can restore the same selected bytes after interruption. Its contents
//! never replace the caller's own row, held files or directory witness.
use super::*;
use crate::store::{frame, unframe};
use catcoms_crypto::{seal, unseal};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

const TAG: &[u8] = b"catcoms/private-chat-v2-founder-review-declaration/v1\0";
const SEED_KEY: &str = "catcoms/private-chat-v2-founder-review-declaration-key/v1";
const FRAME_TAG: &[u8] = b"CTV2RVW\x01";
const FRAME_HEADER: usize = FRAME_TAG.len() + 4;
const OVERHEAD: usize = 4 + TAG.len() + 2 + 6 * 36 + 2 * 4;
const MIN_SEED_FRAME: usize = FRAME_HEADER + OVERHEAD + 2 * 41 + 40;
pub(in crate::store) const MAX_SEED_FRAME: usize =
    FRAME_HEADER + OVERHEAD + 2 * catalog::MAX_CATALOG_FRAMED + 40;

/// Exact labels derived from the independently verified native inventory.
/// The physical caller checks each original leaf even on cold recovery.
#[derive(Clone, Copy)]
pub(in crate::store) struct FounderLeaves {
    pub(in crate::store) row: [u8; 32],
    pub(in crate::store) index: [u8; 32],
    pub(in crate::store) pins: [u8; 32],
    pub(in crate::store) history: [u8; 32],
}

impl FounderLeaves {
    fn fields(self, layout: CurrentLayoutData) -> [[u8; 32]; 6] {
        [
            layout.vault,
            layout.root,
            self.row,
            self.index,
            self.pins,
            self.history,
        ]
    }
}

pub(in crate::store) struct PreparedDeclaration {
    pub(in crate::store) candidate: Zeroizing<Vec<u8>>,
    pub(in crate::store) marker: [u8; MARKER_LEN],
    pub(in crate::store) seed: Zeroizing<Vec<u8>>,
}

impl PreparedDeclaration {
    pub(in crate::store) fn prepare(
        key: &[u8; 32],
        original: &[u8],
        layout: CurrentLayoutData,
        leaves: FounderLeaves,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        let old = open_catalog(key, original)?;
        let new = prepare_declaration(&old, &layout.vault)?;
        let marker = selected_marker(key, layout, &new)?;
        let candidate = Zeroizing::new(frame(&seal(key, &new, rng)?));
        let mut seed = Encoder::with_capacity(OVERHEAD + original.len() + candidate.len());
        let malformed = |_| refused();
        seed.put_bytes(TAG).map_err(malformed)?;
        seed.put_u16(VERSION as u16);
        for field in leaves.fields(layout) {
            seed.put_bytes(&field).map_err(malformed)?;
        }
        seed.put_bytes(original).map_err(malformed)?;
        seed.put_bytes(&candidate).map_err(malformed)?;
        let seed_key = Zeroizing::new(blake3::derive_key(SEED_KEY, key));
        let sealed = seal(&seed_key, &Zeroizing::new(seed.finish()), rng)?;
        let raw = Zeroizing::new(frame(&sealed));
        let length = u32::try_from(FRAME_HEADER + raw.len()).map_err(|_| refused())?;
        let mut seed = Zeroizing::new(Vec::with_capacity(length as usize));
        seed.extend_from_slice(FRAME_TAG);
        seed.extend_from_slice(&length.to_le_bytes());
        seed.extend_from_slice(&raw);
        Ok(Self {
            candidate,
            marker,
            seed,
        })
    }
}

pub(in crate::store) struct AuthenticatedSeed(Zeroizing<Vec<u8>>);

pub(in crate::store) struct OpenedDeclaration<'a> {
    pub(in crate::store) original: &'a [u8],
    pub(in crate::store) candidate: &'a [u8],
    pub(in crate::store) marker: [u8; MARKER_LEN],
}

impl AuthenticatedSeed {
    pub(in crate::store) fn open(key: &[u8; 32], raw: &[u8]) -> Result<Self, AppError> {
        if Self::declared_length(raw) != Some(raw.len()) {
            return Err(refused());
        }
        let key = Zeroizing::new(blake3::derive_key(SEED_KEY, key));
        Ok(Self(Zeroizing::new(unseal(
            &key,
            &unframe(&raw[FRAME_HEADER..])?,
        )?)))
    }

    pub(in crate::store) fn recognizes_frame(raw: &[u8]) -> bool {
        // Leave future versions of this claim to a reader that knows them.
        raw.starts_with(&FRAME_TAG[..FRAME_TAG.len() - 1])
    }

    /// A power cut can leave only a prefix in the private scratch role.
    /// A complete corrupt frame or another format remains on disk.
    pub(in crate::store) fn is_incomplete_scratch(raw: &[u8]) -> bool {
        if raw.len() < FRAME_TAG.len() {
            return FRAME_TAG.starts_with(raw);
        }
        if raw.len() < FRAME_HEADER {
            return raw.starts_with(FRAME_TAG);
        }
        Self::declared_length(raw).is_some_and(|length| raw.len() < length)
    }

    fn declared_length(raw: &[u8]) -> Option<usize> {
        let bytes = raw.strip_prefix(FRAME_TAG)?.get(..4)?;
        let length = u32::from_le_bytes(bytes.try_into().ok()?) as usize;
        (MIN_SEED_FRAME..=MAX_SEED_FRAME)
            .contains(&length)
            .then_some(length)
    }

    pub(in crate::store) fn validate(
        &self,
        key: &[u8; 32],
        layout: CurrentLayoutData,
        leaves: FounderLeaves,
    ) -> Result<OpenedDeclaration<'_>, AppError> {
        let mut input = Decoder::new(&self.0);
        let malformed = |_| refused();
        if input.get_bytes().map_err(malformed)? != TAG
            || input.get_u16().map_err(malformed)? != VERSION as u16
        {
            return Err(refused());
        }
        for field in leaves.fields(layout) {
            if input.get_bytes().map_err(malformed)? != field {
                return Err(refused());
            }
        }
        let original = input.get_bytes().map_err(malformed)?;
        let candidate = input.get_bytes().map_err(malformed)?;
        input.finish().map_err(malformed)?;
        let old = open_catalog(key, original)?;
        let new = open_catalog(key, candidate)?;
        verify_declaration(&old, &new, &layout.vault)?;
        let marker = selected_marker(key, layout, &new)?;
        Ok(OpenedDeclaration {
            original,
            candidate,
            marker,
        })
    }
}

fn selected_marker(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    candidate: &[u8],
) -> Result<[u8; MARKER_LEN], AppError> {
    let selected = catalog::parse_review_hold_family(candidate, &layout.vault, true)?;
    if selected.current_layout != Some(layout) {
        return Err(refused());
    }
    marker_data(key, selected.review_hold_payload.ok_or_else(refused)?)
}

fn open_catalog(key: &[u8; 32], raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(41..=catalog::MAX_CATALOG_FRAMED).contains(&raw.len()) {
        return Err(refused());
    }
    Ok(Zeroizing::new(unseal(key, &unframe(raw)?)?))
}
