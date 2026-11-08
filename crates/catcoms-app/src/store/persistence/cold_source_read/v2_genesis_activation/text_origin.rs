//! Borrow existing originals for the typed operational read guard only.
//!
//! No File clone/open, identity selection, decoder, key or old-head validation occurs.
//! Meaningful original receipt/Scope ownership remains intact after operational promotion.
use super::*;

impl SavedV2GenesisHead {
    /// The exact OLD ciphertext is stable context DATA after operational promotion. The
    /// public source cannot regenerate its original random AEAD nonce. Borrow the already
    /// retained candidates; do not reopen/decode the now-replaced head or seed to obtain it.
    pub(in crate::store) fn text_genesis_context_data(
        &self,
    ) -> Result<(&SavedSourceExpectationData, &[u8]), AppError> {
        Ok((&self.candidates.expected, &self.candidates.head))
    }

    pub(in crate::store) fn group_directory_data(
        &self,
    ) -> (&Path, &File, catcoms_storage::kept::SessionId) {
        self.seed.directory.directory_data()
    }

    pub(in crate::store) fn text_pin_data(
        &self,
    ) -> Result<(&File, catcoms_storage::kept::SessionId, &[u8]), AppError> {
        let id = self.pin.identity.ok_or_else(critical_namespaces::refused)?;
        Ok((&self.pin.file, id, &self.candidates.pin))
    }
}
