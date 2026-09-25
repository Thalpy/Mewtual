//! Strict, inert decoding of the owner record's tag-3 extension. These bytes are not an
//! admitted report or a source transaction permit. In particular, self-signatures and the
//! recorded admission epoch cannot prove the local observer or durable MLS history. A future
//! private contextual restore must establish those facts before consuming this evidence.
//!
//! Retain the validated canonical suffix verbatim. There is deliberately no public constructor,
//! mutable parsed state, admission accessor or production write path while that seam is absent.

use super::{invalid, AppError, Decoder, LogicalDocument, Receipt, Zeroizing};
use catcoms_replication::{epoch::conflicting_receipt_pair, ReceiptRepair};

pub(super) const MAX_FAULT_ADMISSION_ATTESTATION_BYTES: usize = 256;

pub(super) struct InertFaultRecord {
    bytes: Zeroizing<Vec<u8>>,
}

// Temporary structural values never escape decode. A parsed attestation is not a capability.
struct Pair {
    receipts: [Receipt; 2],
    hashes: [[u8; 32]; 2],
}

impl Pair {
    fn decode(d: &mut Decoder<'_>, document: &LogicalDocument) -> Result<Self, AppError> {
        let receipts = [
            Receipt::decode(d.get_bytes().map_err(invalid)?).map_err(invalid)?,
            Receipt::decode(d.get_bytes().map_err(invalid)?).map_err(invalid)?,
        ];
        conflicting_receipt_pair(document, &receipts[0], &receipts[1]).map_err(invalid)?;
        let hashes = [receipts[0].hash(), receipts[1].hash()];
        if hashes[0] >= hashes[1] {
            return Err(invalid("fault pair is not canonically ordered"));
        }
        let pair = Self { receipts, hashes };
        pair.check_attestation(d.get_bytes().map_err(invalid)?)?;
        Ok(pair)
    }

    fn check_attestation(&self, bytes: &[u8]) -> Result<(), AppError> {
        if bytes.len() > MAX_FAULT_ADMISSION_ATTESTATION_BYTES {
            return Err(invalid("fault admission attestation exceeds its bound"));
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(invalid)? != 1 {
            return Err(invalid("unsupported fault admission attestation"));
        }
        // Only its canonical width can be checked here. Comparing this recorded observer to
        // the local device needs authenticated store/custody context, not a caller assertion.
        let _observer = fixed(&mut d)?;
        let owner = fixed(&mut d)?;
        let start = d.get_u64().map_err(invalid)?;
        let tenure = fixed(&mut d)?;
        let hashes = [fixed(&mut d)?, fixed(&mut d)?];
        let admission_epoch = d.get_u64().map_err(invalid)?;
        match d.get_u8().map_err(invalid)? {
            0 if start <= admission_epoch => {}
            1 => {
                let retired_at = d.get_u64().map_err(invalid)?;
                if !(start < retired_at && retired_at <= admission_epoch) {
                    return Err(invalid("invalid archived admission epochs"));
                }
            }
            _ => return Err(invalid("invalid admission origin or epoch")),
        }
        d.finish().map_err(invalid)?;
        if hashes != self.hashes
            || self.receipts.iter().any(|r| {
                r.owner_public_key.as_slice() != owner
                    || r.tenure_start_group_epoch != start
                    || r.tenure_id != tenure
            })
            || tenure
                != catcoms_replication::epoch::tenure_id(
                    &self.receipts[0].document.server_id,
                    &owner,
                    start,
                )
        {
            return Err(invalid("attestation does not bind the complete fault pair"));
        }
        Ok(())
    }

    fn shares_receipt(&self, other: &Self) -> bool {
        self.hashes.iter().any(|hash| other.hashes.contains(hash))
    }
}

impl InertFaultRecord {
    /// Structural-only vault decoding. This never establishes historical owner authority,
    /// local observer identity, present custody, source binding or durability of a snapshot.
    pub(super) fn decode(bytes: &[u8], document: &LogicalDocument) -> Result<Self, AppError> {
        // Also bound direct internal callers, before allocating receipts or copying the suffix.
        if bytes.len() > super::MAX_RECORD_BYTES {
            return Err(invalid("fault record exceeds its bound"));
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(invalid)? != 3 || d.get_u8().map_err(invalid)? != 1 {
            return Err(invalid("unsupported fault record format"));
        }
        let count = d.get_u8().map_err(invalid)?;
        if count > 2 {
            return Err(invalid("too many external fault pairs"));
        }
        let mut pairs: Vec<Pair> = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let pair = Pair::decode(&mut d, document)?;
            if pairs.last().is_some_and(|p| p.hashes[0] >= pair.hashes[0])
                || pairs.iter().any(|p| p.shares_receipt(&pair))
            {
                return Err(invalid("external fault pairs overlap or are out of order"));
            }
            pairs.push(pair);
        }
        let reserved = boolean(&mut d)?
            .then(|| Pair::decode(&mut d, document))
            .transpose()?;
        if reserved
            .as_ref()
            .is_some_and(|p| pairs.iter().any(|q| p.hashes == q.hashes))
        {
            return Err(invalid("reserved pair duplicates an external pair"));
        }
        if boolean(&mut d)? {
            let _tenure = fixed(&mut d)?;
            let count = d.get_u8().map_err(invalid)?;
            if count > 4 {
                return Err(invalid("too many overflow fingerprints"));
            }
            let mut previous = None;
            for _ in 0..count {
                let hash = fixed(&mut d)?;
                if previous.is_some_and(|last| last >= hash) {
                    return Err(invalid(
                        "overflow fingerprints are not ordered and distinct",
                    ));
                }
                previous = Some(hash);
            }
            let unknown = boolean(&mut d)?;
            if count == 0 && !unknown {
                return Err(invalid("empty overflow must be absent"));
            }
        }
        // Binding validation is independent of the source. The contextual resume path must
        // later compare an inline pair against the live fault OR the exact resolved repair.
        let inline;
        let bound = match d.get_u8().map_err(invalid)? {
            0 => None,
            1 => Some(
                pairs
                    .get(usize::from(d.get_u8().map_err(invalid)?))
                    .ok_or_else(|| invalid("repair external index is out of range"))?,
            ),
            2 => {
                inline = Pair::decode(&mut d, document)?;
                if pairs.iter().any(|p| p.hashes == inline.hashes)
                    || reserved.as_ref().is_some_and(|p| p.hashes == inline.hashes)
                {
                    return Err(invalid("inline pair duplicates retained evidence"));
                }
                Some(&inline)
            }
            3 => Some(
                reserved
                    .as_ref()
                    .ok_or_else(|| invalid("reserved repair has no reserved pair"))?,
            ),
            _ => return Err(invalid("unsupported repair binding")),
        };
        if let Some(pair) = bound {
            let repair = ReceiptRepair::decode(d.get_bytes().map_err(invalid)?).map_err(invalid)?;
            repair.verify_signature_only().map_err(invalid)?;
            repair
                .check_evidence(&pair.receipts[0], &pair.receipts[1])
                .map_err(invalid)?;
        }
        let applied = boolean(&mut d)?;
        if bound.is_none() && applied {
            return Err(invalid("applied marker without a repair"));
        }
        d.finish().map_err(invalid)?;
        Ok(Self {
            bytes: Zeroizing::new(bytes.to_vec()),
        })
    }

    pub(super) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn fixed(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    d.get_bytes().map_err(invalid)?.try_into().map_err(invalid)
}

fn boolean(d: &mut Decoder<'_>) -> Result<bool, AppError> {
    match d.get_u8().map_err(invalid)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid("noncanonical fault-record boolean")),
    }
}

#[cfg(test)]
mod tests;
