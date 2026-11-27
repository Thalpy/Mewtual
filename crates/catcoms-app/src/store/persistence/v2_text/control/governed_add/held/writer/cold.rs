//! Exact governing Add journal in the selected cold founder's directory.
use super::super::super::archive::{
    self, recovery::ColdGovernedWriteScope, GovernedArchiveFrames, GovernedArchiveLabels,
};
use super::super::super::header_signing::AddSigningParent;
use super::*;
use crate::store::critical_namespaces::archive_current_codec::GenesisPublicData;
use crate::store::persistence::v2_text::io::archive_child;
use crate::store::persistence::OriginalBootstrapReadObservation;
use crate::store::RetainedBootstrapSourceData;
use catcoms_replication::chat::owner_header::SignedOwnerHeader;
use catcoms_replication::chat::owner_state::transition::ReplayBody;
use catcoms_replication::chat::{join_bindings::WelcomeBindingSpecData, V2GeneralCandidate};
use catcoms_rt::RequestCancellation;
use catcoms_sync::owner_v2::{PreparedV2RecoveredAddInstall, FOUNDER_GOVERNANCE_DATA_BYTES};
use catcoms_sync::{
    V2GoverningJoinDeliveryData, V2GoverningJoinStepData, V2JoinResponseData,
    MAX_V2_GOVERNING_JOIN_PREFIX_BYTES,
};

/// The issuing runtime retains every frozen frame and its held file.
/// Another runtime cannot adopt a stage based on its name or public Header.
pub(in crate::store) struct PreparedColdGovernedAdd {
    controls: [Option<PreparedSmallControl>; ADD_STAGES.len()],
    // The original large outcome invoice outlives its named bytes above.
    outcome: Option<SealedGovernedAddOutcome>,
    header_intent: Option<[u8; 32]>,
    header_signature: Option<[u8; 64]>,
    governing: Option<OwnerHeaderId>,
    welcome_intent: Option<blake3::Hash>,
    welcome_signature: Option<[u8; 64]>,
    packet: Zeroizing<Vec<u8>>,
    parent: (TextContextIdData, TextHeadIdData),
    parent_header: OwnerHeaderId,
    original: OriginalBootstrapReadObservation,
    response_fee: Option<CatalogReadReservation>,
    archive: Option<PreparedColdArchive>,
    _lease: VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

// These indices follow ADD_STAGES. Each attempt and raw result is separate
// from its predecessor. Adding a stage must keep the path and cap in sync.
#[derive(Clone, Copy)]
enum Stage {
    Fence,
    Request,
    Entry,
    Outcome,
    HeaderAttempt,
    RawHeader,
    Governing,
    WelcomeIntent,
    WelcomeAttempt,
    RawWelcome,
    Response,
    Head,
}

const CAPS: [usize; ADD_STAGES.len()] = [
    FRAME_BYTES,
    request::MAX_BYTES,
    ENTRY_BYTES,
    MAX_FRAME,
    ATTEMPT_BYTES,
    RAW_BYTES,
    governing_record::MAX_BYTES,
    welcome_intent::MAX_BYTES,
    WELCOME_ATTEMPT_BYTES,
    WELCOME_RAW_BYTES,
    response::MAX_BYTES,
    selected_head::FRAME_LEN,
];

struct PreparedColdArchive {
    labels: GovernedArchiveLabels,
    record: PreparedSmallControl,
    path: PathBuf,
    head: TextHeadIdData,
    selection_entered: bool,
    selected: bool,
    _fee: CatalogReadReservation,
}

impl PreparedColdGovernedAdd {
    /// Prepare under the checked original row and covered invitation.
    /// No file is created until the owning cold writer takes its file cut.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        packet: &V2JoinRequestPacketData<'_>,
        fetch_verifier: [u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        let (_, epoch) = archive.require_current(store, source)?;
        selected_head::next_supported_epoch(epoch)?;
        let (context, head, _, parent_header, _) = archive.writer_parent_data();
        let bytes = packet.packet_bytes_data();
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee_bytes = WRITER_BYTES
            + FENCE_READ_BYTES
            + FOUNDER_GOVERNANCE_DATA_BYTES
            + 4 * std::mem::size_of::<Self>()
            + 2 * bytes.len();
        let fee = pool.try_reserve(fee_bytes)?;
        fee.validate(pool, fee_bytes)?;
        let lease = store._session.retain_for_physical_work();
        let key = Zeroizing::new(store.keys.db_key()?);
        let fence =
            GovernedAddFenceData::for_request(context, head, packet).seal_data(&key, rng)?;
        let request =
            request::seal_with_fetch_verifier(&key, &fence, packet, Some(fetch_verifier), rng)?;
        archive.require_current(store, source)?;
        let mut controls = std::array::from_fn(|_| None);
        controls[Stage::Fence as usize] = Some(PreparedSmallControl::new(fence));
        controls[Stage::Request as usize] = Some(PreparedSmallControl::new(request));
        Ok(Self {
            controls,
            outcome: None,
            header_intent: None,
            header_signature: None,
            governing: None,
            welcome_intent: None,
            welcome_signature: None,
            packet: Zeroizing::new(bytes.to_vec()),
            parent: (context, head),
            parent_header,
            original: source.observe_original(),
            _lease: lease,
            response_fee: None,
            archive: None,
            _fee: fee,
        })
    }

    pub(in crate::store) fn require_original_packet(
        &self,
        source: &RetainedBootstrapSourceData,
        packet: &V2JoinRequestPacketData<'_>,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        if self.packet.as_slice() != packet.packet_bytes_data() {
            return Err(refused());
        }
        Ok(())
    }

    pub(in crate::store) fn cover_reservation(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        caller: &RequestCancellation,
    ) -> Result<(), AppError> {
        self.cover_through(store, source, archive, Stage::Request, caller)
    }

    pub(in crate::store) fn prepare_sdk_entry(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        source.require_original_negative(store)?;
        if self.controls[Stage::Entry as usize].is_none() {
            self.frame(Stage::Request)?;
            let key = Zeroizing::new(store.keys.db_key()?);
            let entry = GovernedAddSdkEntryData::from_fence_data(self.frame(Stage::Fence)?)?
                .seal_data(&key, rng)?;
            self.controls[Stage::Entry as usize] = Some(PreparedSmallControl::new(entry));
        }
        source.require_original_negative(store)
    }

    pub(in crate::store) fn cover_sdk_entry(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        caller: &RequestCancellation,
    ) -> Result<(), AppError> {
        self.cover_through(store, source, archive, Stage::Entry, caller)
    }

    pub(in crate::store) fn prepare_outcome(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        outcome: &V2FounderAddOutcomeData<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        source.require_original_negative(store)?;
        if outcome.unsigned_data().header_spec_data().parent != self.parent_header {
            return Err(refused());
        }
        let packet = V2JoinRequestPacketData::decode(&self.packet)?;
        outcome.require_request_data(packet.founder_add_data())?;
        if self.outcome.is_none() {
            let mut sealed =
                SealedGovernedAddOutcome::prepare(store, self.frame(Stage::Entry)?, outcome, rng)?;
            let raw = sealed.raw.take().ok_or_else(refused)?;
            self.outcome = Some(sealed);
            self.header_intent = Some(outcome.unsigned_data().intent_commitment_data());
            self.controls[Stage::Outcome as usize] = Some(PreparedSmallControl::new(raw));
        } else if self.header_intent != Some(outcome.unsigned_data().intent_commitment_data()) {
            return Err(refused());
        }
        source.require_original_negative(store)
    }

    pub(in crate::store) fn cover_outcome(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        caller: &RequestCancellation,
    ) -> Result<(), AppError> {
        self.outcome
            .as_ref()
            .ok_or_else(refused)?
            .require_entry(self.frame(Stage::Entry)?)?;
        self.cover_through(store, source, archive, Stage::Outcome, caller)
    }

    pub(in crate::store) fn prepare_header_attempt(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        source.require_original_negative(store)?;
        if self.controls[Stage::HeaderAttempt as usize].is_none() {
            let key = Zeroizing::new(store.keys.db_key()?);
            let parent = AddSigningParent::from_outcome(self.frame(Stage::Outcome)?)?;
            let raw = parent.seal_attempt(&key, rng)?;
            self.controls[Stage::HeaderAttempt as usize] = Some(PreparedSmallControl::new(raw));
        }
        source.require_original_negative(store)
    }

    /// The key tail runs under the same original writer that covered the
    /// attempt. The caller must already hold the actual recovered key loan.
    pub(in crate::store) fn cover_header_attempt(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        caller: &RequestCancellation,
        (authorize, sign): (
            impl FnMut() -> Result<(), AppError>,
            impl FnOnce() -> Result<(), AppError>,
        ),
    ) -> Result<(), AppError> {
        self.cover_then(
            store,
            source,
            archive,
            Stage::HeaderAttempt,
            caller,
            (authorize, sign),
        )
    }

    pub(in crate::store) fn prepare_raw_header(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        outcome: &V2FounderAddOutcomeData<'_>,
        signature: &[u8; 64],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        source.require_original_negative(store)?;
        let intent = outcome.unsigned_data();
        let signer = source.data().view_data().context.spec_data().signer;
        if self.header_intent != Some(intent.intent_commitment_data())
            || !catcoms_crypto::identity::verify_with_public_bytes(
                &signer,
                intent.signing_bytes_data(),
                signature,
            )
        {
            return Err(refused());
        }
        if let Some(expected) = self.header_signature {
            if expected != *signature {
                return Err(refused());
            }
        } else {
            let key = Zeroizing::new(store.keys.db_key()?);
            let parent = AddSigningParent::from_outcome(self.frame(Stage::Outcome)?)?;
            let raw = parent.seal_raw(&key, self.frame(Stage::HeaderAttempt)?, signature, rng)?;
            self.controls[Stage::RawHeader as usize] = Some(PreparedSmallControl::new(raw));
            self.header_signature = Some(*signature);
        }
        source.require_original_negative(store)
    }

    pub(in crate::store) fn cover_raw_header(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        caller: &RequestCancellation,
    ) -> Result<(), AppError> {
        self.cover_through(store, source, archive, Stage::RawHeader, caller)
    }

    pub(in crate::store) fn prepare_governing_record(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        (outcome, header): (&V2FounderAddOutcomeData<'_>, &SignedOwnerHeader),
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        source.require_original_negative(store)?;
        let intent = outcome.unsigned_data();
        if self.header_intent != Some(intent.intent_commitment_data()) {
            return Err(refused());
        }
        governing_record::require_original_signature(
            header,
            self.header_signature.as_ref().ok_or_else(refused)?,
            intent.signing_bytes_data(),
        )?;
        if self.controls[Stage::Governing as usize].is_none() {
            let key = Zeroizing::new(store.keys.db_key()?);
            let frame = governing_record::seal_data(
                &key,
                self.frame(Stage::RawHeader)?,
                header,
                outcome.admission_data(),
                intent.image_data(),
                rng,
            )?;
            self.controls[Stage::Governing as usize] = Some(PreparedSmallControl::new(frame));
            self.governing = Some(header.id());
        }
        source.require_original_negative(store)
    }

    pub(in crate::store) fn cover_governing_record(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        caller: &RequestCancellation,
    ) -> Result<(), AppError> {
        self.cover_through(store, source, archive, Stage::Governing, caller)
    }

    pub(in crate::store) fn prepare_welcome_intent(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        outcome: &V2FounderAddOutcomeData<'_>,
        preimage: &[u8],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        source.require_original_negative(store)?;
        if self.header_intent != Some(outcome.unsigned_data().intent_commitment_data()) {
            return Err(refused());
        }
        let packet = V2JoinRequestPacketData::decode(&self.packet)?;
        outcome.require_request_data(packet.founder_add_data())?;
        let spec =
            WelcomeBindingSpecData::decode_signing_bytes_data(preimage).map_err(|_| refused())?;
        let sdk = outcome.result_data();
        welcome_intent::require_original_binding(
            &spec,
            &packet,
            source.data().view_data().context.spec_data().signer,
            self.governing.ok_or_else(refused)?,
            outcome.admission_data().admission_id(),
            [
                sdk.welcome_data(),
                sdk.sealed_routing_transfer_data().map_err(|_| refused())?,
            ],
        )?;
        let digest = blake3::hash(preimage);
        if let Some(original) = self.welcome_intent {
            if original != digest {
                return Err(refused());
            }
        } else {
            let key = Zeroizing::new(store.keys.db_key()?);
            let frame =
                welcome_intent::seal_data(&key, self.frame(Stage::Governing)?, preimage, rng)?;
            self.controls[Stage::WelcomeIntent as usize] = Some(PreparedSmallControl::new(frame));
            self.welcome_intent = Some(digest);
        }
        source.require_original_negative(store)
    }

    pub(in crate::store) fn cover_welcome_intent(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        caller: &RequestCancellation,
    ) -> Result<(), AppError> {
        self.cover_through(store, source, archive, Stage::WelcomeIntent, caller)
    }

    pub(in crate::store) fn prepare_welcome_attempt(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        source.require_original_negative(store)?;
        if self.controls[Stage::WelcomeAttempt as usize].is_none() {
            let key = Zeroizing::new(store.keys.db_key()?);
            let parent = AddSigningParent::from_welcome_intent(self.frame(Stage::WelcomeIntent)?)?;
            let frame = parent.seal_attempt(&key, rng)?;
            self.controls[Stage::WelcomeAttempt as usize] = Some(PreparedSmallControl::new(frame));
        }
        source.require_original_negative(store)
    }

    pub(in crate::store) fn cover_welcome_attempt(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        caller: &RequestCancellation,
        (authorize, sign): (
            impl FnMut() -> Result<(), AppError>,
            impl FnOnce() -> Result<(), AppError>,
        ),
    ) -> Result<(), AppError> {
        self.cover_then(
            store,
            source,
            archive,
            Stage::WelcomeAttempt,
            caller,
            (authorize, sign),
        )
    }

    pub(in crate::store) fn prepare_raw_welcome(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        preimage: &[u8],
        signature: &[u8; 64],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        source.require_original_negative(store)?;
        let signer = source.data().view_data().context.spec_data().signer;
        if self.welcome_intent != Some(blake3::hash(preimage))
            || !catcoms_crypto::identity::verify_with_public_bytes(&signer, preimage, signature)
        {
            return Err(refused());
        }
        if let Some(original) = self.welcome_signature {
            if original != *signature {
                return Err(refused());
            }
        } else {
            let key = Zeroizing::new(store.keys.db_key()?);
            let parent = AddSigningParent::from_welcome_intent(self.frame(Stage::WelcomeIntent)?)?;
            let frame =
                parent.seal_raw(&key, self.frame(Stage::WelcomeAttempt)?, signature, rng)?;
            self.controls[Stage::RawWelcome as usize] = Some(PreparedSmallControl::new(frame));
            self.welcome_signature = Some(*signature);
        }
        source.require_original_negative(store)
    }

    pub(in crate::store) fn cover_raw_welcome(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        caller: &RequestCancellation,
    ) -> Result<(), AppError> {
        self.cover_through(store, source, archive, Stage::RawWelcome, caller)
    }

    pub(in crate::store) fn prepare_response(
        &mut self,
        (store, source, archive): (
            &ServerStore,
            &RetainedBootstrapSourceData,
            &HeldColdFounderAdd,
        ),
        (preimage, outcome, header, response): (
            &[u8],
            &V2FounderAddOutcomeData<'_>,
            &SignedOwnerHeader,
            &V2JoinResponseData<'_>,
        ),
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        source.require_original_negative(store)?;
        if self.governing != Some(header.id())
            || self.header_intent != Some(outcome.unsigned_data().intent_commitment_data())
            || self.welcome_intent != Some(blake3::hash(preimage))
        {
            return Err(refused());
        }
        let sdk = outcome.result_data();
        let original = response::OriginalResponse {
            preimage,
            signature: self.welcome_signature.as_ref().ok_or_else(refused)?,
            payloads: [
                sdk.welcome_data(),
                sdk.sealed_routing_transfer_data().map_err(|_| refused())?,
            ],
        };
        if self.controls[Stage::Response as usize].is_none() {
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            let bytes =
                6 * (response::MAX_BYTES + 1) + 4 * MAX_V2_GOVERNING_JOIN_PREFIX_BYTES + 4096;
            let fee = pool.try_reserve(bytes)?;
            fee.validate(pool, bytes)?;
            original.require_packet(response)?;
            let genesis = source.data().genesis_public_data();
            let proof = archive.encode_successor_proof(
                &GenesisPublicData {
                    profile: genesis.profile_data(),
                    admission: genesis.admission_data(),
                    header: genesis.header_data(),
                    image: genesis.image_data(),
                },
                V2GoverningJoinStepData {
                    header,
                    image: outcome.unsigned_data().image_data(),
                    body: ReplayBody::Admission(outcome.admission_data()),
                },
            )?;
            let wire = Zeroizing::new(response.encode_packet_data()?);
            // Refuse a delivery that will not fit before selecting its Head.
            drop(V2GoverningJoinDeliveryData::encode(&wire, &proof)?);
            let key = Zeroizing::new(store.keys.db_key()?);
            let frame = response::seal_data(&key, self.frame(Stage::RawWelcome)?, &wire, rng)?;
            self.controls[Stage::Response as usize] = Some(PreparedSmallControl::new(frame));
            self.response_fee = Some(fee);
        } else {
            original.require_packet(response)?;
        }
        source.require_original_negative(store)
    }

    pub(in crate::store) fn cover_response(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        caller: &RequestCancellation,
    ) -> Result<(), AppError> {
        self.cover_through(store, source, archive, Stage::Response, caller)
    }

    /// Freeze the immutable archive and its successor selector without
    /// consuming the retained SDK result or touching the physical parent.
    pub(in crate::store) fn prepare_archive(
        &mut self,
        (store, source, parent): (
            &ServerStore,
            &RetainedBootstrapSourceData,
            &HeldColdFounderAdd,
        ),
        outcome: &V2FounderAddOutcomeData<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        source.require_original_negative(store)?;
        let (context, before_head, before_archive, parent_header, parent_frame) =
            parent.writer_parent_data();
        let intent = outcome.unsigned_data();
        if self.parent != (context, before_head)
            || self.parent_header != parent_header
            || self.header_intent != Some(intent.intent_commitment_data())
            || intent.header_spec_data().parent != parent_header
        {
            return Err(refused());
        }
        let labels = GovernedArchiveLabels {
            context,
            before_head,
            before_archive,
            header: self.governing.ok_or_else(refused)?,
            state: intent.header_spec_data().state,
            epoch: outcome.result_data().after_epoch_data(),
            authenticator: *outcome.result_data().after_authenticator_data(),
        };
        if let Some(prepared) = &self.archive {
            if prepared.labels != labels {
                return Err(refused());
            }
        } else {
            let frames = GovernedArchiveFrames {
                parent_head: parent_frame,
                parent_archive: &[],
                fence: self.frame(Stage::Fence)?,
                request: self.frame(Stage::Request)?,
                entry: self.frame(Stage::Entry)?,
                outcome: self.frame(Stage::Outcome)?,
                header_attempt: self.frame(Stage::HeaderAttempt)?,
                raw_header: self.frame(Stage::RawHeader)?,
                governing: self.frame(Stage::Governing)?,
                welcome_intent: self.frame(Stage::WelcomeIntent)?,
                welcome_attempt: self.frame(Stage::WelcomeAttempt)?,
                raw_welcome: self.frame(Stage::RawWelcome)?,
                response: self.frame(Stage::Response)?,
            };
            let bytes = 5 * archive::later_frame_bytes(&frames)?
                + MAX_TEXT_PATH
                + 6 * selected_head::FRAME_LEN
                + 4 * crate::store::persistence::v2_text::head::MAX_TEXT_HEAD_FRAME
                + 4 * std::mem::size_of::<PreparedColdArchive>()
                + 4096;
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            let fee = pool.try_reserve(bytes)?;
            fee.validate(pool, bytes)?;
            let key = Zeroizing::new(store.keys.db_key()?);
            let output = archive::seal_later_data(&key, labels, &frames, rng)?;
            let id = archive::later_id(&output.archive);
            let path = archive_child(source.original_target_data().0, id.as_bytes())?;
            let head = TextHeadIdData::of_data(&output.head);
            self.controls[Stage::Head as usize] = Some(PreparedSmallControl::new(output.head));
            self.archive = Some(PreparedColdArchive {
                labels,
                record: PreparedSmallControl::new(output.archive),
                path,
                head,
                selection_entered: false,
                selected: false,
                _fee: fee,
            });
        }
        source.require_original_negative(store)
    }

    /// Keep the original cold verifier and SDK until the selector's exact
    /// new inode is covered and synced. On a lost rename result only that
    /// owned successor is eligible for continuation.
    pub(in crate::store) fn select_archive(
        &mut self,
        (store, source, parent): (
            &ServerStore,
            &RetainedBootstrapSourceData,
            &mut HeldColdFounderAdd,
        ),
        (install, candidate): (
            PreparedV2RecoveredAddInstall<'_>,
            &mut Option<V2GeneralCandidate>,
        ),
        caller: &RequestCancellation,
        mut require_native: impl FnMut() -> Result<(), AppError>,
    ) -> Result<TextHeadIdData, AppError> {
        self.original.require_same_original(source)?;
        parent.with_cold_governed_selection(
            store,
            source,
            caller,
            |scope, original, verifier, require_archive| {
                require_native()?;
                install
                    .check_host_selection_data(verifier, candidate.as_ref().ok_or_else(refused)?)?;
                let (target, _, _) = scope.directory_data();
                let stage = add_stage_path(target, selected_head::NAME, Some(self.parent.1))?;
                let head_path = fixed_child(target, "head.bin")?;
                let prepared = self.archive.as_ref().ok_or_else(refused)?;
                let expected = install.expected_data();
                if expected.header != prepared.labels.header
                    || expected.state != prepared.labels.state
                    || expected.epoch != prepared.labels.epoch
                    || expected.authenticator != prepared.labels.authenticator
                {
                    return Err(refused());
                }
                let already_entered = prepared.selection_entered;
                let parent_is_named = require_exact_held(&head_path, original).is_ok();
                if parent_is_named {
                    self.cover_prefix(scope, Stage::Head)?;
                } else if !already_entered {
                    return Err(refused());
                }
                require_archive()?;
                require_native()?;
                if parent_is_named {
                    self.require_covered_prefix(scope, Stage::Head)?;
                    let prepared = self.archive.as_mut().ok_or_else(refused)?;
                    prepared.record.require_covered(&prepared.path)?;
                    select_named_head(
                        scope.directory_data(),
                        &stage,
                        &head_path,
                        original,
                        &mut prepared.selection_entered,
                        || scope.require_live_cached(self.parent),
                    )?;
                }
                require_absent(&stage)?;
                self.controls[Stage::Head as usize]
                    .as_mut()
                    .ok_or_else(refused)?
                    .require_covered(&head_path)?;
                crate::store::persistence::sync_directory(target).map_err(io)?;
                require_archive()?;
                require_native()?;
                require_absent(&stage)?;
                self.require_covered_prefix(scope, Stage::Response)?;
                let prepared = self.archive.as_mut().ok_or_else(refused)?;
                require_successor_stages_absent(target, prepared.head)?;
                prepared.record.require_covered(&prepared.path)?;
                self.controls[Stage::Head as usize]
                    .as_mut()
                    .ok_or_else(refused)?
                    .require_covered(&head_path)?;
                scope.require_live_cached(self.parent)?;
                install.finish_host_selected(verifier, candidate)?;
                prepared.selected = true;
                Ok(prepared.head)
            },
        )
    }

    pub(in crate::store) fn selected_head_data(&self) -> Option<TextHeadIdData> {
        self.archive
            .as_ref()
            .filter(|archive| archive.selected)
            .map(|archive| archive.head)
    }

    pub(in crate::store) fn has_started_selection(&self) -> bool {
        self.archive
            .as_ref()
            .is_some_and(|archive| archive.selection_entered)
    }

    fn frame(&self, stage: Stage) -> Result<&[u8], AppError> {
        self.control(stage)?.covered_frame()
    }

    fn control(&self, stage: Stage) -> Result<&PreparedSmallControl, AppError> {
        self.controls[stage as usize].as_ref().ok_or_else(refused)
    }

    fn cover_through(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        through: Stage,
        caller: &RequestCancellation,
    ) -> Result<(), AppError> {
        archive.with_cold_governed_writer(store, source, caller, |scope, require_parent| {
            self.original.require_same_original(source)?;
            self.cover_prefix(scope, through)?;
            require_parent()?;
            self.require_covered_prefix(scope, through)
        })
    }

    fn cover_then(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        archive: &mut HeldColdFounderAdd,
        through: Stage,
        caller: &RequestCancellation,
        (mut authorize, after): (
            impl FnMut() -> Result<(), AppError>,
            impl FnOnce() -> Result<(), AppError>,
        ),
    ) -> Result<(), AppError> {
        self.original.require_same_original(source)?;
        archive.with_cold_governed_writer(store, source, caller, |scope, require_parent| {
            self.cover_prefix(scope, through)?;
            require_parent()?;
            authorize()?;
            self.require_covered_prefix(scope, through)?;
            let outcome = after();
            require_parent()?;
            authorize()?;
            self.require_covered_prefix(scope, through)?;
            outcome
        })
    }

    fn cover_prefix(
        &mut self,
        scope: &ColdGovernedWriteScope<'_>,
        through: Stage,
    ) -> Result<(), AppError> {
        self.require_stage_inventory(scope, through)?;
        let directory = scope.directory_data();
        for (index, name) in ADD_STAGES[..=through as usize].iter().enumerate() {
            if index == Stage::Head as usize {
                let archive = self.archive.as_mut().ok_or_else(refused)?;
                archive
                    .record
                    .cover_named(&archive.path, archive::MAX_BYTES, directory)?;
            }
            let path = add_stage_path(directory.0, name, Some(self.parent.1))?;
            self.controls[index]
                .as_mut()
                .ok_or_else(refused)?
                .cover_named(&path, CAPS[index], directory)?;
        }
        scope.require_live_cached(self.parent)
    }

    /// No writes or Source/layout waits after the last physical proof.
    fn require_covered_prefix(
        &mut self,
        scope: &ColdGovernedWriteScope<'_>,
        through: Stage,
    ) -> Result<(), AppError> {
        self.require_stage_inventory(scope, through)?;
        let target = scope.directory_data().0;
        for (index, name) in ADD_STAGES[..=through as usize].iter().enumerate() {
            let path = add_stage_path(target, name, Some(self.parent.1))?;
            self.controls[index]
                .as_mut()
                .ok_or_else(refused)?
                .require_covered(&path)?;
        }
        scope.require_live_cached(self.parent)
    }

    fn require_stage_inventory(
        &mut self,
        scope: &ColdGovernedWriteScope<'_>,
        through: Stage,
    ) -> Result<(), AppError> {
        let target = scope.directory_data().0;
        for name in REMOVE_STAGES {
            require_absent(&governed_add_successor_child(
                target,
                name,
                self.parent.1.as_bytes(),
            )?)?;
        }
        for (index, name) in ADD_STAGES.iter().enumerate() {
            let record = self.controls[index].as_mut();
            let path = add_stage_path(target, name, Some(self.parent.1))?;
            if !record.as_ref().is_some_and(|record| record.entered_create) {
                require_absent(&path)?;
            } else if index > through as usize && index != Stage::Head as usize {
                // An exact retry can have reached a later owned stage. A
                // foreign name cannot turn its partial write into authority.
                let record = record.ok_or_else(refused)?;
                if record.covered {
                    record.require_covered(&path)?;
                } else if let Some(original) = &record.original {
                    require_named(&path, original)?;
                } else {
                    require_absent(&path)?;
                }
            }
        }
        Ok(())
    }
}
