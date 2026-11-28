//! Classify one independently authenticated review arrival against the
//! receiver's original selected lane. A plan includes no write authority.
use super::{
    header::ReceiverKey,
    index::{Indexes, Query},
    lane_head::LaneHead,
    *,
};
use crate::store::persistence::v2_text::private_index::ReceivedReviewIndexValueData;
use catcoms_replication::chat::{
    cold_index::NodeRefData,
    owner_bodies::{ReviewDecision, ReviewStreamEntry},
    FounderGenesisAnchorData,
};
use catcoms_storage::private_chat_page::PrivateChatPageId;

pub(super) enum ReviewPlan {
    Duplicate,
    Frozen,
    Missing(u64),
    Append {
        case: Option<ReceivedReviewIndexValueData>,
    },
    Fork(Box<ReviewStreamEntry>),
}

pub(in crate::store) enum ApprovalSelection {
    Selected {
        row: ReceivedReviewIndexValueData,
        header: Box<ReviewStreamEntry>,
    },
    WaitingFor(u64),
    Frozen,
}

pub(super) struct ReviewLaneRead<'a> {
    pub(super) head: Option<LaneHead>,
    pub(super) key: &'a ReceiverKey,
    pub(super) indexes: &'a Indexes,
}

impl ReviewLaneRead<'_> {
    /// Both selected roots must independently address the same exact
    /// public Approve. Possession of a signed full record alone cannot skip
    /// a chain gap, replace the selected case or enter a frozen lane.
    pub(super) fn select_approval(
        &self,
        pin: &FounderGenesisAnchorData,
        supplied: &ReviewStreamEntry,
        mut read_page: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        mut read_header: impl FnMut([u8; 32]) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<ApprovalSelection, AppError> {
        supplied.verify_for_pin(pin).map_err(|_| denied())?;
        let step = supplied.case_step_data().ok_or_else(denied)?;
        if step.choice_data() != ReviewDecision::Approve {
            return Err(denied());
        }
        let Some(head) = self.head else {
            return Ok(ApprovalSelection::WaitingFor(1));
        };
        if head.is_frozen() {
            return Ok(ApprovalSelection::Frozen);
        }
        let sequence = supplied.position_data().sequence;
        if sequence > head.sequence {
            return Ok(ApprovalSelection::WaitingFor(
                head.sequence.checked_add(1).ok_or_else(denied)?,
            ));
        }
        let (ordinal, original) = self
            .load(
                pin,
                head.ordinal,
                Query::Ordinal(sequence),
                &mut read_page,
                &mut read_header,
            )?
            .ok_or_else(denied)?;
        let case = self
            .indexes
            .lookup(head.cases, Query::Case(step.case_data()), &mut read_page)?
            .ok_or_else(denied)?;
        if ordinal != case
            || &original != supplied
            || (sequence == head.sequence && supplied.id().as_bytes() != &head.entry)
        {
            return Err(denied());
        }
        Ok(ApprovalSelection::Selected {
            row: ordinal,
            header: Box::new(original),
        })
    }

    pub(super) fn plan(
        &self,
        pin: &FounderGenesisAnchorData,
        incoming: &ReviewStreamEntry,
        mut read_page: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        mut read_header: impl FnMut([u8; 32]) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<ReviewPlan, AppError> {
        incoming.verify_for_pin(pin).map_err(|_| denied())?;
        let step = incoming.case_step_data().ok_or_else(denied)?;
        let sequence = incoming.position_data().sequence;
        let Some(head) = self.head else {
            if sequence != 1 {
                return Ok(ReviewPlan::Missing(1));
            }
            incoming
                .verify_public_next_for_pin(pin, None, None)
                .map_err(|_| denied())?;
            return Ok(ReviewPlan::Append { case: None });
        };

        if sequence <= head.sequence {
            let selected = self
                .load(
                    pin,
                    head.ordinal,
                    Query::Ordinal(sequence),
                    &mut read_page,
                    &mut read_header,
                )?
                .ok_or_else(denied)?;
            if selected.1.id() == incoming.id() {
                return Ok(ReviewPlan::Duplicate);
            }
            if head.is_frozen() {
                return Ok(ReviewPlan::Frozen);
            }
            return Ok(ReviewPlan::Fork(Box::new(selected.1)));
        }
        if head.is_frozen() {
            return Ok(ReviewPlan::Frozen);
        }
        let expected = head.sequence.checked_add(1).ok_or_else(denied)?;
        if sequence != expected {
            return Ok(ReviewPlan::Missing(expected));
        }
        let (_, previous) = self
            .load(
                pin,
                head.ordinal,
                Query::Ordinal(head.sequence),
                &mut read_page,
                &mut read_header,
            )?
            .ok_or_else(denied)?;
        if previous.id().as_bytes() != &head.entry {
            return Err(denied());
        }
        let case = self.load(
            pin,
            head.cases,
            Query::Case(step.case_data()),
            &mut read_page,
            &mut read_header,
        )?;
        let prior = case
            .as_ref()
            .map(|(row, entry)| {
                // The point case entry must also be the exact original row
                // in the immutable ordinal index under this same head.
                if row.sequence > head.sequence
                    || self.indexes.lookup(
                        head.ordinal,
                        Query::Ordinal(row.sequence),
                        &mut read_page,
                    )? != Some(*row)
                {
                    return Err(denied());
                }
                Ok((
                    entry.decision_id_data(),
                    entry.case_step_data().ok_or_else(denied)?,
                ))
            })
            .transpose()?;
        incoming
            .verify_public_next_for_pin(pin, Some(&previous), prior)
            .map_err(|_| denied())?;
        Ok(ReviewPlan::Append {
            case: case.map(|(row, _)| row),
        })
    }

    fn load(
        &self,
        pin: &FounderGenesisAnchorData,
        root: NodeRefData,
        query: Query,
        read_page: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        read_header: &mut impl FnMut([u8; 32]) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<Option<(ReceivedReviewIndexValueData, ReviewStreamEntry)>, AppError> {
        let Some(row) = self.indexes.lookup(root, query, read_page)? else {
            return Ok(None);
        };
        let bytes = read_header(row.frame)?;
        let entry = self.key.open_header(pin, &bytes)?;
        if Indexes::row(&entry, &bytes)? != row {
            return Err(denied());
        }
        Ok(Some((row, entry)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_crypto::DeviceKeypair;
    use catcoms_replication::chat::{
        acceptance_tree::TreeContext,
        cold_actions::OriginalActionDigest,
        join_bindings::GenesisAnchorBindingData,
        owner_bodies::{
            AcceptanceRecord, AcceptanceSpec, ExactReviewDecisionBody, ExactReviewOriginal,
            ReviewDecision, ReviewDecisionBody, ReviewSpec, ReviewStreamPosition,
            UnsignedReviewStreamEntry,
        },
        owner_facts::OwnerFactId,
        owner_header::{OwnerHeaderId, OwnerStateCommitment, OwnerTenureId},
        ActionId, BasisId, GrantId,
    };
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn recipient_signed_fork_preserves_both_exact_headers_and_the_original_tip() {
        let founder = DeviceKeypair::from_seed(&[2; 32]);
        let pin = FounderGenesisAnchorData {
            group: vec![1; 16],
            founder: founder.verifying_key().to_bytes(),
            profile: OwnerFactId::from_bytes([3; 32]),
            admission: OwnerFactId::from_bytes([4; 32]),
            tenure: OwnerTenureId::from_bytes([5; 32]),
            header: OwnerHeaderId::from_bytes([6; 32]),
            state: OwnerStateCommitment::from_bytes([7; 32]),
        };
        let binding = GenesisAnchorBindingData {
            group: pin.group.clone(),
            founder: pin.founder,
            profile: *pin.profile.as_bytes(),
            admission: *pin.admission.as_bytes(),
            tenure: *pin.tenure.as_bytes(),
            header: *pin.header.as_bytes(),
            state: *pin.state.as_bytes(),
        };
        let position = ReviewStreamPosition {
            sequence: 1,
            previous: None,
            membership_header: OwnerHeaderId::from_bytes([14; 32]),
            membership_epoch: 3,
        };
        let signed = |choice, intent| {
            let target = AcceptanceRecord::new(AcceptanceSpec {
                group: pin.group.clone(),
                context: TreeContext {
                    profile: pin.profile,
                    admission: OwnerFactId::from_bytes([8; 32]),
                },
                channel: 9,
                action: ActionId::from_bytes([10; 32]),
                subject: DeviceKeypair::from_seed(&[11; 32])
                    .verifying_key()
                    .to_bytes(),
                grant: GrantId::from_bytes([12; 32]),
                basis: BasisId::from_bytes([13; 32]),
                create: None,
            })
            .unwrap();
            let body = ReviewDecisionBody::new(ReviewSpec {
                target,
                intent: [intent; 16],
                decision: choice,
                previous: None,
            })
            .unwrap();
            let exact = ExactReviewDecisionBody::new(
                body,
                ExactReviewOriginal {
                    digest: OriginalActionDigest::from_bytes([15; 32]),
                    remove_header: position.membership_header,
                    remove_epoch: position.membership_epoch,
                },
            )
            .unwrap();
            let prepared =
                UnsignedReviewStreamEntry::prepare_public(&pin, position, &exact).unwrap();
            let signature = founder.sign(prepared.signing_bytes());
            prepared.attach_signature(&pin, signature).unwrap()
        };
        let first = signed(ReviewDecision::Reject, 16);
        let contradiction = signed(ReviewDecision::Approve, 17);
        let mut rng = ChaCha20Rng::seed_from_u64(809);
        let scope = manifest::ReviewScope::derive_data(&[18; 32], &binding).unwrap();
        let key = ReceiverKey::new(&[19; 32], &[20; 32], scope.as_bytes());
        let indexes = Indexes::new(&[19; 32], scope);
        let first_frame = key.cover_header_data(&pin, &first, &mut rng).unwrap();
        let row = Indexes::row(&first, &first_frame).unwrap();
        let paths = [Query::Ordinal(1), Query::Case(row.case)].map(|kind| {
            indexes
                .prepare(kind, None, (None, row), |_| unreachable!(), &mut rng)
                .unwrap()
        });
        let head = LaneHead::next(&first, None, paths.each_ref().map(|path| path.root)).unwrap();
        let read_page = |id| {
            let page = paths
                .iter()
                .flat_map(|path| &path.new)
                .find(|page| page.reference.page == id)
                .ok_or_else(denied)?;
            Ok(Zeroizing::new(page.frame.as_bytes().to_vec()))
        };
        let read_header = |id| {
            if id != row.frame {
                return Err(denied());
            }
            Ok(first_frame.clone())
        };
        let read = ReviewLaneRead {
            head: Some(head),
            key: &key,
            indexes: &indexes,
        };
        assert!(matches!(
            read.plan(&pin, &first, read_page, read_header).unwrap(),
            ReviewPlan::Duplicate
        ));
        let ReviewPlan::Fork(selected) = read
            .plan(&pin, &contradiction, read_page, read_header)
            .unwrap()
        else {
            panic!("an authenticated contradiction must be preserved");
        };
        assert!(*selected == first);
        let fork = key
            .cover_fork_data(&pin, &selected, &contradiction, &mut rng)
            .unwrap();
        let original_head = head.seal(&key, &mut rng).unwrap();
        let frozen = head.freeze(&original_head, &fork).unwrap();
        assert_eq!(frozen.sequence, head.sequence);
        assert_eq!(frozen.entry, head.entry);
        assert!(frozen.is_frozen());
        let (older, seen) = key.open_fork(&pin, &fork).unwrap();
        assert!(older == first && seen == contradiction);
        let read = ReviewLaneRead {
            head: Some(frozen),
            ..read
        };
        assert!(matches!(
            read.plan(&pin, &contradiction, read_page, read_header)
                .unwrap(),
            ReviewPlan::Frozen
        ));
    }
}
