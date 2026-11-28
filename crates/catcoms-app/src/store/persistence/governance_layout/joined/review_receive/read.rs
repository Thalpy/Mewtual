//! Paid reads of the selected review journal. Approval readers do not
//! recover pending work or change either history archive.
use super::{
    approval,
    header::ReceiverKey,
    index::Indexes,
    journal::pinned_founder,
    manifest::{ManifestKey, ReviewScope},
    objects::{lane_directory_name, HeldLane, MAX_VIEW_FILES},
    observation::ReviewReceiptObservation,
    plan::{ApprovalSelection, ReviewLaneRead},
    write::{MANIFEST_STAGE, WAL_NAME},
    *,
};
use catcoms_replication::chat::{
    cold_index::{MAX_INDEX_FRAME_BYTES, MAX_INDEX_HEIGHT},
    join_bindings::GenesisAnchorBindingData,
    owner_bodies::{ReviewStreamEntryId, ReviewStreamPosition, ReviewStreamRecord},
    ActionId,
};
use std::ops::RangeInclusive;

const READ_WORK: usize =
    3 * MAX_VIEW_FILES * MAX_INDEX_FRAME_BYTES + 6 * approval::MAX_APPROVAL_FRAME + 128 * 1024;
const POLL_WORK: usize = 128 * 1024;
const APPROVAL_SCAN_ORDINALS: usize = 4;
// Each ordinal may retain one complete point path and header. Only the
// one returned candidate also proves the current case through a second path.
const APPROVAL_SCAN_HELD_FILES: usize =
    (APPROVAL_SCAN_ORDINALS + 1) * (MAX_INDEX_HEIGHT as usize + 1) + APPROVAL_SCAN_ORDINALS;
const _: () = assert!(APPROVAL_SCAN_HELD_FILES <= MAX_VIEW_FILES);
const APPROVAL_SCAN_WORK: usize = POLL_WORK + 3 * APPROVAL_SCAN_HELD_FILES * MAX_INDEX_FRAME_BYTES;

pub(in crate::store) type ReviewApprovalScan = (
    Option<RangeInclusive<u64>>,
    Option<(ReviewStreamPosition, ReviewStreamEntryId)>,
);

pub(in crate::store) enum ReviewHeaderPoll {
    Unavailable,
    Frozen,
    Query(u64),
}

pub(in crate::store) struct SelectedReceivedApproval {
    pub(in crate::store) raw: Zeroizing<Vec<u8>>,
    pub(in crate::store) observation: ReviewReceiptObservation,
    _fee: CatalogReadReservation,
}

impl SelectedReviewReceiver {
    /// Read the durable public polling position. A covered WAL can settle
    /// first, but an ambiguous or frozen lane never becomes a fresh poll.
    pub(super) fn next_header_poll(
        &self,
        store: &ServerStore,
        binding: &GenesisAnchorBindingData,
        check_tip: bool,
    ) -> Result<ReviewHeaderPoll, AppError> {
        self.recover_admitted(store)?;
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let _fee = policy.current.archive.pending.pool.try_reserve(POLL_WORK)?;
        let _lease = store._session.retain_for_physical_work();
        let scope = ReviewScope::derive_data(&store.chat_vault_domain()?, binding)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let marker = self.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let mut manifest = self.read_manifest(store, &ManifestKey::new(&key, marker))?;
        let lane_key = ReceiverKey::new(&key, marker, scope.as_bytes());
        let mut lane = manifest
            .data
            .selected(scope)
            .map(|selected| HeldLane::read(&self.directory, (scope, selected), &lane_key))
            .transpose()?;
        let sequence = if let Some(lane) = lane.as_mut() {
            if lane.head.is_frozen() {
                ReviewHeaderPoll::Frozen
            } else if check_tip {
                ReviewHeaderPoll::Query(lane.head.sequence)
            } else {
                lane.head
                    .sequence
                    .checked_add(1)
                    .map_or(ReviewHeaderPoll::Unavailable, ReviewHeaderPoll::Query)
            }
        } else {
            absent(&self.directory.child(&lane_directory_name(scope)))?;
            ReviewHeaderPoll::Query(1)
        };
        manifest.require_current(&self.directory)?;
        if let Some(lane) = lane.as_mut() {
            lane.require(&self.directory)?;
        }
        self.require_settled()?;
        self.require(store)?;
        Ok(sequence)
    }

    fn require_settled(&self) -> Result<(), AppError> {
        absent(&self.directory.child(WAL_NAME))?;
        absent(&self.directory.child(MANIFEST_STAGE))
    }

    /// Scan one bounded slice of the durable ordinal index. The cursor is
    /// only an in-memory position. Every pass reopens the original selector,
    /// and every returned candidate must still pass exact receipt admission.
    pub(super) fn scan_approval_candidates(
        &self,
        store: &ServerStore,
        binding: &GenesisAnchorBindingData,
        cursor: Option<RangeInclusive<u64>>,
    ) -> Result<ReviewApprovalScan, AppError> {
        self.require(store)?;
        self.require_settled()?;
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let _fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(APPROVAL_SCAN_WORK)
            .map_err(|_| AppError::ChatV2ArchiveBusy)?;
        let _lease = store._session.retain_for_physical_work();
        let scope = ReviewScope::derive_data(&store.chat_vault_domain()?, binding)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let marker = self.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let mut manifest = self.read_manifest(store, &ManifestKey::new(&key, marker))?;
        let lane_key = ReceiverKey::new(&key, marker, scope.as_bytes());
        let indexes = Indexes::new(&key, scope);
        let mut lane = manifest
            .data
            .selected(scope)
            .map(|selected| HeldLane::read(&self.directory, (scope, selected), &lane_key))
            .transpose()?;
        let turn = if let Some(lane) = lane.as_ref() {
            if lane.head.is_frozen() {
                (None, None)
            } else {
                let pin = pinned_founder(binding);
                let selected = ReviewLaneRead {
                    head: Some(lane.head),
                    key: &lane_key,
                    indexes: &indexes,
                };
                scan_approval_ordinals(lane.head.sequence, cursor, |ordinal| {
                    selected
                        .approval_candidate(
                            &pin,
                            ordinal,
                            |id| lane.page(id),
                            |digest| lane.header(digest),
                        )
                        .map(|header| header.map(|header| (header.position_data(), header.id())))
                })?
            }
        } else {
            absent(&self.directory.child(&lane_directory_name(scope)))?;
            (None, None)
        };
        manifest.require_current(&self.directory)?;
        if let Some(lane) = lane.as_mut() {
            lane.require(&self.directory)?;
        }
        self.require_settled()?;
        self.require(store)?;
        Ok(turn)
    }

    pub(super) fn read_approval(
        &self,
        store: &ServerStore,
        binding: &GenesisAnchorBindingData,
        (channel, action): (u128, ActionId),
    ) -> Result<Option<SelectedReceivedApproval>, AppError> {
        self.require(store)?;
        self.require_settled()?;
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let fee = policy.current.archive.pending.pool.try_reserve(READ_WORK)?;
        let _lease = store._session.retain_for_physical_work();
        let scope = ReviewScope::derive_data(&store.chat_vault_domain()?, binding)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let marker = self.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let lane_key = ReceiverKey::new(&key, marker, scope.as_bytes());
        let indexes = Indexes::new(&key, scope);
        let mut manifest = self.read_manifest(store, &ManifestKey::new(&key, marker))?;
        let Some(selected) = manifest.data.selected(scope) else {
            absent(&self.directory.child(&lane_directory_name(scope)))?;
            manifest.require_current(&self.directory)?;
            self.require_settled()?;
            return Ok(None);
        };
        let mut lane = HeldLane::read(&self.directory, (scope, selected), &lane_key)?;
        if lane.head.is_frozen() {
            return Err(denied());
        }
        let row = lane
            .head
            .approvals
            .map(|root| indexes.lookup_approved(root, action, |id| lane.page(id)))
            .transpose()?
            .flatten();
        let Some(indexed) = row else {
            lane.require(&self.directory)?;
            manifest.require_current(&self.directory)?;
            self.require_settled()?;
            return Ok(None);
        };
        let ciphertext = lane.approval(&indexed.receipt)?;
        let pin = pinned_founder(binding);
        let raw = approval::open(&lane_key, &pin, &ciphertext)?;
        let record = ReviewStreamRecord::decode(&pin, &raw).map_err(|_| denied())?;
        if record.held_data().target_data().spec().channel != channel {
            return Err(denied());
        }
        let read = ReviewLaneRead {
            head: Some(lane.head),
            key: &lane_key,
            indexes: &indexes,
        };
        let ApprovalSelection::Selected { row: header, .. } = read.select_approval(
            &pin,
            record.entry_data(),
            |id| lane.page(id),
            |digest| lane.header(digest),
        )?
        else {
            return Err(denied());
        };
        if indexed != approval::row(header, &record, &ciphertext)? {
            return Err(denied());
        }
        lane.require(&self.directory)?;
        manifest.require_current(&self.directory)?;
        self.require_settled()?;
        self.require(store)?;
        let observation = self
            .publication
            .capture(scope, store.chat_mount.observer())?;
        Ok(Some(SelectedReceivedApproval {
            raw,
            observation,
            _fee: fee,
        }))
    }
}

fn scan_approval_ordinals<T>(
    selected_tip: u64,
    cursor: Option<RangeInclusive<u64>>,
    mut inspect: impl FnMut(u64) -> Result<Option<T>, AppError>,
) -> Result<(Option<RangeInclusive<u64>>, Option<T>), AppError> {
    // Freeze the end of this pass. A busy stream cannot postpone wrapping
    // to older deferred approvals by appending more work beyond its end.
    let mut remaining = cursor
        .filter(|range| !range.is_empty() && *range.start() != 0 && *range.end() <= selected_tip)
        .unwrap_or(1..=selected_tip);
    let mut candidate = None;
    // Route and ancestor status of the candidate have no bearing on the
    // next ordinal. The immutable selected header makes it retryable.
    for ordinal in remaining.by_ref().take(APPROVAL_SCAN_ORDINALS) {
        candidate = inspect(ordinal)?;
        if candidate.is_some() {
            break;
        }
    }
    Ok(((!remaining.is_empty()).then_some(remaining), candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_approval_sweep_is_bounded_and_wraps_a_frozen_tip() {
        let mut visited = Vec::new();
        let mut inspect = |ordinal| {
            visited.push(ordinal);
            Ok((ordinal == 6 || ordinal == 9).then_some(ordinal))
        };
        let (cursor, first) = scan_approval_ordinals(10, None, &mut inspect).unwrap();
        assert_eq!((cursor.as_ref(), first), (Some(&(5..=10)), None));

        // The head grows while ordinal 6's body or parent is unavailable.
        // The next turn still takes ordinal 9, then wraps behind both.
        let (cursor, deferred) = scan_approval_ordinals(30, cursor, &mut inspect).unwrap();
        assert_eq!((cursor.as_ref(), deferred), (Some(&(7..=10)), Some(6)));
        let (cursor, next) = scan_approval_ordinals(30, cursor, &mut inspect).unwrap();
        assert_eq!((cursor.as_ref(), next), (Some(&(10..=10)), Some(9)));
        assert_eq!(
            scan_approval_ordinals(30, cursor, &mut inspect).unwrap(),
            (None, None)
        );
        assert_eq!(visited, (1..=10).collect::<Vec<_>>());
        let (cursor, _) = scan_approval_ordinals(30, None, |ordinal| Ok(Some(ordinal))).unwrap();
        assert_eq!(cursor, Some(2..=30));
    }

    #[test]
    fn review_approval_sweep_refuses_damage_and_never_crosses_the_selected_tip() {
        assert!(
            scan_approval_ordinals(4, Some(3..=4), |_| Err::<Option<u64>, _>(denied())).is_err()
        );
        // A previously observed horizon no longer present in the selected
        // journal is only a hint. Start again under this smaller head.
        let (cursor, candidate) =
            scan_approval_ordinals(1, Some(3..=4), |ordinal| Ok(Some(ordinal))).unwrap();
        assert_eq!((cursor, candidate), (None, Some(1)));
        let (cursor, candidate) =
            scan_approval_ordinals(u64::MAX, Some(u64::MAX..=u64::MAX), |ordinal| {
                Ok(Some(ordinal))
            })
            .unwrap();
        assert_eq!((cursor, candidate), (None, Some(u64::MAX)));
    }
}
