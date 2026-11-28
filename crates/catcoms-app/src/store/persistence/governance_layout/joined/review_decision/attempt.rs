//! Private signature custody for a decision intent. These authenticated
//! frames bind bytes and their predecessors. They do not authorize key entry.
use super::*;
use catcoms_replication::chat::{
    owner_bodies::{
        ExactReviewDecisionBody, ReviewStreamEntry, ReviewStreamPosition,
        UnsignedReviewStreamEntry, MAX_EXACT_REVIEW_BODY_BYTES, UNSIGNED_REVIEW_STREAM_ENTRY_BYTES,
    },
    FounderGenesisAnchorData, V2RemovedCreateRecord, MAX_REMOVED_CREATE_RECORD_BYTES,
};
use catcoms_wire::{Decoder, Encoder};

const KEY_DOMAIN: &str = "catcoms/private-chat-v2-review-journal-vault-key/v1";
const INTENT: &[u8] = b"catcoms/private-chat-v2-review-journal-intent/v1\0";
const BOUND_INTENT: &[u8] = b"catcoms/private-chat-v2-review-journal-intent/v2\0";
const ATTEMPT: &[u8] = b"catcoms/private-chat-v2-review-journal-attempt/v1\0";
const RAW: &[u8] = b"catcoms/private-chat-v2-review-journal-raw/v1\0";
const INTENT_COMMITMENT: &str = "catcoms/private-chat-v2-review-journal-intent-frame/v1";
const ATTEMPT_COMMITMENT: &str = "catcoms/private-chat-v2-review-journal-attempt-frame/v1";
const SEAL_BYTES: usize = 40;
const DIGEST_FIELD: usize = 4 + 32;
pub(super) const MAX_INTENT_BYTES: usize = SEAL_BYTES
    + 4
    + INTENT.len()
    + DIGEST_FIELD
    + DIGEST_FIELD
    + 4
    + UNSIGNED_REVIEW_STREAM_ENTRY_BYTES
    + 4
    + MAX_EXACT_REVIEW_BODY_BYTES
    + 4
    + MAX_REMOVED_CREATE_RECORD_BYTES;
pub(super) const ATTEMPT_BYTES: usize = SEAL_BYTES + 4 + ATTEMPT.len() + DIGEST_FIELD;
pub(super) const RAW_BYTES: usize = SEAL_BYTES + 4 + RAW.len() + 2 * DIGEST_FIELD + 4 + 64;

/// Bind all three roles to the independently mounted original ID11 marker.
pub(super) struct JournalKey(Zeroizing<[u8; 32]>);

impl JournalKey {
    pub(super) fn new(vault_key: &[u8; 32], selected_marker: &[u8; 32]) -> Self {
        let mut derivation = blake3::Hasher::new_derive_key(KEY_DOMAIN);
        derivation.update(vault_key);
        derivation.update(selected_marker);
        Self(Zeroizing::new(*derivation.finalize().as_bytes()))
    }

    pub(super) fn seal(
        &self,
        plain: Encoder,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let plain = Zeroizing::new(plain.finish());
        Ok(Zeroizing::new(frame(&seal(&self.0, &plain, rng)?)))
    }

    pub(super) fn open(&self, raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
        Ok(Zeroizing::new(unseal(&self.0, &unframe(raw)?)?))
    }

    pub(super) fn indexes(&self) -> super::selection::index::DecisionIndexes {
        super::selection::index::DecisionIndexes::new(&self.0)
    }

    pub(super) fn initial_head(&self) -> [u8; 32] {
        *blake3::keyed_hash(
            &self.0,
            b"catcoms/private-chat-v2-review-journal-empty-head/v1\0",
        )
        .as_bytes()
    }

    pub(super) fn open_intent(&self, raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if !(SEAL_BYTES + 1..=MAX_INTENT_BYTES).contains(&raw.len()) {
            return Err(denied());
        }
        let plaintext = self.open(raw)?;
        ReviewIntent::decode(&plaintext)?;
        Ok(plaintext)
    }
}

/// Borrowed intent DATA. The caller derives the position from the original
/// selected stream and membership before it can use this for an Attempt.
pub(super) struct ReviewIntent<'a> {
    pub(super) request_digest: [u8; 32],
    pub(super) selected_base: Option<[u8; 32]>,
    pub(super) decision: ExactReviewDecisionBody,
    pub(super) original: &'a [u8],
    unsigned: &'a [u8; UNSIGNED_REVIEW_STREAM_ENTRY_BYTES],
}

impl<'a> ReviewIntent<'a> {
    pub(super) fn seal(
        key: &JournalKey,
        request_digest: [u8; 32],
        decision: &ExactReviewDecisionBody,
        original: &[u8],
        (pin, position, selected_base): (
            &FounderGenesisAnchorData,
            ReviewStreamPosition,
            Option<[u8; 32]>,
        ),
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let unsigned =
            UnsignedReviewStreamEntry::prepare(pin, position, decision).map_err(|_| denied())?;
        let record = V2RemovedCreateRecord::decode(original).map_err(|_| denied())?;
        decision
            .require_record_data(&record)
            .map_err(|_| denied())?;
        let body = decision.encode();
        let domain = if selected_base.is_some() {
            BOUND_INTENT
        } else {
            INTENT
        };
        let mut encoded = Encoder::with_capacity(
            20 + domain.len()
                + 32
                + selected_base.map_or(0, |_| DIGEST_FIELD)
                + unsigned.signing_bytes().len()
                + body.len()
                + original.len(),
        );
        for field in [domain, &request_digest] {
            encoded.put_bytes(field).map_err(|_| denied())?;
        }
        if let Some(base) = selected_base {
            if base == [0; 32] {
                return Err(denied());
            }
            encoded.put_bytes(&base).map_err(|_| denied())?;
        }
        for field in [unsigned.signing_bytes().as_slice(), &body, original] {
            encoded.put_bytes(field).map_err(|_| denied())?;
        }
        let sealed = key.seal(encoded, rng)?;
        if sealed.len() > MAX_INTENT_BYTES {
            return Err(denied());
        }
        Ok(sealed)
    }

    pub(super) fn decode(plaintext: &'a [u8]) -> Result<Self, AppError> {
        if plaintext.len() > MAX_INTENT_BYTES - SEAL_BYTES {
            return Err(denied());
        }
        let mut input = Decoder::new(plaintext);
        let domain = next(&mut input)?;
        if domain != INTENT && domain != BOUND_INTENT {
            return Err(denied());
        }
        let request_digest = next(&mut input)?.try_into().map_err(|_| denied())?;
        let selected_base = if domain == BOUND_INTENT {
            let base: [u8; 32] = next(&mut input)?.try_into().map_err(|_| denied())?;
            if base == [0; 32] {
                return Err(denied());
            }
            Some(base)
        } else {
            None
        };
        let unsigned = next(&mut input)?.try_into().map_err(|_| denied())?;
        let body = next(&mut input)?;
        let original = next(&mut input)?;
        input.finish().map_err(|_| denied())?;
        if body.len() > MAX_EXACT_REVIEW_BODY_BYTES
            || original.len() > MAX_REMOVED_CREATE_RECORD_BYTES
        {
            return Err(denied());
        }
        let decision = ExactReviewDecisionBody::decode(body).map_err(|_| denied())?;
        let held = V2RemovedCreateRecord::decode(original).map_err(|_| denied())?;
        decision.require_record_data(&held).map_err(|_| denied())?;
        Ok(Self {
            request_digest,
            selected_base,
            decision,
            original,
            unsigned,
        })
    }

    pub(super) fn rederive(
        &self,
        pin: &FounderGenesisAnchorData,
        position: ReviewStreamPosition,
    ) -> Result<UnsignedReviewStreamEntry, AppError> {
        let expected = UnsignedReviewStreamEntry::prepare(pin, position, &self.decision)
            .map_err(|_| denied())?;
        if expected.signing_bytes() != self.unsigned {
            return Err(denied());
        }
        Ok(expected)
    }

    pub(super) fn position_data(&self) -> Result<ReviewStreamPosition, AppError> {
        UnsignedReviewStreamEntry::read_position_data(self.unsigned).map_err(|_| denied())
    }
}

/// Parent commitment from an authenticated intent's exact sealed frame.
/// Its presence on disk, covering barrier and native cut are separate.
pub(super) struct AttemptParent([u8; 32]);

impl AttemptParent {
    pub(super) fn from_authenticated_frame(frame: &[u8]) -> Result<Self, AppError> {
        if !(SEAL_BYTES + 1..=MAX_INTENT_BYTES).contains(&frame.len()) {
            return Err(denied());
        }
        Ok(Self(blake3::derive_key(INTENT_COMMITMENT, frame)))
    }

    pub(super) fn seal_attempt(
        &self,
        key: &JournalKey,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut plain = Encoder::with_capacity(ATTEMPT_BYTES - SEAL_BYTES);
        plain.put_bytes(ATTEMPT).map_err(|_| denied())?;
        plain.put_bytes(&self.0).map_err(|_| denied())?;
        key.seal(plain, rng)
    }

    pub(super) fn require_attempt(&self, key: &JournalKey, attempt: &[u8]) -> Result<(), AppError> {
        if attempt.len() != ATTEMPT_BYTES {
            return Err(denied());
        }
        let plain = key.open(attempt)?;
        let mut reader = Decoder::new(&plain);
        if next(&mut reader)? != ATTEMPT || next(&mut reader)? != self.0 {
            return Err(denied());
        }
        reader.finish().map_err(|_| denied())
    }

    pub(super) fn retain_raw(
        &self,
        key: &JournalKey,
        attempt: &[u8],
        signature: [u8; 64],
        (pin, prepared): (&FounderGenesisAnchorData, UnsignedReviewStreamEntry),
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(ReviewStreamEntry, Zeroizing<Vec<u8>>), AppError> {
        self.require_attempt(key, attempt)?;
        let signed = prepared
            .attach_signature(pin, signature)
            .map_err(|_| denied())?;
        let attempt_digest = blake3::derive_key(ATTEMPT_COMMITMENT, attempt);
        let mut plain = Encoder::with_capacity(RAW_BYTES - SEAL_BYTES);
        for field in [RAW, &self.0, &attempt_digest, &signature] {
            plain.put_bytes(field).map_err(|_| denied())?;
        }
        Ok((signed, key.seal(plain, rng)?))
    }

    pub(super) fn recover_raw(
        &self,
        key: &JournalKey,
        attempt: &[u8],
        raw: &[u8],
        (pin, prepared): (&FounderGenesisAnchorData, UnsignedReviewStreamEntry),
    ) -> Result<ReviewStreamEntry, AppError> {
        let signature = self.open_raw_data(key, attempt, raw)?;
        prepared
            .attach_signature(pin, signature)
            .map_err(|_| denied())
    }

    /// Authenticate storage roles separately from the exact signing bytes
    /// and original founder signature's verification.
    pub(super) fn open_raw_data(
        &self,
        key: &JournalKey,
        attempt: &[u8],
        raw: &[u8],
    ) -> Result<[u8; 64], AppError> {
        self.require_attempt(key, attempt)?;
        if raw.len() != RAW_BYTES {
            return Err(denied());
        }
        let plain = key.open(raw)?;
        let mut reader = Decoder::new(&plain);
        if next(&mut reader)? != RAW
            || next(&mut reader)? != self.0
            || next(&mut reader)? != blake3::derive_key(ATTEMPT_COMMITMENT, attempt)
        {
            return Err(denied());
        }
        let signature = next(&mut reader)?.try_into().map_err(|_| denied())?;
        reader.finish().map_err(|_| denied())?;
        Ok(signature)
    }
}

fn next<'a>(input: &mut Decoder<'a>) -> Result<&'a [u8], AppError> {
    input.get_bytes().map_err(|_| denied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::persistence::v2_review_hold::tests::{fixture, scope};
    use catcoms_crypto::DeviceKeypair;
    use catcoms_replication::chat::{
        owner_bodies::{ExactReviewOriginal, ReviewDecision, ReviewDecisionBody, ReviewSpec},
        owner_facts::OwnerFactId,
        owner_header::{OwnerHeaderId, OwnerStateCommitment},
    };
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn review_attempt_reopens_only_its_exact_intent_and_raw_signature() {
        let mut rng = ChaCha20Rng::seed_from_u64(711);
        let scope = scope();
        let (held, original) = fixture(scope, 21, 22);
        let target = V2RemovedCreateRecord::decode(&held)
            .unwrap()
            .target_data()
            .clone();
        let decision = ExactReviewDecisionBody::new(
            ReviewDecisionBody::new(ReviewSpec {
                target,
                intent: [23; 16],
                decision: ReviewDecision::Reject,
                previous: None,
            })
            .unwrap(),
            ExactReviewOriginal {
                digest: original.original,
                remove_header: original.removal.0,
                remove_epoch: original.removal.1,
            },
        )
        .unwrap();
        let founder = DeviceKeypair::from_seed(&[74; 32]);
        let pin = FounderGenesisAnchorData {
            group: scope.group.to_vec(),
            founder: founder.verifying_key().to_bytes(),
            profile: scope.profile,
            admission: OwnerFactId::from_bytes([24; 32]),
            tenure: scope.tenure,
            header: OwnerHeaderId::from_bytes([25; 32]),
            state: OwnerStateCommitment::from_bytes([26; 32]),
        };
        let position = ReviewStreamPosition {
            sequence: 1,
            previous: None,
            membership_header: original.removal.0,
            membership_epoch: original.removal.1,
        };
        let key = JournalKey::new(&[27; 32], &[28; 32]);
        let wrong_mount = JournalKey::new(&[27; 32], &[29; 32]);
        let seal_intent = |rng: &mut ChaCha20Rng| {
            ReviewIntent::seal(
                &key,
                [30; 32],
                &decision,
                &held,
                (&pin, position, None),
                rng,
            )
            .unwrap()
        };
        let intent = seal_intent(&mut rng);
        let plain = key.open_intent(&intent).unwrap();
        assert!(wrong_mount.open_intent(&intent).is_err());
        let checked = ReviewIntent::decode(&plain).unwrap();
        assert_eq!(checked.request_digest, [30; 32]);
        assert!(checked.selected_base.is_none());
        let bound = ReviewIntent::seal(
            &key,
            [30; 32],
            &decision,
            &held,
            (&pin, position, Some([32; 32])),
            &mut rng,
        )
        .unwrap();
        let bound = key.open_intent(&bound).unwrap();
        assert_eq!(
            ReviewIntent::decode(&bound).unwrap().selected_base,
            Some([32; 32])
        );
        assert_eq!(checked.original, held.as_slice());
        assert_eq!(checked.decision, decision);
        assert!(checked
            .rederive(
                &pin,
                ReviewStreamPosition {
                    membership_header: OwnerHeaderId::from_bytes([31; 32]),
                    ..position
                }
            )
            .is_err());
        let prepared = || checked.rederive(&pin, position).unwrap();
        let parent = AttemptParent::from_authenticated_frame(&intent).unwrap();
        let attempt = parent.seal_attempt(&key, &mut rng).unwrap();
        let second_attempt = parent.seal_attempt(&key, &mut rng).unwrap();
        let second_parent =
            AttemptParent::from_authenticated_frame(&seal_intent(&mut rng)).unwrap();
        assert!(second_parent.require_attempt(&key, &attempt).is_err());
        assert!(parent
            .require_attempt(&key, &attempt[..attempt.len() - 1])
            .is_err());
        let signature = founder.sign(prepared().signing_bytes());
        assert!(parent
            .retain_raw(&key, &attempt, [0; 64], (&pin, prepared()), &mut rng)
            .is_err());
        let (entry, raw) = parent
            .retain_raw(&key, &attempt, signature, (&pin, prepared()), &mut rng)
            .unwrap();
        assert_eq!(
            parent
                .recover_raw(&key, &attempt, &raw, (&pin, prepared()))
                .unwrap(),
            entry
        );
        assert!(parent
            .recover_raw(&key, &second_attempt, &raw, (&pin, prepared()))
            .is_err());
        assert!(parent
            .recover_raw(&key, &attempt, &raw[..raw.len() - 1], (&pin, prepared()))
            .is_err());
        assert!(parent
            .recover_raw(&wrong_mount, &attempt, &raw, (&pin, prepared()))
            .is_err());
    }
}
