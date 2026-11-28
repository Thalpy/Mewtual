//! Bounded authenticated point/successor traversals and exact COW replay.
use super::*;
const KEY_DOMAIN: &str = "catcoms/private-chat-indexed-history-page-key/v1";
const CONTEXT: &str = "catcoms/private-chat-indexed-history-index-context/v1";
pub(super) struct ActionIndex {
    key: Zeroizing<[u8; 32]>,
    context: PrivateIndexContextData,
}
impl ActionIndex {
    pub(super) fn new(
        key: &[u8; 32],
        layout: IndexedHistoryLayout,
        scope: SharedHistoryScope,
    ) -> Self {
        let family = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        let mut h = blake3::Hasher::new_keyed(&family);
        h.update(&layout.0);
        h.update(&scope.id());
        let key = Zeroizing::new(*h.finalize().as_bytes());
        let context = PrivateIndexContextData {
            cid: TextContextIdData::from_bytes(blake3::derive_key(CONTEXT, &*key)),
            kind: PrivateIndexKindData::SharedArchiveAction,
        };
        Self { key, context }
    }
    pub(super) fn lookup(
        &self,
        root: Option<NodeRefData>,
        action: ActionId,
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<Option<SharedArchiveIndexValueData>, AppError> {
        let Some(root) = root else {
            return Ok(None);
        };
        let path = self.path(root, *action.as_bytes(), read)?;
        path.row_data().map_err(|_| refused())?.map(row).transpose()
    }
    fn path(
        &self,
        root: NodeRefData,
        key: [u8; 32],
        mut read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<PrivateIndexPathData, AppError> {
        let mut path =
            PrivateIndexPathData::new_data(self.context, root, key).map_err(|_| refused())?;
        while let Some(page) = path.next_page_data().map_err(|_| refused())? {
            path.accept_frame_data(&self.key, &read(page)?)
                .map_err(|_| refused())?;
        }
        Ok(path)
    }
    pub(super) fn next(
        &self,
        root: Option<NodeRefData>,
        after: Option<ActionId>,
        mut read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<Option<SharedArchiveIndexValueData>, AppError> {
        let Some(root) = root else {
            return Ok(None);
        };
        let mut key = after.map_or([0; 32], |id| *id.as_bytes());
        let mut exclusive = after.is_some();
        // One successor boundary per level plus the actual found descent.
        for _ in 0..=MAX_PATH {
            let path = self.path(root, key, &mut read)?;
            match path.successor_data(exclusive).map_err(|_| refused())? {
                PrivateIndexSuccessorData::Row(found) => return row(found).map(Some),
                PrivateIndexSuccessorData::End => return Ok(None),
                PrivateIndexSuccessorData::NextLower(next) => {
                    if next <= key {
                        return Err(refused());
                    }
                    key = next;
                    exclusive = false;
                }
            }
        }
        Err(refused())
    }
    pub(super) fn rewrite(
        &self,
        root: Option<NodeRefData>,
        replacement: SharedArchiveIndexValueData,
        mut read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        mut output: impl FnMut(&[u8], NodeSummaryData) -> Result<NodeRefData, AppError>,
    ) -> Result<(NodeRefData, Vec<NodeRefData>), AppError> {
        let replacement = PrivateIndexLeafRowData {
            key: *replacement.action.as_bytes(),
            value: PrivateIndexValueData::SharedArchive(replacement),
        };
        let Some(root) = root else {
            let plain =
                encode_private_leaf_data(self.context, &[replacement], NodePlacementData::Root)
                    .map_err(|_| refused())?;
            let summary =
                PrivateIndexNodeFrame::decode(&plain, self.context, NodePlacementData::Root)
                    .map_err(|_| refused())?
                    .summary();
            return Ok((output(&plain, summary)?, Vec::new()));
        };
        let path = self.path(root, replacement.key, &mut read)?;
        if path.row_data().map_err(|_| refused())?.is_some() {
            return Err(refused());
        }
        path.rewrite_selected_path(self.context, &replacement, &self.key, read, output)
    }
    pub(super) fn prepare(
        &self,
        root: Option<NodeRefData>,
        row: SharedArchiveIndexValueData,
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<
        (
            NodeRefData,
            Vec<(NodeRefData, PreparedPrivateChatPage)>,
            Vec<(NodeRefData, usize)>,
        ),
        AppError,
    > {
        let mut outputs = Vec::with_capacity(MAX_OUTPUTS);
        let mut read = read;
        let original = if let Some(root) = root {
            self.path(root, *row.action.as_bytes(), &mut read)?
                .original_frame_sizes_data()
                .collect()
        } else {
            Vec::new()
        };
        let (next, _) = self.rewrite(root, row, read, |plain, summary| {
            if outputs.len() == MAX_OUTPUTS {
                return Err(refused());
            }
            let page = PreparedPrivateChatPage::prepare(&self.key, plain, rng)?;
            let node = NodeRefData {
                page: page.id(),
                summary,
            };
            outputs.push((node, page));
            Ok(node)
        })?;
        Ok((next, outputs, original))
    }
    pub(super) fn verify(
        &self,
        old: Option<NodeRefData>,
        next: NodeRefData,
        row: SharedArchiveIndexValueData,
        pages: &[(NodeRefData, &[u8])],
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<Vec<NodeRefData>, AppError> {
        let mut outputs = pages.iter();
        let (actual, original) = self.rewrite(old, row, read, |plain, summary| {
            let (expected, bytes) = outputs.next().ok_or_else(refused)?;
            if expected.summary != summary
                || open_private_chat_page(&self.key, expected.page, bytes)?.as_slice() != plain
            {
                return Err(refused());
            }
            Ok(*expected)
        })?;
        if actual != next || outputs.next().is_some() {
            return Err(refused());
        }
        Ok(original)
    }
    pub(super) fn require_old_unreachable(
        &self,
        root: NodeRefData,
        old: NodeRefData,
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<(), AppError> {
        let path = self.path(root, old.summary.min, read)?;
        if path
            .original_frame_sizes_data()
            .any(|(selected, _)| selected.page == old.page)
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn require_old_page(
        &self,
        old: NodeRefData,
        depth: usize,
        bytes: &[u8],
    ) -> Result<(), AppError> {
        let plain = open_private_chat_page(&self.key, old.page, bytes)?;
        let placement = if depth == 0 {
            NodePlacementData::Root
        } else {
            NodePlacementData::Child
        };
        if PrivateIndexNodeFrame::decode(&plain, self.context, placement)
            .map_err(|_| refused())?
            .summary()
            != old.summary
        {
            return Err(refused());
        }
        Ok(())
    }
}
fn row(row: PrivateIndexLeafRowData) -> Result<SharedArchiveIndexValueData, AppError> {
    match row.value {
        PrivateIndexValueData::SharedArchive(row) => Ok(row),
        _ => Err(refused()),
    }
}
