//! Incremental inspection of an immutable prefix of the ORIGINAL retained signed log.
//!
//! This is deliberately not a complete Automerge graph inventory or an authorization service.
//! In particular an unsigned change, orphan queue entry or mismatched saved envelope cannot be
//! inferred from these rows. The privileged consumer must report unknown/incomplete graph
//! coverage until it has independently inspected that inventory. No native caller is wired here.
//!
//! The log is private and append-only for the lifetime of one EncryptedDoc. Capturing its upper
//! bound lets inspection finish even if fifty writers append between every visit. Graph changes,
//! including partial legacy edit failures, cannot rewrite an already retained original. Document
//! replacement/restore changes the allocation identity and refuses every old page/detail read.
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::Arc;

use catcoms_crypto::DeviceId;
use catcoms_wire::DocType;

use super::EncryptedDoc;
use crate::history_inspection::{inspect_legacy_change, LegacyChangeInspection};
use crate::op::{InspectionSignature, SignedOpFrame, MAX_INSPECTION_FRAME_CHUNK};

/// Independent ceilings: many short records cannot turn a byte allowance into unbounded crypto.
pub const HISTORY_RECORDS_PER_TURN: usize = 128;
pub const HISTORY_HASH_BYTES_PER_TURN: usize = 512 * 1024;
pub const HISTORY_SIGNATURE_BYTES_PER_TURN: usize = 512 * 1024;
pub const HISTORY_SIGNATURES_PER_TURN: usize = 16;

/// Observer refusal only. None of these outcomes changes retained history or authoring rights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedHistoryError {
    /// The original document was replaced; equal labels/counts are insufficient.
    Stale,
    /// Caller quota cannot contain this captured inventory. Never silently truncate it.
    Capacity,
    /// Malformed historical field length cannot use the existing exact frame grammar.
    Frame,
    /// The captured prefix has not finished hashing and attribution checks.
    Busy,
    /// Page/chunk/reference request is outside the narrow read grammar.
    Request,
}

/// Fixed-size metadata only; no large original key/delta, mutable message text or membership
/// decision is retained here. `signature` attributes the frame, not a group or historical role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedHistoryRow {
    pub op_hash: [u8; 32],
    pub signed_doc_type: DocType,
    pub signed_doc_id: u128,
    pub scope_matches: bool,
    pub claimed_key_bytes: usize,
    pub claimed_public_key: Option<[u8; 32]>,
    pub claimed_device: Option<DeviceId>,
    pub signature: InspectionSignature,
    pub original_bytes: usize,
    // An ordinal is meaningful only under this index's exact retained source allocation.
    position: usize,
}

/// Work actually performed by one visit, including a huge frame's resumed hashing. Completion
/// means only the captured signed-log prefix; it MUST NOT become full-ledger coverage.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SignedHistoryProgress {
    pub complete_signed_prefix: bool,
    pub source_records_completed: usize,
    pub frame_bytes: usize,
    pub signature_bytes: usize,
    pub signature_checks: usize,
}

struct HashingOriginal {
    offset: usize,
    hasher: blake3::Hasher,
    digest: Option<[u8; 32]>,
}

/// One bounded metadata index, owned by a privileged reader/Actor. The host must reserve its
/// aggregate index quota BEFORE construction and bind access to the original session, current
/// full-key reader policy, namespace and Actor/store incarnation. This type grants none of them.
pub struct SignedHistoryIndex {
    owner: Arc<()>,
    upper_bound: usize,
    next: usize,
    active: Option<HashingOriginal>,
    rows: BTreeMap<[u8; 32], SignedHistoryRow>,
}

impl std::fmt::Debug for SignedHistoryIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedHistoryIndex")
            .field("captured_records", &self.upper_bound)
            .field("completed_records", &self.next)
            .finish_non_exhaustive()
    }
}

impl EncryptedDoc {
    /// Capture the current append-only signed-log prefix without cloning any original, graph or
    /// whole metadata inventory. `max_records` is a prepaid HOST quota, not a history retention cap.
    pub fn signed_history_index(
        &self,
        max_records: usize,
    ) -> Result<SignedHistoryIndex, SignedHistoryError> {
        if self.log.len() > max_records {
            return Err(SignedHistoryError::Capacity);
        }
        Ok(SignedHistoryIndex {
            owner: Arc::clone(&self.history_owner),
            upper_bound: self.log.len(),
            next: 0,
            active: None,
            rows: BTreeMap::new(),
        })
    }
}

impl SignedHistoryIndex {
    fn require_source(&self, doc: &EncryptedDoc) -> Result<(), SignedHistoryError> {
        if !Arc::ptr_eq(&self.owner, &doc.history_owner) || doc.log.len() < self.upper_bound {
            return Err(SignedHistoryError::Stale);
        }
        Ok(())
    }

    pub fn captured_record_count(&self) -> usize {
        self.upper_bound
    }

    /// Distinct original frames already indexed, rather than physical log entries. Exact
    /// duplicate envelopes do not inflate the admin's known row count.
    pub fn known_row_count(&self) -> usize {
        self.rows.len()
    }

    /// Stricter observation fence for policy sources whose appended evidence must stale a
    /// view. History pages deliberately permit appends through `require_source` instead.
    pub fn is_current_version(&self, doc: &EncryptedDoc) -> bool {
        Arc::ptr_eq(&self.owner, &doc.history_owner) && doc.log.len() == self.upper_bound
    }

    /// The captured original prefix still exists in this exact document. Appending ordinary
    /// history does not invalidate it; replacing a document always does.
    pub fn is_same_source(&self, doc: &EncryptedDoc) -> bool {
        self.require_source(doc).is_ok()
    }

    /// Fixed metadata for one exact original after host scope and permission checks.
    pub fn row(
        &self,
        doc: &EncryptedDoc,
        hash: &[u8; 32],
    ) -> Result<Option<SignedHistoryRow>, SignedHistoryError> {
        self.require_ready(doc)?;
        Ok(self.rows.get(hash).cloned())
    }

    /// Exact retained signature without allocating the original frame or preimage.
    pub fn signature(
        &self,
        doc: &EncryptedDoc,
        hash: &[u8; 32],
    ) -> Result<[u8; 64], SignedHistoryError> {
        self.require_ready(doc)?;
        let row = self.rows.get(hash).ok_or(SignedHistoryError::Request)?;
        Ok(doc.log[row.position].signature)
    }

    /// Advance one fixed work allowance. New appends do not move the captured upper bound and
    /// cannot starve a finite snapshot; a replacement cannot borrow the old allocation identity.
    pub fn advance(
        &mut self,
        doc: &EncryptedDoc,
    ) -> Result<SignedHistoryProgress, SignedHistoryError> {
        self.require_source(doc)?;
        let mut work = SignedHistoryProgress::default();
        while self.next < self.upper_bound
            && work.source_records_completed < HISTORY_RECORDS_PER_TURN
        {
            let original = &doc.log[self.next];
            let frame = SignedOpFrame::new(original).map_err(|_| SignedHistoryError::Frame)?;
            let active = self.active.get_or_insert_with(|| HashingOriginal {
                offset: 0,
                hasher: blake3::Hasher::new(),
                digest: None,
            });
            while active.offset < frame.encoded_len() {
                let available = HISTORY_HASH_BYTES_PER_TURN - work.frame_bytes;
                if available == 0 {
                    return Ok(work);
                }
                let copied = frame
                    .visit_chunk(
                        active.offset,
                        available.min(MAX_INSPECTION_FRAME_CHUNK),
                        |part| {
                            active.hasher.update(part);
                        },
                    )
                    .map_err(|_| SignedHistoryError::Frame)?;
                active.offset += copied;
                work.frame_bytes += copied;
            }
            let signature_bytes = frame.signature_work_bytes();
            if work.signature_checks == HISTORY_SIGNATURES_PER_TURN
                || signature_bytes > HISTORY_SIGNATURE_BYTES_PER_TURN - work.signature_bytes
            {
                return Ok(work);
            }
            let digest = *active
                .digest
                .get_or_insert_with(|| *active.hasher.finalize().as_bytes());
            let claimed_public_key = original.author_pubkey.as_slice().try_into().ok();
            let row = SignedHistoryRow {
                op_hash: digest,
                signed_doc_type: original.doc_type,
                signed_doc_id: original.doc_id,
                scope_matches: original.doc_type == doc.doc_type && original.doc_id == doc.doc_id,
                claimed_key_bytes: original.author_pubkey.len(),
                claimed_public_key,
                claimed_device: claimed_public_key
                    .as_ref()
                    .map(|key| DeviceId::from_public_key_bytes(key)),
                signature: frame.verify_signature(),
                original_bytes: frame.encoded_len(),
                position: self.next,
            };
            // Only identical complete-frame CIDs deduplicate. Message IDs, actor sequences,
            // timestamps, nonces and text never choose a retained branch. Earliest exact
            // duplicate keeps its source ordinal; all signature variants have distinct hashes.
            self.rows.entry(digest).or_insert(row);
            work.signature_bytes += signature_bytes;
            work.signature_checks += 1;
            work.source_records_completed += 1;
            self.next += 1;
            self.active = None;
        }
        work.complete_signed_prefix = self.next == self.upper_bound;
        Ok(work)
    }

    fn require_ready(&self, doc: &EncryptedDoc) -> Result<(), SignedHistoryError> {
        self.require_source(doc)?;
        if self.next != self.upper_bound {
            return Err(SignedHistoryError::Busy);
        }
        Ok(())
    }

    /// Bounded CID-ordered page of the completed captured prefix. Source authorization must run
    /// before calling this method, so stale cursor information cannot become an access oracle.
    pub fn page(
        &self,
        doc: &EncryptedDoc,
        after: Option<[u8; 32]>,
        limit: usize,
    ) -> Result<Vec<SignedHistoryRow>, SignedHistoryError> {
        self.require_ready(doc)?;
        if !(1..=32).contains(&limit) {
            return Err(SignedHistoryError::Request);
        }
        let bounds = after.map_or((Unbounded, Unbounded), |key| (Excluded(key), Unbounded));
        Ok(self
            .rows
            .range(bounds)
            .take(limit)
            .map(|(_, row)| row.clone())
            .collect())
    }

    /// Exact original bytes, copied only after the caller's session/policy and revision checks.
    pub fn original_chunk(
        &self,
        doc: &EncryptedDoc,
        hash: &[u8; 32],
        offset: usize,
        limit: usize,
    ) -> Result<Vec<u8>, SignedHistoryError> {
        self.require_ready(doc)?;
        let row = self.rows.get(hash).ok_or(SignedHistoryError::Request)?;
        SignedOpFrame::new(&doc.log[row.position])
            .map_err(|_| SignedHistoryError::Frame)?
            .read_chunk(offset, limit)
            .map_err(|_| SignedHistoryError::Request)
    }

    /// Independent bounded raw decoding, never a current-message lookup or authority upgrade.
    /// Unsupported/large originals remain available through exact chunks.
    pub fn inspect_original_change(
        &self,
        doc: &EncryptedDoc,
        hash: &[u8; 32],
    ) -> Result<LegacyChangeInspection, SignedHistoryError> {
        self.require_ready(doc)?;
        let row = self.rows.get(hash).ok_or(SignedHistoryError::Request)?;
        Ok(inspect_legacy_change(&doc.log[row.position].delta))
    }
}

#[cfg(test)]
mod tests;
