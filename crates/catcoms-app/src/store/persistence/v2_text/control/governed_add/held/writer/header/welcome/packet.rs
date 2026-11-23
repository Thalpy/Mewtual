//! Warm coverage of the exact complete SDK Welcome response. A covered
//! response stays private until the same Add is selected and installed.
use super::*;
use crate::store::critical_namespaces::archive_current_codec::GenesisPublicData;
use crate::store::native_text_invite::PreparedNativeTextJoinResponse;
use crate::store::NativeTextSavedJoinResponse;
use catcoms_replication::chat::owner_header::{OwnerHeaderId, SignedOwnerHeader};
use catcoms_replication::chat::owner_state::transition::ReplayBody;
use catcoms_sync::{
    V2AdmissionStepData, V2FirstAddProofData, V2GoverningJoinDeliveryData, V2GoverningJoinStepData,
    V2JoinResponseData, V2MemberAddFrameData, V2MemberAddRequestData, MAX_V2_FIRST_ADD_PROOF_BYTES,
    MAX_V2_GOVERNING_JOIN_PREFIX_BYTES, MAX_V2_MEMBER_ADD_FRAME_BYTES,
};

mod archive_file;

pub(super) struct PreparedResponse {
    record: PreparedSmallControl,
    packet: PreparedNativeTextJoinResponse,
    member_add: Option<PreparedMemberAddFrame>,
    archive: Option<archive_file::PreparedArchive>,
    _fee: CatalogReadReservation,
}

struct PreparedMemberAddFrame {
    parent: (OwnerHeaderId, u64),
    frame: Vec<u8>,
}

impl PreparedGovernedAddFence {
    pub(in crate::store) fn prepare_response(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        preimage: &[u8],
        outcome: &V2FounderAddOutcomeData<'_>,
        signed_header: &SignedOwnerHeader,
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
        let (governing, _) = header
            .governing
            .as_ref()
            .filter(|(_, governing)| governing.covered)
            .ok_or_else(refused)?;
        if *governing != signed_header.id() {
            return Err(refused());
        }
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
            if prepared.packet.packet_bytes_data() != packet.as_slice() {
                return Err(refused());
            }
        } else {
            let later = self.held.later.as_deref();
            let proof_bound = if later.is_some() {
                MAX_V2_GOVERNING_JOIN_PREFIX_BYTES
            } else {
                MAX_V2_FIRST_ADD_PROOF_BYTES
            };
            let bytes = 6 * (response::MAX_BYTES + 1)
                + 4 * proof_bound
                + 4 * MAX_V2_MEMBER_ADD_FRAME_BYTES * usize::from(later.is_some())
                + 4 * std::mem::size_of::<PreparedResponse>()
                + 4096;
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            let fee = pool.try_reserve(bytes)?;
            fee.validate(pool, bytes)?;
            original.require_packet(response)?;
            let (signature, _) = header.raw.as_ref().ok_or_else(refused)?;
            governing_record::require_original_signature(
                signed_header,
                signature,
                outcome.unsigned_data().signing_bytes_data(),
            )?;
            let genesis = source.data();
            let proof = if let Some(later) = later {
                later.encode_successor_proof(
                    &GenesisPublicData {
                        profile: genesis.profile,
                        admission: genesis.admission,
                        header: genesis.header,
                        image: genesis.state,
                    },
                    V2GoverningJoinStepData {
                        header: signed_header,
                        image: outcome.unsigned_data().image_data(),
                        body: ReplayBody::Admission(outcome.admission_data()),
                    },
                )?
            } else {
                V2FirstAddProofData::encode(
                    genesis.profile,
                    V2AdmissionStepData {
                        admission: genesis.admission,
                        header: genesis.header,
                        image: genesis.state,
                    },
                    V2AdmissionStepData {
                        admission: outcome.admission_data(),
                        header: signed_header,
                        image: outcome.unsigned_data().image_data(),
                    },
                )?
            };
            let packet = Zeroizing::new(response.encode_packet_data()?);
            if later.is_some() {
                // Verify the complete frame fits before covering the private
                // archive and committing the governing successor.
                drop(V2GoverningJoinDeliveryData::encode(&packet, &proof)?);
            }
            let key = Zeroizing::new(store.keys.db_key()?);
            let frame = response::seal_data(&key, raw_frame, &packet, rng)?;
            let packet =
                PreparedNativeTextJoinResponse::prepare(pool, packet, proof, later.is_some())?;
            let member_add = if later.is_some() {
                let catcoms_replication::chat::owner_profile::AdmissionOrigin::Add {
                    before_epoch,
                    ..
                } = outcome.admission_data().spec().origin
                else {
                    return Err(refused());
                };
                let frame = V2MemberAddFrameData::encode(
                    V2AdmissionStepData {
                        admission: outcome.admission_data(),
                        header: signed_header,
                        image: outcome.unsigned_data().image_data(),
                    },
                    result.commit_data(),
                )?;
                Some(PreparedMemberAddFrame {
                    parent: (signed_header.spec().parent, before_epoch),
                    frame,
                })
            } else {
                None
            };
            stage.response = Some(PreparedResponse {
                record: PreparedSmallControl::new(frame),
                packet,
                member_add,
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

    pub(in crate::store) fn saved_response_data(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
        installed: TextHeadIdData,
    ) -> Result<NativeTextSavedJoinResponse, AppError> {
        self.require_selected_head2(guard, selected, installed)?;
        let response = self
            .header
            .as_ref()
            .and_then(|header| header.welcome_signing.as_ref()?.response.as_ref())
            .ok_or_else(refused)?;
        Ok(response.packet.saved_data())
    }

    pub(in crate::store) fn encode_selected_member_add(
        &mut self,
        store: &ServerStore,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
        installed: TextHeadIdData,
        request: &V2MemberAddRequestData,
    ) -> Result<Vec<u8>, AppError> {
        self.require_selected_head2(guard, selected, installed)?;
        let latest = self
            .header
            .as_ref()
            .and_then(|header| header.welcome_signing.as_ref()?.response.as_ref())
            .and_then(|response| response.member_add.as_ref())
            .ok_or_else(refused)?;
        let (_, epoch) = request.parent_data();
        let frame = if epoch == latest.parent.1 {
            if request.parent_data() != latest.parent
                || !self
                    .fetch_verifier
                    .is_some_and(|expected| request.matches_verifier_data(expected))
            {
                return Err(refused());
            }
            latest.frame.clone()
        } else {
            self.held
                .later
                .as_ref()
                .ok_or_else(refused)?
                .encode_retained_member_add(store, guard.source().source(), request)?
        };
        self.require_selected_head2(guard, selected, installed)?;
        Ok(frame)
    }
}
