//! Read-only recovery of the original governed Welcome journal.
use super::*;
use crate::store::persistence::v2_text::control::governed_add::outcome::welcome_bodies;
use catcoms_replication::chat::owner_profile::AdmissionBody;

#[derive(Default)]
pub(super) struct ObservedWelcome {
    intent: Option<OpenedTextControl>,
    attempt: Option<OpenedTextControl>,
    raw: Option<OpenedTextControl>,
    complete_capture: bool,
    state: Option<GovernedAddWelcomeRecoveryState>,
}

/// Historical classification for this exact request, after fresh Current and
/// exact file reproof. The possible-entry branch cannot authorize a new sign.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) enum GovernedAddWelcomeRecoveryState {
    GoverningOnly,
    IntentCovered,
    PossiblyEntered,
    SignatureRecovered([u8; 64]),
}

impl std::fmt::Debug for GovernedAddWelcomeRecoveryState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::GoverningOnly => "GoverningOnly",
            Self::IntentCovered => "IntentCovered",
            Self::PossiblyEntered => "PossiblyEntered",
            Self::SignatureRecovered(_) => "SignatureRecovered",
        })
    }
}

impl HeldGovernedAddFence {
    pub(in crate::store) fn capture_welcome(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if self.require_governing_current(guard, selected)?.is_none() {
            return Err(refused());
        }
        let governing = self
            .header
            .as_mut()
            .and_then(|header| header.governing.as_mut())
            .ok_or_else(refused)?;
        if governing.welcome.is_some() {
            return Err(refused());
        }
        let observed = governing.welcome.insert(ObservedWelcome::default());
        observed.intent = read_stage(&self.welcome_paths.intent, welcome_intent::MAX_BYTES)?;
        observed.attempt = read_stage(&self.welcome_paths.attempt, WELCOME_ATTEMPT_BYTES)?;
        observed.raw = read_stage(&self.welcome_paths.raw, WELCOME_RAW_BYTES)?;
        guard.validate_source_only()?;
        observed.complete_capture = true;
        Ok(())
    }

    pub(in crate::store) fn authenticate_welcome(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        packet: &V2JoinRequestPacketData<'_>,
    ) -> Result<(), AppError> {
        self.resources
            .require_bound_original_negative(store, source)?;
        self.expected.require_packet(packet)?;
        let governing = self
            .header
            .as_mut()
            .and_then(|header| header.governing.as_mut())
            .filter(|governing| governing.verified)
            .ok_or_else(refused)?;
        let header = governing.header.as_ref().ok_or_else(refused)?;
        let record = governing.original.as_ref().ok_or_else(refused)?;
        let observed = governing
            .welcome
            .as_mut()
            .filter(|observed| observed.complete_capture)
            .ok_or_else(refused)?;
        if observed.state.is_none() {
            observed.state = Some(match (&observed.intent, &observed.attempt, &observed.raw) {
                (None, None, None) => GovernedAddWelcomeRecoveryState::GoverningOnly,
                (Some(intent), attempt, raw) => {
                    let (_, outcome) = self
                        .outcome
                        .as_ref()
                        .ok_or_else(refused)?
                        .authenticated_bytes()?;
                    let Some(ObservedSdkEntry::Present(entry)) = self.entry.as_ref() else {
                        return Err(refused());
                    };
                    let [admission, ..] = governing_bodies(&entry.frame, outcome)?;
                    let admission = AdmissionBody::decode(admission).map_err(|_| refused())?;
                    let key = Zeroizing::new(store.keys.db_key()?);
                    let spec = welcome_intent::open_data(&key, &record.frame, &intent.frame)?;
                    let signer = source.data().context.spec_data().signer;
                    welcome_intent::require_original_binding(
                        &spec,
                        packet,
                        signer,
                        header.id(),
                        admission.admission_id(),
                        welcome_bodies(&entry.frame, outcome)?,
                    )?;
                    match (attempt, raw) {
                        (None, None) => GovernedAddWelcomeRecoveryState::IntentCovered,
                        (Some(attempt), None) => {
                            AddSigningParent::from_welcome_intent(&intent.frame)?
                                .require_attempt(&key, &attempt.frame)?;
                            GovernedAddWelcomeRecoveryState::PossiblyEntered
                        }
                        (Some(attempt), Some(raw)) => {
                            let signature = AddSigningParent::from_welcome_intent(&intent.frame)?
                                .open_raw(&key, &attempt.frame, &raw.frame)?;
                            let preimage = spec.signing_bytes_data().map_err(|_| refused())?;
                            if !catcoms_crypto::identity::verify_with_public_bytes(
                                &signer, &preimage, &signature,
                            ) {
                                return Err(refused());
                            }
                            GovernedAddWelcomeRecoveryState::SignatureRecovered(signature)
                        }
                        (None, Some(_)) => return Err(refused()),
                    }
                }
                (None, _, _) => return Err(refused()),
            });
        }
        self.resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn require_welcome_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<GovernedAddWelcomeRecoveryState, AppError> {
        if self.require_governing_current(guard, selected)?.is_none() {
            return Err(refused());
        }
        let observed = self
            .header
            .as_mut()
            .and_then(|header| header.governing.as_mut()?.welcome.as_mut())
            .ok_or_else(refused)?;
        let state = observed.state.ok_or_else(refused)?;
        require_stage(&self.welcome_paths.intent, observed.intent.as_mut())?;
        require_stage(&self.welcome_paths.attempt, observed.attempt.as_mut())?;
        require_stage(&self.welcome_paths.raw, observed.raw.as_mut())?;
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()?;
        Ok(state)
    }
}
