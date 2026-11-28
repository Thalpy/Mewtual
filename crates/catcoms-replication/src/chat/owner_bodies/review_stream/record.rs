//! Exact decided record data for the private journal and approval proof.
//! Callers still prove the selected membership and journal histories.
use super::*;
use crate::chat::{V2RemovedCreateRecord, MAX_REMOVED_CREATE_RECORD_BYTES};
use zeroize::Zeroizing;

const RECORD_DOMAIN: &[u8] = b"mewtual/chat/removed-post-decision-record/v1\0";
pub const MAX_REVIEW_STREAM_RECORD_BYTES: usize = 4
    + RECORD_DOMAIN.len()
    + 4
    + REVIEW_STREAM_ENTRY_BYTES
    + 4
    + MAX_EXACT_REVIEW_BODY_BYTES
    + 4
    + MAX_REMOVED_CREATE_RECORD_BYTES;

/// A signed entry, exact decision and the original retained Create.
/// Decoding authenticates their agreement, without selecting any case.
pub struct ReviewStreamRecord<'a> {
    entry: ReviewStreamEntry,
    decision: ExactReviewDecisionBody,
    held: V2RemovedCreateRecord<'a>,
}

catcoms_rt::redacted_debug! { ReviewStreamRecord<'_>; }

impl<'a> ReviewStreamRecord<'a> {
    /// Prepare exact data for covering before selecting the stream head.
    pub fn encode(
        trusted: &FounderGenesisAnchorData,
        entry: &ReviewStreamEntry,
        decision: &ExactReviewDecisionBody,
        held_bytes: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, ChatError> {
        let held = V2RemovedCreateRecord::decode(held_bytes)?;
        entry.verify_decision_for_pin(trusted, decision)?;
        decision.require_record_data(&held)?;
        let body = decision.encode();
        let mut packed = Encoder::with_capacity(
            16 + RECORD_DOMAIN.len() + REVIEW_STREAM_ENTRY_BYTES + body.len() + held_bytes.len(),
        );
        packed.put_bytes(RECORD_DOMAIN)?;
        packed.put_bytes(&entry.encode())?;
        packed.put_bytes(&body)?;
        packed.put_bytes(held_bytes)?;
        Ok(Zeroizing::new(packed.finish()))
    }

    pub fn decode(trusted: &FounderGenesisAnchorData, raw: &'a [u8]) -> Result<Self, ChatError> {
        if raw.len() > MAX_REVIEW_STREAM_RECORD_BYTES {
            return Err(ChatError::Bound);
        }
        let mut reader = Decoder::new(raw);
        if reader.get_bytes()? != RECORD_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let header = reader.get_bytes()?;
        let body = reader.get_bytes()?;
        let held = reader.get_bytes()?;
        reader.finish()?;
        if body.len() > MAX_EXACT_REVIEW_BODY_BYTES || held.len() > MAX_REMOVED_CREATE_RECORD_BYTES
        {
            return Err(ChatError::Bound);
        }
        let entry = ReviewStreamEntry::decode(header)?;
        let decision = ExactReviewDecisionBody::decode(body)?;
        entry.verify_decision_for_pin(trusted, &decision)?;
        let held = V2RemovedCreateRecord::decode(held)?;
        decision.require_record_data(&held)?;
        Ok(Self {
            entry,
            decision,
            held,
        })
    }

    pub fn entry_data(&self) -> &ReviewStreamEntry {
        &self.entry
    }

    pub fn decision_data(&self) -> &ExactReviewDecisionBody {
        &self.decision
    }

    pub fn held_data(&self) -> &V2RemovedCreateRecord<'a> {
        &self.held
    }
}
