//! Bounded immutable updates to the ordinal and case indexes of a
//! recipient lane. Their selection is covered by the vault manifest.
use super::{manifest::ReviewScope, *};
use crate::store::persistence::v2_text::{
    private_index::{
        encode_private_leaf_data, ApprovedReviewIndexValueData, PrivateIndexContextData,
        PrivateIndexKindData, PrivateIndexLeafRowData, PrivateIndexPathData,
        PrivateIndexSuccessorData, PrivateIndexValueData, ReceivedReviewIndexValueData,
    },
    TextContextIdData,
};
use catcoms_replication::chat::{
    cold_index::{
        NodePlacementData, NodeRefData, NodeSummaryData, MAX_INDEX_FRAME_BYTES, MAX_INDEX_HEIGHT,
    },
    owner_bodies::{ReviewCaseId, ReviewStreamEntry},
    ActionId,
};
use catcoms_rt::CryptoRngCore;
use catcoms_storage::private_chat_page::{
    open_private_chat_page, PreparedPrivateChatPage, PrivateChatPageId,
};

pub(super) const MAX_PAGES: usize = 2 * (MAX_INDEX_HEIGHT as usize + 1) + 1;

#[derive(Clone, Copy)]
pub(super) enum Query {
    Ordinal(u64),
    Case(ReviewCaseId),
}

pub(super) struct Indexes {
    key: Zeroizing<[u8; 32]>,
    cid: TextContextIdData,
}

pub(super) struct Page {
    pub(super) reference: NodeRefData,
    pub(super) frame: PreparedPrivateChatPage,
}

pub(super) struct Update {
    pub(super) root: NodeRefData,
    pub(super) new: Vec<Page>,
    pub(super) old: Vec<NodeRefData>,
}

type PageWriter<'a> = dyn FnMut(&[u8], NodeSummaryData) -> Result<NodeRefData, AppError> + 'a;
type ApprovedSuccessor = ([u8; 32], ApprovedReviewIndexValueData);

impl Indexes {
    pub(super) fn new(base: &[u8; 32], scope: ReviewScope) -> Self {
        Self {
            key: Zeroizing::new(blake3::derive_key(
                "catcoms/private-chat-review-recipient-index-key/v1",
                base,
            )),
            cid: TextContextIdData::from_bytes(*scope.as_bytes()),
        }
    }

    pub(super) fn row(
        entry: &ReviewStreamEntry,
        frame: &[u8],
    ) -> Result<ReceivedReviewIndexValueData, AppError> {
        let step = entry.case_step_data().ok_or_else(denied)?;
        Ok(ReceivedReviewIndexValueData {
            sequence: entry.position_data().sequence,
            case: step.case_data(),
            entry: *entry.id().as_bytes(),
            frame: Self::frame_digest(frame),
        })
    }

    pub(super) fn frame_digest(frame: &[u8]) -> [u8; 32] {
        blake3::derive_key(
            "catcoms/private-chat-review-recipient-header-frame/v1",
            frame,
        )
    }

    pub(super) fn lookup(
        &self,
        root: NodeRefData,
        query: Query,
        mut read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<Option<ReceivedReviewIndexValueData>, AppError> {
        let (context, point) = self.coordinates(query);
        let path = self.open_path(root, context, point, &mut read)?;
        read_row(&path)
    }

    pub(super) fn prepare(
        &self,
        query: Query,
        source: Option<NodeRefData>,
        rows: (
            Option<ReceivedReviewIndexValueData>,
            ReceivedReviewIndexValueData,
        ),
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Update, AppError> {
        self.prepare_update(rng, |output| {
            self.rewrite(query, source, rows, read, output)
        })
    }

    pub(super) fn prepare_approved(
        &self,
        source: Option<NodeRefData>,
        row: ApprovedReviewIndexValueData,
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Update, AppError> {
        let (context, key) = self.approval_coordinates(row.action);
        let data = PrivateIndexLeafRowData {
            key,
            value: PrivateIndexValueData::ApprovedReview(row),
        };
        self.prepare_update(rng, |output| {
            self.rewrite_row(context, source, (None, data), read, output)
        })
    }

    fn prepare_update(
        &self,
        rng: &mut dyn CryptoRngCore,
        rewrite: impl FnOnce(&mut PageWriter<'_>) -> Result<(NodeRefData, Vec<NodeRefData>), AppError>,
    ) -> Result<Update, AppError> {
        let mut new = Vec::with_capacity(MAX_PAGES);
        let (root, old) = rewrite(&mut |plain, summary| {
            if new.len() == MAX_PAGES {
                return Err(denied());
            }
            let frame =
                PreparedPrivateChatPage::prepare(&self.key, plain, rng).map_err(|_| denied())?;
            let reference = NodeRefData {
                page: frame.id(),
                summary,
            };
            new.push(Page { reference, frame });
            Ok(reference)
        })?;
        Ok(Update { root, old, new })
    }

    pub(super) fn require_delta(
        &self,
        (query, source): (Query, Option<NodeRefData>),
        rows: (
            Option<ReceivedReviewIndexValueData>,
            ReceivedReviewIndexValueData,
        ),
        (root, exact): (NodeRefData, &[(NodeRefData, &[u8])]),
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<Vec<NodeRefData>, AppError> {
        self.verify_update(root, exact, |output| {
            self.rewrite(query, source, rows, read, output)
        })
    }

    pub(super) fn require_approved_delta(
        &self,
        source: Option<NodeRefData>,
        row: ApprovedReviewIndexValueData,
        (root, exact): (NodeRefData, &[(NodeRefData, &[u8])]),
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<Vec<NodeRefData>, AppError> {
        let (context, key) = self.approval_coordinates(row.action);
        let data = PrivateIndexLeafRowData {
            key,
            value: PrivateIndexValueData::ApprovedReview(row),
        };
        self.verify_update(root, exact, |output| {
            self.rewrite_row(context, source, (None, data), read, output)
        })
    }

    fn verify_update(
        &self,
        root: NodeRefData,
        exact: &[(NodeRefData, &[u8])],
        rewrite: impl FnOnce(&mut PageWriter<'_>) -> Result<(NodeRefData, Vec<NodeRefData>), AppError>,
    ) -> Result<Vec<NodeRefData>, AppError> {
        if !(1..=MAX_PAGES).contains(&exact.len()) {
            return Err(denied());
        }
        let mut outputs = exact.iter();
        let (reconstructed, old) = rewrite(&mut |plain, summary| {
            let (reference, bytes) = outputs.next().ok_or_else(denied)?;
            if reference.summary != summary
                || bytes.len() > MAX_INDEX_FRAME_BYTES
                || open_private_chat_page(&self.key, reference.page, bytes)
                    .map_err(|_| denied())?
                    .as_slice()
                    != plain
            {
                return Err(denied());
            }
            Ok(*reference)
        })?;
        if reconstructed != root || outputs.next().is_some() {
            return Err(denied());
        }
        Ok(old)
    }

    pub(super) fn lookup_approved(
        &self,
        root: NodeRefData,
        action: ActionId,
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<Option<ApprovedReviewIndexValueData>, AppError> {
        let (context, point) = self.approval_coordinates(action);
        let path = self.open_path(root, context, point, read)?;
        path.row_data()
            .map_err(|_| denied())?
            .map(read_approved_row)
            .transpose()
            .map(|entry| entry.map(|(_, row)| row))
    }

    pub(super) fn next_approved(
        &self,
        root: NodeRefData,
        after: Option<[u8; 32]>,
        mut read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<Option<ApprovedSuccessor>, AppError> {
        let context = self.approval_coordinates_context();
        let start = after.unwrap_or(root.summary.min);
        let path = self.open_path(root, context, start, &mut read)?;
        let row = match path.successor_data(after.is_some()).map_err(|_| denied())? {
            PrivateIndexSuccessorData::Row(row) => row,
            PrivateIndexSuccessorData::NextLower(point) => {
                // Reopen against the same selected root and require the
                // exact sibling key. A promised but unreadable child is
                // never equivalent to reaching the end.
                drop(path);
                self.open_path(root, context, point, read)?
                    .row_data()
                    .map_err(|_| denied())?
                    .ok_or_else(denied)?
            }
            PrivateIndexSuccessorData::End => {
                if start < root.summary.max || after.is_none() {
                    return Err(denied());
                }
                return Ok(None);
            }
        };
        read_approved_row(row).map(Some)
    }

    fn rewrite(
        &self,
        query: Query,
        source: Option<NodeRefData>,
        (expected, replacement): (
            Option<ReceivedReviewIndexValueData>,
            ReceivedReviewIndexValueData,
        ),
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        output: impl FnMut(&[u8], NodeSummaryData) -> Result<NodeRefData, AppError>,
    ) -> Result<(NodeRefData, Vec<NodeRefData>), AppError> {
        if expected.is_some_and(|prior| {
            matches!(query, Query::Ordinal(_)) || prior.sequence >= replacement.sequence
        }) {
            return Err(denied());
        }
        let (context, point) = self.coordinates(query);
        let replacement = PrivateIndexLeafRowData {
            key: point,
            value: PrivateIndexValueData::ReceivedReview(replacement),
        };
        self.rewrite_row(
            context,
            source,
            (
                expected.map(PrivateIndexValueData::ReceivedReview),
                replacement,
            ),
            read,
            output,
        )
    }

    fn rewrite_row(
        &self,
        context: PrivateIndexContextData,
        source: Option<NodeRefData>,
        (expected, replacement): (Option<PrivateIndexValueData>, PrivateIndexLeafRowData),
        mut read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        mut output: impl FnMut(&[u8], NodeSummaryData) -> Result<NodeRefData, AppError>,
    ) -> Result<(NodeRefData, Vec<NodeRefData>), AppError> {
        let point = replacement.key;
        if let Some(root) = source {
            let path = self.open_path(root, context, point, &mut read)?;
            if path.row_data().map_err(|_| denied())?.map(|row| row.value) != expected {
                return Err(denied());
            }
            return path.rewrite_selected_path(context, &replacement, &self.key, read, output);
        }
        if expected.is_some() {
            return Err(denied());
        }
        let plain = encode_private_leaf_data(context, &[replacement], NodePlacementData::Root)
            .map_err(|_| denied())?;
        let summary = NodeSummaryData {
            height: 0,
            keys: 1,
            min: point,
            max: point,
        };
        Ok((output(&plain, summary)?, Vec::new()))
    }

    fn open_path(
        &self,
        root: NodeRefData,
        context: PrivateIndexContextData,
        point: [u8; 32],
        mut read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<PrivateIndexPathData, AppError> {
        let mut path =
            PrivateIndexPathData::new_data(context, root, point).map_err(|_| denied())?;
        while let Some(page) = path.next_page_data().map_err(|_| denied())? {
            let bytes = read(page)?;
            if bytes.len() > MAX_INDEX_FRAME_BYTES {
                return Err(denied());
            }
            path.accept_frame_data(&self.key, &bytes)
                .map_err(|_| denied())?;
        }
        Ok(path)
    }

    fn coordinates(&self, query: Query) -> (PrivateIndexContextData, [u8; 32]) {
        let (kind, point) = match query {
            Query::Ordinal(n) => (
                PrivateIndexKindData::ReviewReceivedOrdinal,
                ReceivedReviewIndexValueData::ordinal_key(self.cid, n),
            ),
            Query::Case(case) => (
                PrivateIndexKindData::ReviewReceivedCase,
                ReceivedReviewIndexValueData::case_key(self.cid, case),
            ),
        };
        (
            PrivateIndexContextData {
                cid: self.cid,
                kind,
            },
            point,
        )
    }

    fn approval_coordinates(&self, action: ActionId) -> (PrivateIndexContextData, [u8; 32]) {
        (
            self.approval_coordinates_context(),
            ApprovedReviewIndexValueData::action_key(self.cid, action),
        )
    }

    fn approval_coordinates_context(&self) -> PrivateIndexContextData {
        PrivateIndexContextData {
            cid: self.cid,
            kind: PrivateIndexKindData::ReviewApprovedAction,
        }
    }
}

fn read_approved_row(row: PrivateIndexLeafRowData) -> Result<ApprovedSuccessor, AppError> {
    match row.value {
        PrivateIndexValueData::ApprovedReview(value) => Ok((row.key, value)),
        _ => Err(denied()),
    }
}

fn read_row(path: &PrivateIndexPathData) -> Result<Option<ReceivedReviewIndexValueData>, AppError> {
    match path.row_data().map_err(|_| denied())? {
        Some(PrivateIndexLeafRowData {
            value: PrivateIndexValueData::ReceivedReview(value),
            ..
        }) => Ok(Some(value)),
        Some(_) => Err(denied()),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_replication::chat::join_bindings::GenesisAnchorBindingData;
    use rand_chacha::{rand_core::SeedableRng, ChaCha20Rng};
    use std::collections::BTreeMap;

    #[test]
    fn approved_index_successor_visits_every_exact_row_and_refuses_a_missing_split_child() {
        let scope = ReviewScope::derive_data(
            &[1; 32],
            &GenesisAnchorBindingData {
                group: vec![2; 16],
                founder: [3; 32],
                profile: [4; 32],
                admission: [5; 32],
                tenure: [6; 32],
                header: [7; 32],
                state: [8; 32],
            },
        )
        .unwrap();
        let indexes = Indexes::new(&[9; 32], scope);
        let mut rng = ChaCha20Rng::seed_from_u64(1213);
        let mut root = None;
        let mut pages = BTreeMap::new();
        let mut expected = Vec::new();
        for n in 1u8..=34 {
            let row = ApprovedReviewIndexValueData {
                action: ActionId::from_bytes([n; 32]),
                header: ReceivedReviewIndexValueData {
                    sequence: u64::from(n),
                    case: ReviewCaseId::from_bytes([n; 32]),
                    entry: [n; 32],
                    frame: [n; 32],
                },
                receipt: [n; 32],
            };
            let update = indexes
                .prepare_approved(
                    root,
                    row,
                    |id| pages.get(&id).cloned().ok_or_else(denied),
                    &mut rng,
                )
                .unwrap();
            root = Some(update.root);
            for old in update.old {
                pages.remove(&old.page).unwrap();
            }
            for page in update.new {
                pages.insert(
                    page.reference.page,
                    Zeroizing::new(page.frame.as_bytes().to_vec()),
                );
            }
            expected.push((indexes.approval_coordinates(row.action).1, row));
        }
        expected.sort_unstable_by_key(|(point, _)| *point);
        let root = root.unwrap();
        assert_eq!(root.summary.height, 1);
        let mut after = None;
        let mut split = None;
        for row in expected {
            let mut visited = Vec::new();
            let next = indexes
                .next_approved(root, after, |id| {
                    visited.push(id);
                    pages.get(&id).cloned().ok_or_else(denied)
                })
                .unwrap();
            assert_eq!(next, Some(row));
            if visited.len() > 2 {
                split = Some((after, *visited.last().unwrap()));
            }
            after = Some(row.0);
        }
        assert!(indexes
            .next_approved(root, after, |id| pages.get(&id).cloned().ok_or_else(denied))
            .unwrap()
            .is_none());
        let (after, missing) = split.expect("the successor required the right leaf");
        let mut corrupted = pages.get(&missing).unwrap().clone();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 1;
        pages.remove(&missing).unwrap();
        assert!(indexes
            .next_approved(root, after, |id| pages.get(&id).cloned().ok_or_else(denied))
            .is_err());
        assert!(indexes
            .next_approved(root, after, |id| {
                if id == missing {
                    Ok(corrupted.clone())
                } else {
                    pages.get(&id).cloned().ok_or_else(denied)
                }
            })
            .is_err());
    }
}
