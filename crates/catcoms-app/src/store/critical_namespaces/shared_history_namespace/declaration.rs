//! Exact presealed restart record for declaring the private history owner.
use super::*;
use crate::store::critical_namespaces::joined_pin::{declaration::DeclarationLeaves, index};
use crate::store::{frame, unframe};
use catcoms_crypto::{seal, unseal};
use catcoms_rt::CryptoRngCore;

const TAG: &[u8] = b"catcoms/private-chat-v2-shared-history-declaration/v1\0";
const SEED_KEY: &str = "catcoms/private-chat-v2-shared-history-declaration-key/v1";
const MARKER_TAG: &[u8] = b"catcoms/private-chat-v2-shared-history-root/v1\0";
const MARKER_KEY: &str = "catcoms/private-chat-v2-shared-history-root-key/v1";
pub(in crate::store) const MARKER_BYTES: usize = 4 + MARKER_TAG.len() + 2 + 3 * 36 + 40;
const OVERHEAD: usize = 4 + TAG.len() + 2 + 7 * 36 + 3 * 4;
pub(in crate::store) const MAX_SEED_FRAME: usize =
    OVERHEAD + 2 * catalog::MAX_CATALOG_FRAMED + MARKER_BYTES + 40;

/// Data commitments to separately captured physical files. A successful seed
/// decode cannot replace their original file handles and namespace barrier.
#[derive(Clone, Copy)]
pub(in crate::store) struct Leaves {
    native: DeclarationLeaves,
    pin_index: [u8; 32],
}

impl Leaves {
    pub(in crate::store) fn from_frames(
        native: DeclarationLeaves,
        pin_index: &[u8],
    ) -> Result<Self, AppError> {
        if !(41..=index::MAX_FRAME).contains(&pin_index.len()) {
            return Err(refused());
        }
        Ok(Self {
            native,
            pin_index: *blake3::hash(pin_index).as_bytes(),
        })
    }

    fn iter(self, layout: CurrentLayoutData) -> impl Iterator<Item = [u8; 32]> {
        [layout.vault, layout.root]
            .into_iter()
            .chain(self.native.fields())
            .chain([self.pin_index])
    }
}

pub(in crate::store) struct PreparedDeclaration {
    pub(in crate::store) candidate: Zeroizing<Vec<u8>>,
    pub(in crate::store) marker: Zeroizing<Vec<u8>>,
    pub(in crate::store) seed: Zeroizing<Vec<u8>>,
}

impl PreparedDeclaration {
    pub(in crate::store) fn prepare(
        key: &[u8; 32],
        original: &[u8],
        layout: CurrentLayoutData,
        leaves: Leaves,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        let old = open_catalog(key, original)?;
        let new = prepare_declaration(&old, &layout.vault)?;
        let descriptor = selected_descriptor(&new, layout, true)?;
        let marker = seal_marker(key, layout, descriptor, rng)?;
        let candidate = Zeroizing::new(frame(&seal(key, &new, rng)?));
        let invalid = |_| refused();
        let mut plain =
            Encoder::with_capacity(OVERHEAD + original.len() + candidate.len() + marker.len());
        plain.put_bytes(TAG).map_err(invalid)?;
        plain.put_u16(VERSION as u16);
        for field in leaves.iter(layout) {
            plain.put_bytes(&field).map_err(invalid)?;
        }
        plain.put_bytes(original).map_err(invalid)?;
        plain.put_bytes(&candidate).map_err(invalid)?;
        plain.put_bytes(&marker).map_err(invalid)?;
        let seed_key = Zeroizing::new(blake3::derive_key(SEED_KEY, key));
        let seed = Zeroizing::new(frame(&seal(
            &seed_key,
            &Zeroizing::new(plain.finish()),
            rng,
        )?));
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
    pub(in crate::store) marker: &'a [u8],
}

impl AuthenticatedSeed {
    pub(in crate::store) fn open(key: &[u8; 32], raw: &[u8]) -> Result<Self, AppError> {
        if !(41..=MAX_SEED_FRAME).contains(&raw.len()) {
            return Err(refused());
        }
        let key = Zeroizing::new(blake3::derive_key(SEED_KEY, key));
        Ok(Self(Zeroizing::new(unseal(&key, &unframe(raw)?)?)))
    }

    pub(in crate::store) fn validate(
        &self,
        key: &[u8; 32],
        layout: CurrentLayoutData,
        leaves: Leaves,
    ) -> Result<OpenedDeclaration<'_>, AppError> {
        let mut input = Decoder::new(&self.0);
        let invalid = |_| refused();
        if input.get_bytes().map_err(invalid)? != TAG
            || input.get_u16().map_err(invalid)? != VERSION as u16
        {
            return Err(refused());
        }
        for field in leaves.iter(layout) {
            if input.get_bytes().map_err(invalid)? != field {
                return Err(refused());
            }
        }
        let original = input.get_bytes().map_err(invalid)?;
        let candidate = input.get_bytes().map_err(invalid)?;
        let marker = input.get_bytes().map_err(invalid)?;
        input.finish().map_err(invalid)?;
        let old = open_catalog(key, original)?;
        let new = open_catalog(key, candidate)?;
        verify_declaration(&old, &new, &layout.vault)?;
        validate_marker(
            key,
            layout,
            selected_descriptor(&new, layout, true)?,
            marker,
        )?;
        Ok(OpenedDeclaration {
            original,
            candidate,
            marker,
        })
    }
}

fn open_catalog(key: &[u8; 32], raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(41..=catalog::MAX_CATALOG_FRAMED).contains(&raw.len()) {
        return Err(refused());
    }
    Ok(Zeroizing::new(unseal(key, &unframe(raw)?)?))
}

pub(in crate::store) fn selected_descriptor(
    raw: &[u8],
    layout: CurrentLayoutData,
    executing: bool,
) -> Result<&[u8], AppError> {
    let parsed =
        catalog::parse_shared_history_native_registry_layout(raw, &layout.vault, executing)?;
    if parsed.current_layout != Some(layout) {
        return Err(refused());
    }
    parsed.shared_history_payload.ok_or_else(refused)
}

fn marker_fields(layout: CurrentLayoutData, descriptor: &[u8]) -> [[u8; 32]; 3] {
    [
        layout.vault,
        layout.root,
        blake3::derive_key(
            "catcoms/private-chat-v2-shared-history-descriptor/v1",
            descriptor,
        ),
    ]
}

fn seal_marker(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    descriptor: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut marker = Encoder::with_capacity(MARKER_BYTES - 40);
    let invalid = |_| refused();
    marker.put_bytes(MARKER_TAG).map_err(invalid)?;
    marker.put_u16(VERSION as u16);
    for field in marker_fields(layout, descriptor) {
        marker.put_bytes(&field).map_err(invalid)?;
    }
    let key = Zeroizing::new(blake3::derive_key(MARKER_KEY, key));
    Ok(Zeroizing::new(frame(&seal(
        &key,
        &Zeroizing::new(marker.finish()),
        rng,
    )?)))
}

pub(in crate::store) fn validate_marker(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    descriptor: &[u8],
    raw: &[u8],
) -> Result<(), AppError> {
    if raw.len() != MARKER_BYTES {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(MARKER_KEY, key));
    let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
    let mut input = Decoder::new(&plain);
    let invalid = |_| refused();
    if input.get_bytes().map_err(invalid)? != MARKER_TAG
        || input.get_u16().map_err(invalid)? != VERSION as u16
    {
        return Err(refused());
    }
    for field in marker_fields(layout, descriptor) {
        if input.get_bytes().map_err(invalid)? != field {
            return Err(refused());
        }
    }
    input.finish().map_err(invalid)
}
