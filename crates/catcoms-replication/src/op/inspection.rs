//! Bounded read-only inspection of retained operation frames.
//!
//! Restored logs are evidence to inspect, not authenticated authority. In particular, a retained
//! entry can have an arbitrary key length or the wrong signed document scope. These helpers do
//! not parse Automerge or accept an operation. They preserve the existing encode/hash grammar
//! while avoiding a full encode/preimage allocation just to inspect one small chunk.
use super::{SignedOp, OP_DOMAIN};

// Test-only work observations live on the calling thread: parallel signing/replication tests
// cannot alter another inspector's measurement. Neither bytes nor authority are retained.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
struct ObservedWork {
    full_encodes: usize,
    preimage_bytes: usize,
}
#[cfg(test)]
std::thread_local! {
    static OBSERVED_WORK: std::cell::Cell<ObservedWork> = const { std::cell::Cell::new(ObservedWork {
        full_encodes: 0, preimage_bytes: 0,
    }) };
}
#[cfg(test)]
pub(super) fn record_full_encode() {
    OBSERVED_WORK.with(|cell| {
        let mut work = cell.get();
        work.full_encodes = work.full_encodes.saturating_add(1);
        cell.set(work);
    });
}
#[cfg(test)]
pub(super) fn record_signing_preimage(bytes: usize) {
    OBSERVED_WORK.with(|cell| {
        let mut work = cell.get();
        work.preimage_bytes = work.preimage_bytes.saturating_add(bytes);
        cell.set(work);
    });
}
#[cfg(test)]
fn take_observed_work() -> ObservedWork {
    OBSERVED_WORK.with(|cell| cell.replace(ObservedWork::default()))
}

/// Maximum original-frame bytes copied by one detail read, before hex encoding.
pub const MAX_INSPECTION_FRAME_CHUNK: usize = 64 * 1024;
/// Maximum contiguous v1 signature preimage inspected in one verification operation.
pub const MAX_INSPECTION_SIGNATURE_BYTES: usize = 256 * 1024;

/// Refusals of a read request, never a claim that retained signed evidence should be removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameReadError {
    /// A stored vector length cannot be represented by the existing u32 wire framing.
    Length,
    /// The requested byte offset is beyond the exact end of the frame.
    Offset,
    /// A chunk must request between one byte and the fixed inspection ceiling.
    ChunkLimit,
}

/// Reasons an inspection does not perform cryptographic verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectionSignatureLimit {
    /// The original preimage exceeds the read-only verification policy; bytes remain readable.
    TooLarge,
    /// Domain-operation verification has separate admission/parser rules. Do not silently use
    /// this legacy-only inspector to parse or assert that distinct format's authority.
    UnsupportedDomain,
}

/// Signature attribution only. Even Valid says nothing about MLS group, epoch or past roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectionSignature {
    /// Exact v1 signature and public-key/device binding verified within the work ceiling.
    Valid,
    /// Key shape, content-addressed device identity or cryptographic signature is invalid.
    Invalid,
    /// Explicit terminal work-policy result; a caller must not retry this same check forever.
    NotChecked(InspectionSignatureLimit),
}

/// A borrowed exact encoding, with only fixed-size framing stored locally.
///
/// The caller must retain the same immutable original/revision across chunk or incremental hash
/// work. This object carries no membership proof, publication permission or native reader gate.
pub struct SignedOpFrame<'a> {
    op: &'a SignedOp,
    prefix: [u8; 22],
    delta_length: [u8; 4],
    signature_length: [u8; 4],
    domain_prefix: [u8; 5],
    encoded_length: usize,
}
impl std::fmt::Debug for SignedOpFrame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedOpFrame").finish_non_exhaustive()
    }
}
impl<'a> SignedOpFrame<'a> {
    /// Borrow the original bytes. Length checks precede framing; malformed keys are still
    /// representable and inspectable instead of being silently dropped from the admin ledger.
    pub fn new(op: &'a SignedOp) -> Result<Self, FrameReadError> {
        let key_length =
            u32::try_from(op.author_pubkey.len()).map_err(|_| FrameReadError::Length)?;
        let delta_length = u32::try_from(op.delta.len()).map_err(|_| FrameReadError::Length)?;
        let domain_length = op.domain_op.as_ref().map_or(Ok(0), |bytes| {
            u32::try_from(bytes.len()).map_err(|_| FrameReadError::Length)
        })?;
        let mut prefix = [0; 22];
        prefix[..2].copy_from_slice(&op.doc_type.tag().to_be_bytes());
        prefix[2..18].copy_from_slice(&op.doc_id.to_be_bytes());
        prefix[18..].copy_from_slice(&key_length.to_be_bytes());
        let mut domain_prefix = [0; 5];
        domain_prefix[0] = 2;
        domain_prefix[1..].copy_from_slice(&domain_length.to_be_bytes());
        let mut encoded_length = 22usize;
        for length in [op.author_pubkey.len(), 4, op.delta.len(), 4, 64] {
            encoded_length = encoded_length
                .checked_add(length)
                .ok_or(FrameReadError::Length)?;
        }
        if op.domain_op.is_some() {
            encoded_length = encoded_length
                .checked_add(5)
                .and_then(|length| length.checked_add(domain_length as usize))
                .ok_or(FrameReadError::Length)?;
        }
        Ok(Self {
            op,
            prefix,
            delta_length: delta_length.to_be_bytes(),
            signature_length: 64u32.to_be_bytes(),
            domain_prefix,
            encoded_length,
        })
    }

    /// Exact existing SignedOp::encode length, without allocating that encoding.
    pub fn encoded_len(&self) -> usize {
        self.encoded_length
    }

    /// Visit only the requested original byte range, without constructing an intermediate
    /// full frame. An index can feed these slices into its own bounded incremental hasher;
    /// its source/revision owner must prevent mixing different originals between turns.
    pub fn visit_chunk(
        &self,
        offset: usize,
        limit: usize,
        mut visit: impl FnMut(&[u8]),
    ) -> Result<usize, FrameReadError> {
        if limit == 0 || limit > MAX_INSPECTION_FRAME_CHUNK {
            return Err(FrameReadError::ChunkLimit);
        }
        if offset > self.encoded_length {
            return Err(FrameReadError::Offset);
        }
        let end = offset.saturating_add(limit).min(self.encoded_length);
        let mut at = 0usize;
        let mut part = |bytes: &[u8]| {
            let first = offset.saturating_sub(at).min(bytes.len());
            let last = end.saturating_sub(at).min(bytes.len());
            if first < last {
                visit(&bytes[first..last]);
            }
            // Constructor checked this exact concatenation for address-size overflow.
            at += bytes.len();
        };
        part(&self.prefix);
        part(&self.op.author_pubkey);
        part(&self.delta_length);
        part(&self.op.delta);
        part(&self.signature_length);
        part(&self.op.signature);
        if let Some(domain) = &self.op.domain_op {
            part(&self.domain_prefix);
            part(domain);
        }
        Ok(end - offset)
    }

    /// Copy at most one detail chunk. Empty is legal only at the exact frame end.
    pub fn read_chunk(&self, offset: usize, limit: usize) -> Result<Vec<u8>, FrameReadError> {
        if limit == 0 || limit > MAX_INSPECTION_FRAME_CHUNK {
            return Err(FrameReadError::ChunkLimit);
        }
        if offset > self.encoded_length {
            return Err(FrameReadError::Offset);
        }
        let mut bytes = Vec::with_capacity(limit.min(self.encoded_length - offset));
        self.visit_chunk(offset, limit, |part| bytes.extend_from_slice(part))?;
        Ok(bytes)
    }

    /// Contiguous preimage work charged by verify_signature(), or zero when that check will
    /// terminate without cryptographic work. Callers also need their own per-turn byte budget.
    pub fn signature_work_bytes(&self) -> usize {
        if self.op.author_pubkey.len() != 32 || self.op.domain_op.is_some() {
            return 0;
        }
        // Domain length-prefix/domain + type/id + key framing + delta framing. Every variable
        // length already fits u32; checked addition also preserves correctness on 32-bit hosts.
        let length =
            (4 + OP_DOMAIN.len() + 2 + 16 + 4 + 32 + 4usize).checked_add(self.op.delta.len());
        length
            .filter(|length| *length <= MAX_INSPECTION_SIGNATURE_BYTES)
            .unwrap_or(0)
    }

    /// Verify bounded legacy attribution without parsing change/domain payloads. Unsupported
    /// or oversized entries stay readable and do not acquire a fabricated verification label.
    pub fn verify_signature(&self) -> InspectionSignature {
        if self.op.author_pubkey.len() != 32 {
            return InspectionSignature::Invalid;
        }
        if self.op.domain_op.is_some() {
            return InspectionSignature::NotChecked(InspectionSignatureLimit::UnsupportedDomain);
        }
        if self.signature_work_bytes() == 0 {
            return InspectionSignature::NotChecked(InspectionSignatureLimit::TooLarge);
        }
        // The exact original verifier is now bounded: v1 has no domain parser, key size is
        // fixed, and the entire allocated signing preimage is within the explicit ceiling.
        if self.op.verify() {
            InspectionSignature::Valid
        } else {
            InspectionSignature::Invalid
        }
    }
}

#[cfg(test)]
mod tests;
