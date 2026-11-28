//! A separate signed decision chain anchored in the original founder pin.
//! Entries leave the MLS epoch and ordinary membership Header untouched.
use super::*;
use crate::chat::FounderGenesisAnchorData;
use catcoms_crypto::verify_with_public_bytes;

mod public_delivery;
mod public_step;
mod record;
pub use public_delivery::ReviewStreamQuery;
pub use public_step::ReviewCaseStep;
use public_step::REVIEW_CASE_STEP_BYTES;
pub use record::{ReviewStreamRecord, MAX_REVIEW_STREAM_RECORD_BYTES};

const PRIVATE_DOMAIN: &[u8; 44] = b"mewtual/chat/removed-post-decision-entry/v1\0";
const PUBLIC_DOMAIN: &[u8; PRIVATE_DOMAIN.len()] = b"mewtual/chat/removed-post-decision-entry/v2\0";
pub const UNSIGNED_REVIEW_STREAM_ENTRY_BYTES: usize = PRIVATE_DOMAIN.len() + 4 * 32 + 2 * 8;
pub const REVIEW_STREAM_ENTRY_BYTES: usize = UNSIGNED_REVIEW_STREAM_ENTRY_BYTES + 64;
pub const MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES: usize =
    UNSIGNED_REVIEW_STREAM_ENTRY_BYTES + REVIEW_CASE_STEP_BYTES;
pub const MAX_REVIEW_STREAM_ENTRY_BYTES: usize = MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES + 64;

/// Entry identity commits the signed fields and excludes the signature.
/// Verify every received representation before using this ID.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReviewStreamEntryId([u8; 32]);

impl ReviewStreamEntryId {
    /// A wire coordinate for an independently verified entry.
    pub fn from_bytes_data(bytes: [u8; 32]) -> Result<Self, ChatError> {
        if bytes == [0; 32] {
            return Err(ChatError::Malformed);
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// The chain sequence is independent of the membership epoch. The supplied
/// membership anchor must be proved against actual selected ancestry.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReviewStreamPosition {
    pub sequence: u64,
    pub previous: Option<ReviewStreamEntryId>,
    pub membership_header: OwnerHeaderId,
    pub membership_epoch: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReviewStreamEntry {
    stream: [u8; 32],
    position: ReviewStreamPosition,
    body: OwnerBodyId,
    step: Option<ReviewCaseStep>,
    signature: [u8; 64],
}

/// Exact immutable input to cover before the dedicated native signature.
/// This data carries no authority to open the founder key.
pub struct UnsignedReviewStreamEntry {
    bytes: [u8; MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES],
    len: usize,
}

catcoms_rt::redacted_debug! {
    ReviewStreamEntry;
    ReviewStreamEntryId;
    ReviewStreamPosition;
    UnsignedReviewStreamEntry;
}

impl UnsignedReviewStreamEntry {
    /// Prepare the legacy private representation. The old signature and
    /// entry ID remain valid for exact local recovery.
    pub fn prepare(
        trusted: &FounderGenesisAnchorData,
        position: ReviewStreamPosition,
        decision: &ExactReviewDecisionBody,
    ) -> Result<Self, ChatError> {
        Self::from_decision(trusted, position, decision, None)
    }

    /// Prepare a signed case transition whose full private record can be
    /// withheld while the other members verify the sequence.
    pub fn prepare_public(
        trusted: &FounderGenesisAnchorData,
        position: ReviewStreamPosition,
        decision: &ExactReviewDecisionBody,
    ) -> Result<Self, ChatError> {
        let step = ReviewCaseStep::from_decision(decision)?;
        Self::from_decision(trusted, position, decision, Some(step))
    }

    /// Re-derive the exact preimage in a covered intent, including its
    /// version. A legacy Raw signature must never be replaced on retry.
    pub fn rederive(
        trusted: &FounderGenesisAnchorData,
        position: ReviewStreamPosition,
        decision: &ExactReviewDecisionBody,
        covered: &[u8],
    ) -> Result<Self, ChatError> {
        let expected = match covered.len() {
            UNSIGNED_REVIEW_STREAM_ENTRY_BYTES => Self::prepare(trusted, position, decision)?,
            MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES => {
                Self::prepare_public(trusted, position, decision)?
            }
            _ => return Err(ChatError::Malformed),
        };
        if expected.signing_bytes() != covered {
            return Err(ChatError::Scope);
        }
        Ok(expected)
    }

    fn from_decision(
        trusted: &FounderGenesisAnchorData,
        position: ReviewStreamPosition,
        decision: &ExactReviewDecisionBody,
        step: Option<ReviewCaseStep>,
    ) -> Result<Self, ChatError> {
        let entry = ReviewStreamEntry {
            stream: stream_data(trusted)?,
            position,
            body: decision.id(),
            step,
            signature: [0; 64],
        };
        entry.require_shape()?;
        entry.require_decision(trusted, decision)?;
        Ok(entry.semantic_bytes())
    }

    pub fn signing_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Borrowing an authenticated local intent can name a historical
    /// membership position. Its ancestry and preimage still need reproof
    /// against the selected journal and membership.
    pub fn read_position_data(bytes: &[u8]) -> Result<ReviewStreamPosition, ChatError> {
        let mut frame = [0; MAX_REVIEW_STREAM_ENTRY_BYTES];
        let len = match bytes.len() {
            UNSIGNED_REVIEW_STREAM_ENTRY_BYTES => REVIEW_STREAM_ENTRY_BYTES,
            MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES => MAX_REVIEW_STREAM_ENTRY_BYTES,
            _ => return Err(ChatError::Malformed),
        };
        frame[..bytes.len()].copy_from_slice(bytes);
        Ok(ReviewStreamEntry::decode(&frame[..len])?.position_data())
    }

    /// Verify retained raw output against the exact input. An interrupted
    /// producer can finish from verified Raw without entering the key again.
    pub fn attach_signature(
        self,
        trusted: &FounderGenesisAnchorData,
        signature: [u8; 64],
    ) -> Result<ReviewStreamEntry, ChatError> {
        let mut bytes = [0; MAX_REVIEW_STREAM_ENTRY_BYTES];
        let len = self.len + signature.len();
        bytes[..self.len].copy_from_slice(self.signing_bytes());
        bytes[self.len..len].copy_from_slice(&signature);
        let entry = ReviewStreamEntry::decode(&bytes[..len])?;
        entry.verify_for_pin(trusted)?;
        Ok(entry)
    }
}

impl ReviewStreamEntry {
    /// Sign with the exact original founder key held by the caller.
    /// A durable producer covers prepared bytes and Raw separately.
    pub fn sign_using(
        trusted: &FounderGenesisAnchorData,
        position: ReviewStreamPosition,
        decision: &ExactReviewDecisionBody,
        sign: impl FnOnce(&[u8]) -> Result<[u8; 64], ChatError>,
    ) -> Result<Self, ChatError> {
        let prepared = UnsignedReviewStreamEntry::prepare(trusted, position, decision)?;
        let signature = sign(prepared.signing_bytes())?;
        prepared.attach_signature(trusted, signature)
    }

    /// Independent host pin, exact signature and stream binding only.
    pub fn verify_for_pin(&self, trusted: &FounderGenesisAnchorData) -> Result<(), ChatError> {
        self.require_shape()?;
        if self.stream != stream_data(trusted)? {
            return Err(ChatError::Scope);
        }
        let exact = self.semantic_bytes();
        if !verify_with_public_bytes(&trusted.founder, exact.signing_bytes(), &self.signature) {
            return Err(ChatError::Signature);
        }
        Ok(())
    }

    /// Compare only against an independently selected preceding entry.
    /// Membership ancestry, the specific case predecessor and covering
    /// durability remain separate from signed position checks.
    pub fn verify_next_for_pin(
        &self,
        trusted: &FounderGenesisAnchorData,
        previous: Option<&Self>,
    ) -> Result<(), ChatError> {
        self.verify_for_pin(trusted)?;
        if let Some(previous) = previous {
            previous.verify_for_pin(trusted)?;
            if self.position.previous != Some(previous.id())
                || previous.position.sequence.checked_add(1) != Some(self.position.sequence)
                || previous.position.membership_epoch > self.position.membership_epoch
            {
                return Err(ChatError::Dependency);
            }
        } else if self.position.sequence != 1 {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }

    /// Verify the signature, exact body and its group/Profile coordinates.
    /// The caller still proves the selected membership anchor and Remove.
    pub fn verify_decision_for_pin(
        &self,
        trusted: &FounderGenesisAnchorData,
        decision: &ExactReviewDecisionBody,
    ) -> Result<(), ChatError> {
        self.verify_for_pin(trusted)?;
        self.require_decision(trusted, decision)
    }

    pub fn id(&self) -> ReviewStreamEntryId {
        ReviewStreamEntryId(blake3::derive_key(
            "mewtual/chat/removed-post-decision-entry-id/v1",
            self.semantic_bytes().signing_bytes(),
        ))
    }

    pub fn position_data(&self) -> ReviewStreamPosition {
        self.position
    }

    /// Exact decision-body reference for a later request's case revision.
    /// Its selected state must be proved through the original journal.
    pub fn decision_id_data(&self) -> OwnerBodyId {
        self.body
    }

    /// Only the public representation has the signed per-case step needed
    /// to select an approval while keeping earlier review bodies private.
    pub fn case_step_data(&self) -> Option<ReviewCaseStep> {
        self.step
    }

    pub fn encode(&self) -> Vec<u8> {
        let exact = self.semantic_bytes();
        let mut bytes = Vec::with_capacity(exact.len + self.signature.len());
        bytes.extend_from_slice(exact.signing_bytes());
        bytes.extend_from_slice(&self.signature);
        bytes
    }

    /// Decode bounded fixed data. Proof uses the locally pinned full key.
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        let (domain, public) = match bytes.len() {
            REVIEW_STREAM_ENTRY_BYTES => (PRIVATE_DOMAIN, false),
            MAX_REVIEW_STREAM_ENTRY_BYTES => (PUBLIC_DOMAIN, true),
            _ => return Err(ChatError::Malformed),
        };
        let remaining = bytes
            .strip_prefix(domain.as_slice())
            .ok_or(ChatError::Malformed)?;
        let (stream, remaining) = remaining
            .split_first_chunk::<32>()
            .ok_or(ChatError::Malformed)?;
        let (sequence, remaining) = remaining
            .split_first_chunk::<8>()
            .ok_or(ChatError::Malformed)?;
        let (previous, remaining) = remaining
            .split_first_chunk::<32>()
            .ok_or(ChatError::Malformed)?;
        let (membership_header, remaining) = remaining
            .split_first_chunk::<32>()
            .ok_or(ChatError::Malformed)?;
        let (membership_epoch, remaining) = remaining
            .split_first_chunk::<8>()
            .ok_or(ChatError::Malformed)?;
        let (body, remaining) = remaining
            .split_first_chunk::<32>()
            .ok_or(ChatError::Malformed)?;
        let (step, signature) = if public {
            let (step, signature) = remaining.split_at(REVIEW_CASE_STEP_BYTES);
            (Some(ReviewCaseStep::decode(step)?), signature)
        } else {
            (None, remaining)
        };
        let entry = Self {
            stream: *stream,
            position: ReviewStreamPosition {
                sequence: u64::from_be_bytes(*sequence),
                previous: (*previous != [0; 32]).then_some(ReviewStreamEntryId(*previous)),
                membership_header: OwnerHeaderId::from_bytes(*membership_header),
                membership_epoch: u64::from_be_bytes(*membership_epoch),
            },
            body: OwnerBodyId::from_bytes(*body),
            step,
            signature: signature.try_into().map_err(|_| ChatError::Malformed)?,
        };
        entry.require_shape()?;
        Ok(entry)
    }

    fn require_shape(&self) -> Result<(), ChatError> {
        if self.position.sequence == 0
            || (self.position.sequence == 1) != self.position.previous.is_none()
            || self.position.previous.is_some_and(|id| id.0 == [0; 32])
            || self.position.membership_header.as_bytes() == &[0; 32]
            || self.position.membership_epoch == 0
        {
            return Err(ChatError::Malformed);
        }
        Ok(())
    }

    fn require_decision(
        &self,
        trusted: &FounderGenesisAnchorData,
        decision: &ExactReviewDecisionBody,
    ) -> Result<(), ChatError> {
        let target = decision.decision().spec().target.spec();
        if self.body != decision.id()
            || target.group != trusted.group
            || target.context.profile != trusted.profile
            || decision.original_data().remove_epoch > self.position.membership_epoch
        {
            return Err(ChatError::Scope);
        }
        if let Some(step) = self.step {
            step.require_decision(decision)?;
        }
        Ok(())
    }

    fn semantic_bytes(&self) -> UnsignedReviewStreamEntry {
        let position = self.position;
        let sequence = position.sequence.to_be_bytes();
        let previous = position.previous.map_or([0; 32], |id| id.0);
        let epoch = position.membership_epoch.to_be_bytes();
        let domain = if self.step.is_some() {
            PUBLIC_DOMAIN
        } else {
            PRIVATE_DOMAIN
        };
        let fields: [&[u8]; 7] = [
            domain,
            &self.stream,
            &sequence,
            &previous,
            position.membership_header.as_bytes(),
            &epoch,
            self.body.as_bytes(),
        ];
        let mut bytes = [0; MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES];
        let mut next = bytes.as_mut_slice();
        for field in fields {
            let (prefix, tail) = next.split_at_mut(field.len());
            prefix.copy_from_slice(field);
            next = tail;
        }
        let len = if let Some(step) = self.step {
            next.copy_from_slice(&step.encode());
            MAX_UNSIGNED_REVIEW_STREAM_ENTRY_BYTES
        } else {
            UNSIGNED_REVIEW_STREAM_ENTRY_BYTES
        };
        UnsignedReviewStreamEntry { bytes, len }
    }
}

fn stream_data(trusted: &FounderGenesisAnchorData) -> Result<[u8; 32], ChatError> {
    check_group(&trusted.group)?;
    let mut hash = blake3::Hasher::new_derive_key("mewtual/chat/founder-review-stream/v1");
    hash.update(&trusted.group);
    hash.update(&trusted.founder);
    hash.update(trusted.profile.as_bytes());
    hash.update(trusted.admission.as_bytes());
    hash.update(trusted.tenure.as_bytes());
    hash.update(trusted.header.as_bytes());
    hash.update(trusted.state.as_bytes());
    Ok(*hash.finalize().as_bytes())
}
