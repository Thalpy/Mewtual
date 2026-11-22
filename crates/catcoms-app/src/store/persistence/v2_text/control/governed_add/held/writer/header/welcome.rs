//! Physical coverage of the exact unsigned Welcome from the original Add.
use super::*;
use catcoms_replication::chat::join_bindings::{JoinPayloadKindData, WelcomeBindingSpecData};

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
        let request = input.request.spec_data();
        let invite = input.invite.spec_data();
        let result = outcome.result_data();
        let sealed = result
            .sealed_routing_transfer_data()
            .map_err(|_| refused())?;
        if spec.invite != input.invite.id_data()
            || spec.anchor != request.anchor
            || spec.request_binding != input.request.id_data()
            || spec.key_package_wire != request.key_package_wire
            || spec.key_package_reference != request.key_package_reference
            || spec.admitter != source.data().context.spec_data().signer
            || spec.add_header != *id.as_bytes()
            || spec.admission != *outcome.admission_data().admission_id().as_bytes()
            || spec.policy != invite.policy
            || spec.public_routing_plan != invite.public_routing_plan
            || spec.welcome
                != JoinPayloadKindData::Welcome
                    .commitment_data(result.welcome_data())
                    .map_err(|_| refused())?
            || spec.sealed_routing_transfer
                != JoinPayloadKindData::RoutingTransfer
                    .commitment_data(sealed)
                    .map_err(|_| refused())?
        {
            return Err(refused());
        }

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
        welcome.cover(
            &self.held.welcome_intent_path,
            welcome_intent::MAX_BYTES,
            guard,
        )?;
        self.cover_governing_record(guard, selected)?;
        let (_, welcome) = self
            .header
            .as_mut()
            .and_then(|header| header.welcome.as_mut())
            .ok_or_else(refused)?;
        welcome.require_covered(&self.held.welcome_intent_path)?;
        guard.validate_source_only()
    }
}
