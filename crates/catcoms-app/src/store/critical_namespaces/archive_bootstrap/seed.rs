//! Exact private declaration repair input. Authenticating this DATA does not admit a source:
//! the caller still proves the real complete parent and both fixed catalog destinations.
use super::*;

const SEED_TAG: &str = "catcoms/private-chat-archive-declaration/v1";
const SEED_KEY: &str = "catcoms/private-chat-archive-declaration-key/v1";
const MARKER_TAG: &str = "catcoms/private-chat-archive-root/v1";
const MARKER_KEY: &str = "catcoms/private-chat-archive-root-key/v1";

pub(super) struct SeedData<'a> {
    pub(super) descriptor: Descriptor,
    pub(super) expected_plain: [u8; 32],
    pub(super) expected_frame: [u8; 32],
    pub(super) candidate: &'a [u8],
    pub(super) marker: &'a [u8],
}

pub(super) fn encode_marker(descriptor: &Descriptor) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let encoded = descriptor.encode()?;
    let mut output = Encoder::with_capacity(MAX_MARKER_FRAMED - 40);
    output.put_str(MARKER_TAG).map_err(|_| refused())?;
    output.put_bytes(&encoded).map_err(|_| refused())?;
    let plain = Zeroizing::new(output.finish());
    if plain.len() > MAX_MARKER_FRAMED - 40 {
        return Err(refused());
    }
    Ok(plain)
}

pub(super) fn encode_seed(
    descriptor: &Descriptor,
    expected: &super::super::pending_join_codec::Summary,
    candidate: &[u8],
    marker: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if candidate.is_empty()
        || candidate.len() > super::super::pending_join_codec::MAX_CATALOG_FRAMED
        || marker.is_empty()
        || marker.len() > MAX_MARKER_FRAMED
        || expected.archive.is_some()
    {
        return Err(refused());
    }
    let encoded = descriptor.encode()?;
    let mut output = Encoder::with_capacity(MAX_SEED_FRAMED - 40);
    output.put_str(SEED_TAG).map_err(|_| refused())?;
    output.put_bytes(&encoded).map_err(|_| refused())?;
    output.put_bytes(&expected.digest).map_err(|_| refused())?;
    output.put_bytes(&expected.framed).map_err(|_| refused())?;
    output.put_bytes(candidate).map_err(|_| refused())?;
    output.put_bytes(marker).map_err(|_| refused())?;
    let plain = Zeroizing::new(output.finish());
    if plain.len() > MAX_SEED_FRAMED - 40 {
        return Err(refused());
    }
    Ok(plain)
}

fn seal_domain(
    key: &[u8; 32],
    domain: &'static str,
    plain: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&derived, plain, rng)?)))
}
pub(super) fn seal_marker(
    key: &[u8; 32],
    plain: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if plain.len() > MAX_MARKER_FRAMED - 40 {
        return Err(refused());
    }
    seal_domain(key, MARKER_KEY, plain, rng)
}
pub(super) fn seal_seed(
    key: &[u8; 32],
    plain: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if plain.len() > MAX_SEED_FRAMED - 40 {
        return Err(refused());
    }
    seal_domain(key, SEED_KEY, plain, rng)
}

pub(super) fn open_seed_data(key: &[u8; 32], raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if raw.len() < 40 || raw.len() > MAX_SEED_FRAMED {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(SEED_KEY, key));
    Ok(Zeroizing::new(unseal(&derived, &unframe(raw)?)?))
}

pub(super) fn parse_seed_data<'a>(
    plain: &'a [u8],
    vault: &[u8; 32],
) -> Result<SeedData<'a>, AppError> {
    if plain.len() > MAX_SEED_FRAMED - 40 {
        return Err(refused());
    }
    let mut input = Decoder::new(plain);
    if input.get_str().map_err(|_| refused())? != SEED_TAG {
        return Err(refused());
    }
    let descriptor = Descriptor::parse(input.get_bytes().map_err(|_| refused())?, vault)?;
    let expected_plain = input
        .get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())?;
    let expected_frame = input
        .get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())?;
    let candidate = input.get_bytes().map_err(|_| refused())?;
    let marker = input.get_bytes().map_err(|_| refused())?;
    input.finish().map_err(|_| refused())?;
    if candidate.is_empty()
        || candidate.len() > super::super::pending_join_codec::MAX_CATALOG_FRAMED
        || marker.is_empty()
        || marker.len() > MAX_MARKER_FRAMED
    {
        return Err(refused());
    }
    Ok(SeedData {
        descriptor,
        expected_plain,
        expected_frame,
        candidate,
        marker,
    })
}

pub(super) fn open_marker_data(
    key: &[u8; 32],
    descriptor: &Descriptor,
    raw: &[u8],
) -> Result<(), AppError> {
    if open_marker_descriptor_data(key, &descriptor.vault, raw)? != *descriptor {
        return Err(refused());
    }
    Ok(())
}

/// Read authenticated marker DATA before any mounted archive/Current opener.
/// The actual vault domain is external to the marker; no root/source, SDK or
/// mounted capability is created by decoding its descriptor.
pub(super) fn open_marker_descriptor_data(
    key: &[u8; 32],
    vault: &[u8; 32],
    raw: &[u8],
) -> Result<Descriptor, AppError> {
    if raw.len() < 40 || raw.len() > MAX_MARKER_FRAMED {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(MARKER_KEY, key));
    let plain = Zeroizing::new(unseal(&derived, &unframe(raw)?)?);
    let mut input = Decoder::new(&plain);
    if input.get_str().map_err(|_| refused())? != MARKER_TAG {
        return Err(refused());
    }
    let descriptor = Descriptor::parse(input.get_bytes().map_err(|_| refused())?, vault)?;
    input.finish().map_err(|_| refused())?;
    Ok(descriptor)
}
