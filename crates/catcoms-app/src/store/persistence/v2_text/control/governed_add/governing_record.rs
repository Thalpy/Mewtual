//! The signed Admission and exact state image for the covered Raw Header.
use super::header_signing::RAW_BYTES;
use super::*;
use catcoms_replication::chat::{
    cold_records::{encode_header_row, HeaderRow, MAX_HEADER_ROW_BYTES},
    owner_header::{HeaderKind, SignedOwnerHeader},
    owner_profile::{AdmissionBody, AdmissionOrigin, MAX_ADMISSION_BYTES},
};

pub(super) const NAME: &str = "text.add.governing.v2";
const TAG: &[u8] = b"catcoms/private-chat-text-governed-add-record/v2\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-record-key/v2";
const RAW_DOMAIN: &str = "catcoms/private-chat-text-governed-add-record-parent/v2";
const VERSION: u16 = 2;
pub(super) const MIN_BYTES: usize = 4 + TAG.len() + 2 + 36 + 2 * 4 + 40;
pub(super) const MAX_BYTES: usize = MIN_BYTES + MAX_HEADER_ROW_BYTES + MAX_ADMISSION_BYTES;

fn raw_commitment(raw: &[u8]) -> Result<[u8; 32], AppError> {
    if raw.len() != RAW_BYTES {
        return Err(refused());
    }
    Ok(blake3::derive_key(RAW_DOMAIN, raw))
}

/// Encode the existing public history row and Add body under the exact Raw.
/// The caller pays codec scratch before calling and supplies its original data.
pub(super) fn seal_data(
    key: &[u8; 32],
    raw: &[u8],
    header: &SignedOwnerHeader,
    admission: &AdmissionBody,
    image: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let parent = raw_commitment(raw)?;
    require_links(header, admission)?;
    let header_row = encode_header_row(header, image).map_err(|_| refused())?;
    let body = admission.encode();
    let mut e = Encoder::with_capacity(MIN_BYTES - 40 + header_row.len() + body.len());
    put(&mut e, TAG)?;
    e.put_u16(VERSION);
    put(&mut e, &parent)?;
    put(&mut e, &header_row)?;
    put(&mut e, &body)?;
    let plain = Zeroizing::new(e.finish());
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let result = Zeroizing::new(frame(&seal(&key, &plain, rng)?));
    if result.len() > MAX_BYTES {
        return Err(refused());
    }
    Ok(result)
}

/// Bind the decoded history row to the already authenticated original Outcome
/// and exact Raw signature. Current still selects the actual governing prefix.
pub(super) fn open_data(
    key: &[u8; 32],
    raw: &[u8],
    frame: &[u8],
    signature: &[u8; 64],
    [body, preimage, image]: [&[u8]; 3],
) -> Result<SignedOwnerHeader, AppError> {
    if !(MIN_BYTES..=MAX_BYTES).contains(&frame.len()) {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let plain = Zeroizing::new(unseal(&key, &unframe(frame)?)?);
    let mut d = Decoder::new(&plain);
    if field(&mut d, TAG.len())? != TAG
        || d.get_u16().map_err(|_| refused())? != VERSION
        || b32(&mut d)? != raw_commitment(raw)?
    {
        return Err(refused());
    }
    let row = field(&mut d, MAX_HEADER_ROW_BYTES)?;
    let admission = field(&mut d, MAX_ADMISSION_BYTES)?;
    d.finish().map_err(|_| refused())?;
    let row = HeaderRow::decode(row).map_err(|_| refused())?;
    if admission != body || row.image() != image {
        return Err(refused());
    }
    let admission = AdmissionBody::decode(admission).map_err(|_| refused())?;
    require_links(row.header(), &admission)?;
    require_original_signature(row.header(), signature, preimage)?;
    Ok(row.header().clone())
}

pub(super) fn require_original_signature(
    header: &SignedOwnerHeader,
    signature: &[u8; 64],
    preimage: &[u8],
) -> Result<(), AppError> {
    let recovered = SignedOwnerHeader::sign_using(header.spec().clone(), |bytes| {
        if bytes != preimage {
            return Err(catcoms_replication::chat::ChatError::Dependency);
        }
        Ok(*signature)
    })
    .map_err(|_| refused())?;
    if &recovered != header {
        return Err(refused());
    }
    Ok(())
}

fn require_links(header: &SignedOwnerHeader, admission: &AdmissionBody) -> Result<(), AppError> {
    let spec = header.spec();
    let body = admission.spec();
    if spec.kind != HeaderKind::Admission
        || !matches!(body.origin, AdmissionOrigin::Add { .. })
        || spec.body != admission.id()
        || spec.group != body.group
        || spec.profile != body.profile
    {
        return Err(refused());
    }
    Ok(())
}
