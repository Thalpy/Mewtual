//! Recover exactly the previously prepared manifest update from authenticated
//! outputs. A repaired transaction keeps its original selected predecessor.
use super::{
    approval,
    header::ReceiverKey,
    index::{Indexes, Query},
    lane_head::LaneHead,
    manifest::{head_digest, ManifestKey, ReviewScope, SelectedLane},
    objects::HeldLane,
    plan::{ApprovalSelection, ReviewLaneRead, ReviewPlan},
    wal::{Effect, WriteAhead},
    *,
};
use catcoms_replication::chat::{owner_bodies::ReviewStreamRecord, FounderGenesisAnchorData};

pub(super) struct TransactionScope<'a> {
    pub(super) scope: ReviewScope,
    pub(super) pin: &'a FounderGenesisAnchorData,
    pub(super) manifest_key: &'a ManifestKey,
    pub(super) lane_key: &'a ReceiverKey,
    pub(super) indexes: &'a Indexes,
}

impl TransactionScope<'_> {
    /// `lane` was opened from the original manifest. It cannot be derived
    /// from the proposed WAL head, even when that head is already visible.
    pub(super) fn verify(
        &self,
        source: Option<&HeldLane>,
        transaction: &WriteAhead<'_>,
    ) -> Result<SelectedLane, AppError> {
        if self.scope.as_bytes() != &transaction.scope {
            return Err(denied());
        }
        let mut original = self.manifest_key.open(transaction.manifests[0])?;
        let next = self.manifest_key.open(transaction.manifests[1])?;
        let previous = original.selected(self.scope);
        if previous.map(|lane| (lane.head, lane.frozen))
            != source.map(|lane| (head_digest(&lane.sealed_head.bytes), lane.head.is_frozen()))
        {
            return Err(denied());
        }
        let read = ReviewLaneRead {
            head: source.map(|lane| lane.head),
            key: self.lane_key,
            indexes: self.indexes,
        };
        let read_page = |id| source.ok_or_else(denied)?.page(id);
        let read_header = |digest| source.ok_or_else(denied)?.header(digest);
        let effect_head = LaneHead::open(self.lane_key, transaction.head)?;
        let selected = match &transaction.effect {
            Effect::Header { frame, paths } => {
                let incoming = self.lane_key.open_header(self.pin, frame)?;
                let ReviewPlan::Append { case } =
                    read.plan(self.pin, &incoming, read_page, read_header)?
                else {
                    return Err(denied());
                };
                let proposed = Indexes::row(&incoming, frame)?;
                let pair = [
                    (
                        Query::Ordinal(proposed.sequence),
                        source.map(|lane| lane.head.ordinal),
                        None,
                        effect_head.ordinal,
                    ),
                    (
                        Query::Case(proposed.case),
                        source.map(|lane| lane.head.cases),
                        case,
                        effect_head.cases,
                    ),
                ];
                for ((kind, previous, expected, root), path) in pair.into_iter().zip(paths) {
                    let actual = self.indexes.require_delta(
                        (kind, previous),
                        (expected, proposed),
                        (root, &path.exact),
                        read_page,
                    )?;
                    if actual != path.old {
                        return Err(denied());
                    }
                }
                let recomputed = LaneHead::next(
                    &incoming,
                    source.map(|lane| (lane.head, lane.sealed_head.bytes.as_slice())),
                    [effect_head.ordinal, effect_head.cases],
                )?;
                if effect_head != recomputed {
                    return Err(denied());
                }
                SelectedLane::successor(
                    previous,
                    transaction.head,
                    std::iter::once(frame.len()).chain(
                        paths
                            .iter()
                            .flat_map(|path| path.exact.iter().map(|(_, raw)| raw.len())),
                    ),
                    false,
                )?
            }
            Effect::Fork(frame) => {
                let source = source.ok_or_else(denied)?;
                let (older, incoming) = self.lane_key.open_fork(self.pin, frame)?;
                let ReviewPlan::Fork(original) =
                    read.plan(self.pin, &incoming, read_page, read_header)?
                else {
                    return Err(denied());
                };
                if older != *original
                    || effect_head != source.head.freeze(&source.sealed_head.bytes, frame)?
                {
                    return Err(denied());
                }
                SelectedLane::successor(previous, transaction.head, [frame.len()], true)?
            }
            Effect::Approval { frame, path } => {
                let source = source.ok_or_else(denied)?;
                let raw = approval::open(self.lane_key, self.pin, frame)?;
                let record = ReviewStreamRecord::decode(self.pin, &raw).map_err(|_| denied())?;
                let ApprovalSelection::Selected { row: header, .. } =
                    read.select_approval(self.pin, record.entry_data(), read_page, read_header)?
                else {
                    return Err(denied());
                };
                let row = approval::row(header, &record, frame)?;
                let proposed = effect_head.approvals.ok_or_else(denied)?;
                let old = self.indexes.require_approved_delta(
                    source.head.approvals,
                    row,
                    (proposed, &path.exact),
                    read_page,
                )?;
                if old != path.old
                    || effect_head
                        != source
                            .head
                            .cover_approval(&source.sealed_head.bytes, proposed)?
                {
                    return Err(denied());
                }
                SelectedLane::successor(
                    previous,
                    transaction.head,
                    std::iter::once(frame.len())
                        .chain(path.exact.iter().map(|(_, frame)| frame.len())),
                    false,
                )?
            }
        };
        original.replace(self.scope, previous, selected)?;
        if original != next {
            return Err(denied());
        }
        Ok(selected)
    }
}
