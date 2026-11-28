//! A separate signed decision chain anchored in the original founder pin.
//! Entries leave the MLS epoch and ordinary membership Header untouched.
use super::*;
use crate::chat::FounderGenesisAnchorData;
use catcoms_crypto::verify_with_public_bytes;

mod record;
pub use record::{ReviewStreamRecord, MAX_REVIEW_STREAM_RECORD_BYTES};

const DOMAIN: &[u8] = b"mewtual/chat/removed-post-decision-entry/v1\0";
pub const UNSIGNED_REVIEW_STREAM_ENTRY_BYTES: usize = DOMAIN.len() + 4 * 32 + 2 * 8;
pub const REVIEW_STREAM_ENTRY_BYTES: usize = UNSIGNED_REVIEW_STREAM_ENTRY_BYTES + 64;

/// Entry identity commits the signed fields and excludes the signature.
/// Verify every received representation before using this ID.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReviewStreamEntryId([u8; 32]);

impl ReviewStreamEntryId {
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
    signature: [u8; 64],
}

/// Exact immutable input to cover before the dedicated native signature.
/// This data carries no authority to open the founder key.
pub struct UnsignedReviewStreamEntry([u8; UNSIGNED_REVIEW_STREAM_ENTRY_BYTES]);

catcoms_rt::redacted_debug! {
    ReviewStreamEntry;
    ReviewStreamEntryId;
    ReviewStreamPosition;
    UnsignedReviewStreamEntry;
}

impl UnsignedReviewStreamEntry {
    /// Prepare the input the journal will retain. Exact recovery rederives
    /// it from the pinned scope, reserved position and original body.
    pub fn prepare(
        trusted: &FounderGenesisAnchorData,
        position: ReviewStreamPosition,
        decision: &ExactReviewDecisionBody,
    ) -> Result<Self, ChatError> {
        let entry = ReviewStreamEntry {
            stream: stream_data(trusted)?,
            position,
            body: decision.id(),
            signature: [0; 64],
        };
        entry.require_shape()?;
        entry.require_decision(trusted, decision)?;
        Ok(Self(entry.semantic_bytes()))
    }

    pub fn signing_bytes(&self) -> &[u8; UNSIGNED_REVIEW_STREAM_ENTRY_BYTES] {
        &self.0
    }

    /// Borrowing an authenticated local intent can name a historical
    /// membership position. Its ancestry and preimage still need reproof
    /// against the selected journal and membership.
    pub fn read_position_data(
        bytes: &[u8; UNSIGNED_REVIEW_STREAM_ENTRY_BYTES],
    ) -> Result<ReviewStreamPosition, ChatError> {
        let mut frame = [0; REVIEW_STREAM_ENTRY_BYTES];
        frame[..UNSIGNED_REVIEW_STREAM_ENTRY_BYTES].copy_from_slice(bytes);
        Ok(ReviewStreamEntry::decode(&frame)?.position_data())
    }

    /// Verify retained raw output against the exact input. An interrupted
    /// producer can finish from verified Raw without entering the key again.
    pub fn attach_signature(
        self,
        trusted: &FounderGenesisAnchorData,
        signature: [u8; 64],
    ) -> Result<ReviewStreamEntry, ChatError> {
        let mut bytes = [0; REVIEW_STREAM_ENTRY_BYTES];
        bytes[..UNSIGNED_REVIEW_STREAM_ENTRY_BYTES].copy_from_slice(&self.0);
        bytes[UNSIGNED_REVIEW_STREAM_ENTRY_BYTES..].copy_from_slice(&signature);
        let entry = ReviewStreamEntry::decode(&bytes)?;
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
        if !verify_with_public_bytes(&trusted.founder, &self.semantic_bytes(), &self.signature) {
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
            &self.semantic_bytes(),
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

    pub fn encode(&self) -> [u8; REVIEW_STREAM_ENTRY_BYTES] {
        let mut bytes = [0; REVIEW_STREAM_ENTRY_BYTES];
        bytes[..UNSIGNED_REVIEW_STREAM_ENTRY_BYTES].copy_from_slice(&self.semantic_bytes());
        bytes[UNSIGNED_REVIEW_STREAM_ENTRY_BYTES..].copy_from_slice(&self.signature);
        bytes
    }

    /// Decode bounded fixed data. Proof uses the locally pinned full key.
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() != REVIEW_STREAM_ENTRY_BYTES {
            return Err(ChatError::Malformed);
        }
        let remaining = bytes.strip_prefix(DOMAIN).ok_or(ChatError::Malformed)?;
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
        let (body, signature) = remaining
            .split_first_chunk::<32>()
            .ok_or(ChatError::Malformed)?;
        let entry = Self {
            stream: *stream,
            position: ReviewStreamPosition {
                sequence: u64::from_be_bytes(*sequence),
                previous: (*previous != [0; 32]).then_some(ReviewStreamEntryId(*previous)),
                membership_header: OwnerHeaderId::from_bytes(*membership_header),
                membership_epoch: u64::from_be_bytes(*membership_epoch),
            },
            body: OwnerBodyId::from_bytes(*body),
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
        Ok(())
    }

    fn semantic_bytes(&self) -> [u8; UNSIGNED_REVIEW_STREAM_ENTRY_BYTES] {
        let position = self.position;
        let sequence = position.sequence.to_be_bytes();
        let previous = position.previous.map_or([0; 32], |id| id.0);
        let epoch = position.membership_epoch.to_be_bytes();
        let fields: [&[u8]; 7] = [
            DOMAIN,
            &self.stream,
            &sequence,
            &previous,
            position.membership_header.as_bytes(),
            &epoch,
            self.body.as_bytes(),
        ];
        let mut bytes = [0; UNSIGNED_REVIEW_STREAM_ENTRY_BYTES];
        let mut next = bytes.as_mut_slice();
        for field in fields {
            let (prefix, tail) = next.split_at_mut(field.len());
            prefix.copy_from_slice(field);
            next = tail;
        }
        bytes
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
