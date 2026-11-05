//! Test-only namespace DATA dependency for the reviewed pending Current metadata codecs.
//! No production module/handler recognizes ID3. A parsed layout, marker, group name or
//! declaration seed never proves an actual supported mount, SourceSaved or Current effect.
use crate::store::{frame, unframe};
use crate::AppError;
use catcoms_crypto::keystore::{seal, unseal};
use catcoms_wire::{Decoder, Encoder};
use rand_core::CryptoRngCore;
use zeroize::Zeroizing;

pub(in crate::store) const NAMESPACE_ID: u32 = 3;
pub(in crate::store) const NAMESPACE_VERSION: u32 = 1;
pub(in crate::store) const MAX_LAYOUT_PLAIN: usize = 512;
pub(in crate::store) const MAX_MARKER_PLAIN: usize = 1024;
pub(in crate::store) const MAX_MARKER_FRAME: usize = MAX_MARKER_PLAIN + 40;
pub(in crate::store) const MAX_CATALOG_FRAME: usize = 16 * 1024 * 1024 + 4096 + 512 + 40;
pub(in crate::store) const MAX_DECLARATION_PLAIN: usize = MAX_CATALOG_FRAME + 4096;
pub(in crate::store) const MAX_DECLARATION_FRAME: usize = MAX_DECLARATION_PLAIN + 40;
pub(in crate::store) const DECLARATION_FEE: usize = 12 * MAX_DECLARATION_FRAME + 256 * 1024;
pub(in crate::store) const NAMESPACE_METADATA_FEE: usize = 64 * 1024;
const LAYOUT_TAG: &[u8] = b"catcoms/private-chat-v2-current-layout/v1\0";
const MARKER_TAG: &[u8] = b"catcoms/private-chat-v2-current-marker/v1\0";
const DECLARATION_TAG: &[u8] = b"catcoms/private-chat-v2-current-declaration/v1\0";
const VERSION: u16 = 1;
const MAX_GROUP: usize = 256;
const _: () = assert!(122 <= MAX_LAYOUT_PLAIN && 174 <= MAX_MARKER_PLAIN);
const _: () = assert!(MAX_CATALOG_FRAME + 1233 <= MAX_DECLARATION_PLAIN);

fn refused() -> AppError {
    AppError::Invalid("V2 genesis metadata is unavailable or unsupported".into())
}
fn field<'a>(d: &mut Decoder<'a>, maximum: usize) -> Result<&'a [u8], AppError> {
    let bytes = d.get_bytes().map_err(|_| refused())?;
    if bytes.len() > maximum {
        return Err(refused());
    }
    Ok(bytes)
}
fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    field(d, 32)?.try_into().map_err(|_| refused())
}
fn put(e: &mut Encoder, value: &[u8]) -> Result<(), AppError> {
    e.put_bytes(value).map_err(|_| refused())?;
    Ok(())
}
fn tag(e: &mut Encoder, value: &[u8]) -> Result<(), AppError> {
    put(e, value)?;
    e.put_u16(VERSION);
    Ok(())
}
fn input<'a>(raw: &'a [u8], maximum: usize, expected: &[u8]) -> Result<Decoder<'a>, AppError> {
    if raw.len() > maximum {
        return Err(refused());
    }
    let mut d = Decoder::new(raw);
    if field(&mut d, expected.len())? != expected || d.get_u16().map_err(|_| refused())? != VERSION
    {
        return Err(refused());
    }
    Ok(d)
}
fn finish(e: Encoder, maximum: usize) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if e.len() > maximum {
        return Err(refused());
    }
    Ok(Zeroizing::new(e.finish()))
}
fn seal_data(
    key: &[u8; 32],
    domain: &str,
    plain: &[u8],
    maximum: usize,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if plain.is_empty() || plain.len() > maximum {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&derived, plain, rng)?)))
}
fn open_data(
    key: &[u8; 32],
    domain: &str,
    raw: &[u8],
    maximum: usize,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if raw.len() <= 40 || raw.len() > maximum + 40 {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    let plain = Zeroizing::new(unseal(&derived, &unframe(raw)?)?);
    if plain.is_empty() || plain.len() > maximum {
        return Err(refused());
    }
    Ok(plain)
}

/// Independently selected namespace expectation DATA, never an actual mounted Root/session.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct CurrentLayoutData {
    pub(in crate::store) vault: [u8; 32],
    pub(in crate::store) root: [u8; 32],
}
impl std::fmt::Debug for CurrentLayoutData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurrentLayoutData").finish_non_exhaustive()
    }
}
impl CurrentLayoutData {
    pub(in crate::store) fn encode_data(self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(MAX_LAYOUT_PLAIN);
        tag(&mut e, LAYOUT_TAG)?;
        put(&mut e, &self.vault)?;
        put(&mut e, &self.root)?;
        e.put_u8(2).put_u8(1); // V2 root, founder-genesis-only handler
        finish(e, MAX_LAYOUT_PLAIN)
    }
    pub(in crate::store) fn decode_data(raw: &[u8], expected: Self) -> Result<Self, AppError> {
        let mut d = input(raw, MAX_LAYOUT_PLAIN, LAYOUT_TAG)?;
        let value = Self {
            vault: digest(&mut d)?,
            root: digest(&mut d)?,
        };
        if d.get_u8().map_err(|_| refused())? != 2 || d.get_u8().map_err(|_| refused())? != 1 {
            return Err(refused());
        }
        d.finish().map_err(|_| refused())?;
        if value != expected {
            return Err(refused());
        }
        Ok(value)
    }
}

/// Fixed GROUP hash suffix only; the actual closed path selector must prepend the literal
/// `g-` to obtain the reviewed physical directory name. Local signer is deliberately not an
/// input. Caller cannot choose a filename or infer directory absence/ownership/Current.
pub(in crate::store) fn group_name_data(
    layout: CurrentLayoutData,
    group: &[u8],
) -> Result<[u8; 64], AppError> {
    if group.is_empty() || group.len() > MAX_GROUP {
        return Err(refused());
    }
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/private-chat-v2-group-root/v1\0");
    hash.update(&layout.vault);
    hash.update(&layout.root);
    hash.update(&(group.len() as u32).to_be_bytes());
    hash.update(group);
    let mut name = [0; 64];
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in hash.finalize().as_bytes().iter().copied().enumerate() {
        name[2 * index] = HEX[(byte >> 4) as usize];
        name[2 * index + 1] = HEX[(byte & 15) as usize];
    }
    Ok(name)
}

pub(in crate::store) fn seal_marker_data(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let descriptor = layout.encode_data()?;
    let mut e = Encoder::with_capacity(MAX_MARKER_PLAIN);
    tag(&mut e, MARKER_TAG)?;
    put(&mut e, &descriptor)?;
    let plain = finish(e, MAX_MARKER_PLAIN)?;
    seal_data(
        key,
        "catcoms/private-chat-v2-current-marker-key/v1",
        &plain,
        MAX_MARKER_PLAIN,
        rng,
    )
}
pub(in crate::store) fn open_marker_data(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    raw: &[u8],
) -> Result<(), AppError> {
    #[cfg(test)]
    MARKER_OPEN_ENTRIES.with(|entries| entries.set(entries.get() + 1));
    let plain = open_data(
        key,
        "catcoms/private-chat-v2-current-marker-key/v1",
        raw,
        MAX_MARKER_PLAIN,
    )?;
    let mut d = input(&plain, MAX_MARKER_PLAIN, MARKER_TAG)?;
    let descriptor = field(&mut d, MAX_LAYOUT_PLAIN)?;
    d.finish().map_err(|_| refused())?;
    CurrentLayoutData::decode_data(descriptor, layout)?;
    Ok(())
}

/// Bounded whole-catalog ciphertext references only. Parsing does NOT authenticate the new
/// candidate's JOIN/ID2/ID3 semantics, choose an original parent or complete any declaration.
pub(in crate::store) struct DeclarationSeedData<'a> {
    pub(in crate::store) expected_catalog_id: [u8; 32],
    pub(in crate::store) marker_frame: &'a [u8],
    pub(in crate::store) catalog_frame: &'a [u8],
}
impl std::fmt::Debug for DeclarationSeedData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeclarationSeedData")
            .finish_non_exhaustive()
    }
}
pub(in crate::store) fn encode_declaration_seed_data(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    expected_catalog_id: [u8; 32],
    marker_frame: &[u8],
    catalog_frame: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if marker_frame.len() > MAX_MARKER_FRAME
        || catalog_frame.len() <= 40
        || catalog_frame.len() > MAX_CATALOG_FRAME
    {
        return Err(refused());
    }
    open_marker_data(key, layout, marker_frame)?;
    let length =
        4 + DECLARATION_TAG.len() + 2 + 3 * 36 + 8 + marker_frame.len() + catalog_frame.len();
    let mut e = Encoder::with_capacity(length);
    tag(&mut e, DECLARATION_TAG)?;
    put(&mut e, &layout.vault)?;
    put(&mut e, &layout.root)?;
    put(&mut e, &expected_catalog_id)?;
    put(&mut e, marker_frame)?;
    put(&mut e, catalog_frame)?;
    finish(e, MAX_DECLARATION_PLAIN)
}
pub(in crate::store) fn parse_declaration_seed_data<'a>(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    plain: &'a [u8],
) -> Result<DeclarationSeedData<'a>, AppError> {
    let mut d = input(plain, MAX_DECLARATION_PLAIN, DECLARATION_TAG)?;
    let actual = CurrentLayoutData {
        vault: digest(&mut d)?,
        root: digest(&mut d)?,
    };
    let expected_catalog_id = digest(&mut d)?;
    let marker_frame = field(&mut d, MAX_MARKER_FRAME)?;
    let catalog_frame = field(&mut d, MAX_CATALOG_FRAME)?;
    d.finish().map_err(|_| refused())?;
    // Every variable range/EOF precedes nested unframe/decrypt or copied metadata.
    if actual != layout || catalog_frame.len() <= 40 {
        return Err(refused());
    }
    open_marker_data(key, layout, marker_frame)?;
    Ok(DeclarationSeedData {
        expected_catalog_id,
        marker_frame,
        catalog_frame,
    })
}
pub(in crate::store) fn seal_declaration_seed_data(
    key: &[u8; 32],
    plain: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    seal_data(
        key,
        "catcoms/private-chat-v2-current-declaration-key/v1",
        plain,
        MAX_DECLARATION_PLAIN,
        rng,
    )
}
pub(in crate::store) fn open_declaration_seed_data(
    key: &[u8; 32],
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    open_data(
        key,
        "catcoms/private-chat-v2-current-declaration-key/v1",
        raw,
        MAX_DECLARATION_PLAIN,
    )
}

#[cfg(test)]
std::thread_local! {
    // Real nested opener entry, rather than a counter in the syntax-only test helper. A
    // malformed late outer field must refuse before nested ciphertext/plain allocation.
    static MARKER_OPEN_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
mod tests;
