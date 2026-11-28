//! Exact encrypted input for selecting the recipient namespace once.
//! The owning opener independently holds each parent and physical file.
use super::*;
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

const TAG: &[u8] = b"catcoms/private-chat-v2-review-receiver-declaration/v1\0";
const SEED_DOMAIN: &str = "catcoms/private-chat-v2-review-receiver-declaration-key/v1";
const FRAME_TAG: &[u8; 8] = b"CTV2RVR\x01";
const FRAME_HEADER: usize = 12;
const MAX_INITIAL_MANIFEST: usize = 256;
pub(in crate::store) const MAX_SEED_BYTES: usize = FRAME_HEADER
    + 40
    + 4
    + TAG.len()
    + 5 * 36
    + 3 * 4
    + 2 * catalog::MAX_CATALOG_FRAMED
    + MAX_INITIAL_MANIFEST;

/// These commitments come from the separate checked layout owner. The
/// seed reader never learns its authority from the seed itself.
pub(in crate::store) struct RecipientParent {
    pub(in crate::store) layout: CurrentLayoutData,
    pub(in crate::store) joined: [u8; 32],
    pub(in crate::store) native: [u8; 32],
    pub(in crate::store) history: [u8; 32],
}

impl RecipientParent {
    fn fields(&self) -> [&[u8; 32]; 5] {
        [
            &self.layout.vault,
            &self.layout.root,
            &self.joined,
            &self.native,
            &self.history,
        ]
    }
}

pub(in crate::store) struct PreparedReceiverDeclaration {
    pub(in crate::store) seed: Zeroizing<Vec<u8>>,
}

impl PreparedReceiverDeclaration {
    pub(in crate::store) fn prepare(
        key: &[u8; 32],
        parent: &RecipientParent,
        original: &[u8],
        make_initial: impl FnOnce(
            &[u8; 32],
            &mut dyn CryptoRngCore,
        ) -> Result<Zeroizing<Vec<u8>>, AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        let old = open_catalog(key, original)?;
        require_layout(&old, parent.layout)?;
        let next = prepare_declaration(&old, &parent.layout.vault)?;
        let parsed = catalog::parse_review_receive_family(&next, &parent.layout.vault, true)?;
        let marker = marker_data(key, parsed.review_receive_payload.ok_or_else(refused)?)?;
        let initial = make_initial(&marker, rng)?;
        if initial.is_empty() || initial.len() > MAX_INITIAL_MANIFEST {
            return Err(refused());
        }
        let candidate =
            Zeroizing::new(crate::store::frame(&catcoms_crypto::seal(key, &next, rng)?));
        let mut plain = Encoder::with_capacity(
            4 + TAG.len() + 5 * 36 + 3 * 4 + original.len() + candidate.len() + initial.len(),
        );
        plain.put_bytes(TAG).map_err(|_| refused())?;
        for digest in parent.fields() {
            plain.put_bytes(digest).map_err(|_| refused())?;
        }
        for bytes in [original, candidate.as_slice(), initial.as_slice()] {
            plain.put_bytes(bytes).map_err(|_| refused())?;
        }
        let plain = Zeroizing::new(plain.finish());
        let seed_key = Zeroizing::new(blake3::derive_key(SEED_DOMAIN, key));
        let sealed = crate::store::frame(&catcoms_crypto::seal(&seed_key, &plain, rng)?);
        let len = FRAME_HEADER + sealed.len();
        if len > MAX_SEED_BYTES {
            return Err(refused());
        }
        let mut seed = Zeroizing::new(Vec::with_capacity(len));
        seed.extend_from_slice(FRAME_TAG);
        seed.extend_from_slice(&(len as u32).to_le_bytes());
        seed.extend_from_slice(&sealed);
        Ok(Self { seed })
    }
}

pub(in crate::store) struct ReceiverDeclarationSeed(Zeroizing<Vec<u8>>);

pub(in crate::store) struct ReceiverDeclarationData<'a> {
    pub(in crate::store) original: &'a [u8],
    pub(in crate::store) candidate: &'a [u8],
    pub(in crate::store) initial: &'a [u8],
    pub(in crate::store) marker: [u8; 32],
}

impl ReceiverDeclarationSeed {
    pub(in crate::store) fn recognizes(raw: &[u8]) -> bool {
        raw.starts_with(FRAME_TAG)
    }

    pub(in crate::store) fn incomplete_scratch(raw: &[u8]) -> bool {
        if raw.len() < FRAME_TAG.len() {
            return FRAME_TAG.starts_with(raw);
        }
        if !Self::recognizes(raw) {
            return false;
        }
        if raw.len() < FRAME_HEADER {
            return true;
        }
        let declared =
            u32::from_le_bytes(raw[8..FRAME_HEADER].try_into().expect("checked width")) as usize;
        (FRAME_HEADER + 41..=MAX_SEED_BYTES).contains(&declared) && raw.len() < declared
    }

    pub(in crate::store) fn open(key: &[u8; 32], raw: &[u8]) -> Result<Self, AppError> {
        if !(FRAME_HEADER + 41..=MAX_SEED_BYTES).contains(&raw.len()) || !raw.starts_with(FRAME_TAG)
        {
            return Err(refused());
        }
        let length = u32::from_le_bytes(raw[8..FRAME_HEADER].try_into().map_err(|_| refused())?);
        if length as usize != raw.len() {
            return Err(refused());
        }
        let seed_key = Zeroizing::new(blake3::derive_key(SEED_DOMAIN, key));
        Ok(Self(Zeroizing::new(catcoms_crypto::unseal(
            &seed_key,
            &crate::store::unframe(&raw[FRAME_HEADER..])?,
        )?)))
    }

    pub(in crate::store) fn validate(
        &self,
        key: &[u8; 32],
        parent: &RecipientParent,
    ) -> Result<ReceiverDeclarationData<'_>, AppError> {
        let mut input = Decoder::new(&self.0);
        if input.get_bytes().map_err(|_| refused())? != TAG {
            return Err(refused());
        }
        for expected in parent.fields() {
            if input.get_bytes().map_err(|_| refused())? != expected {
                return Err(refused());
            }
        }
        let original = input.get_bytes().map_err(|_| refused())?;
        let candidate = input.get_bytes().map_err(|_| refused())?;
        let initial = input.get_bytes().map_err(|_| refused())?;
        input.finish().map_err(|_| refused())?;
        if initial.is_empty() || initial.len() > MAX_INITIAL_MANIFEST {
            return Err(refused());
        }
        let old = open_catalog(key, original)?;
        let new = open_catalog(key, candidate)?;
        require_layout(&old, parent.layout)?;
        verify_declaration(&old, &new, &parent.layout.vault)?;
        let selected = catalog::parse_review_receive_family(&new, &parent.layout.vault, true)?;
        let marker = marker_data(key, selected.review_receive_payload.ok_or_else(refused)?)?;
        Ok(ReceiverDeclarationData {
            original,
            candidate,
            initial,
            marker,
        })
    }
}

fn require_layout(raw: &[u8], layout: CurrentLayoutData) -> Result<(), AppError> {
    let parents = catalog::parse_shared_history_native_registry_layout(raw, &layout.vault, true)?;
    if parents.current_layout != Some(layout) {
        return Err(refused());
    }
    Ok(())
}

fn open_catalog(key: &[u8; 32], raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(41..=catalog::MAX_CATALOG_FRAMED).contains(&raw.len()) {
        return Err(refused());
    }
    Ok(Zeroizing::new(catcoms_crypto::unseal(
        key,
        &crate::store::unframe(raw)?,
    )?))
}
