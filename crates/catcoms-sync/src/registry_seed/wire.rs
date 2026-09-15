//! Additive kind-22 framing. The expected hash is an Automerge change hash, not a fileshare CID.
//! The signed outer response binds the whole query; its body is padded inside the group AEAD.
use super::*;

pub(super) const MAX_QUERY: usize = 256;
// Nonce/ciphertext length framing, nonce, tag and authenticated padding footer. Above the
// 1-MiB ladder the body stays unpadded. Reuse encode_sealed's framing rather than inventing one.
pub(super) const MAX_SEALED: usize = MAX_CHECKPOINT_BYTES + 8 + 24 + 16 + 4;
pub(super) const MAX_RESPONSE: usize = MAX_SEALED + 108;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Query {
    pub bucket: u8,
    pub doc_id: u128,
    pub hash: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ScopedQuery {
    pub target: CheckpointTarget,
    pub doc_id: u128,
    pub hash: [u8; 32],
}
fn wire<V>(value: Result<V, catcoms_wire::WireError>) -> Result<V, SyncError> {
    value.map_err(|_| SyncError::Malformed)
}
pub(super) fn encode_query(q: &Query, group: &[u8]) -> Result<Vec<u8>, SyncError> {
    let document = registry_document(group, q.bucket)?;
    let mut e = Encoder::new();
    e.put_u8(1).put_u16(DocType::DocRegistry.tag());
    wire(e.put_bytes(&document.logical_key))?;
    e.put_u128(q.doc_id);
    wire(e.put_bytes(&q.hash))?;
    Ok(e.finish())
}
pub(super) fn decode_query(bytes: &[u8], group: &[u8]) -> Result<Query, SyncError> {
    if bytes.len() > MAX_QUERY {
        return Err(SyncError::Malformed);
    }
    let mut d = Decoder::new(bytes);
    if wire(d.get_u8())? != 1 || wire(d.get_u16())? != DocType::DocRegistry.tag() {
        return Err(SyncError::Malformed);
    }
    let logical_key = wire(d.get_bytes())?;
    if logical_key.len() != 32 {
        return Err(SyncError::Malformed);
    }
    let doc_id = wire(d.get_u128())?;
    let hash = wire(d.get_bytes())?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    wire(d.finish())?;
    // Fixed 256-key search, after authentication/global admission, never an untrusted disk path.
    let bucket = (0..=255u8)
        .find(|b| registry_document(group, *b).is_ok_and(|doc| doc.logical_key == logical_key))
        .ok_or(SyncError::NoSuchDoc)?;
    Ok(Query {
        bucket,
        doc_id,
        hash,
    })
}

pub(super) fn encode_scoped_query(q: &ScopedQuery, group: &[u8]) -> Result<Vec<u8>, SyncError> {
    match q.target {
        CheckpointTarget::Registry(bucket) => encode_query(
            &Query {
                bucket,
                doc_id: q.doc_id,
                hash: q.hash,
            },
            group,
        ),
        CheckpointTarget::Studio(target) => {
            let mut e = Encoder::new();
            e.put_u8(1).put_u16(q.target.doc_type().tag());
            wire(e.put_bytes(&target.channel()))?;
            let object = match target {
                catcoms_replication::studio::StudioTarget::Index { .. } => [0; 16],
                catcoms_replication::studio::StudioTarget::Flipnote { object, .. } => object,
            };
            wire(e.put_bytes(&object))?;
            e.put_u128(q.doc_id);
            wire(e.put_bytes(&q.hash))?;
            Ok(e.finish())
        }
    }
}
pub(super) fn decode_scoped_query(
    kind: u8,
    bytes: &[u8],
    group: &[u8],
) -> Result<ScopedQuery, SyncError> {
    if kind == KIND_REGISTRY_SEED {
        let q = decode_query(bytes, group)?;
        return Ok(ScopedQuery {
            target: CheckpointTarget::Registry(q.bucket),
            doc_id: q.doc_id,
            hash: q.hash,
        });
    }
    if kind != KIND_STUDIO_SEED || bytes.len() > MAX_QUERY {
        return Err(SyncError::Malformed);
    }
    use catcoms_replication::studio::StudioTarget;
    let mut d = Decoder::new(bytes);
    if wire(d.get_u8())? != 1 {
        return Err(SyncError::Malformed);
    }
    let tag = wire(d.get_u16())?;
    let channel: [u8; 16] = wire(d.get_bytes())?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let object: [u8; 16] = wire(d.get_bytes())?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let doc_id = wire(d.get_u128())?;
    let hash = wire(d.get_bytes())?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    wire(d.finish())?;
    let target = match tag {
        15 if object == [0; 16] => StudioTarget::Index { channel },
        16 => StudioTarget::Flipnote { channel, object },
        _ => return Err(SyncError::Malformed),
    };
    Ok(ScopedQuery {
        target: CheckpointTarget::Studio(target),
        doc_id,
        hash,
    })
}

pub(super) fn seal_seed(
    seed: &[u8],
    key: &[u8; 32],
    rng: &mut impl CryptoRngCore,
) -> Result<Vec<u8>, SyncError> {
    if seed.is_empty() || seed.len() > MAX_CHECKPOINT_BYTES {
        return Err(SyncError::Malformed);
    }
    let padded = Zeroizing::new(pad::pad(seed, OP_PAD_FLOOR, OP_PAD_CEILING)?);
    Ok(encode_sealed(
        &seal(key, &padded, rng).map_err(|_| SyncError::Malformed)?,
    ))
}
pub(super) fn open_seed(bytes: &[u8], key: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>, SyncError> {
    if bytes.len() > MAX_SEALED {
        return Err(SyncError::Malformed);
    }
    let plain =
        Zeroizing::new(unseal(key, &decode_sealed(bytes)?).map_err(|_| SyncError::Malformed)?);
    let raw = pad::unpad(&plain, OP_PAD_FLOOR, OP_PAD_CEILING)?;
    if raw.is_empty() || raw.len() > MAX_CHECKPOINT_BYTES {
        return Err(SyncError::Malformed);
    }
    Ok(Zeroizing::new(raw.to_vec()))
}

type SignedSeedResponse<'a> = (&'a [u8], [u8; 64], &'a [u8]);
pub(super) fn decode_response(bytes: &[u8]) -> Result<SignedSeedResponse<'_>, SyncError> {
    // The transport has buffered its own bounded frame; cap before copying ciphertext or crypto.
    if bytes.len() > MAX_RESPONSE {
        return Err(SyncError::Malformed);
    }
    let mut d = Decoder::new(bytes);
    let key = wire(d.get_bytes())?;
    if key.len() != 32 {
        return Err(SyncError::Malformed);
    }
    let signature = wire(d.get_bytes())?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let body = wire(d.get_bytes())?;
    if body.len() > MAX_SEALED {
        return Err(SyncError::Malformed);
    }
    wire(d.finish())?;
    Ok((key, signature, body))
}
