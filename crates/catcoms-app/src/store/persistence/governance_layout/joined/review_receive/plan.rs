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
    cold_index::NodeRefData, owner_bodies::ReviewStreamEntry, FounderGenesisAnchorData,
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

pub(super) struct ReviewLaneRead<'a> {
    pub(super) head: Option<LaneHead>,
    pub(super) key: &'a ReceiverKey,
    pub(super) indexes: &'a Indexes,
}

impl ReviewLaneRead<'_> {
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
