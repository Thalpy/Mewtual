//! Canonical bounded keyed-head framing. Parsing a historical receipt is not authorizing it.
use super::*;
use catcoms_replication::{epoch::MAX_RECEIPT_BYTES, ReceiptRepair};

pub(super) const MAX_QUERY: usize = 256;
// W-1: a counted report of exactly zero or two receipts, after the unchanged v1 header fields.
const MAX_REPORT: usize = 1 + 2 * (4 + MAX_RECEIPT_BYTES);
pub(super) const MAX_QUERY_V2: usize = MAX_QUERY + MAX_REPORT;
pub(super) const MAX_ANSWER: usize = 1 + 3 * (4 + MAX_RECEIPT_BYTES);
const MAX_RESPONSE: usize = MAX_ANSWER + 108;

/// Checked query-bound transport answer. Receipt/repair without `proof` are provisional hints;
/// the public values remain uninstalled. A provider sends a repair only once it is durably
/// applied locally; the receiver verifies it under its own observed tenure before any use.
#[derive(Clone, PartialEq, Eq)]
pub struct ReceiptHeadAnswer {
    pub receipt: Option<Receipt>,
    pub repair: Option<ReceiptRepair>,
    pub proof: Option<ReceiptHeadProof>,
}
impl fmt::Debug for ReceiptHeadAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReceiptHeadAnswer")
            .field("receipt", &self.receipt.is_some())
            .field("repair", &self.repair.is_some())
            .field("proof", &self.proof.is_some())
            .finish()
    }
}
fn wire<T>(value: Result<T, catcoms_wire::WireError>) -> Result<T, SyncError> {
    value.map_err(|_| SyncError::Malformed)
}
pub(super) fn encode_query(
    document: &LogicalDocument,
    nonce: [u8; 16],
) -> Result<Vec<u8>, SyncError> {
    if document.doc_type != DocType::DocRegistry || document.logical_key.len() != 32 {
        return Err(SyncError::Malformed);
    }
    let mut e = Encoder::new();
    e.put_u8(1).put_u16(document.doc_type.tag());
    wire(e.put_bytes(&document.logical_key))?;
    wire(e.put_bytes(&nonce))?;
    Ok(e.finish())
}
pub(super) fn decode_query(
    bytes: &[u8],
    server: &[u8],
) -> Result<(LogicalDocument, [u8; 16]), SyncError> {
    if bytes.len() > MAX_QUERY {
        return Err(SyncError::Malformed);
    }
    let mut d = Decoder::new(bytes);
    if wire(d.get_u8())? != 1 || wire(d.get_u16())? != DocType::DocRegistry.tag() {
        return Err(SyncError::Malformed);
    }
    let key = wire(d.get_bytes())?;
    if key.len() != 32 {
        return Err(SyncError::Malformed);
    }
    let nonce = wire(d.get_bytes())?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    wire(d.finish())?;
    Ok((
        LogicalDocument::new(server.to_vec(), DocType::DocRegistry, key.to_vec())?,
        nonce,
    ))
}

// Studio queries include the channel even for an object whose logical key is the object id.
// Exact lengths/zero index object prevent alternate encodings and cross-channel confusion.
// Without a report the bytes are exactly v1; with one they are v2, whose leading byte a v1
// decoder already rejects, so an older provider simply never sees a report.
pub(super) fn encode_scoped_query(
    target: CheckpointTarget,
    group: &[u8],
    nonce: [u8; 16],
    report: Option<&[Receipt; 2]>,
) -> Result<Vec<u8>, SyncError> {
    let Some(report) = report else {
        return encode_scoped_query_v1(target, group, nonce);
    };
    let document = target.document(group)?;
    if report.iter().any(|r| r.document != document) {
        return Err(SyncError::Malformed);
    }
    let mut v1 = encode_scoped_query_v1(target, group, nonce)?;
    v1[0] = 2;
    let mut e = Encoder::new();
    e.put_u8(2);
    for receipt in report {
        let bytes = receipt.encode();
        if bytes.len() > MAX_RECEIPT_BYTES {
            return Err(SyncError::Malformed);
        }
        wire(e.put_bytes(&bytes))?;
    }
    v1.extend_from_slice(&e.finish());
    if v1.len() > MAX_QUERY_V2 {
        return Err(SyncError::Malformed);
    }
    Ok(v1)
}

fn encode_scoped_query_v1(
    target: CheckpointTarget,
    group: &[u8],
    nonce: [u8; 16],
) -> Result<Vec<u8>, SyncError> {
    match target {
        CheckpointTarget::Registry(_) => encode_query(&target.document(group)?, nonce),
        CheckpointTarget::Studio(target) => {
            use catcoms_replication::studio::StudioTarget;
            let mut e = Encoder::new();
            e.put_u8(1);
            match target {
                StudioTarget::Index { channel } => {
                    e.put_u16(DocType::StudioIndex.tag());
                    wire(e.put_bytes(&channel))?;
                    wire(e.put_bytes(&[0; 16]))?;
                }
                StudioTarget::Flipnote { channel, object } => {
                    e.put_u16(DocType::StudioObject.tag());
                    wire(e.put_bytes(&channel))?;
                    wire(e.put_bytes(&object))?;
                }
            }
            wire(e.put_bytes(&nonce))?;
            Ok(e.finish())
        }
    }
}
/// A decoded scoped query header: target, nonce and the still-opaque v2 report section.
pub(super) type ScopedQuery = (CheckpointTarget, [u8; 16], Option<Vec<u8>>);

/// Header-only decode. A v2 report section is returned as bounded opaque bytes and is not
/// parsed here: the caller decodes it only after the requester has paid its own rail.
pub(super) fn decode_scoped_query(
    kind: u8,
    bytes: &[u8],
    group: &[u8],
) -> Result<ScopedQuery, SyncError> {
    if bytes.first() != Some(&2) {
        let (target, nonce) = decode_scoped_query_v1(kind, bytes, group)?;
        return Ok((target, nonce, None));
    }
    if bytes.len() > MAX_QUERY_V2 {
        return Err(SyncError::Malformed);
    }
    // Walk exactly the v1 field layout to find where the report begins, then validate those
    // header bytes through the unchanged v1 decoder so v2 admits no alternate header encoding.
    let mut d = Decoder::new(bytes);
    wire(d.get_u8())?;
    wire(d.get_u16())?;
    wire(d.get_bytes())?;
    if kind == KIND_STUDIO_HEAD {
        wire(d.get_bytes())?;
    }
    wire(d.get_bytes())?;
    let header_end = bytes.len() - d.remaining();
    let report = &bytes[header_end..];
    if report.is_empty() || report.len() > MAX_REPORT {
        return Err(SyncError::Malformed);
    }
    let mut header = bytes[..header_end].to_vec();
    header[0] = 1;
    let (target, nonce) = decode_scoped_query_v1(kind, &header, group)?;
    Ok((target, nonce, Some(report.to_vec())))
}

/// Admission-time decode of a v2 report: exactly zero or two canonical receipts for exactly this
/// document. A count of one is malformed: the pair is always complete. Decoding grants nothing.
pub(super) fn decode_fault_report(
    bytes: &[u8],
    document: &LogicalDocument,
) -> Result<Option<[Receipt; 2]>, SyncError> {
    let mut d = Decoder::new(bytes);
    let report = match wire(d.get_u8())? {
        0 => None,
        2 => {
            let mut receipt = || {
                let bytes = wire(d.get_bytes())?;
                if bytes.len() > MAX_RECEIPT_BYTES {
                    return Err(SyncError::Malformed);
                }
                Ok::<_, SyncError>(Receipt::decode(bytes)?)
            };
            let pair = [receipt()?, receipt()?];
            if pair.iter().any(|r| &r.document != document) {
                return Err(SyncError::Malformed);
            }
            Some(pair)
        }
        _ => return Err(SyncError::Malformed),
    };
    wire(d.finish())?;
    Ok(report)
}

fn decode_scoped_query_v1(
    kind: u8,
    bytes: &[u8],
    group: &[u8],
) -> Result<(CheckpointTarget, [u8; 16]), SyncError> {
    if kind == KIND_RECEIPT_HEAD {
        let (document, nonce) = decode_query(bytes, group)?;
        let bucket = (0..=255u8)
            .find(|b| registry_document(group, *b).is_ok_and(|d| d == document))
            .ok_or(SyncError::NoSuchDoc)?;
        return Ok((CheckpointTarget::Registry(bucket), nonce));
    }
    if kind != KIND_STUDIO_HEAD || bytes.len() > MAX_QUERY {
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
    let nonce = wire(d.get_bytes())?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    wire(d.finish())?;
    let target = match tag {
        15 if object == [0; 16] => StudioTarget::Index { channel },
        16 => StudioTarget::Flipnote { channel, object },
        _ => return Err(SyncError::Malformed),
    };
    Ok((CheckpointTarget::Studio(target), nonce))
}

pub(super) fn encode_answer(
    answer: &ReceiptHeadAnswer,
    document: &LogicalDocument,
) -> Result<Vec<u8>, SyncError> {
    // Public record structs have Vec fields. Bound them before encoding, not only afterward.
    let scoped = |d: &LogicalDocument, key: &[u8]| d == document && key.len() == 32;
    if answer
        .receipt
        .as_ref()
        .is_some_and(|r| !scoped(&r.document, &r.owner_public_key))
        || answer
            .repair
            .as_ref()
            .is_some_and(|r| !scoped(&r.document, &r.owner_public_key))
        || answer
            .proof
            .as_ref()
            .is_some_and(|r| !scoped(&r.document, &r.owner_public_key))
    {
        return Err(SyncError::Malformed);
    }
    let mut e = Encoder::new();
    e.put_u8(1);
    for bytes in [
        answer.receipt.as_ref().map(Receipt::encode),
        answer.repair.as_ref().map(ReceiptRepair::encode),
        answer.proof.as_ref().map(ReceiptHeadProof::encode),
    ] {
        let bytes = bytes.unwrap_or_default();
        if bytes.len() > MAX_RECEIPT_BYTES {
            return Err(SyncError::Malformed);
        }
        wire(e.put_bytes(&bytes))?;
    }
    let bytes = e.finish();
    decode_answer(&bytes, document)?;
    Ok(bytes)
}
pub(super) fn decode_answer(
    bytes: &[u8],
    document: &LogicalDocument,
) -> Result<ReceiptHeadAnswer, SyncError> {
    if bytes.len() > MAX_ANSWER {
        return Err(SyncError::Malformed);
    }
    let mut d = Decoder::new(bytes);
    if wire(d.get_u8())? != 1 {
        return Err(SyncError::Malformed);
    }
    let receipt = match wire(d.get_bytes())? {
        [] => None,
        v => Some(Receipt::decode(v)?),
    };
    let repair = match wire(d.get_bytes())? {
        [] => None,
        v => Some(ReceiptRepair::decode(v)?),
    };
    let proof = match wire(d.get_bytes())? {
        [] => None,
        v => Some(ReceiptHeadProof::decode(v)?),
    };
    wire(d.finish())?;
    if receipt.as_ref().is_some_and(|r| &r.document != document)
        || repair.as_ref().is_some_and(|r| &r.document != document)
        || proof.as_ref().is_some_and(|p| {
            &p.document != document || receipt.as_ref().is_none_or(|r| r.hash() != p.receipt_hash)
        })
    {
        return Err(SyncError::Malformed);
    }
    Ok(ReceiptHeadAnswer {
        receipt,
        repair,
        proof,
    })
}
type SignedHeadResponse<'a> = (&'a [u8], [u8; 64], &'a [u8]);
pub(super) fn decode_response(bytes: &[u8]) -> Result<SignedHeadResponse<'_>, SyncError> {
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
    let answer = wire(d.get_bytes())?;
    if answer.len() > MAX_ANSWER {
        return Err(SyncError::Malformed);
    }
    wire(d.finish())?;
    Ok((key, signature, answer))
}
