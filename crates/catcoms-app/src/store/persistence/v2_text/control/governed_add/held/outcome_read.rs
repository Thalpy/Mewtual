//! Independently authenticated outcome data for the original held SDK entry.
use super::super::outcome::{open_outcome_data, MAX_FRAME};
use super::*;
use crate::store::persistence::v2_text::io::read_control_into;

pub(super) struct ObservedOutcome {
    buffer: Option<Zeroizing<Vec<u8>>>,
    capture: Option<CapturedOutcome>,
    plain: Option<Zeroizing<Vec<u8>>>,
    authenticated: bool,
    // The captured File and both cipher/plain buffers drain before this debit.
    _fee: CatalogReadReservation,
}

enum CapturedOutcome {
    Absent,
    Present(OpenedTextControl),
}

impl HeldGovernedAddFence {
    /// Reserve the full outcome read and AEAD inventory outside Current.
    /// Only the reader already holding an authenticated Entry can observe it.
    pub(in crate::store) fn prepare_outcome_read(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        if !self.authenticated || !matches!(self.entry, Some(ObservedSdkEntry::Present(_))) {
            return Err(refused());
        }
        self.resources
            .require_bound_original_negative(store, source)?;
        if self.outcome.is_none() {
            const BYTES: usize =
                4 * (MAX_FRAME + 1) + 4 * std::mem::size_of::<ObservedOutcome>() + 4096;
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            let fee = pool.try_reserve(BYTES)?;
            fee.validate(pool, BYTES)?;
            self.outcome = Some(ObservedOutcome {
                buffer: Some(Zeroizing::new(Vec::with_capacity(MAX_FRAME + 1))),
                capture: None,
                plain: None,
                authenticated: false,
                _fee: fee,
            });
        }
        self.resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn capture_outcome(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if self.require_recovery_state(guard, selected)?
            != GovernedAddRecoveryState::PossiblyEntered
        {
            return Err(refused());
        }
        let outcome = self.outcome.as_mut().ok_or_else(refused)?;
        if outcome.capture.is_some() {
            return Err(refused());
        }
        outcome.capture = Some(match fs::symlink_metadata(&self.outcome_path) {
            Ok(_) => CapturedOutcome::Present(read_control_into(
                &self.outcome_path,
                MAX_FRAME,
                &mut outcome.buffer,
            )?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => CapturedOutcome::Absent,
            Err(error) => return Err(io(error)),
        });
        guard.validate_source_only()
    }

    /// The payload remains historical data. It cannot select, rebuild or run
    /// an SDK candidate until the owning recovery path verifies its lineage.
    pub(in crate::store) fn authenticate_outcome(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.resources
            .require_bound_original_negative(store, source)?;
        let Some(ObservedSdkEntry::Present(entry)) = self.entry.as_ref() else {
            return Err(refused());
        };
        let outcome = self.outcome.as_mut().ok_or_else(refused)?;
        if !outcome.authenticated {
            if let CapturedOutcome::Present(data) = outcome.capture.as_ref().ok_or_else(refused)? {
                let key = Zeroizing::new(store.keys.db_key()?);
                outcome.plain = Some(open_outcome_data(&key, &entry.frame, &data.frame)?);
            }
            outcome.authenticated = true;
        }
        self.resources
            .require_bound_original_negative(store, source)
    }

    /// A late Current check must see the same original outcome or the same
    /// absence. An earlier missing file cannot hide a newly covered outcome.
    pub(in crate::store) fn require_outcome_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<Option<&[u8]>, AppError> {
        if self.require_recovery_state(guard, selected)?
            != GovernedAddRecoveryState::PossiblyEntered
        {
            return Err(refused());
        }
        let outcome = self
            .outcome
            .as_mut()
            .filter(|outcome| outcome.authenticated)
            .ok_or_else(refused)?;
        match outcome.capture.as_mut().ok_or_else(refused)? {
            CapturedOutcome::Absent => require_absent(&self.outcome_path)?,
            CapturedOutcome::Present(data) => require_exact_held(&self.outcome_path, data)?,
        }
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()?;
        Ok(outcome.plain.as_ref().map(|bytes| bytes.as_slice()))
    }
}
