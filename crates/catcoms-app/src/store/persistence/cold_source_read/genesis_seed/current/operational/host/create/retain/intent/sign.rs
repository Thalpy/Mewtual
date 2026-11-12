//! Actual selected-I raw-cut composition. Declared with both genuine guard/forwarder
//! consumers; never constructs a Header, accepted B, projection or publication.
use super::*;
use std::sync::Arc;

impl PendingUnsignedIntent {
    fn sign_original<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        lane: &V2TextCreateLane,
    ) -> Result<(), AppError> {
        self.drain_admitted()?;
        let transaction = self.transaction.as_mut().ok_or_else(refused)?;
        // Clone the genuine receipt OUTSIDE Current. The signing guard borrows this
        // independent paid Arc, so it cannot alias transaction's later mutable loan.
        let saved = Arc::clone(transaction.saved_unsigned_intent()?);
        let resources = transaction.resources().clone();
        let current_owner = owner.borrow_current()?;
        store.with_v2_text_current(&current_owner, |admission| {
            let mut current = store.lock_v2_text_current_group(admission, &resources)?;
            let signing = current.admit_unsigned_signing(lane, &saved, transaction)?;
            transaction.sign_reserved_original(store, owner, &signing)
        })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn raw_signature_digest_for_test(&self) -> Result<[u8; 32], AppError> {
        let transaction = self
            .request_preparation
            .as_ref()
            .ok_or_else(refused)?
            .retained
            .as_ref()
            .ok_or_else(refused)?
            .intent
            .as_ref()
            .ok_or_else(refused)?
            .transaction
            .as_ref()
            .ok_or_else(refused)?;
        Ok(*blake3::hash(transaction.existing_raw_signature_data()?).as_bytes())
    }
    /// Produce/cache only the original raw signature under actual selected-I authority.
    /// Durable S and B selection still need their distinct finite Current admissions.
    pub(in crate::store) fn retry_sign_original_create_data(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2CreateRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.retry_unsigned_intent_create_request(layout, request, rng)?;
        let pending = self.request_preparation.as_mut().ok_or_else(refused)?;
        pending
            .retained
            .as_mut()
            .ok_or_else(refused)?
            .intent
            .as_mut()
            .ok_or_else(refused)?
            .sign_original(
                layout.checked_store()?,
                &self.owner,
                pending.lane.as_ref().ok_or_else(refused)?,
            )
    }
}
