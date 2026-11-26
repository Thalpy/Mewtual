//! The bounded public genesis and first Add records needed by a new joiner.
catcoms_rt::redacted_debug! {
    V2AdmissionStepData<'_>;
    PreparedV2JoinedPrefix;
    V2FirstJoinDeliveryData<'_>;
}

use super::*;
use catcoms_replication::chat::{
    cold_records::{encode_header_row, HeaderRow, MAX_HEADER_ROW_BYTES},
    owner_facts::OwnerFactId,
    owner_header::{OwnerHeaderId, OwnerStateCommitment, OwnerTenureId, SignedOwnerHeader},
    owner_profile::{
        AdmissionBody, AdmissionOrigin, SignedOwnerProfile, MAX_ADMISSION_BYTES, MAX_PROFILE_BYTES,
    },
    owner_state::transition::{ReplayBody, ReplayMath},
    ActionDraft, FounderGenesisAnchorData, ProjectionGroupView, ProjectionView,
    V2AuthenticatedAdmission, V2GeneralCandidate, V2GeneralExpectation, V2GeneralPrefixVerifier,
    V2GeneralRetiredData, V2HostSelectionExpectation, V2OrdinaryDependency, V2OrdinarySigningPlan,
    V2PreparedSelectedHistoryData, V2ProjectionOrigin,
};
use std::sync::{Arc, Weak};

const TAG: &[u8] = b"mewtual/chat/first-add-proof/v2\0";
pub(super) const FIRST_JOIN_DELIVERY_TAG: &[u8] = b"mewtual/chat/first-join-delivery/v2\0";
const FRAME_OVERHEAD: usize = 4 + TAG.len() + 2 + 5 * 4;
pub const MAX_V2_FIRST_ADD_PROOF_BYTES: usize =
    FRAME_OVERHEAD + MAX_PROFILE_BYTES + 2 * (MAX_ADMISSION_BYTES + MAX_HEADER_ROW_BYTES);
pub const MAX_V2_FIRST_JOIN_DELIVERY_BYTES: usize = 4
    + FIRST_JOIN_DELIVERY_TAG.len()
    + 2
    + 2 * 4
    + MAX_V2_FIRST_ADD_PROOF_BYTES
    + MAX_V2_JOIN_RESPONSE_PACKET_BYTES;

pub struct V2AdmissionStepData<'a> {
    pub admission: &'a AdmissionBody,
    pub header: &'a SignedOwnerHeader,
    pub image: &'a [u8],
}

pub struct V2FirstAddProofData<'a> {
    profile: SignedOwnerProfile,
    founder: AdmissionBody,
    genesis: HeaderRow<'a>,
    admission: AdmissionBody,
    add: HeaderRow<'a>,
    bytes: usize,
}

impl<'a> V2FirstAddProofData<'a> {
    pub fn encode(
        profile: &SignedOwnerProfile,
        genesis: V2AdmissionStepData<'_>,
        add: V2AdmissionStepData<'_>,
    ) -> Result<Vec<u8>, SyncError> {
        let encoded_profile = profile.encode();
        let founder = genesis.admission.encode();
        let original =
            encode_header_row(genesis.header, genesis.image).map_err(|_| SyncError::Malformed)?;
        let admission = add.admission.encode();
        let next = encode_header_row(add.header, add.image).map_err(|_| SyncError::Malformed)?;
        let fields = [
            encoded_profile.as_slice(),
            founder.as_slice(),
            original.as_slice(),
            admission.as_slice(),
            next.as_slice(),
        ];
        let length = FRAME_OVERHEAD + fields.iter().map(|field| field.len()).sum::<usize>();
        if length > MAX_V2_FIRST_ADD_PROOF_BYTES {
            return Err(SyncError::Malformed);
        }
        let mut e = Encoder::with_capacity(length);
        e.put_bytes(TAG).map_err(|_| SyncError::Malformed)?;
        e.put_u16(VERSION);
        for field in fields {
            e.put_bytes(field).map_err(|_| SyncError::Malformed)?;
        }
        Ok(e.finish())
    }

    pub fn decode(raw: &'a [u8]) -> Result<Self, SyncError> {
        let malformed = |_| SyncError::Malformed;
        if raw.len() > MAX_V2_FIRST_ADD_PROOF_BYTES {
            return Err(SyncError::Malformed);
        }
        let mut d = Decoder::new(raw);
        if d.get_bytes().map_err(malformed)? != TAG || d.get_u16().map_err(malformed)? != VERSION {
            return Err(SyncError::Malformed);
        }
        let profile = d.get_bytes().map_err(malformed)?;
        let founder = d.get_bytes().map_err(malformed)?;
        let genesis = d.get_bytes().map_err(malformed)?;
        let admission = d.get_bytes().map_err(malformed)?;
        let add = d.get_bytes().map_err(malformed)?;
        d.finish().map_err(malformed)?;
        let invalid = |_| SyncError::JoinRejected;
        Ok(Self {
            profile: SignedOwnerProfile::decode(profile).map_err(invalid)?,
            founder: AdmissionBody::decode(founder).map_err(invalid)?,
            genesis: HeaderRow::decode(genesis).map_err(invalid)?,
            admission: AdmissionBody::decode(admission).map_err(invalid)?,
            add: HeaderRow::decode(add).map_err(invalid)?,
            bytes: raw.len(),
        })
    }

    pub(super) fn verified_admission(
        &self,
        accepted: &GenesisAnchorBindingData,
    ) -> Result<(&SignedOwnerProfile, &AdmissionBody, &SignedOwnerHeader), SyncError> {
        self.replay_through_add(accepted)?;
        Ok((&self.profile, &self.admission, self.add.header()))
    }

    pub(super) fn replay_through_add(
        &self,
        accepted: &GenesisAnchorBindingData,
    ) -> Result<ReplayMath, SyncError> {
        let genesis = self.genesis.header();
        if genesis.spec().group != accepted.group
            || genesis.spec().issuer != accepted.founder
            || self.profile.id().as_bytes() != &accepted.profile
            || self.profile.tenure().as_bytes() != &accepted.tenure
            || self.founder.admission_id().as_bytes() != &accepted.admission
            || genesis.id().as_bytes() != &accepted.header
            || genesis.spec().state.as_bytes() != &accepted.state
        {
            return Err(SyncError::JoinRejected);
        }
        let through_add =
            ReplayMath::from_genesis(&self.profile, &self.founder, genesis, self.genesis.image())
                .and_then(|prior| {
                    prior.derive_successor(
                        self.add.header(),
                        ReplayBody::Admission(&self.admission),
                        self.add.image(),
                    )
                })
                .map_err(|_| SyncError::JoinRejected)?;
        Ok(through_add.next)
    }

    pub(super) fn frame_bytes(&self) -> usize {
        self.bytes
    }

    /// Original decoded header labels for an independently selected first
    /// record. Decoding alone establishes no selected history.
    pub fn header_ids_data(&self) -> (OwnerHeaderId, OwnerHeaderId) {
        (self.genesis.header().id(), self.add.header().id())
    }

    pub(super) fn add_image_data(&self) -> &[u8] {
        self.add.image()
    }

    pub(super) fn profile_data(&self) -> &SignedOwnerProfile {
        &self.profile
    }

    pub(super) fn prepare_joined_prefix(
        &self,
        accepted: &GenesisAnchorBindingData,
        group: &ProjectionGroupView,
        origin: V2ProjectionOrigin,
    ) -> Result<PreparedV2JoinedPrefix, SyncError> {
        let (profile, own, add) = self.verified_admission(accepted)?;
        let reject = |_| SyncError::JoinRejected;
        let expected = V2GeneralExpectation::from_trusted_host(FounderGenesisAnchorData {
            group: accepted.group.clone(),
            founder: accepted.founder,
            profile: OwnerFactId::from_bytes(accepted.profile),
            admission: OwnerFactId::from_bytes(accepted.admission),
            tenure: OwnerTenureId::from_bytes(accepted.tenure),
            header: OwnerHeaderId::from_bytes(accepted.header),
            state: OwnerStateCommitment::from_bytes(accepted.state),
        })
        .map_err(reject)?;
        let mut verifier = V2GeneralPrefixVerifier::from_genesis(
            expected,
            profile,
            &self.founder,
            self.genesis.header(),
            self.genesis.image(),
            group,
            origin,
        )
        .map_err(reject)?;
        let candidate = verifier
            .derive_successor(add, ReplayBody::Admission(own), self.add.image())
            .map_err(reject)?;
        Ok(PreparedV2JoinedPrefix {
            verifier,
            candidate: Some(candidate),
            own: own.admission_id(),
            source_origin: None,
        })
    }
}

/// Real Core candidate for the original authenticated first Add. Its App
/// owner keeps the matching selected journal and native scope before advancing.
pub struct PreparedV2JoinedPrefix {
    verifier: V2GeneralPrefixVerifier,
    candidate: Option<V2GeneralCandidate>,
    own: OwnerFactId,
    source_origin: Option<Weak<()>>,
}

pub(super) struct SelectedJoinData<'a> {
    pub anchor: [u8; 32],
    pub welcome: &'a catcoms_replication::chat::join_bindings::SignedWelcomeBindingData,
    pub state: OwnerStateCommitment,
    pub signer: &'a [u8; 32],
    pub key_package: &'a [u8; 32],
    pub credential: &'a [u8; 32],
    pub epoch: u64,
}

impl PreparedV2JoinedPrefix {
    pub(super) fn bind_source_origin(&mut self, original: &Arc<()>) {
        self.source_origin = Some(Arc::downgrade(original));
    }

    pub(super) fn require_join_selection(
        &self,
        original: &Arc<()>,
        join: SelectedJoinData<'_>,
    ) -> Result<V2AuthenticatedAdmission, SyncError> {
        let reject = |_| SyncError::JoinRejected;
        if self
            .source_origin
            .as_ref()
            .is_none_or(|other| !std::ptr::eq(other.as_ptr(), Arc::as_ptr(original)))
        {
            return Err(SyncError::JoinRejected);
        }
        self.verifier
            .require_join_genesis_data(join.anchor)
            .map_err(reject)?;
        let replay = self.selected_replay_data().ok_or(SyncError::JoinRejected)?;
        let admission = self.verifier.admission(self.own).map_err(reject)?;
        let spec = admission.admission_data().spec();
        let AdmissionOrigin::Add {
            key_package,
            credential,
            after_epoch,
            ..
        } = spec.origin
        else {
            return Err(SyncError::JoinRejected);
        };
        if !admission.is_open_data()
            || &spec.subject != join.signer
            || &key_package != join.key_package
            || &credential != join.credential
            || after_epoch != join.epoch
            || replay.mls_epoch_data() != join.epoch
            || replay.header().spec().state != join.state
            || replay.header().id().as_bytes() != &join.welcome.spec_data().add_header
            || self.own.as_bytes() != &join.welcome.spec_data().admission
        {
            return Err(SyncError::JoinRejected);
        }
        Ok(admission)
    }

    pub fn own_admission_data(&self) -> OwnerFactId {
        self.own
    }

    /// The authenticated root Profile is shared by genesis and its first Add.
    /// This is stable context DATA and can be read before selecting the Add.
    pub fn initial_profile_data(&self) -> &SignedOwnerProfile {
        self.verifier.replay_data().profile()
    }

    /// Borrow the selected verifier for the joined source's ordinary plan.
    /// An unselected first Add still owes the host its original file proof.
    pub fn selected_member_verifier(&self) -> Result<&V2GeneralPrefixVerifier, SyncError> {
        self.candidate
            .is_none()
            .then_some(&self.verifier)
            .ok_or(SyncError::JoinRejected)
    }

    pub(super) fn prepare_member_successor(
        &mut self,
        step: V2AdmissionStepData<'_>,
    ) -> Result<V2GeneralCandidate, SyncError> {
        self.selected_member_verifier()?;
        self.verifier
            .derive_successor(
                step.header,
                ReplayBody::Admission(step.admission),
                step.image,
            )
            .map_err(|_| SyncError::JoinRejected)
    }

    pub(super) fn select_member_successor(
        &mut self,
        selection: &V2HostSelectionExpectation,
        candidate: &mut Option<V2GeneralCandidate>,
    ) -> Result<V2GeneralRetiredData, SyncError> {
        self.selected_member_verifier()?;
        self.verifier
            .advance_host_selected(selection, candidate)
            .map_err(|_| SyncError::JoinRejected)
    }

    pub(super) fn prepare_member_removal(
        &mut self,
        header: &SignedOwnerHeader,
        body: &catcoms_replication::chat::owner_bodies::RemovalBody,
        image: &[u8],
    ) -> Result<V2GeneralCandidate, SyncError> {
        self.selected_member_verifier()?;
        self.verifier
            .derive_successor(header, ReplayBody::Removal(body), image)
            .map_err(|_| SyncError::JoinRejected)
    }

    /// Replay one more historical step privately before the original Welcome
    /// selection. Only the last candidate can be selected by the mounted host.
    pub(super) fn continue_initial_prefix(
        &mut self,
        header: &SignedOwnerHeader,
        image: &[u8],
        body: ReplayBody<'_>,
    ) -> Result<(), SyncError> {
        let history = self.candidate.as_ref().ok_or(SyncError::JoinRejected)?;
        let selected = history.header_data();
        let expectation =
            V2HostSelectionExpectation::from_trusted_host(selected.id(), selected.spec().state);
        let mut retired = self.finish_host_selected(&expectation)?;
        if retired
            .take_close_completion()
            .is_some_and(|mut close| !close.try_complete())
        {
            return Err(SyncError::JoinRejected);
        }
        drop(retired);
        let own = match &body {
            ReplayBody::Admission(admission) => admission.admission_id(),
            _ => self.own,
        };
        self.candidate = Some(
            self.verifier
                .derive_successor(header, body, image)
                .map_err(|_| SyncError::JoinRejected)?,
        );
        self.own = own;
        Ok(())
    }

    pub(super) fn observe_member_successor_header(
        &mut self,
        header: &SignedOwnerHeader,
    ) -> Result<(), SyncError> {
        self.selected_member_verifier()?;
        self.verifier
            .observe_current_header(header)
            .map_err(|_| SyncError::JoinRejected)
    }

    /// The two exact signed successors that closed this selected member view.
    pub fn member_successor_contradiction_data(
        &self,
    ) -> Option<(&SignedOwnerHeader, &SignedOwnerHeader)> {
        self.candidate.is_none().then_some(())?;
        self.verifier.current_contradiction_data()
    }

    /// Use the same selected Core verifier and original own Admission to plan
    /// an ordinary action. The Join candidate must already have been selected.
    pub fn prepare_member_plan(
        &self,
        draft: &ActionDraft,
        dependencies: &[V2OrdinaryDependency<'_>],
        view: &ProjectionView,
    ) -> Result<V2OrdinarySigningPlan, SyncError> {
        let verifier = self.selected_member_verifier()?;
        let admission = verifier
            .admission(self.own)
            .map_err(|_| SyncError::JoinRejected)?;
        verifier
            .prepare_ordinary_signing_plan(draft, &admission, dependencies, view)
            .map_err(|_| SyncError::JoinRejected)
    }

    pub fn pending_history_data(&self) -> Option<&V2PreparedSelectedHistoryData> {
        self.candidate
            .as_ref()
            .map(V2GeneralCandidate::selected_history_append_data)
    }

    pub fn finish_host_selected(
        &mut self,
        selection: &V2HostSelectionExpectation,
    ) -> Result<V2GeneralRetiredData, SyncError> {
        self.verifier
            .advance_host_selected(selection, &mut self.candidate)
            .map_err(|_| SyncError::JoinRejected)
    }

    pub fn selected_replay_data(&self) -> Option<&ReplayMath> {
        self.candidate
            .is_none()
            .then(|| self.verifier.replay_data())
    }
}

impl fmt::Debug for V2FirstAddProofData<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("V2FirstAddProofData")
            .finish_non_exhaustive()
    }
}

/// Bounded wire pairing of a signed response and its public proof.
pub struct V2FirstJoinDeliveryData<'a> {
    response: V2JoinResponsePacketData<'a>,
    proof: V2FirstAddProofData<'a>,
    proof_bytes: &'a [u8],
}

impl<'a> V2FirstJoinDeliveryData<'a> {
    pub fn encode(response: &[u8], proof: &[u8]) -> Result<Vec<u8>, SyncError> {
        if response.is_empty()
            || response.len() > MAX_V2_JOIN_RESPONSE_PACKET_BYTES
            || proof.is_empty()
            || proof.len() > MAX_V2_FIRST_ADD_PROOF_BYTES
        {
            return Err(SyncError::Malformed);
        }
        let length = 4 + FIRST_JOIN_DELIVERY_TAG.len() + 2 + 2 * 4 + response.len() + proof.len();
        let mut e = Encoder::with_capacity(length);
        e.put_bytes(FIRST_JOIN_DELIVERY_TAG)
            .map_err(|_| SyncError::Malformed)?;
        e.put_u16(VERSION);
        e.put_bytes(response).map_err(|_| SyncError::Malformed)?;
        e.put_bytes(proof).map_err(|_| SyncError::Malformed)?;
        Ok(e.finish())
    }

    pub fn decode(raw: &'a [u8]) -> Result<Self, SyncError> {
        if raw.len() > MAX_V2_FIRST_JOIN_DELIVERY_BYTES {
            return Err(SyncError::Malformed);
        }
        let malformed = |_| SyncError::Malformed;
        let mut d = Decoder::new(raw);
        if d.get_bytes().map_err(malformed)? != FIRST_JOIN_DELIVERY_TAG
            || d.get_u16().map_err(malformed)? != VERSION
        {
            return Err(SyncError::Malformed);
        }
        let response = d.get_bytes().map_err(malformed)?;
        let proof_bytes = d.get_bytes().map_err(malformed)?;
        d.finish().map_err(malformed)?;
        if response.len() > MAX_V2_JOIN_RESPONSE_PACKET_BYTES
            || proof_bytes.len() > MAX_V2_FIRST_ADD_PROOF_BYTES
        {
            return Err(SyncError::Malformed);
        }
        Ok(Self {
            response: V2JoinResponsePacketData::decode(response)?,
            proof: V2FirstAddProofData::decode(proof_bytes)?,
            proof_bytes,
        })
    }

    pub(super) fn data(&self) -> (V2JoinResponseData<'_>, &V2FirstAddProofData<'a>) {
        (self.response.response_data(), &self.proof)
    }

    /// Original public proof bytes. The private Welcome/route transfer and
    /// transport admission are outside this representation.
    pub fn governing_proof_data(&self) -> (&V2FirstAddProofData<'a>, &'a [u8]) {
        (&self.proof, self.proof_bytes)
    }
}
