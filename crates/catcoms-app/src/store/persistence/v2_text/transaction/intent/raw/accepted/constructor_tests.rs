//! Private readonly nonce-cache regression loan; never an original/custody extractor.
use super::*;
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) const fn metadata_fee_bytes_for_test() -> usize {
        ACCEPTED_METADATA_FEE
    }
    pub(in crate::store) fn cached_first_artifact_frame_for_test(&self) -> Option<&[u8]> {
        self.records.as_ref().map(|r| r.page.as_bytes())
    }
}
