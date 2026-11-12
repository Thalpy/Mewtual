//! Kind-separated declaration/marker DATA for the private offline Current3→4 migration.
//! A complete authenticated seed retains BOTH whole catalog frames even after promotion.
use super::*;
use crate::store::{frame, unframe};
use catcoms_crypto::keystore::{seal, unseal};
use catcoms_wire::{Decoder, Encoder};

pub(in crate::store) const MAX_MARKER_PLAIN: usize = 1024;
pub(in crate::store) const MAX_MARKER_FRAME: usize = MAX_MARKER_PLAIN + 40;
pub(in crate::store) const MAX_CATALOG_FRAME: usize =
    super::super::pending_join_codec::MAX_CATALOG_FRAMED;
pub(in crate::store) const MAX_SEED_PLAIN: usize = 2 * MAX_CATALOG_FRAME + MAX_MARKER_FRAME + 1024;
pub(in crate::store) const MAX_SEED_FRAME: usize = MAX_SEED_PLAIN + 40;
// Independent logical class, not the old Current3 allowance. It includes complete two-frame
// seed/raw/unframe/plain/copy/seal overlap. Wrapper/marker metadata is separately funded.
pub(in crate::store) const DECLARATION_FEE: usize = 12 * MAX_SEED_FRAME + 512 * 1024;
pub(in crate::store) const PERSISTENT_MINIMUM: usize =
    2 * MAX_CATALOG_FRAME + MAX_SEED_FRAME + MAX_MARKER_FRAME;
const MARKER_TAG: &[u8] = b"catcoms/private-chat-v2-governance-marker/v1\0";
const SEED_TAG: &[u8] = b"catcoms/private-chat-v2-governance-declaration/v1\0";
const MARKER_KEY: &str = "catcoms/private-chat-v2-governance-marker-key/v1";
const SEED_KEY: &str = "catcoms/private-chat-v2-governance-declaration-key/v1";
const VERSION: u16 = 1;

fn put(e: &mut Encoder, raw: &[u8]) -> Result<(), AppError> {
    e.put_bytes(raw).map_err(|_| refused())?;
    Ok(())
}
fn tag(e: &mut Encoder, raw: &[u8]) -> Result<(), AppError> {
    put(e, raw)?;
    e.put_u16(VERSION);
    Ok(())
}
fn field<'a>(d: &mut Decoder<'a>, cap: usize) -> Result<&'a [u8], AppError> {
    let raw = d.get_bytes().map_err(|_| refused())?;
    if raw.len() > cap {
        return Err(refused());
    }
    Ok(raw)
}
fn input<'a>(raw: &'a [u8], cap: usize, expected: &[u8]) -> Result<Decoder<'a>, AppError> {
    if raw.is_empty() || raw.len() > cap {
        return Err(refused());
    }
    let mut d = Decoder::new(raw);
    if field(&mut d, expected.len())? != expected || d.get_u16().map_err(|_| refused())? != VERSION
    {
        return Err(refused());
    }
    Ok(d)
}
fn seal_data(
    key: &[u8; 32],
    domain: &str,
    plain: &[u8],
    cap: usize,
    rng: &mut impl CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if plain.is_empty() || plain.len() > cap {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&derived, plain, rng)?)))
}
fn open_data(
    key: &[u8; 32],
    domain: &str,
    raw: &[u8],
    cap: usize,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if raw.len() <= 40 || raw.len() > cap + 40 {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    let plain = Zeroizing::new(unseal(&derived, &unframe(raw)?)?);
    if plain.is_empty() || plain.len() > cap {
        return Err(refused());
    }
    Ok(plain)
}
pub(in crate::store) fn seal_marker(
    key: &[u8; 32],
    capability: GovernanceCapabilityData,
    rng: &mut impl CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let descriptor = capability.encode_data()?;
    let mut e = Encoder::with_capacity(MAX_MARKER_PLAIN);
    tag(&mut e, MARKER_TAG)?;
    put(&mut e, &descriptor)?;
    seal_data(key, MARKER_KEY, &e.finish(), MAX_MARKER_PLAIN, rng)
}
pub(in crate::store) fn require_marker(
    key: &[u8; 32],
    capability: GovernanceCapabilityData,
    raw: &[u8],
) -> Result<(), AppError> {
    let plain = open_data(key, MARKER_KEY, raw, MAX_MARKER_PLAIN)?;
    let mut d = input(&plain, MAX_MARKER_PLAIN, MARKER_TAG)?;
    let descriptor = field(&mut d, 512)?;
    if descriptor != capability.encode_data()?.as_slice() {
        return Err(refused());
    }
    d.finish().map_err(|_| refused())
}

pub(in crate::store) struct OpenedSeedData<'a> {
    pub(in crate::store) capability: GovernanceCapabilityData,
    pub(in crate::store) original: &'a [u8],
    pub(in crate::store) candidate: &'a [u8],
    pub(in crate::store) marker: &'a [u8],
}

fn check_catalog_pair(
    key: &[u8; 32],
    domain: &[u8; 32],
    original: &[u8],
    candidate: &[u8],
) -> Result<GovernanceCapabilityData, AppError> {
    if original.len() <= 40
        || original.len() > MAX_CATALOG_FRAME
        || candidate.len() <= 40
        || candidate.len() > MAX_CATALOG_FRAME
    {
        return Err(refused());
    }
    let old_plain = Zeroizing::new(unseal(key, &unframe(original)?)?);
    let new_plain = Zeroizing::new(unseal(key, &unframe(candidate)?)?);
    super::super::governance_namespace_codec::validate_declaration_pair_data(
        &old_plain, &new_plain, domain,
    )?;
    GovernanceCapabilityData::from_original_catalog_data(&old_plain, domain)
}
pub(in crate::store) fn seal_seed(
    key: &[u8; 32],
    domain: &[u8; 32],
    original: &[u8],
    candidate: &[u8],
    marker: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let capability = check_catalog_pair(key, domain, original, candidate)?;
    require_marker(key, capability, marker)?;
    let mut e = Encoder::with_capacity(MAX_SEED_PLAIN);
    tag(&mut e, SEED_TAG)?;
    put(&mut e, &capability.encode_data()?)?;
    put(&mut e, original)?;
    put(&mut e, candidate)?;
    put(&mut e, marker)?;
    seal_data(key, SEED_KEY, &e.finish(), MAX_SEED_PLAIN, rng)
}
pub(in crate::store) fn open_seed(
    key: &[u8; 32],
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    open_data(key, SEED_KEY, raw, MAX_SEED_PLAIN)
}
pub(in crate::store) fn parse_seed<'a>(
    key: &[u8; 32],
    domain: &[u8; 32],
    plain: &'a [u8],
) -> Result<OpenedSeedData<'a>, AppError> {
    let mut d = input(plain, MAX_SEED_PLAIN, SEED_TAG)?;
    let descriptor = field(&mut d, 512)?;
    let original = field(&mut d, MAX_CATALOG_FRAME)?;
    let candidate = field(&mut d, MAX_CATALOG_FRAME)?;
    let marker = field(&mut d, MAX_MARKER_FRAME)?;
    d.finish().map_err(|_| refused())?;
    let capability = check_catalog_pair(key, domain, original, candidate)?;
    if descriptor != capability.encode_data()?.as_slice() {
        return Err(refused());
    }
    require_marker(key, capability, marker)?;
    Ok(OpenedSeedData {
        capability,
        original,
        candidate,
        marker,
    })
}
