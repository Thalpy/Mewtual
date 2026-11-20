//! Owning handoff of the actual Create packet AND its independent SDK payment.
//!
//! The transaction must not outlive that payment merely because Root moved the packet
//! into a different wrapper. This constructor only moves completed original DATA; it
//! grants no Current authority, save receipt, filesystem access or retry allowance.
use super::*;
use catcoms_sync::owner_v2::CREATE_DATA_BUFFER_BYTES;

pub(in crate::store) struct PreparedV2RetainOriginalCustody {
    producer: PreparedV2RetainData,
    // LAST: original packet and every cached page drain before SDK credit refunds.
    sdk_fee: CatalogReadReservation,
}

impl PreparedV2RetainOriginalCustody {
    /// Validate every fallible prerequisite BEFORE taking either input. A local refusal
    /// leaves both originals with the caller, including after Scope cancellation.
    pub(in crate::store) fn take_complete_into(
        producer: &mut Option<PreparedV2RetainData>,
        sdk_fee: &mut Option<CatalogReadReservation>,
        retained: &mut Option<Self>,
    ) -> Result<(), AppError> {
        if let Some(existing) = retained.as_ref() {
            if producer.is_some() || sdk_fee.is_some() {
                return Err(refused());
            }
            return existing.validate_original_accounts();
        }
        let original = producer.as_ref().ok_or_else(refused)?;
        if !original.has_packet_data() || !original.has_complete_data() {
            return Err(refused());
        }
        original.metadata.validate_prepaid_account()?;
        original.work.validate_prepaid_account()?;
        sdk_fee
            .as_ref()
            .ok_or_else(refused)?
            .validate(&original.metadata.body.pool, CREATE_DATA_BUFFER_BYTES)?;
        // These takes are infallible after the checks above: no callback, allocation,
        // Source borrow or IO lies between validation and the owning move.
        *retained = Some(Self {
            producer: producer.take().expect("validated original producer"),
            sdk_fee: sdk_fee.take().expect("validated original SDK payment"),
        });
        Ok(())
    }

    fn validate_original_accounts(&self) -> Result<(), AppError> {
        self.producer.metadata.validate_prepaid_account()?;
        self.producer.work.validate_prepaid_account()?;
        self.sdk_fee
            .validate(&self.producer.metadata.body.pool, CREATE_DATA_BUFFER_BYTES)
    }

    pub(in crate::store) fn validate_accounts(&self, store: &ServerStore) -> Result<(), AppError> {
        self.validate_original_accounts()?;
        self.sdk_fee.validate(
            &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool,
            CREATE_DATA_BUFFER_BYTES,
        )
    }

    pub(in crate::store) fn producer(&self) -> &PreparedV2RetainData {
        &self.producer
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        self.producer.resources()
    }
    pub(in crate::store) fn work_resources(&self) -> &Arc<V2TextWorkResources> {
        self.producer.work_resources()
    }
    pub(in crate::store) fn metadata_resources(&self) -> &V2TextTransactionMetadataResources {
        self.producer.metadata_resources()
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2RetainObservation {
        self.producer.observe_original()
    }
}

impl std::fmt::Debug for PreparedV2RetainOriginalCustody {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("PreparedV2RetainOriginalCustody")
            .finish_non_exhaustive()
    }
}
