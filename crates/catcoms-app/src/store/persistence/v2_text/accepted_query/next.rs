//! Additional, original-bound root-LEAF publication DATA for the completed-parent
//! profile. This runs under the SAME actual accepted-query Source/reader; no caller
//! root, separate read lifetime, boolean miss or later obsolete-A reopen is accepted.
use super::*;
use crate::store::persistence::v2_text::private_index::{
    PrivateIndexContextData, PrivateIndexKindData, PrivateIndexPathData,
    PRIVATE_INDEX_BOUNDARY_OPEN_BYTES, PRIVATE_INDEX_PATH_METADATA_BYTES,
};
use crate::store::persistence::v2_text::request::PublicationValueData;

// Directory/Actions retain their entire existing invoice. This EXTRA term pays the
// one permitted publication plaintext, full path backing, and its sequential open
// scratch. A branch cannot consume ten pages against this one-leaf allowance.
pub(super) const COMPLETED_PUBLICATION_QUERY_LAYOUT_BYTES: usize = PRIVATE_INDEX_PATH_METADATA_BYTES
    + MAX_INDEX_NODE_BYTES
    + 16
    + PRIVATE_INDEX_BOUNDARY_OPEN_BYTES
    + 2 * (MAX_PATH + 256);

impl PreparedV2TextAcceptedQuery {
    pub(super) fn read_publication_insertion_data(
        &mut self,
        source: &CapturedV2TextAcceptedQuery,
    ) -> Result<(), AppError> {
        let p = &self.plan.parent;
        let TextRootData::Present(root) = p.head.publications else {
            // Initial source has a genuinely empty publication root. The original
            // first-B producer remains the sole consumer of that profile.
            if self.publication_path.is_some() {
                return Err(refused());
            }
            return Ok(());
        };
        if root.summary.height != 0
            || root.summary.keys != 1
            || p.head.queue_front > p.head.queue_next
            || p.head.queue_next.checked_add(1).is_none()
        {
            return Err(refused());
        }
        if self.publication_path.is_none() {
            self.publication_path = Some(
                PrivateIndexPathData::new_data(
                    PrivateIndexContextData {
                        cid: p.head.context.id_data()?,
                        kind: PrivateIndexKindData::Publication,
                    },
                    root,
                    PublicationValueData::ordinal_key_data(p.head.queue_next),
                )
                .map_err(|_| refused())?,
            );
        }
        let path = self.publication_path.as_mut().ok_or_else(refused)?;
        if let Some(page) = path.next_page_data().map_err(|_| refused())? {
            if page != root.page {
                return Err(refused());
            }
            let name = page_child(&p.pages, page.as_bytes())?;
            let mut frame = HeldAcceptedFrame::empty();
            frame.capture(&name, MAX_INDEX_FRAME_BYTES)?;
            source.pages.require_name(&p.pages, p.pages_id)?;
            frame.require_unchanged(&name)?;
            path.accept_frame_data(source.key.as_ref().ok_or_else(refused)?, frame.raw()?)
                .map_err(|_| refused())?;
            // Authenticated mathematical progress survives a late name error, but
            // reuse still requires a new SAME-root actual read admission.
            frame.require_name(&name)?;
        }
        if path.next_page_data().map_err(|_| refused())?.is_some()
            || path.row_data().map_err(|_| refused())?.is_some()
            || path.original_frame_sizes_data().len() != 1
        {
            return Err(refused());
        }
        Ok(())
    }

    pub(in crate::store) fn borrow_completed_actions_path_data(
        &self,
        retained: &PreparedV2TextRetainTransaction,
    ) -> Result<&AcceptedIndexPathData, AppError> {
        self.borrow_completed_directory_path_data(retained)?;
        let actions = self.actions.as_ref().ok_or_else(refused)?;
        if actions.next_page_data()?.is_some() || actions.row_data()?.is_some() {
            return Err(refused());
        }
        Ok(actions)
    }
    pub(in crate::store) fn borrow_completed_publication_path_data(
        &self,
        retained: &PreparedV2TextRetainTransaction,
    ) -> Result<&PrivateIndexPathData, AppError> {
        self.borrow_completed_directory_path_data(retained)?;
        if !retained.is_completed_parent_data() {
            return Err(refused());
        }
        let TextRootData::Present(root) = self.plan.parent.head.publications else {
            return Err(refused());
        };
        let path = self.publication_path.as_ref().ok_or_else(refused)?;
        let mut observed = path.original_frame_sizes_data();
        if root.summary.height != 0
            || root.summary.keys != 1
            || observed.next().map(|(node, _)| node) != Some(root)
            || observed.next().is_some()
            || path.next_page_data().map_err(|_| refused())?.is_some()
            || path.row_data().map_err(|_| refused())?.is_some()
        {
            return Err(refused());
        }
        Ok(path)
    }
}
