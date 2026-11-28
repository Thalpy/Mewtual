//! Bounded exact page outputs of one append's two private index paths.
//! The enclosing authenticated WAL commits the old and new journal heads.
use super::update::{DecisionIndexUpdate, MAX_UPDATE_PAGES};
use super::*;
use catcoms_replication::chat::cold_index::MAX_INDEX_HEIGHT;

const MAX_PATH: usize = MAX_INDEX_HEIGHT as usize + 1;
const MAX_REF: usize = 1 + 1 + 8 + 3 * 36;
pub(in super::super) const MAX_INDEX_APPEND_BYTES: usize =
    2 * (2 + MAX_PATH * MAX_REF + MAX_UPDATE_PAGES * (MAX_REF + 4 + MAX_INDEX_FRAME_BYTES));

pub(in super::super) struct ReviewIndexDelta<'a> {
    pub(in super::super) old_path: Vec<NodeRefData>,
    pub(in super::super) outputs: Vec<(NodeRefData, &'a [u8])>,
}

impl<'a> ReviewIndexDelta<'a> {
    pub(in super::super) fn encode(
        update: &DecisionIndexUpdate,
        encoder: &mut Encoder,
    ) -> Result<(), AppError> {
        require_counts(update.original.len(), update.pages.len())?;
        encoder.put_u8(update.original.len() as u8);
        for node in &update.original {
            TextRootData::Present(*node).encode(encoder)?;
        }
        encoder.put_u8(update.pages.len() as u8);
        for output in &update.pages {
            TextRootData::Present(output.root).encode(encoder)?;
            if output.page.as_bytes().len() > MAX_INDEX_FRAME_BYTES {
                return Err(denied());
            }
            encoder
                .put_bytes(output.page.as_bytes())
                .map_err(|_| denied())?;
        }
        Ok(())
    }

    pub(in super::super) fn decode(decoder: &mut Decoder<'a>) -> Result<Self, AppError> {
        let path_len = usize::from(decoder.get_u8().map_err(|_| denied())?);
        if !(1..=MAX_PATH).contains(&path_len) {
            return Err(denied());
        }
        let mut old_path = Vec::with_capacity(path_len);
        for _ in 0..path_len {
            let reference = read_ref(decoder)?;
            if old_path
                .iter()
                .any(|old: &NodeRefData| old.page == reference.page)
            {
                return Err(denied());
            }
            old_path.push(reference);
        }
        let page_len = usize::from(decoder.get_u8().map_err(|_| denied())?);
        require_counts(path_len, page_len)?;
        let mut outputs = Vec::with_capacity(page_len);
        for _ in 0..page_len {
            let reference = read_ref(decoder)?;
            let bytes = decoder.get_bytes().map_err(|_| denied())?;
            if bytes.is_empty()
                || bytes.len() > MAX_INDEX_FRAME_BYTES
                || old_path.iter().any(|old| old.page == reference.page)
                || outputs
                    .iter()
                    .any(|(prior, _): &(NodeRefData, &[u8])| prior.page == reference.page)
            {
                return Err(denied());
            }
            outputs.push((reference, bytes));
        }
        Ok(Self { old_path, outputs })
    }

    /// The page list alone is data. A current source must open the old path
    /// and rederive its exact leaf and branch edits before writer admission.
    pub(in super::super) fn require_delta(
        &self,
        indexes: &DecisionIndexes,
        (source, selected, query): (NodeRefData, NodeRefData, DecisionIndexQuery),
        values: (Option<ReviewIndexValueData>, ReviewIndexValueData),
        read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<(), AppError> {
        let original = indexes.require_recovered_update(
            (source, query),
            values,
            (selected, &self.outputs),
            read,
        )?;
        if original != self.old_path {
            return Err(denied());
        }
        Ok(())
    }
}

fn read_ref(decoder: &mut Decoder<'_>) -> Result<NodeRefData, AppError> {
    match TextRootData::decode(decoder)? {
        TextRootData::Present(reference) => Ok(reference),
        TextRootData::Empty => Err(denied()),
    }
}

fn require_counts(path: usize, pages: usize) -> Result<(), AppError> {
    if !(1..=MAX_PATH).contains(&path) || !(1..=MAX_UPDATE_PAGES).contains(&pages) {
        return Err(denied());
    }
    Ok(())
}
