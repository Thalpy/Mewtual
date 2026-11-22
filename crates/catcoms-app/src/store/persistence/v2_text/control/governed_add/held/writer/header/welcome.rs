//! Physical coverage of the original Add's Welcome intent and key stages.
use super::*;
use catcoms_replication::chat::join_bindings::WelcomeBindingSpecData;

mod packet;

pub(super) struct PreparedWelcomeSigning {
    parent: AddSigningParent,
    attempt: PreparedSmallControl,
    raw: Option<([u8; 64], PreparedSmallControl)>,
    response: Option<packet::PreparedResponse>,
    unexpected_raw: bool,
}

impl PreparedGovernedAddFence {
    pub(in crate::store) fn prepare_welcome_intent(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        packet: &V2JoinRequestPacketData<'_>,
        outcome: &V2FounderAddOutcomeData<'_>,
        preimage: &[u8],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        self.held.expected.require_packet(packet)?;
        let header = self.header.as_mut().ok_or_else(refused)?;
        let (id, record) = header
            .governing
            .as_ref()
            .filter(|(_, record)| record.covered)
            .ok_or_else(refused)?;
        let original = self
            .outcome
            .as_ref()
            .filter(|original| original.covered)
            .ok_or_else(refused)?;
        if original.header_intent != outcome.unsigned_data().intent_commitment_data() {
            return Err(refused());
        }
        let input = packet.founder_add_data();
        outcome.require_request_data(input)?;
        let spec =
            WelcomeBindingSpecData::decode_signing_bytes_data(preimage).map_err(|_| refused())?;
        let result = outcome.result_data();
        let sealed = result
            .sealed_routing_transfer_data()
            .map_err(|_| refused())?;
        welcome_intent::require_original_binding(
            &spec,
            packet,
            source.data().context.spec_data().signer,
            *id,
            outcome.admission_data().admission_id(),
            [result.welcome_data(), sealed],
        )?;

        let digest = *blake3::hash(preimage).as_bytes();
        if let Some((original, _)) = &header.welcome {
            if *original != digest {
                return Err(refused());
            }
        } else {
            let key = Zeroizing::new(store.keys.db_key()?);
            let raw = welcome_intent::seal_data(
                &key,
                &record.original.as_ref().ok_or_else(refused)?.frame,
                preimage,
                rng,
            )?;
            header.welcome = Some((digest, PreparedSmallControl::new(raw)));
        }
        self.held
            .resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn cover_welcome_intent(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if self
            .header
            .as_ref()
            .and_then(|header| header.welcome.as_ref())
            .is_none()
        {
            return Err(refused());
        }
        self.cover_governing_record(guard, selected)?;
        let (_, welcome) = self
            .header
            .as_mut()
            .and_then(|header| header.welcome.as_mut())
            .ok_or_else(refused)?;
        if !welcome.covered {
            require_absent(&self.held.welcome_paths.attempt)?;
            require_absent(&self.held.welcome_paths.raw)?;
            require_absent(&self.held.welcome_paths.response)?;
            welcome.cover(
                &self.held.welcome_paths.intent,
                welcome_intent::MAX_BYTES,
                guard,
            )?;
            self.cover_governing_record(guard, selected)?;
        }
        let (_, welcome) = self
            .header
            .as_mut()
            .and_then(|header| header.welcome.as_mut())
            .ok_or_else(refused)?;
        welcome.require_covered(&self.held.welcome_paths.intent)?;
        guard.validate_source_only()
    }

    pub(in crate::store) fn prepare_welcome_attempt(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        let header = self.header.as_mut().ok_or_else(refused)?;
        if header.welcome_signing.is_none() {
            let (_, intent) = header
                .welcome
                .as_ref()
                .filter(|(_, intent)| intent.covered)
                .ok_or_else(refused)?;
            let parent = AddSigningParent::from_welcome_intent(
                &intent.original.as_ref().ok_or_else(refused)?.frame,
            )?;
            let key = Zeroizing::new(store.keys.db_key()?);
            let attempt = PreparedSmallControl::new(parent.seal_attempt(&key, rng)?);
            header.welcome_signing = Some(PreparedWelcomeSigning {
                parent,
                attempt,
                raw: None,
                response: None,
                unexpected_raw: false,
            });
        }
        self.held
            .resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn cover_welcome_attempt(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if self
            .header
            .as_ref()
            .and_then(|header| header.welcome_signing.as_ref())
            .is_none()
        {
            return Err(refused());
        }
        self.cover_welcome_intent(guard, selected)?;
        require_absent(&self.held.welcome_paths.response)?;
        let stage = self
            .header
            .as_mut()
            .and_then(|header| header.welcome_signing.as_mut())
            .ok_or_else(refused)?;
        stage.require_raw_absent(&self.held.welcome_paths.raw)?;
        stage.attempt.cover(
            &self.held.welcome_paths.attempt,
            WELCOME_ATTEMPT_BYTES,
            guard,
        )?;
        self.cover_welcome_intent(guard, selected)?;
        let stage = self
            .header
            .as_mut()
            .and_then(|header| header.welcome_signing.as_mut())
            .ok_or_else(refused)?;
        stage
            .attempt
            .require_covered(&self.held.welcome_paths.attempt)?;
        stage.require_raw_absent(&self.held.welcome_paths.raw)?;
        require_absent(&self.held.welcome_paths.response)?;
        guard.validate_source_only()
    }

    pub(in crate::store) fn prepare_raw_welcome(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        preimage: &[u8],
        signature: &[u8; 64],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        let header = self.header.as_mut().ok_or_else(refused)?;
        let (digest, _) = header
            .welcome
            .as_ref()
            .filter(|(_, intent)| intent.covered)
            .ok_or_else(refused)?;
        let stage = header.welcome_signing.as_mut().ok_or_else(refused)?;
        if !stage.attempt.covered
            || stage.unexpected_raw
            || preimage.len() > catcoms_replication::chat::join_bindings::MAX_JOIN_BINDING_BYTES
            || digest != blake3::hash(preimage).as_bytes()
            || !catcoms_crypto::identity::verify_with_public_bytes(
                &source.data().context.spec_data().signer,
                preimage,
                signature,
            )
        {
            return Err(refused());
        }
        if let Some((cached, _)) = &stage.raw {
            if cached != signature {
                return Err(refused());
            }
        } else {
            let key = Zeroizing::new(store.keys.db_key()?);
            let attempt = &stage.attempt.original.as_ref().ok_or_else(refused)?.frame;
            let raw = stage.parent.seal_raw(&key, attempt, signature, rng)?;
            stage.raw = Some((*signature, PreparedSmallControl::new(raw)));
        }
        self.held
            .resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn cover_raw_welcome(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if self
            .header
            .as_ref()
            .and_then(|header| header.welcome_signing.as_ref())
            .and_then(|stage| stage.raw.as_ref())
            .is_none()
        {
            return Err(refused());
        }
        self.cover_welcome_intent(guard, selected)?;
        let stage = self
            .header
            .as_mut()
            .and_then(|header| header.welcome_signing.as_mut())
            .ok_or_else(refused)?;
        if stage.unexpected_raw {
            return Err(refused());
        }
        stage
            .attempt
            .require_covered(&self.held.welcome_paths.attempt)?;
        let (_, raw) = stage.raw.as_mut().ok_or_else(refused)?;
        if !raw.covered {
            require_absent(&self.held.welcome_paths.response)?;
            raw.cover(&self.held.welcome_paths.raw, WELCOME_RAW_BYTES, guard)?;
            self.cover_welcome_intent(guard, selected)?;
        }
        let stage = self
            .header
            .as_mut()
            .and_then(|header| header.welcome_signing.as_mut())
            .ok_or_else(refused)?;
        stage
            .attempt
            .require_covered(&self.held.welcome_paths.attempt)?;
        let (_, raw) = stage.raw.as_mut().ok_or_else(refused)?;
        raw.require_covered(&self.held.welcome_paths.raw)?;
        guard.validate_source_only()
    }
}

impl PreparedWelcomeSigning {
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
