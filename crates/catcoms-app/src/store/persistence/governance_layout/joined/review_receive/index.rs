//! Bounded immutable updates to the ordinal and case indexes of a
//! recipient lane. Their selection is covered by the vault manifest.
use super::{manifest::ReviewScope, *};
use crate::store::persistence::v2_text::{
    private_index::{
        encode_private_leaf_data, PrivateIndexContextData, PrivateIndexKindData,
        PrivateIndexLeafRowData, PrivateIndexPathData, PrivateIndexValueData,
        ReceivedReviewIndexValueData,
    },
    TextContextIdData,
};
use catcoms_replication::chat::{
    cold_index::{
        NodePlacementData, NodeRefData, NodeSummaryData, MAX_INDEX_FRAME_BYTES, MAX_INDEX_HEIGHT,
    },
    owner_bodies::{ReviewCaseId, ReviewStreamEntry},
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
        let mut new = Vec::with_capacity(MAX_PAGES);
        let (root, old) = self.rewrite(query, source, rows, read, |plain, summary| {
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
        if !(1..=MAX_PAGES).contains(&exact.len()) {
            return Err(denied());
        }
        let mut outputs = exact.iter();
        let (reconstructed, old) = self.rewrite(query, source, rows, read, |plain, summary| {
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

    fn rewrite(
        &self,
        query: Query,
        source: Option<NodeRefData>,
        (expected, replacement): (
            Option<ReceivedReviewIndexValueData>,
            ReceivedReviewIndexValueData,
        ),
        mut read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        mut output: impl FnMut(&[u8], NodeSummaryData) -> Result<NodeRefData, AppError>,
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
        if let Some(root) = source {
            let path = self.open_path(root, context, point, &mut read)?;
            if read_row(&path)? != expected {
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
