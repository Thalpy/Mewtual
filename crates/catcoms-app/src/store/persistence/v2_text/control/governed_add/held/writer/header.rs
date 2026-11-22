//! Header key-attempt and Raw coverage on the original warm Add writer.
use super::super::super::header_signing::{
    AddSigningParent, ATTEMPT_BYTES, RAW_BYTES, WELCOME_ATTEMPT_BYTES, WELCOME_RAW_BYTES,
};
use super::*;

mod governing;
mod welcome;

pub(super) const BUFFER_BYTES: usize = 6
    * (ATTEMPT_BYTES + RAW_BYTES + WELCOME_ATTEMPT_BYTES + WELCOME_RAW_BYTES)
    + 10 * governing_record::MAX_BYTES
    + 8 * welcome_intent::MAX_BYTES;

pub(super) struct PreparedHeaderSigning {
    outcome: AddSigningParent,
    attempt: PreparedSmallControl,
    raw: Option<([u8; 64], PreparedSmallControl)>,
    governing: Option<(
        catcoms_replication::chat::owner_header::OwnerHeaderId,
        PreparedSmallControl,
    )>,
    welcome: Option<([u8; 32], PreparedSmallControl)>,
    welcome_signing: Option<welcome::PreparedWelcomeSigning>,
    unexpected_raw: bool,
}

impl PreparedGovernedAddFence {
    /// Freeze one exact descendant of the physically covered outcome. Hashing
    /// the original outcome and sealing this small journal run outside writers.
    pub(in crate::store) fn prepare_header_attempt(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        if self.header.is_none() {
            let outcome = self
                .outcome
                .as_ref()
                .filter(|outcome| outcome.covered)
                .ok_or_else(refused)?;
            let frame = &outcome.original.as_ref().ok_or_else(refused)?.frame;
            let outcome = AddSigningParent::from_outcome(frame)?;
            let key = Zeroizing::new(store.keys.db_key()?);
            let frozen = outcome.seal_attempt(&key, rng)?;
            self.header = Some(PreparedHeaderSigning {
                outcome,
                attempt: PreparedSmallControl::new(frozen),
                raw: None,
                governing: None,
                welcome: None,
                welcome_signing: None,
                unexpected_raw: false,
            });
        }
        self.held
            .resources
            .require_bound_original_negative(store, source)
    }

    /// A preexisting Raw or uncertain attempt cannot authorize a key use. The
    /// genuine native stage separately settles local refusals and cached raw.
    pub(in crate::store) fn cover_header_attempt(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.require_covered_outcome(guard, selected)?;
        guard.require_text_control_io_quiescent()?;
        let header = self.header.as_mut().ok_or_else(refused)?;
        require_absent(&self.held.governing_path)?;
        self.held.welcome_paths.require_absent()?;
        header.require_raw_absent(&self.held.raw_header_path)?;
        header
            .attempt
            .cover(&self.held.header_attempt_path, ATTEMPT_BYTES, guard)?;
        self.require_covered_outcome(guard, selected)?;
        let header = self.header.as_mut().ok_or_else(refused)?;
        header
            .attempt
            .require_covered(&self.held.header_attempt_path)?;
        header.require_raw_absent(&self.held.raw_header_path)?;
        require_absent(&self.held.governing_path)?;
        self.held.welcome_paths.require_absent()?;
        guard.validate_source_only()
    }

    /// Verify the real signer and exact original unsigned intent before
    /// preparing the Raw descendant. A retry compares the cached raw64 bytes.
    pub(in crate::store) fn prepare_raw_header(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        outcome: &V2FounderAddOutcomeData<'_>,
        signature: &[u8; 64],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        let header = self.header.as_mut().ok_or_else(refused)?;
        let original = self.outcome.as_ref().ok_or_else(refused)?;
        let intent = outcome.unsigned_data();
        let signer = source.data().context.spec_data().signer;
        if !header.attempt.covered
            || header.unexpected_raw
            || original.header_intent != intent.intent_commitment_data()
            || !catcoms_crypto::identity::verify_with_public_bytes(
                &signer,
                intent.signing_bytes_data(),
                signature,
            )
        {
            return Err(refused());
        }
        if let Some((cached, _)) = &header.raw {
            if cached != signature {
                return Err(refused());
            }
        } else {
            let attempt = &header.attempt.original.as_ref().ok_or_else(refused)?.frame;
            let key = Zeroizing::new(store.keys.db_key()?);
            let frozen = header.outcome.seal_raw(&key, attempt, signature, rng)?;
            header.raw = Some((*signature, PreparedSmallControl::new(frozen)));
        }
        self.held
            .resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn cover_raw_header(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.require_covered_outcome(guard, selected)?;
        guard.require_text_control_io_quiescent()?;
        let header = self.header.as_mut().ok_or_else(refused)?;
        if header.unexpected_raw {
            return Err(refused());
        }
        header
            .attempt
            .require_covered(&self.held.header_attempt_path)?;
        let (_, raw) = header.raw.as_mut().ok_or_else(refused)?;
        if !raw.covered {
            require_absent(&self.held.governing_path)?;
            self.held.welcome_paths.require_absent()?;
            raw.cover(&self.held.raw_header_path, RAW_BYTES, guard)?;
            self.require_covered_outcome(guard, selected)?;
        }
        let header = self.header.as_mut().ok_or_else(refused)?;
        header
            .attempt
            .require_covered(&self.held.header_attempt_path)?;
        let (_, raw) = header.raw.as_mut().ok_or_else(refused)?;
        raw.require_covered(&self.held.raw_header_path)?;
        guard.validate_source_only()
    }
}

impl PreparedHeaderSigning {
    fn require_raw_absent(&mut self, path: &Path) -> Result<(), AppError> {
        if self.unexpected_raw || self.raw.is_some() {
            return Err(refused());
        }
        if let Err(error) = require_absent(path) {
            self.unexpected_raw = true;
            return Err(error);
        }
        Ok(())
    }
}
