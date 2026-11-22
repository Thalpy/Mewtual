//! Independent capture of the exact original Header attempt and signature.
use super::super::header_signing::HeaderOutcomeCommitment;
use super::super::outcome::header_signing_bytes;
use super::*;

mod governing;
use governing::ObservedGoverning;

pub(super) struct ObservedHeader {
    attempt: Option<OpenedTextControl>,
    raw: Option<OpenedTextControl>,
    complete_capture: bool,
    state: Option<GovernedAddHeaderRecoveryState>,
    governing: Option<ObservedGoverning>,
}

/// Historical status only. A missing attempt is no authority to sign, and a
/// present attempt means key entry cannot be ruled out after a restart.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) enum GovernedAddHeaderRecoveryState {
    OutcomeOnly,
    PossiblyEntered,
    SignatureRecovered([u8; 64]),
}

impl std::fmt::Debug for GovernedAddHeaderRecoveryState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::OutcomeOnly => "OutcomeOnly",
            Self::PossiblyEntered => "PossiblyEntered",
            Self::SignatureRecovered(_) => "SignatureRecovered",
        })
    }
}

impl HeldGovernedAddFence {
    pub(in crate::store) fn capture_header(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if self.header.is_some() || self.require_outcome_current(guard, selected)?.is_none() {
            return Err(refused());
        }
        self.header = Some(ObservedHeader {
            attempt: None,
            raw: None,
            complete_capture: false,
            state: None,
            governing: None,
        });
        let header = self.header.as_mut().ok_or_else(refused)?;
        header.attempt = read_stage(&self.header_attempt_path, ATTEMPT_BYTES)?;
        header.raw = read_stage(&self.raw_header_path, RAW_BYTES)?;
        if header.raw.is_none() {
            require_absent(&self.governing_path)?;
            require_absent(&self.welcome_intent_path)?;
        }
        guard.validate_source_only()?;
        header.complete_capture = true;
        Ok(())
    }

    pub(in crate::store) fn authenticate_header(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.resources
            .require_bound_original_negative(store, source)?;
        let (outcome_frame, outcome_plain) = self
            .outcome
            .as_ref()
            .ok_or_else(refused)?
            .authenticated_bytes()?;
        let Some(ObservedSdkEntry::Present(entry)) = self.entry.as_ref() else {
            return Err(refused());
        };
        let header = self
            .header
            .as_mut()
            .filter(|header| header.complete_capture)
            .ok_or_else(refused)?;
        if header.state.is_none() {
            let parent = HeaderOutcomeCommitment::from_outcome(outcome_frame)?;
            let key = Zeroizing::new(store.keys.db_key()?);
            let state = match (&header.attempt, &header.raw) {
                (None, None) => GovernedAddHeaderRecoveryState::OutcomeOnly,
                (Some(attempt), None) => {
                    parent.require_attempt(&key, &attempt.frame)?;
                    GovernedAddHeaderRecoveryState::PossiblyEntered
                }
                (Some(attempt), Some(raw)) => {
                    let signature = parent.open_raw(&key, &attempt.frame, &raw.frame)?;
                    let preimage = header_signing_bytes(&entry.frame, outcome_plain)?;
                    let signer = source.data().context.spec_data().signer;
                    if !catcoms_crypto::identity::verify_with_public_bytes(
                        &signer, preimage, &signature,
                    ) {
                        return Err(refused());
                    }
                    GovernedAddHeaderRecoveryState::SignatureRecovered(signature)
                }
                (None, Some(_)) => return Err(refused()),
            };
            header.state = Some(state);
        }
        self.resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn require_header_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<GovernedAddHeaderRecoveryState, AppError> {
        if self.require_outcome_current(guard, selected)?.is_none() {
            return Err(refused());
        }
        let header = self.header.as_mut().ok_or_else(refused)?;
        let state = header.state.ok_or_else(refused)?;
        require_stage(&self.header_attempt_path, header.attempt.as_mut())?;
        require_stage(&self.raw_header_path, header.raw.as_mut())?;
        if header.raw.is_none() {
            require_absent(&self.governing_path)?;
            require_absent(&self.welcome_intent_path)?;
        }
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()?;
        Ok(state)
    }
}

fn read_stage(path: &Path, maximum: usize) -> Result<Option<OpenedTextControl>, AppError> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_control(path, maximum).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io(error)),
    }
}

fn require_stage(path: &Path, capture: Option<&mut OpenedTextControl>) -> Result<(), AppError> {
    match capture {
        Some(original) => require_exact_held(path, original),
        None => require_absent(path),
    }
}
