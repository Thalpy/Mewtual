//! Exact signed body data for a removed-post decision. Selection still
//! requires an authenticated founder and a compatible governing journal.
use super::*;
use crate::chat::{
    cold_actions::OriginalActionDigest, V2RemovedCreateEvidence, V2RemovedCreateRecord,
};

const DOMAIN: &[u8] = b"mewtual/chat/held-review-decision/v1\0";
pub const MAX_EXACT_REVIEW_BODY_BYTES: usize =
    4 + DOMAIN.len() + 4 + MAX_REVIEW_BODY_BYTES + 2 * 36 + 8;

/// Byte custody and governing removal that stay fixed across a case.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ExactReviewOriginal {
    pub digest: OriginalActionDigest,
    pub remove_header: OwnerHeaderId,
    pub remove_epoch: u64,
}

/// Versioned decision data with a separate canonical body ID from the
/// semantic-only Review body. A caller must separately verify current case
/// selection and retain the exact original and signed governing Header.
#[derive(Clone, PartialEq, Eq)]
pub struct ExactReviewDecisionBody {
    decision: ReviewDecisionBody,
    original: ExactReviewOriginal,
}

catcoms_rt::redacted_debug! {
    ExactReviewOriginal;
    ExactReviewDecisionBody;
}

impl ExactReviewDecisionBody {
    pub fn new(
        decision: ReviewDecisionBody,
        original: ExactReviewOriginal,
    ) -> Result<Self, ChatError> {
        // A review holds a Create or reply. Edit and Delete targets cannot
        // acquire approval through this body.
        if decision.spec().target.spec().create.is_some() || original.remove_epoch == 0 {
            return Err(ChatError::Malformed);
        }
        Ok(Self { decision, original })
    }

    pub fn decision(&self) -> &ReviewDecisionBody {
        &self.decision
    }

    pub fn original_data(&self) -> ExactReviewOriginal {
        self.original
    }

    pub fn case_id(&self) -> ReviewCaseId {
        self.decision.case_id()
    }

    pub fn id(&self) -> OwnerBodyId {
        body_id(&self.encode())
    }

    /// Check the signed Header's exact canonical body ID and scope.
    /// Current predecessor and governing selection remain separate checks.
    pub fn verify_header_binding(&self, header: &SignedOwnerHeader) -> Result<(), ChatError> {
        let target = self.decision.spec().target.spec();
        check_header(
            header,
            HeaderKind::ReviewDecision,
            &target.group,
            target.context.profile,
            self.id(),
        )
    }

    /// Match a decoded record's data. The caller must still prove that the
    /// actual selected case contains this record and its exact original.
    pub fn require_record_data(&self, held: &V2RemovedCreateRecord<'_>) -> Result<(), ChatError> {
        self.require_binding(
            held.target_data(),
            held.original_digest_data(),
            held.removal_data(),
        )
    }

    /// A missing reply parent can remain visible to the reviewer. It
    /// prevents Approval until reverified against exact selected ancestry.
    /// Before signing, rederive with the current verifier under physical
    /// selection and separately call its `require_current_removed_create`.
    pub fn require_evidence_data(&self, held: &V2RemovedCreateEvidence) -> Result<(), ChatError> {
        self.require_binding(
            held.target_data(),
            held.original_digest_data(),
            held.removal_data(),
        )?;
        if self.decision.spec().decision == ReviewDecision::Approve
            && held.waiting_for_reply_parent_data().is_some()
        {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }

    fn require_binding(
        &self,
        target: &AcceptanceRecord,
        digest: OriginalActionDigest,
        (remove_header, remove_epoch): (OwnerHeaderId, u64),
    ) -> Result<(), ChatError> {
        if self.decision.spec().target != *target
            || self.original
                != (ExactReviewOriginal {
                    digest,
                    remove_header,
                    remove_epoch,
                })
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }

    /// Check transition semantics against the supplied current case body.
    /// Callers must obtain that body from durable, current selection.
    pub fn verify_new_transition_from(&self, previous: Option<&Self>) -> Result<(), ChatError> {
        if previous.is_some_and(|previous| previous.original != self.original) {
            return Err(ChatError::Scope);
        }
        self.decision
            .verify_transition(previous.map(|previous| (&previous.decision, previous.id())))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Encoder::with_capacity(MAX_EXACT_REVIEW_BODY_BYTES);
        out.put_bytes(DOMAIN).expect("fixed review domain");
        out.put_bytes(&self.decision.encode())
            .expect("bounded review decision");
        out.put_bytes(self.original.digest.as_bytes())
            .expect("fixed original digest");
        out.put_bytes(self.original.remove_header.as_bytes())
            .expect("fixed Remove Header");
        out.put_u64(self.original.remove_epoch);
        out.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_EXACT_REVIEW_BODY_BYTES {
            return Err(ChatError::Bound);
        }
        let mut input = Decoder::new(bytes);
        if input.get_bytes()? != DOMAIN {
            return Err(ChatError::Malformed);
        }
        let decision = input.get_bytes()?;
        if decision.len() > MAX_REVIEW_BODY_BYTES {
            return Err(ChatError::Bound);
        }
        let original = ExactReviewOriginal {
            digest: OriginalActionDigest::from_bytes(digest(&mut input)?),
            remove_header: OwnerHeaderId::from_bytes(digest(&mut input)?),
            remove_epoch: input.get_u64()?,
        };
        input.finish()?;
        Self::new(ReviewDecisionBody::decode(decision)?, original)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::owner_header::{HeaderSpec, OwnerStateCommitment, OwnerTenureId};
    use catcoms_crypto::DeviceKeypair;

    fn original() -> ExactReviewOriginal {
        ExactReviewOriginal {
            digest: OriginalActionDigest::from_bytes([12; 32]),
            remove_header: OwnerHeaderId::from_bytes([13; 32]),
            remove_epoch: 3,
        }
    }

    fn body(
        kind: ReviewDecision,
        previous: Option<OwnerBodyId>,
        exact: ExactReviewOriginal,
    ) -> ExactReviewDecisionBody {
        let target = AcceptanceRecord::new(AcceptanceSpec {
            group: vec![4; MAX_GROUP_ID_BYTES],
            context: TreeContext {
                profile: OwnerFactId::from_bytes([5; 32]),
                admission: OwnerFactId::from_bytes([6; 32]),
            },
            channel: 7,
            action: ActionId::from_bytes([8; 32]),
            subject: DeviceKeypair::from_seed(&[43; 32])
                .verifying_key()
                .to_bytes(),
            grant: GrantId::from_bytes([9; 32]),
            basis: BasisId::from_bytes([10; 32]),
            create: None,
        })
        .unwrap();
        let decision = ReviewDecisionBody::new(ReviewSpec {
            target,
            intent: [11; 16],
            decision: kind,
            previous,
        })
        .unwrap();
        ExactReviewDecisionBody::new(decision, exact).unwrap()
    }

    fn header(body: &ExactReviewDecisionBody, id: OwnerBodyId) -> SignedOwnerHeader {
        let key = DeviceKeypair::from_seed(&[44; 32]);
        let target = body.decision().spec().target.spec();
        SignedOwnerHeader::sign_using(
            HeaderSpec {
                group: target.group.clone(),
                profile: target.context.profile,
                issuer: key.verifying_key().to_bytes(),
                tenure: OwnerTenureId::from_bytes([14; 32]),
                parent: OwnerHeaderId::from_bytes([15; 32]),
                sequence: 5,
                kind: HeaderKind::ReviewDecision,
                body: id,
                state: OwnerStateCommitment::from_bytes([16; 32]),
            },
            |bytes| Ok(key.sign(bytes)),
        )
        .unwrap()
    }

    #[test]
    fn exact_review_body_commits_the_original_and_remove_under_the_signed_header() {
        let current = body(
            ReviewDecision::Reject,
            Some(OwnerBodyId::from_bytes([17; 32])),
            original(),
        );
        let bytes = current.encode();
        assert_eq!(bytes.len(), MAX_EXACT_REVIEW_BODY_BYTES - (4 + 32));
        assert_eq!(ExactReviewDecisionBody::decode(&bytes).unwrap(), current);
        assert!(ReviewDecisionBody::decode(&bytes).is_err());
        assert!(ExactReviewDecisionBody::decode(&current.decision().encode()).is_err());
        for cut in 0..bytes.len() {
            assert!(ExactReviewDecisionBody::decode(&bytes[..cut]).is_err());
        }
        let mut tail = bytes;
        tail.push(0);
        assert!(ExactReviewDecisionBody::decode(&tail).is_err());
        let signed = header(&current, current.id());
        current.verify_header_binding(&signed).unwrap();
        assert!(current
            .verify_header_binding(&header(&current, current.decision().id()))
            .is_err());
        for changed in [
            ExactReviewOriginal {
                digest: OriginalActionDigest::from_bytes([18; 32]),
                ..original()
            },
            ExactReviewOriginal {
                remove_header: OwnerHeaderId::from_bytes([19; 32]),
                ..original()
            },
            ExactReviewOriginal {
                remove_epoch: 4,
                ..original()
            },
        ] {
            let other = ExactReviewDecisionBody::new(current.decision().clone(), changed).unwrap();
            assert_eq!(other.case_id(), current.case_id());
            assert_ne!(other.id(), current.id());
            assert!(other.verify_header_binding(&signed).is_err());
        }
    }

    #[test]
    fn exact_review_transitions_use_the_previous_exact_body_and_keep_approval_terminal() {
        let rejected = body(ReviewDecision::Reject, None, original());
        assert!(rejected.verify_new_transition_from(None).is_ok());
        let wrong = body(
            ReviewDecision::Reopen,
            Some(rejected.decision().id()),
            original(),
        );
        assert!(wrong.verify_new_transition_from(Some(&rejected)).is_err());
        let changed = body(
            ReviewDecision::Reopen,
            Some(rejected.id()),
            ExactReviewOriginal {
                digest: OriginalActionDigest::from_bytes([20; 32]),
                ..original()
            },
        );
        assert_eq!(
            changed.verify_new_transition_from(Some(&rejected)),
            Err(ChatError::Scope)
        );
        let reopened = body(ReviewDecision::Reopen, Some(rejected.id()), original());
        reopened
            .verify_new_transition_from(Some(&rejected))
            .unwrap();
        let approved = body(ReviewDecision::Approve, Some(reopened.id()), original());
        approved
            .verify_new_transition_from(Some(&reopened))
            .unwrap();
        for kind in [
            ReviewDecision::Reject,
            ReviewDecision::Reopen,
            ReviewDecision::Approve,
        ] {
            let next = body(kind, Some(approved.id()), original());
            assert!(next.verify_new_transition_from(Some(&approved)).is_err());
        }
    }

    #[test]
    fn exact_review_stream_keeps_a_separate_chain_in_the_same_membership_epoch() {
        use crate::chat::FounderGenesisAnchorData;
        let rejected = body(ReviewDecision::Reject, None, original());
        let target = rejected.decision().spec().target.spec();
        let founder = DeviceKeypair::from_seed(&[44; 32]);
        let anchor = FounderGenesisAnchorData {
            group: target.group.clone(),
            founder: founder.verifying_key().to_bytes(),
            profile: target.context.profile,
            admission: OwnerFactId::from_bytes([25; 32]),
            tenure: OwnerTenureId::from_bytes([26; 32]),
            header: OwnerHeaderId::from_bytes([27; 32]),
            state: OwnerStateCommitment::from_bytes([28; 32]),
        };
        let position = ReviewStreamPosition {
            sequence: 1,
            previous: None,
            membership_header: OwnerHeaderId::from_bytes([29; 32]),
            membership_epoch: original().remove_epoch,
        };
        let sign = |position, decision: &ExactReviewDecisionBody| {
            ReviewStreamEntry::sign_using(&anchor, position, decision, |bytes| {
                Ok(founder.sign(bytes))
            })
        };
        let first = sign(position, &rejected).unwrap();
        let prepared = UnsignedReviewStreamEntry::prepare(&anchor, position, &rejected).unwrap();
        let encoded = first.encode();
        assert_eq!(
            prepared.signing_bytes(),
            &encoded[..UNSIGNED_REVIEW_STREAM_ENTRY_BYTES]
        );
        let cached_raw: [u8; 64] = encoded[UNSIGNED_REVIEW_STREAM_ENTRY_BYTES..]
            .try_into()
            .unwrap();
        assert_eq!(
            prepared.attach_signature(&anchor, cached_raw).unwrap(),
            first
        );
        let changed_epoch = UnsignedReviewStreamEntry::prepare(
            &anchor,
            ReviewStreamPosition {
                membership_epoch: position.membership_epoch + 1,
                ..position
            },
            &rejected,
        )
        .unwrap();
        assert_eq!(
            changed_epoch.attach_signature(&anchor, cached_raw),
            Err(ChatError::Signature)
        );
        first.verify_next_for_pin(&anchor, None).unwrap();
        first.verify_decision_for_pin(&anchor, &rejected).unwrap();
        let reopened = body(ReviewDecision::Reopen, Some(rejected.id()), original());
        let next = sign(
            ReviewStreamPosition {
                sequence: 2,
                previous: Some(first.id()),
                ..position
            },
            &reopened,
        )
        .unwrap();
        next.verify_next_for_pin(&anchor, Some(&first)).unwrap();
        next.verify_decision_for_pin(&anchor, &reopened).unwrap();
        assert_eq!(
            next.position_data().membership_header,
            position.membership_header
        );
        assert_eq!(
            next.position_data().membership_epoch,
            position.membership_epoch
        );
        assert_eq!(
            next.verify_next_for_pin(&anchor, None),
            Err(ChatError::Dependency)
        );
        assert_eq!(
            next.verify_decision_for_pin(&anchor, &rejected),
            Err(ChatError::Scope)
        );

        let installed = body(ReviewDecision::Approve, None, original());
        let sibling = sign(position, &installed).unwrap();
        sibling.verify_next_for_pin(&anchor, None).unwrap();
        assert_ne!(first.id(), sibling.id());
        assert_eq!(
            next.verify_next_for_pin(&anchor, Some(&sibling)),
            Err(ChatError::Dependency)
        );

        let mut original_bytes = first.encode();
        assert_eq!(ReviewStreamEntry::decode(&original_bytes).unwrap(), first);
        assert!(SignedOwnerHeader::decode(&original_bytes).is_err());
        for cut in 0..original_bytes.len() {
            assert!(ReviewStreamEntry::decode(&original_bytes[..cut]).is_err());
        }
        *original_bytes.last_mut().unwrap() ^= 1;
        let bad_signature = ReviewStreamEntry::decode(&original_bytes).unwrap();
        assert_eq!(bad_signature.id(), first.id());
        assert_eq!(
            bad_signature.verify_for_pin(&anchor),
            Err(ChatError::Signature)
        );
        assert_eq!(
            next.verify_next_for_pin(&anchor, Some(&bad_signature)),
            Err(ChatError::Signature)
        );
        let same_founder_wrong_root = FounderGenesisAnchorData {
            header: OwnerHeaderId::from_bytes([30; 32]),
            ..anchor
        };
        assert_eq!(
            first.verify_for_pin(&same_founder_wrong_root),
            Err(ChatError::Scope)
        );
    }
}
