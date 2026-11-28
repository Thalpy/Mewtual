//! Reuse the private text copy-on-write index for two independent
//! point lookups under the same selected review head.
use super::*;
use crate::store::persistence::v2_text::{
    private_index::{
        encode_private_leaf_data, PrivateIndexContextData, PrivateIndexKindData,
        PrivateIndexLeafRowData, PrivateIndexPathData, PrivateIndexValueData, ReviewIndexValueData,
    },
    TextContextIdData,
};
use catcoms_replication::chat::{
    cold_index::{NodePlacementData, NodeRefData, NodeSummaryData},
    owner_bodies::ReviewCaseId,
};
use catcoms_storage::private_chat_page::{PreparedPrivateChatPage, PrivateChatPageId};

const INDEX_KEY: &str = "catcoms/private-chat-v2-review-journal-index-key/v1";
const INDEX_CONTEXT: &str = "catcoms/private-chat-v2-review-journal-index-context/v1";

pub(in super::super) struct DecisionIndexes {
    key: Zeroizing<[u8; 32]>,
    context: TextContextIdData,
}

pub(in super::super) struct DecisionIndexPage {
    pub(in super::super) root: NodeRefData,
    pub(in super::super) page: PreparedPrivateChatPage,
}

#[derive(Clone, Copy)]
pub(super) enum DecisionIndexQuery {
    Intent([u8; 16]),
    Case(ReviewCaseId),
}

impl DecisionIndexQuery {
    fn coordinates(self, cid: TextContextIdData) -> (PrivateIndexContextData, [u8; 32]) {
        let (kind, key) = match self {
            Self::Intent(intent) => (
                PrivateIndexKindData::ReviewIntent,
                ReviewIndexValueData::intent_key(cid, intent),
            ),
            Self::Case(case) => (
                PrivateIndexKindData::ReviewCase,
                ReviewIndexValueData::case_key(cid, case),
            ),
        };
        (PrivateIndexContextData { cid, kind }, key)
    }
}

impl DecisionIndexes {
    pub(in super::super) fn new(journal_key: &[u8; 32]) -> Self {
        Self {
            key: Zeroizing::new(blake3::derive_key(INDEX_KEY, journal_key)),
            context: TextContextIdData::from_bytes(blake3::derive_key(INDEX_CONTEXT, journal_key)),
        }
    }

    pub(in super::super) fn prepare_first(
        &self,
        record: &RetainedDecisionRecord<'_>,
        sealed_record: &[u8],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<[DecisionIndexPage; 2], AppError> {
        let value = Self::row(record, sealed_record);
        if value.sequence != 1 {
            return Err(denied());
        }
        Ok([
            self.first_root(DecisionIndexQuery::Intent(value.intent), value, rng)?,
            self.first_root(DecisionIndexQuery::Case(value.case), value, rng)?,
        ])
    }

    pub(super) fn require_first(
        &self,
        roots: [NodeRefData; 2],
        pages: [&[u8]; 2],
        record: &RetainedDecisionRecord<'_>,
        sealed_record: &[u8],
    ) -> Result<(), AppError> {
        let value = Self::row(record, sealed_record);
        if value.sequence != 1 {
            return Err(denied());
        }
        let queries = [
            DecisionIndexQuery::Intent(value.intent),
            DecisionIndexQuery::Case(value.case),
        ];
        for ((root, bytes), query) in roots.into_iter().zip(pages).zip(queries) {
            if root.summary.height != 0 || root.summary.keys != 1 {
                return Err(denied());
            }
            let hit = self.lookup(root, query, |page| {
                if page != root.page {
                    return Err(denied());
                }
                Ok(Zeroizing::new(bytes.to_vec()))
            })?;
            if hit != Some(value) {
                return Err(denied());
            }
        }
        Ok(())
    }

    fn row(record: &RetainedDecisionRecord<'_>, sealed_record: &[u8]) -> ReviewIndexValueData {
        let entry = record.record.entry_data();
        let decision = record.record.decision_data();
        ReviewIndexValueData {
            intent: decision.decision().spec().intent,
            request: record.request_digest,
            case: decision.case_id(),
            sequence: entry.position_data().sequence,
            entry: *entry.id().as_bytes(),
            record: blake3::derive_key(RECORD_DIGEST_DOMAIN, sealed_record),
        }
    }

    fn first_root(
        &self,
        query: DecisionIndexQuery,
        value: ReviewIndexValueData,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<DecisionIndexPage, AppError> {
        let (context, key) = query.coordinates(self.context);
        let plain = encode_private_leaf_data(
            context,
            &[PrivateIndexLeafRowData {
                key,
                value: PrivateIndexValueData::Review(value),
            }],
            NodePlacementData::Root,
        )
        .map_err(|_| denied())?;
        let page =
            PreparedPrivateChatPage::prepare(&self.key, &plain, rng).map_err(|_| denied())?;
        let root = NodeRefData {
            page: page.id(),
            summary: NodeSummaryData {
                height: 0,
                keys: 1,
                min: key,
                max: key,
            },
        };
        Ok(DecisionIndexPage { root, page })
    }

    /// The caller retains and reproves the selected journal head. Missing
    /// or unauthenticated pages are errors and never constitute a miss.
    pub(super) fn lookup(
        &self,
        root: NodeRefData,
        query: DecisionIndexQuery,
        mut read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<Option<ReviewIndexValueData>, AppError> {
        let (context, key) = query.coordinates(self.context);
        let mut path = PrivateIndexPathData::new_data(context, root, key).map_err(|_| denied())?;
        while let Some(page) = path.next_page_data().map_err(|_| denied())? {
            path.accept_frame_data(&self.key, &read(page)?)
                .map_err(|_| denied())?;
        }
        match path.row_data().map_err(|_| denied())? {
            Some(PrivateIndexLeafRowData {
                value: PrivateIndexValueData::Review(value),
                ..
            }) => Ok(Some(value)),
            Some(_) => Err(denied()),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn review_index_lookup_distinguishes_absence_from_a_bad_page_or_context() {
        let mut rng = ChaCha20Rng::seed_from_u64(721);
        let indexes = DecisionIndexes::new(&[19; 32]);
        let foreign = DecisionIndexes::new(&[20; 32]);
        let row = ReviewIndexValueData {
            intent: [21; 16],
            request: [22; 32],
            case: ReviewCaseId::from_bytes([23; 32]),
            sequence: 1,
            entry: [24; 32],
            record: [25; 32],
        };
        let intent = DecisionIndexQuery::Intent(row.intent);
        let root = indexes.first_root(intent, row, &mut rng).unwrap();
        let read = |id| {
            assert_eq!(id, root.page.id());
            Ok(Zeroizing::new(root.page.as_bytes().to_vec()))
        };
        assert_eq!(indexes.lookup(root.root, intent, read).unwrap(), Some(row));
        assert_eq!(
            indexes
                .lookup(root.root, DecisionIndexQuery::Intent([26; 16]), read)
                .unwrap(),
            None,
        );
        assert!(indexes
            .lookup(root.root, DecisionIndexQuery::Case(row.case), read)
            .is_err());
        assert!(indexes
            .lookup(root.root, intent, |_| Err(denied()))
            .is_err());
        assert!(foreign.lookup(root.root, intent, read).is_err());
    }
}
