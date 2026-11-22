//! Warm coverage of the exact complete SDK Welcome response. A covered
//! response stays private until the same Add is selected and installed.
use super::*;
use catcoms_sync::V2JoinResponseData;

mod archive_file;

pub(super) struct PreparedResponse {
    record: PreparedSmallControl,
    packet_digest: [u8; 32],
    archive: Option<archive_file::PreparedArchive>,
    _fee: CatalogReadReservation,
}

impl PreparedGovernedAddFence {
    pub(in crate::store) fn prepare_response(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        preimage: &[u8],
        outcome: &V2FounderAddOutcomeData<'_>,
        response: &V2JoinResponseData<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        let original = self
            .outcome
            .as_ref()
            .filter(|outcome| outcome.covered)
            .ok_or_else(refused)?;
        let header = self.header.as_mut().ok_or_else(refused)?;
        let (intent_digest, _) = header
            .welcome
            .as_ref()
            .filter(|(_, intent)| intent.covered)
            .ok_or_else(refused)?;
        if original.header_intent != outcome.unsigned_data().intent_commitment_data()
            || preimage.len() > catcoms_replication::chat::join_bindings::MAX_JOIN_BINDING_BYTES
            || intent_digest != blake3::hash(preimage).as_bytes()
        {
            return Err(refused());
        }
        let stage = header.welcome_signing.as_mut().ok_or_else(refused)?;
        let (signature, raw) = stage
            .raw
            .as_ref()
            .filter(|(_, raw)| raw.covered)
            .ok_or_else(refused)?;
        let raw_frame = &raw.original.as_ref().ok_or_else(refused)?.frame;
        let result = outcome.result_data();
        let original = response::OriginalResponse {
            preimage,
            signature,
            payloads: [
                result.welcome_data(),
                result
                    .sealed_routing_transfer_data()
                    .map_err(|_| refused())?,
            ],
        };
        if let Some(prepared) = &stage.response {
            original.require_packet(response)?;
            let packet = Zeroizing::new(response.encode_packet_data()?);
            if prepared.packet_digest != *blake3::hash(&packet).as_bytes() {
                return Err(refused());
            }
        } else {
            const BYTES: usize =
                6 * (response::MAX_BYTES + 1) + 4 * std::mem::size_of::<PreparedResponse>() + 4096;
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            let fee = pool.try_reserve(BYTES)?;
            fee.validate(pool, BYTES)?;
            original.require_packet(response)?;
            let packet = Zeroizing::new(response.encode_packet_data()?);
            let key = Zeroizing::new(store.keys.db_key()?);
            let frame = response::seal_data(&key, raw_frame, &packet, rng)?;
            stage.response = Some(PreparedResponse {
                record: PreparedSmallControl::new(frame),
                packet_digest: *blake3::hash(&packet).as_bytes(),
                archive: None,
                _fee: fee,
            });
        }
        self.held
            .resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn cover_response(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.cover_raw_welcome(guard, selected)?;
        let path = &self.held.welcome_paths.response;
        let head_stage = &self.held.welcome_paths.head_stage;
        let prepared = &mut self
            .header
            .as_mut()
            .and_then(|header| header.welcome_signing.as_mut()?.response.as_mut())
            .ok_or_else(refused)?
            .record;
        if !prepared.covered {
            require_absent(head_stage)?;
            prepared.cover(path, response::MAX_BYTES, guard)?;
            self.cover_raw_welcome(guard, selected)?;
        }
        let path = &self.held.welcome_paths.response;
        let prepared = &mut self
            .header
            .as_mut()
            .and_then(|header| header.welcome_signing.as_mut()?.response.as_mut())
            .ok_or_else(refused)?
            .record;
        prepared.require_covered(path)?;
        guard.validate_source_only()
    }
}
