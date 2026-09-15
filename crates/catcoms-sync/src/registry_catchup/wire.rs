//! Versioned canonical wire framing, separate from signatures and durable document admission.
use super::*;

// Version/bucket/id/count, 64 length-framed heads, optional seed and optional cursor.
pub(super) const MAX_QUERY: usize = 1 + 1 + 16 + 1 + 64 * 36 + 36 + 85;
// Studio replaces the single bucket byte with type:u16, channel:16 and object:16.
pub(super) const MAX_SCOPED_QUERY: usize = MAX_QUERY + 33;
// Version/status/count plus optional length-framed cursor. Signed envelope adds 108 bytes.
pub(super) const MAX_ANSWER: usize = MAX_REGISTRY_PAGE_BYTES + 3 + 85;
pub(super) const MAX_RESPONSE: usize = MAX_ANSWER + 108;

/// Non-owning query; repeat the original heads/seed when using a continuation. There is no
/// caller-supplied requester identity: the sync layer always signs with its actual device.
pub struct RegistryPageQuery<'a> {
    pub bucket: u8,
    pub doc_id: u128,
    pub heads: &'a [[u8; 32]],
    pub seed: Option<[u8; 32]>,
    pub cursor: Option<&'a [u8]>,
}
impl fmt::Debug for RegistryPageQuery<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegistryPageQuery")
            .field("heads", &self.heads.len())
            .field("continuation", &self.cursor.is_some())
            .finish_non_exhaustive()
    }
}

pub(super) struct OwnedQuery {
    pub scope: PageScope,
    pub doc_id: u128,
    heads: Vec<[u8; 32]>,
    seed: Option<[u8; 32]>,
    cursor: Option<RegistryPageCursor>,
}
impl OwnedQuery {
    pub(super) fn request(&self, requester: DeviceId) -> RegistryPageRequest<'_> {
        RegistryPageRequest {
            requester,
            doc_id: self.doc_id,
            heads: &self.heads,
            seed: self.seed,
            cursor: self.cursor.as_ref().map(RegistryPageCursor::as_bytes),
        }
    }
}

fn malformed<T>(result: Result<T, catcoms_wire::WireError>) -> Result<T, SyncError> {
    result.map_err(|_| SyncError::Malformed)
}

pub(super) fn encode_query(query: &RegistryPageQuery<'_>) -> Result<Vec<u8>, SyncError> {
    if query.heads.len() > MAX_REGISTRY_PAGE_HEADS
        || query.heads.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(SyncError::Malformed);
    }
    if let Some(cursor) = query.cursor {
        RegistryPageCursor::from_bytes(cursor)?;
    }
    encode_scoped_query(
        PageScope::Registry(query.bucket),
        query.doc_id,
        query.heads,
        query.seed,
        query.cursor,
    )
}

pub(super) fn encode_scoped_query(
    scope: PageScope,
    doc_id: u128,
    heads: &[[u8; 32]],
    seed: Option<[u8; 32]>,
    cursor: Option<&[u8]>,
) -> Result<Vec<u8>, SyncError> {
    if heads.len() > MAX_REGISTRY_PAGE_HEADS || heads.windows(2).any(|p| p[0] >= p[1]) {
        return Err(SyncError::Malformed);
    }
    if let Some(cursor) = cursor {
        RegistryPageCursor::from_bytes(cursor)?;
    }
    let query = RegistryPageQuery {
        bucket: 0,
        doc_id,
        heads,
        seed,
        cursor,
    };
    let mut e = Encoder::new();
    e.put_u8(1);
    match scope {
        PageScope::Registry(bucket) => {
            e.put_u8(bucket);
        }
        PageScope::Studio(target) => {
            e.put_u16(scope.doc_type().tag());
            // Fixed-width identifiers, not renderer-controlled strings.
            e.put_u128(u128::from_be_bytes(target.channel()));
            e.put_u128(match target {
                StudioTarget::Index { .. } => 0,
                StudioTarget::Flipnote { object, .. } => u128::from_be_bytes(object),
            });
        }
    }
    e.put_u128(query.doc_id);
    e.put_u8(query.heads.len() as u8);
    for head in query.heads {
        e.put_bytes(head).expect("fixed hash");
    }
    e.put_bytes(query.seed.as_ref().map_or(&[][..], |seed| seed.as_slice()))
        .expect("optional hash");
    e.put_bytes(query.cursor.unwrap_or(&[]))
        .expect("cursor bound");
    Ok(e.finish())
}

pub(super) fn decode_query(bytes: &[u8]) -> Result<OwnedQuery, SyncError> {
    decode_scoped_query(KIND_REGISTRY_PAGE, bytes)
}

pub(super) fn decode_scoped_query(kind: u8, bytes: &[u8]) -> Result<OwnedQuery, SyncError> {
    if bytes.len()
        > if kind == KIND_REGISTRY_PAGE {
            MAX_QUERY
        } else {
            MAX_SCOPED_QUERY
        }
    {
        return Err(SyncError::Malformed);
    }
    let mut d = Decoder::new(bytes);
    if malformed(d.get_u8())? != 1 {
        return Err(SyncError::Malformed);
    }
    let scope = match kind {
        KIND_REGISTRY_PAGE => PageScope::Registry(malformed(d.get_u8())?),
        KIND_STUDIO_PAGE => {
            let tag = malformed(d.get_u16())?;
            let channel = malformed(d.get_u128())?.to_be_bytes();
            let object = malformed(d.get_u128())?.to_be_bytes();
            PageScope::Studio(match tag {
                15 if object == [0; 16] => StudioTarget::Index { channel },
                16 => StudioTarget::Flipnote { channel, object },
                _ => return Err(SyncError::Malformed),
            })
        }
        _ => return Err(SyncError::Malformed),
    };
    let doc_id = malformed(d.get_u128())?;
    let count = malformed(d.get_u8())? as usize;
    if count > MAX_REGISTRY_PAGE_HEADS {
        return Err(SyncError::Malformed);
    }
    let mut heads = Vec::with_capacity(count);
    for _ in 0..count {
        heads.push(
            malformed(d.get_bytes())?
                .try_into()
                .map_err(|_| SyncError::Malformed)?,
        );
    }
    if heads.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(SyncError::Malformed);
    }
    let seed = match malformed(d.get_bytes())? {
        [] => None,
        bytes => Some(bytes.try_into().map_err(|_| SyncError::Malformed)?),
    };
    let cursor = match malformed(d.get_bytes())? {
        [] => None,
        bytes => Some(RegistryPageCursor::from_bytes(bytes)?),
    };
    malformed(d.finish())?;
    Ok(OwnedQuery {
        scope,
        doc_id,
        heads,
        seed,
        cursor,
    })
}

fn checked_op(
    bytes: &[u8],
    doc_type: DocType,
    doc_id: u128,
    epoch: u64,
) -> Result<SealedOp, SyncError> {
    if bytes.len() > MAX_SIGNED_EPOCH_OP_BYTES + 78 {
        return Err(SyncError::Malformed);
    }
    let op = SealedOp::decode(bytes)?;
    if op.doc_type != doc_type || op.doc_id != doc_id || op.epoch != epoch {
        return Err(SyncError::Malformed);
    }
    Ok(op)
}

pub(super) fn encode_answer(
    outcome: &RegistryPageOutcome,
    doc_id: u128,
    epoch: u64,
) -> Result<Vec<u8>, SyncError> {
    encode_scoped_answer(outcome, DocType::DocRegistry, doc_id, epoch)
}

pub(super) fn encode_scoped_answer(
    outcome: &RegistryPageOutcome,
    doc_type: DocType,
    doc_id: u128,
    epoch: u64,
) -> Result<Vec<u8>, SyncError> {
    let mut e = Encoder::new();
    e.put_u8(1);
    match outcome {
        RegistryPageOutcome::Restart => {
            e.put_u8(1);
        }
        RegistryPageOutcome::CheckpointRequired => {
            e.put_u8(2);
        }
        RegistryPageOutcome::HistoricalAuthorizationRequired => {
            e.put_u8(3);
        }
        RegistryPageOutcome::Page(page) => {
            if page.operations.len() > MAX_REGISTRY_PAGE_OPS
                || (page.operations.is_empty() && page.next.is_some())
            {
                return Err(SyncError::Malformed);
            }
            e.put_u8(0).put_u8(page.operations.len() as u8);
            let mut total = 0;
            for op in &page.operations {
                // Bound before encoding or cloning any caller-provided ciphertext.
                if op.blob.ciphertext.len() > MAX_SIGNED_EPOCH_OP_BYTES + 20 {
                    return Err(SyncError::Malformed);
                }
                let bytes = op.encode();
                total += bytes.len() + 4;
                if total > MAX_REGISTRY_PAGE_BYTES {
                    return Err(SyncError::Malformed);
                }
                checked_op(&bytes, doc_type, doc_id, epoch)?;
                e.put_bytes(&bytes).expect("bounded op");
            }
            e.put_bytes(
                page.next
                    .as_ref()
                    .map_or(&[][..], RegistryPageCursor::as_bytes),
            )
            .expect("cursor bound");
        }
    }
    Ok(e.finish())
}

pub(super) fn decode_answer(
    bytes: &[u8],
    doc_id: u128,
    epoch: u64,
) -> Result<RegistryPageOutcome, SyncError> {
    decode_scoped_answer(bytes, DocType::DocRegistry, doc_id, epoch)
}

pub(super) fn decode_scoped_answer(
    bytes: &[u8],
    doc_type: DocType,
    doc_id: u128,
    epoch: u64,
) -> Result<RegistryPageOutcome, SyncError> {
    if bytes.len() > MAX_ANSWER {
        return Err(SyncError::Malformed);
    }
    let mut d = Decoder::new(bytes);
    if malformed(d.get_u8())? != 1 {
        return Err(SyncError::Malformed);
    }
    let outcome = match malformed(d.get_u8())? {
        1 => RegistryPageOutcome::Restart,
        2 => RegistryPageOutcome::CheckpointRequired,
        3 => RegistryPageOutcome::HistoricalAuthorizationRequired,
        0 => {
            let count = malformed(d.get_u8())? as usize;
            if count > MAX_REGISTRY_PAGE_OPS {
                return Err(SyncError::Malformed);
            }
            let mut operations = Vec::with_capacity(count);
            let mut total = 0;
            for _ in 0..count {
                let bytes = malformed(d.get_bytes())?;
                total += bytes.len() + 4;
                if total > MAX_REGISTRY_PAGE_BYTES {
                    return Err(SyncError::Malformed);
                }
                operations.push(checked_op(bytes, doc_type, doc_id, epoch)?);
            }
            let next = match malformed(d.get_bytes())? {
                [] => None,
                bytes => Some(RegistryPageCursor::from_bytes(bytes)?),
            };
            if count == 0 && next.is_some() {
                return Err(SyncError::Malformed);
            }
            RegistryPageOutcome::Page(RegistryOpPage { operations, next })
        }
        _ => return Err(SyncError::Malformed),
    };
    malformed(d.finish())?;
    Ok(outcome)
}

/// The transport already buffered a globally capped frame. This is a post-frame, pre-copy and
/// pre-signature cap, not a streaming allocation bound. Strictly borrow the untrusted payload.
pub(super) fn decode_response(bytes: &[u8]) -> Result<SignedPageResponse<'_>, SyncError> {
    if bytes.len() > MAX_RESPONSE {
        return Err(SyncError::Malformed);
    }
    let mut d = Decoder::new(bytes);
    let key = malformed(d.get_bytes())?;
    if key.len() != 32 {
        return Err(SyncError::Malformed);
    }
    let signature = malformed(d.get_bytes())?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let answer = malformed(d.get_bytes())?;
    if answer.len() > MAX_ANSWER {
        return Err(SyncError::Malformed);
    }
    malformed(d.finish())?;
    Ok((key, signature, answer))
}

type SignedPageResponse<'a> = (&'a [u8], [u8; 64], &'a [u8]);
