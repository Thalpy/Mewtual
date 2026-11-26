//! Independent verification of the exact signed history row, after Raw.
use super::super::super::outcome::governing_bodies;
use super::*;
use catcoms_replication::chat::owner_header::SignedOwnerHeader;

mod welcome;
pub(in crate::store) use welcome::GovernedAddWelcomeRecoveryState;
use welcome::ObservedWelcome;

pub(super) struct ObservedGoverning {
    original: Option<OpenedTextControl>,
    verified: bool,
    header: Option<SignedOwnerHeader>,
    welcome: Option<ObservedWelcome>,
}

impl HeldGovernedAddFence {
    pub(in crate::store) fn capture_governing_record(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if !matches!(
            self.require_header_current(guard, selected)?,
            GovernedAddHeaderRecoveryState::SignatureRecovered(_)
        ) {
            return Err(refused());
        }
        let header = self.header.as_mut().ok_or_else(refused)?;
        if header.governing.is_some() {
            return Err(refused());
        }
        let original = read_stage(&self.governing_path, governing_record::MAX_BYTES)?;
        if original.is_none() {
            self.welcome_paths.require_absent()?;
        }
        header.governing = Some(ObservedGoverning {
            original,
            verified: false,
            header: None,
            welcome: None,
        });
        guard.validate_source_only()
    }

    pub(in crate::store) fn authenticate_governing_record(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.resources
            .require_bound_original_negative(store, source)?;
        let header = self.header.as_mut().ok_or_else(refused)?;
        let Some(GovernedAddHeaderRecoveryState::SignatureRecovered(signature)) = header.state
        else {
            return Err(refused());
        };
        let raw = header.raw.as_ref().ok_or_else(refused)?;
        let governing = header.governing.as_mut().ok_or_else(refused)?;
        if !governing.verified {
            if let Some(original) = &governing.original {
                let (_, plain) = self
                    .outcome
                    .as_ref()
                    .ok_or_else(refused)?
                    .authenticated_bytes()?;
                let Some(ObservedSdkEntry::Present(entry)) = self.entry.as_ref() else {
                    return Err(refused());
                };
                let expected = governing_bodies(&entry.frame, plain)?;
                let key = Zeroizing::new(store.keys.db_key()?);
                governing.header = Some(governing_record::open_data(
                    &key,
                    &raw.frame,
                    &original.frame,
                    &signature,
                    expected,
                )?);
            }
            governing.verified = true;
        }
        self.resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn require_governing_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<Option<&SignedOwnerHeader>, AppError> {
        self.require_header_current(guard, selected)?;
        let governing = self
            .header
            .as_mut()
            .and_then(|header| header.governing.as_mut())
            .filter(|governing| governing.verified)
            .ok_or_else(refused)?;
        require_stage(&self.governing_path, governing.original.as_mut())?;
        if governing.original.is_none() {
            self.welcome_paths.require_absent()?;
        }
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()?;
        Ok(governing.header.as_ref())
    }
}
