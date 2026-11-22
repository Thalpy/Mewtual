//! Exact private records for possible Welcome entry and its completed result.
use super::*;
use catcoms_sync::{
    SealedV2JoinedOutcome, V2PendingJoinContextData, MAX_V2_FIRST_JOIN_DELIVERY_BYTES,
};

const ENTRY_TAG: &[u8] = b"catcoms/private-chat-v2-welcome-entry/v1\0";
const ENTRY_KEY: &str = "catcoms/private-chat-v2-welcome-entry-key/v1";
const OUTCOME_TAG: &[u8] = b"catcoms/private-chat-v2-welcome-outcome/v1\0";
const OUTCOME_KEY: &str = "catcoms/private-chat-v2-welcome-outcome-key/v1";
const VERSION: u16 = 1;
const SEALED_OVERHEAD: usize = 40;
const DIGESTS: usize = 5 * 36;
const MAX_OUTCOME_CIPHERTEXT: usize = 512 * 1024 + 4096;
pub(in crate::store) const ENTRY_FRAME: usize =
    4 + ENTRY_TAG.len() + 2 + DIGESTS + 4 + MAX_V2_FIRST_JOIN_DELIVERY_BYTES + SEALED_OVERHEAD;
pub(in crate::store) const OUTCOME_FRAME: usize =
    4 + OUTCOME_TAG.len() + 2 + DIGESTS + 4 + 24 + 4 + MAX_OUTCOME_CIPHERTEXT + SEALED_OVERHEAD;

fn context_fields(context: V2PendingJoinContextData, parent: &[u8]) -> [[u8; 32]; 5] {
    [
        context.vault,
        context.root,
        context.original_pending,
        context.pin,
        *blake3::hash(parent).as_bytes(),
    ]
}

fn seal_record(
    key: &[u8; 32],
    domain: &str,
    tag: &[u8],
    labels: [[u8; 32]; 5],
    fields: &[&[u8]],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let size =
        4 + tag.len() + 2 + DIGESTS + fields.iter().map(|field| 4 + field.len()).sum::<usize>();
    let mut e = Encoder::with_capacity(size);
    let bad = |_| refused();
    e.put_bytes(tag).map_err(bad)?;
    e.put_u16(VERSION);
    for field in labels {
        e.put_bytes(&field).map_err(bad)?;
    }
    for field in fields {
        e.put_bytes(field).map_err(bad)?;
    }
    let plain = Zeroizing::new(e.finish());
    let key = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&key, &plain, rng)?)))
}

fn open_record(
    key: &[u8; 32],
    domain: &str,
    frame_bytes: &[u8],
    cap: usize,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(SEALED_OVERHEAD + 1..=cap).contains(&frame_bytes.len()) {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(unseal(&key, &unframe(frame_bytes)?)?))
}

fn decoder<'a>(raw: &'a [u8], tag: &[u8], labels: [[u8; 32]; 5]) -> Result<Decoder<'a>, AppError> {
    let mut d = Decoder::new(raw);
    let bad = |_| refused();
    if d.get_bytes().map_err(bad)? != tag || d.get_u16().map_err(bad)? != VERSION {
        return Err(refused());
    }
    for label in labels {
        if d.get_bytes().map_err(bad)? != label {
            return Err(refused());
        }
    }
    Ok(d)
}

pub(in crate::store) fn seal_entry(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    pending: &[u8],
    delivery: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(1..=MAX_V2_FIRST_JOIN_DELIVERY_BYTES).contains(&delivery.len()) {
        return Err(refused());
    }
    seal_record(
        key,
        ENTRY_KEY,
        ENTRY_TAG,
        context_fields(context, pending),
        &[delivery],
        rng,
    )
}

pub(in crate::store) fn open_entry(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    pending: &[u8],
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let plain = open_record(key, ENTRY_KEY, raw, ENTRY_FRAME)?;
    let mut d = decoder(&plain, ENTRY_TAG, context_fields(context, pending))?;
    let bytes = d.get_bytes().map_err(|_| refused())?;
    d.finish().map_err(|_| refused())?;
    if !(1..=MAX_V2_FIRST_JOIN_DELIVERY_BYTES).contains(&bytes.len()) {
        return Err(refused());
    }
    Ok(Zeroizing::new(bytes.to_vec()))
}

pub(in crate::store) fn seal_outcome(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    entry: &[u8],
    result: &SealedV2JoinedOutcome,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let (nonce, bytes) = result.encrypted_parts();
    if bytes.len() > MAX_OUTCOME_CIPHERTEXT {
        return Err(refused());
    }
    seal_record(
        key,
        OUTCOME_KEY,
        OUTCOME_TAG,
        context_fields(context, entry),
        &[nonce, bytes],
        rng,
    )
}

pub(in crate::store) fn open_outcome(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    entry: &[u8],
    raw: &[u8],
) -> Result<SealedV2JoinedOutcome, AppError> {
    let plain = open_record(key, OUTCOME_KEY, raw, OUTCOME_FRAME)?;
    let mut d = decoder(&plain, OUTCOME_TAG, context_fields(context, entry))?;
    let nonce: [u8; 24] = d
        .get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())?;
    let bytes = d.get_bytes().map_err(|_| refused())?;
    d.finish().map_err(|_| refused())?;
    if bytes.len() > MAX_OUTCOME_CIPHERTEXT {
        return Err(refused());
    }
    SealedV2JoinedOutcome::from_encrypted_parts(nonce, bytes).map_err(|_| refused())
}
