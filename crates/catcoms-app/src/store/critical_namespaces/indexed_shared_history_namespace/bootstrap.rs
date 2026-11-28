//! Exact one-entry declaration and empty indexed archive anchor. These
//! commitments must be reproved against the held files by the physical owner.
use super::*;
use crate::store::{frame, persistence::v2_shared_history::INDEXED_HEAD_CAP, unframe};
use catcoms_crypto::{seal, unseal};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

const TAG: &[u8] = b"catcoms/private-chat-v2-indexed-shared-history-declaration/v1\0";
const SEED_KEY: &str = "catcoms/private-chat-v2-indexed-shared-history-declaration-key/v1";
const FRAME_TAG: &[u8] = b"CV2-IN-HIST\x01";
const FRAME_HEADER: usize = FRAME_TAG.len() + 4;
const OVERHEAD: usize = 4 + TAG.len() + 2 + 5 * 36 + 4 * 4 + 32;
const MAX_SEALED: usize = OVERHEAD + 2 * catalog::MAX_CATALOG_FRAMED + INDEXED_HEAD_CAP + 40;
pub(in crate::store) const MAX_SEED: usize = FRAME_HEADER + MAX_SEALED;

#[derive(Clone, Copy)]
pub(in crate::store) struct IndexedArchiveParent {
    pub(in crate::store) layout: CurrentLayoutData,
    pub(in crate::store) native_registry: [u8; 32],
    pub(in crate::store) joined_pins: [u8; 32],
    pub(in crate::store) original_archive: [u8; 32],
}

impl IndexedArchiveParent {
    fn fields(self) -> [[u8; 32]; 5] {
        [
            self.layout.vault,
            self.layout.root,
            self.native_registry,
            self.joined_pins,
            self.original_archive,
        ]
    }
}

pub(in crate::store) struct PreparedArchiveDeclaration {
    pub(in crate::store) seed: Zeroizing<Vec<u8>>,
}

pub(in crate::store) struct AuthenticatedArchiveSeed(Zeroizing<Vec<u8>>);

pub(in crate::store) struct IndexedArchiveDeclaration<'a> {
    pub(in crate::store) original: &'a [u8],
    pub(in crate::store) candidate: &'a [u8],
    pub(in crate::store) marker: [u8; 32],
    pub(in crate::store) initial_head: &'a [u8],
}

impl PreparedArchiveDeclaration {
    pub(in crate::store) fn prepare(
        key: &[u8; 32],
        parent: IndexedArchiveParent,
        original: &[u8],
        prepare_empty_head: impl FnOnce(
            &[u8; 32],
            &mut dyn CryptoRngCore,
        ) -> Result<Zeroizing<Vec<u8>>, AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        let old = open_catalog(key, original)?;
        let next = prepare_declaration(&old, &parent.layout.vault)?;
        let target = selected_descriptor(&next, parent.layout)?;
        let marker = marker_data(key, &target)?;
        let initial_head = prepare_empty_head(&target, rng)?;
        require_head_size(&initial_head)?;
        let candidate = Zeroizing::new(frame(&seal(key, &next, rng)?));
        let mut plain = Encoder::with_capacity(
            OVERHEAD + original.len() + candidate.len() + initial_head.len(),
        );
        plain.put_bytes(TAG).map_err(|_| refused())?;
        plain.put_u16(VERSION as u16);
        for value in parent.fields() {
            plain.put_bytes(&value).map_err(|_| refused())?;
        }
        for value in [
            original,
            candidate.as_slice(),
            &marker,
            initial_head.as_slice(),
        ] {
            plain.put_bytes(value).map_err(|_| refused())?;
        }
        let seed_key = Zeroizing::new(blake3::derive_key(SEED_KEY, key));
        let sealed = Zeroizing::new(frame(&seal(
            &seed_key,
            &Zeroizing::new(plain.finish()),
            rng,
        )?));
        if !(41..=MAX_SEALED).contains(&sealed.len()) {
            return Err(refused());
        }
        let mut seed = Zeroizing::new(Vec::with_capacity(FRAME_HEADER + sealed.len()));
        seed.extend_from_slice(FRAME_TAG);
        seed.extend_from_slice(&(sealed.len() as u32).to_le_bytes());
        seed.extend_from_slice(&sealed);
        Ok(Self { seed })
    }
}

impl AuthenticatedArchiveSeed {
    pub(in crate::store) fn open(key: &[u8; 32], raw: &[u8]) -> Result<Self, AppError> {
        let expected = seed_length(raw).ok_or_else(refused)?;
        if raw.len() != expected {
            return Err(refused());
        }
        open_sealed(key, &raw[FRAME_HEADER..]).map(Self)
    }

    /// Only the private scratch can use this negative indication. A
    /// complete corrupt frame or another version remains preserved.
    pub(in crate::store) fn is_incomplete_scratch(key: &[u8; 32], raw: &[u8]) -> bool {
        if raw.len() < FRAME_TAG.len() {
            return FRAME_TAG.starts_with(raw);
        }
        if raw.len() < FRAME_HEADER {
            return raw.starts_with(FRAME_TAG);
        }
        seed_length(raw).is_some_and(|length| raw.len() < length)
            // A damaged outer length can make a complete ciphertext appear
            // short. Authenticate solely to preserve it for inspection.
            && open_sealed(key, &raw[FRAME_HEADER..]).is_err()
    }

    pub(in crate::store) fn validate(
        &self,
        key: &[u8; 32],
        parent: IndexedArchiveParent,
        verify_empty_head: impl FnOnce(&[u8; 32], &[u8]) -> Result<(), AppError>,
    ) -> Result<IndexedArchiveDeclaration<'_>, AppError> {
        let mut decoder = self.decoder()?;
        for value in parent.fields() {
            if decoder.get_bytes().map_err(|_| refused())? != value {
                return Err(refused());
            }
        }
        let original = decoder.get_bytes().map_err(|_| refused())?;
        let candidate = decoder.get_bytes().map_err(|_| refused())?;
        let found_marker = decoder.get_bytes().map_err(|_| refused())?;
        let initial_head = decoder.get_bytes().map_err(|_| refused())?;
        decoder.finish().map_err(|_| refused())?;
        require_head_size(initial_head)?;
        let original_plain = open_catalog(key, original)?;
        let selected_plain = open_catalog(key, candidate)?;
        verify_declaration(&original_plain, &selected_plain, &parent.layout.vault)?;
        let target = selected_descriptor(&selected_plain, parent.layout)?;
        let marker = marker_data(key, &target)?;
        if found_marker != marker {
            return Err(refused());
        }
        verify_empty_head(&target, initial_head)?;
        Ok(IndexedArchiveDeclaration {
            original,
            candidate,
            marker,
            initial_head,
        })
    }

    /// Extract the authenticated old catalog to locate its existing
    /// witnesses. A recovery owner must validate those witnesses before
    /// using any field of this seed to write.
    pub(in crate::store) fn original_catalog(&self) -> Result<&[u8], AppError> {
        let mut decoder = self.decoder()?;
        for _ in 0..5 {
            if decoder.get_bytes().map_err(|_| refused())?.len() != 32 {
                return Err(refused());
            }
        }
        let catalog = decoder.get_bytes().map_err(|_| refused())?;
        if !(41..=catalog::MAX_CATALOG_FRAMED).contains(&catalog.len()) {
            return Err(refused());
        }
        Ok(catalog)
    }

    fn decoder(&self) -> Result<Decoder<'_>, AppError> {
        let mut decoder = Decoder::new(&self.0);
        if decoder.get_bytes().map_err(|_| refused())? != TAG
            || decoder.get_u16().map_err(|_| refused())? != VERSION as u16
        {
            return Err(refused());
        }
        Ok(decoder)
    }
}

fn seed_length(raw: &[u8]) -> Option<usize> {
    if !raw.starts_with(FRAME_TAG) {
        return None;
    }
    let body =
        u32::from_le_bytes(raw.get(FRAME_TAG.len()..FRAME_HEADER)?.try_into().ok()?) as usize;
    (41..=MAX_SEALED)
        .contains(&body)
        .then_some(FRAME_HEADER + body)
}

fn open_sealed(key: &[u8; 32], raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let seed_key = Zeroizing::new(blake3::derive_key(SEED_KEY, key));
    Ok(Zeroizing::new(unseal(&seed_key, &unframe(raw)?)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn indexed_shared_history_damaged_seed_length_preserves_an_authentic_body() {
        let key = [29; 32];
        let seed_key = blake3::derive_key(SEED_KEY, &key);
        let mut rng = ChaCha20Rng::seed_from_u64(13);
        let body = frame(&seal(&seed_key, b"original indexed seed", &mut rng).unwrap());
        let mut raw = FRAME_TAG.to_vec();
        raw.extend_from_slice(&((body.len() + 1) as u32).to_le_bytes());
        raw.extend_from_slice(&body);

        assert!(AuthenticatedArchiveSeed::open(&key, &raw).is_err());
        assert!(!AuthenticatedArchiveSeed::is_incomplete_scratch(&key, &raw));
        raw.pop();
        assert!(AuthenticatedArchiveSeed::is_incomplete_scratch(&key, &raw));
    }
}

fn selected_descriptor(plain: &[u8], layout: CurrentLayoutData) -> Result<[u8; 32], AppError> {
    let parsed = catalog::parse_indexed_shared_history_base(plain, &layout.vault, true)?;
    if parsed.current_layout != Some(layout) {
        return Err(refused());
    }
    parsed
        .indexed_history_payload
        .and_then(|payload| payload.try_into().ok())
        .ok_or_else(refused)
}

fn open_catalog(key: &[u8; 32], raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(41..=catalog::MAX_CATALOG_FRAMED).contains(&raw.len()) {
        return Err(refused());
    }
    Ok(Zeroizing::new(unseal(key, &unframe(raw)?)?))
}

fn require_head_size(frame: &[u8]) -> Result<(), AppError> {
    if !(41..=INDEXED_HEAD_CAP).contains(&frame.len()) {
        return Err(refused());
    }
    Ok(())
}
