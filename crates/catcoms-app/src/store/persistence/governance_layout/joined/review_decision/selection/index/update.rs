//! One bounded copy-on-write path update in an already authenticated
//! root. The journal owner still covers all new pages before selecting
//! that root, and proves old pages unreachable before any retirement.
use super::*;
use crate::store::persistence::v2_text::private_index::{
    encode_private_branch_data, PrivateIndexChildRefData, PrivateIndexNodeFrame,
    PrivateIndexRewriteData,
};
use catcoms_replication::chat::cold_index::MAX_INDEX_HEIGHT;
use catcoms_storage::private_chat_page::open_private_chat_page;

/// A full descent can split each level into two outputs and add one
/// root. The transaction pays for every retained ciphertext separately.
pub(in super::super) const MAX_UPDATE_PAGES: usize = 2 * (MAX_INDEX_HEIGHT as usize + 1) + 1;

pub(in super::super::super) struct DecisionIndexUpdate {
    pub(in super::super::super) root: NodeRefData,
    pub(in super::super::super) pages: Vec<DecisionIndexPage>,
    pub(in super::super::super) original: Vec<NodeRefData>,
}

impl DecisionIndexes {
    /// Prepare a successor from an exact checked miss or hit at the held
    /// source root. A nonce has no overwrite path. A case change must
    /// compare the whole prior pointer with the original selected case.
    pub(in super::super::super) fn prepare_update(
        &self,
        root: NodeRefData,
        query: DecisionIndexQuery,
        (expected, replacement): (Option<ReviewIndexValueData>, ReviewIndexValueData),
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<DecisionIndexUpdate, AppError> {
        let mut pages = Vec::with_capacity(MAX_UPDATE_PAGES);
        let (root, original) = self.rewrite_path(
            (root, query),
            (expected, replacement),
            read,
            |plain, summary| {
                if pages.len() >= MAX_UPDATE_PAGES {
                    return Err(denied());
                }
                let page = PreparedPrivateChatPage::prepare(&self.key, plain, rng)
                    .map_err(|_| denied())?;
                let root = NodeRefData {
                    page: page.id(),
                    summary,
                };
                pages.push(DecisionIndexPage { root, page });
                Ok(root)
            },
        )?;
        Ok(DecisionIndexUpdate {
            root,
            pages,
            original,
        })
    }

    /// Re-derive the exact structural delta and open its original
    /// ciphertext instead of generating a second set of outputs.
    pub(in super::super) fn require_recovered_update(
        &self,
        source: (NodeRefData, DecisionIndexQuery),
        values: (Option<ReviewIndexValueData>, ReviewIndexValueData),
        (selected, pages): (NodeRefData, &[(NodeRefData, &[u8])]),
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<Vec<NodeRefData>, AppError> {
        if pages.is_empty() || pages.len() > MAX_UPDATE_PAGES {
            return Err(denied());
        }
        let mut remaining = pages.iter();
        let (root, original) = self.rewrite_path(source, values, read, |plain, summary| {
            let (expected, bytes) = remaining.next().ok_or_else(denied)?;
            if bytes.len() > MAX_INDEX_FRAME_BYTES
                || expected.summary != summary
                || open_private_chat_page(&self.key, expected.page, bytes)
                    .map_err(|_| denied())?
                    .as_slice()
                    != plain
            {
                return Err(denied());
            }
            Ok(*expected)
        })?;
        if root != selected || remaining.next().is_some() {
            return Err(denied());
        }
        Ok(original)
    }

    fn rewrite_path(
        &self,
        (root, query): (NodeRefData, DecisionIndexQuery),
        (expected, replacement): (Option<ReviewIndexValueData>, ReviewIndexValueData),
        mut read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        mut output: impl FnMut(&[u8], NodeSummaryData) -> Result<NodeRefData, AppError>,
    ) -> Result<(NodeRefData, Vec<NodeRefData>), AppError> {
        let (context, key) = query.coordinates(self.context);
        if expected.is_some_and(|prior| {
            matches!(query, DecisionIndexQuery::Intent(_)) || replacement.sequence <= prior.sequence
        }) {
            return Err(denied());
        }
        let path = self.open_path(root, (context, key), &mut read)?;
        let actual = Self::row_at(&path)?;
        if actual != expected {
            return Err(denied());
        }
        let original = path
            .original_frame_sizes_data()
            .map(|(reference, _)| reference)
            .collect::<Vec<_>>();
        let parents = original.len().checked_sub(1).ok_or_else(denied)?;
        let leaf = path
            .rewrite_leaf_data(&PrivateIndexLeafRowData {
                key,
                value: PrivateIndexValueData::Review(replacement),
            })
            .map_err(|_| denied())?;
        let mut children = revised_children(context, &leaf, &mut output)?;
        for level in (0..parents).rev() {
            let boundary = path
                .boundary_page_data(level, children.len())
                .map_err(|_| denied())?
                .map(&mut read)
                .transpose()?;
            let rewritten = path
                .rewrite_branch_data(
                    level,
                    &children,
                    boundary
                        .as_deref()
                        .map(|frame| (&*self.key, frame.as_slice())),
                )
                .map_err(|_| denied())?;
            children = revised_children(context, &rewritten, &mut output)?;
        }
        let root = match children.as_slice() {
            [child] => child.node,
            [_, _] => {
                let plain = encode_private_branch_data(context, &children, NodePlacementData::Root)
                    .map_err(|_| denied())?;
                let node = PrivateIndexNodeFrame::decode(&plain, context, NodePlacementData::Root)
                    .map_err(|_| denied())?;
                output(&plain, node.summary())?
            }
            _ => return Err(denied()),
        };
        Ok((root, original))
    }
}

fn revised_children(
    context: PrivateIndexContextData,
    update: &PrivateIndexRewriteData,
    output: &mut impl FnMut(&[u8], NodeSummaryData) -> Result<NodeRefData, AppError>,
) -> Result<Vec<PrivateIndexChildRefData>, AppError> {
    update
        .nodes_data()
        .iter()
        .map(|node| {
            let node = output(node.plain_data(), node.summary_data())?;
            Ok(PrivateIndexChildRefData { context, node })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_replication::chat::cold_index::MAX_LEAF_ROWS;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    use std::collections::BTreeMap;

    type Pages = BTreeMap<PrivateChatPageId, Zeroizing<Vec<u8>>>;

    fn row(sequence: u8) -> ReviewIndexValueData {
        ReviewIndexValueData {
            intent: [sequence; 16],
            request: [2; 32],
            case: ReviewCaseId::from_bytes([3; 32]),
            sequence: u64::from(sequence),
            entry: [4; 32],
            record: [5; 32],
        }
    }

    fn keep(pages: &mut Pages, frame: PreparedPrivateChatPage) {
        assert!(pages
            .insert(frame.id(), Zeroizing::new(frame.as_bytes().to_vec()))
            .is_none());
    }

    #[test]
    fn review_index_cow_splits_leaf_and_preserves_exact_old_outcomes() {
        let indexes = DecisionIndexes::new(&[33; 32]);
        let mut rng = ChaCha20Rng::seed_from_u64(733);
        let first = row(1);
        let old = indexes
            .first_root(DecisionIndexQuery::Intent(first.intent), first, &mut rng)
            .unwrap();
        let mut pages = Pages::new();
        let original = old.root;
        keep(&mut pages, old.page);
        let mut root = original;
        for sequence in 2..=u8::try_from(MAX_LEAF_ROWS + 2).unwrap() {
            let value = row(sequence);
            let query = DecisionIndexQuery::Intent(value.intent);
            let update = indexes
                .prepare_update(
                    root,
                    query,
                    (None, value),
                    |id| pages.get(&id).cloned().ok_or_else(denied),
                    &mut rng,
                )
                .unwrap();
            assert_eq!(update.original.first(), Some(&root));
            assert!(update.pages.len() <= MAX_UPDATE_PAGES);
            if update.pages.len() > 1 {
                let read = |id| pages.get(&id).cloned().ok_or_else(denied);
                let mut frames = update
                    .pages
                    .iter()
                    .map(|output| (output.root, output.page.as_bytes()))
                    .collect::<Vec<_>>();
                let input = (root, query);
                assert_eq!(
                    indexes
                        .require_recovered_update(
                            input,
                            (None, value),
                            (update.root, &frames),
                            read,
                        )
                        .unwrap(),
                    update.original
                );
                frames.swap(0, 1);
                assert!(indexes
                    .require_recovered_update(input, (None, value), (update.root, &frames), read,)
                    .is_err());
                frames.swap(0, 1);
                frames.push(frames[0]);
                assert!(indexes
                    .require_recovered_update(input, (None, value), (update.root, &frames), read,)
                    .is_err());
            }
            for frame in update.pages {
                keep(&mut pages, frame.page);
            }
            root = update.root;
            let read = |id| pages.get(&id).cloned().ok_or_else(denied);
            assert_eq!(indexes.lookup(root, query, read).unwrap(), Some(value));
            assert_eq!(indexes.lookup(original, query, read).unwrap(), None);
            assert!(indexes
                .prepare_update(root, query, (None, value), read, &mut rng)
                .is_err());
        }
        let read = |id| pages.get(&id).cloned().ok_or_else(denied);
        assert_eq!(root.summary.height, 1);
        let query = DecisionIndexQuery::Intent(first.intent);
        let path = indexes
            .open_path(root, query.coordinates(indexes.context), read)
            .unwrap();
        let (live_leaf, _) = path.original_frame_sizes_data().last().unwrap();
        let kind = PrivateIndexKindData::ReviewIntent;
        assert!(indexes
            .require_unreachable(root, kind, live_leaf, read)
            .is_err());
        assert!(indexes
            .require_unreachable(root, kind, original, |page| {
                if page == live_leaf.page {
                    return Err(denied());
                }
                read(page)
            })
            .is_err());
        assert!(indexes
            .require_unreachable(root, kind, original, |page| {
                let mut frame = read(page)?;
                if page == live_leaf.page {
                    *frame.last_mut().unwrap() ^= 1;
                }
                Ok(frame)
            })
            .is_err());
        indexes
            .require_unreachable(root, kind, original, read)
            .unwrap();
        indexes
            .require_old_page(
                kind,
                NodePlacementData::Root,
                original,
                pages.get(&original.page).unwrap(),
            )
            .unwrap();
        pages.remove(&original.page).unwrap();
        let read = |id| pages.get(&id).cloned().ok_or_else(denied);
        assert_eq!(
            indexes
                .lookup(root, DecisionIndexQuery::Intent(first.intent), read)
                .unwrap(),
            Some(first)
        );
    }

    #[test]
    fn review_index_case_update_requires_the_complete_selected_predecessor() {
        let indexes = DecisionIndexes::new(&[34; 32]);
        let mut rng = ChaCha20Rng::seed_from_u64(734);
        let prior = row(1);
        let next = row(2);
        let query = DecisionIndexQuery::Case(prior.case);
        let first = indexes.first_root(query, prior, &mut rng).unwrap();
        let root = first.root;
        let mut pages = Pages::new();
        keep(&mut pages, first.page);
        let read = |id| pages.get(&id).cloned().ok_or_else(denied);
        assert!(indexes
            .prepare_update(root, query, (None, next), read, &mut rng)
            .is_err());
        let stale = ReviewIndexValueData {
            record: [6; 32],
            ..prior
        };
        assert!(indexes
            .prepare_update(root, query, (Some(stale), next), read, &mut rng)
            .is_err());
        assert!(indexes
            .prepare_update(root, query, (Some(prior), prior), read, &mut rng,)
            .is_err());
        let update = indexes
            .prepare_update(root, query, (Some(prior), next), read, &mut rng)
            .unwrap();
        assert_eq!(update.root.summary.keys, 1);
        let frames = update
            .pages
            .iter()
            .map(|output| (output.root, output.page.as_bytes()))
            .collect::<Vec<_>>();
        assert_eq!(
            indexes
                .require_recovered_update(
                    (root, query),
                    (Some(prior), next),
                    (update.root, &frames),
                    read,
                )
                .unwrap(),
            update.original
        );
        let mut changed = frames[0].1.to_vec();
        *changed.last_mut().unwrap() ^= 1;
        assert!(indexes
            .require_recovered_update(
                (root, query),
                (Some(prior), next),
                (update.root, &[(frames[0].0, &changed)]),
                read,
            )
            .is_err());
        for frame in update.pages {
            keep(&mut pages, frame.page);
        }
        let read = |id| pages.get(&id).cloned().ok_or_else(denied);
        assert_eq!(
            indexes.lookup(update.root, query, read).unwrap(),
            Some(next)
        );
        assert_eq!(indexes.lookup(root, query, read).unwrap(), Some(prior));
    }
}
