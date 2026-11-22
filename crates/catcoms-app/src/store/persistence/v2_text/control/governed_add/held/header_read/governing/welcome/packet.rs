//! The independent reader owns its response capture and read payment. The
//! decoded bytes can support recovery after a fresh check under Current.
use super::*;
use crate::store::persistence::v2_text::io::read_control_into;

pub(super) struct ObservedResponse {
    buffer: Option<Zeroizing<Vec<u8>>>,
    captured: Option<CapturedResponse>,
    packet: Option<Zeroizing<Vec<u8>>>,
    authenticated: bool,
    _fee: CatalogReadReservation,
}

enum CapturedResponse {
    Absent,
    Present(OpenedTextControl),
}

impl HeldGovernedAddFence {
    pub(in crate::store) fn prepare_response_read(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.resources
            .require_bound_original_negative(store, source)?;
        let welcome = self
            .header
            .as_mut()
            .and_then(|header| header.governing.as_mut()?.welcome.as_mut())
            .ok_or_else(refused)?;
        if !matches!(
            welcome.state,
            Some(GovernedAddWelcomeRecoveryState::SignatureRecovered(_))
        ) {
            return Err(refused());
        }
        if welcome.response.is_none() {
            const BYTES: usize =
                6 * (response::MAX_BYTES + 1) + 4 * std::mem::size_of::<ObservedResponse>() + 4096;
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            let fee = pool.try_reserve(BYTES)?;
            fee.validate(pool, BYTES)?;
            welcome.response = Some(ObservedResponse {
                buffer: Some(Zeroizing::new(Vec::with_capacity(response::MAX_BYTES + 1))),
                captured: None,
                packet: None,
                authenticated: false,
                _fee: fee,
            });
        }
        self.resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn capture_response(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if !matches!(
            self.require_welcome_current(guard, selected)?,
            GovernedAddWelcomeRecoveryState::SignatureRecovered(_)
        ) {
            return Err(refused());
        }
        let read = self
            .header
            .as_mut()
            .and_then(|header| {
                header
                    .governing
                    .as_mut()?
                    .welcome
                    .as_mut()?
                    .response
                    .as_mut()
            })
            .ok_or_else(refused)?;
        if read.captured.is_some() {
            return Err(refused());
        }
        let path = &self.welcome_paths.response;
        read.captured = Some(match fs::symlink_metadata(path) {
            Ok(_) => CapturedResponse::Present(read_control_into(
                path,
                response::MAX_BYTES,
                &mut read.buffer,
            )?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                require_absent(&self.welcome_paths.head_stage)?;
                CapturedResponse::Absent
            }
            Err(error) => return Err(io(error)),
        });
        guard.validate_source_only()
    }

    pub(in crate::store) fn authenticate_response(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.resources
            .require_bound_original_negative(store, source)?;
        let governing = self
            .header
            .as_mut()
            .and_then(|header| header.governing.as_mut())
            .ok_or_else(refused)?;
        let welcome = governing.welcome.as_mut().ok_or_else(refused)?;
        let Some(GovernedAddWelcomeRecoveryState::SignatureRecovered(signature)) = welcome.state
        else {
            return Err(refused());
        };
        let read = welcome.response.as_mut().ok_or_else(refused)?;
        if !read.authenticated {
            if let CapturedResponse::Present(captured) =
                read.captured.as_ref().ok_or_else(refused)?
            {
                let raw = welcome.raw.as_ref().ok_or_else(refused)?;
                let intent = welcome.intent.as_ref().ok_or_else(refused)?;
                let record = governing.original.as_ref().ok_or_else(refused)?;
                let (_, outcome) = self
                    .outcome
                    .as_ref()
                    .ok_or_else(refused)?
                    .authenticated_bytes()?;
                let Some(ObservedSdkEntry::Present(entry)) = self.entry.as_ref() else {
                    return Err(refused());
                };
                let key = Zeroizing::new(store.keys.db_key()?);
                let spec = welcome_intent::open_data(&key, &record.frame, &intent.frame)?;
                let preimage = spec.signing_bytes_data().map_err(|_| refused())?;
                read.packet = Some(response::open_data(
                    &key,
                    &raw.frame,
                    &captured.frame,
                    response::OriginalResponse {
                        preimage: &preimage,
                        signature: &signature,
                        payloads: welcome_bodies(&entry.frame, outcome)?,
                    },
                )?);
            }
            read.authenticated = true;
        }
        self.resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn require_response_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<Option<&[u8]>, AppError> {
        self.require_welcome_current(guard, selected)?;
        let read = self
            .header
            .as_mut()
            .and_then(|header| {
                header
                    .governing
                    .as_mut()?
                    .welcome
                    .as_mut()?
                    .response
                    .as_mut()
            })
            .filter(|read| read.authenticated)
            .ok_or_else(refused)?;
        let path = &self.welcome_paths.response;
        match read.captured.as_mut().ok_or_else(refused)? {
            CapturedResponse::Absent => {
                require_absent(path)?;
                require_absent(&self.welcome_paths.head_stage)?;
            }
            CapturedResponse::Present(original) => require_exact_held(path, original)?,
        }
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()?;
        Ok(read.packet.as_ref().map(|bytes| bytes.as_slice()))
    }
}
