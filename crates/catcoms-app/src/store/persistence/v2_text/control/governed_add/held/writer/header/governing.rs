//! Warm coverage of the exact signed Admission and its state image.
use super::*;
use catcoms_replication::chat::owner_header::SignedOwnerHeader;

impl PreparedGovernedAddFence {
    pub(in crate::store) fn prepare_governing_record(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        outcome: &V2FounderAddOutcomeData<'_>,
        materialized: &SignedOwnerHeader,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        let header = self.header.as_mut().ok_or_else(refused)?;
        let (signature, raw) = header
            .raw
            .as_ref()
            .filter(|(_, raw)| raw.covered)
            .ok_or_else(refused)?;
        let original = self
            .outcome
            .as_ref()
            .filter(|outcome| outcome.covered)
            .ok_or_else(refused)?;
        let intent = outcome.unsigned_data();
        if original.header_intent != intent.intent_commitment_data() {
            return Err(refused());
        }
        governing_record::require_original_signature(
            materialized,
            signature,
            intent.signing_bytes_data(),
        )?;
        if let Some((id, _)) = &header.governing {
            if *id != materialized.id() {
                return Err(refused());
            }
        } else {
            let key = Zeroizing::new(store.keys.db_key()?);
            let frame = governing_record::seal_data(
                &key,
                &raw.original.as_ref().ok_or_else(refused)?.frame,
                materialized,
                outcome.admission_data(),
                intent.image_data(),
                rng,
            )?;
            header.governing = Some((materialized.id(), PreparedSmallControl::new(frame)));
        }
        self.held
            .resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn cover_governing_record(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if self
            .header
            .as_ref()
            .and_then(|header| header.governing.as_ref())
            .is_none()
        {
            return Err(refused());
        }
        self.cover_raw_header(guard, selected)?;
        let (_, governing) = self
            .header
            .as_mut()
            .and_then(|header| header.governing.as_mut())
            .ok_or_else(refused)?;
        if !governing.covered {
            self.held.welcome_paths.require_absent()?;
        }
        governing.cover(
            &self.held.governing_path,
            governing_record::MAX_BYTES,
            guard,
        )?;
        self.cover_raw_header(guard, selected)?;
        let (_, governing) = self
            .header
            .as_mut()
            .and_then(|header| header.governing.as_mut())
            .ok_or_else(refused)?;
        governing.require_covered(&self.held.governing_path)?;
        guard.validate_source_only()
    }
}
