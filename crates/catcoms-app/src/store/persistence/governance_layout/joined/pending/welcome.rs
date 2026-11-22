//! Covers possible MLS entry on the original warm owner, and keeps successful
//! recovery historical until the native source can install it.
use super::*;
use catcoms_mls::persist::{
    text_current_add::TextCurrentAddLimits, TextCurrentSdkGraphReservation,
    TextCurrentSdkRestoreAdmission, TextCurrentSdkRestoreLimits,
};
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_replication::chat::{
    cold_selected_history::{encode_selected_history_node_page, SelectedHistoryNodePage},
    owner_state::transition::REPLAY_MATH_PRIVATE_INVOICE,
    ProjectionGroup, ProjectionGroupView, V2GeneralPrefixVerifier, V2GeneralRetiredData,
    V2HostSelectionExpectation, V2OriginNegative, V2ProjectionOrigin,
};
use catcoms_sync::{
    PreparedV2JoinedPrefix, RecoveredV2JoinedOutcome, V2FirstJoinDeliveryData,
    V2PendingJoinContextData, MAX_V2_FIRST_ADD_PROOF_BYTES,
};
use critical_namespaces::joined_pin::pending::SelectedJoinData;
use critical_namespaces::joined_pin::welcome as codec;

pub(super) const ENTRY: &str = "chat-critical-namespaces.joined-welcome-entry.bin";
pub(super) const OUTCOME: &str = "chat-critical-namespaces.joined-welcome-outcome.bin";
type RegistrationFiles<'a> = ((&'a File, &'a [u8]), (&'a File, &'a [u8]));
const FRAME_FEE: usize = 12 * (codec::ENTRY_FRAME + codec::OUTCOME_FRAME) + 4 * 1024 * 1024;

fn paid_frames(store: &ServerStore) -> Result<CatalogReadReservation, AppError> {
    let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
    policy.current.archive.pending.pool.try_reserve(FRAME_FEE)
}

pub(super) fn require_absent(store: &ServerStore) -> Result<(), AppError> {
    absent(&fixed(store, false, ENTRY)?)?;
    absent(&fixed(store, false, OUTCOME)?)
}

pub(super) struct WarmWelcome {
    pub(super) delivery: Zeroizing<Vec<u8>>,
    entry: Zeroizing<Vec<u8>>,
    outcome: Option<Zeroizing<Vec<u8>>>,
    held_entry: Option<CreatedStage>,
    held_outcome: Option<CreatedStage>,
    selection: Option<Box<JoinedCore>>,
    _fee: CatalogReadReservation,
}

impl WarmWelcome {
    pub(super) fn member_prefix(
        &self,
    ) -> Result<(&PreparedV2JoinedPrefix, &ProjectionGroupView), AppError> {
        let selected = self
            .selection
            .as_ref()
            .filter(|selected| selected.retired.is_some())
            .ok_or_else(denied)?;
        Ok((&selected.core, &selected._view))
    }
    pub(super) fn registration_files(&self) -> Result<RegistrationFiles<'_>, AppError> {
        let entry = self.held_entry.as_ref().ok_or_else(denied)?;
        let outcome = self.held_outcome.as_ref().ok_or_else(denied)?;
        Ok((
            (&entry.file, &self.entry),
            (&outcome.file, self.outcome.as_ref().ok_or_else(denied)?),
        ))
    }
    pub(super) fn prepare(
        store: &ServerStore,
        required: &PreparedPinnedPending,
        key: &[u8; 32],
        pending_frame: &[u8],
        delivery: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, AppError> {
        require_absent(store)?;
        let fee = paid_frames(store)?;
        required.validate_delivery_data(delivery)?;
        let entry = codec::seal_entry(key, required.context_data(), pending_frame, delivery, rng)?;
        Ok(Self {
            delivery: Zeroizing::new(delivery.to_vec()),
            entry,
            outcome: None,
            held_entry: None,
            held_outcome: None,
            selection: None,
            _fee: fee,
        })
    }

    pub(super) fn cover_entry(&mut self, store: &ServerStore) -> Result<(), AppError> {
        if self.held_outcome.is_none() {
            absent(&fixed(store, false, OUTCOME)?)?;
        }
        let path = fixed(store, false, ENTRY)?;
        write_held(store, &path, &self.entry, &mut self.held_entry)
    }

    pub(super) fn require_entry(&self, store: &ServerStore) -> Result<(), AppError> {
        exact_retained(
            &fixed(store, false, ENTRY)?,
            self.held_entry.as_ref().ok_or_else(denied)?,
            &self.entry,
        )
    }

    pub(super) fn prepare_outcome(
        &mut self,
        store: &ServerStore,
        required: &mut PreparedPinnedPending,
        key: &[u8; 32],
        rng: &mut impl CryptoRngCore,
        negative: Option<&catcoms_rt::RequestCancellation>,
        keys: Option<NativeV2JoinKeyEntry<'_>>,
    ) -> Result<(), AppError> {
        self.require_entry(store)?;
        if self.outcome.is_none() {
            let context = required.context_data();
            required.finish_delivery(&self.delivery, negative, keys)?;
            if negative.is_some_and(catcoms_rt::RequestCancellation::is_cancelled) {
                return Err(denied());
            }
            if self.selection.is_none() {
                // Sync prepares the provisional prefix only beside its own
                // sealed SDK graph. The outer record covers both originals.
                required.seal_outcome(key, rng)?;
                let group = required.group_data()?;
                self.selection = Some(Box::new(JoinedCore::prepare(
                    store,
                    &group,
                    &self.delivery,
                    |view, origin| required.prepare_initial_prefix(&self.delivery, view, origin),
                )?));
            }
            let append = self
                .selection
                .as_ref()
                .and_then(|selection| selection.core.pending_history_data())
                .ok_or_else(denied)?;
            let history =
                encode_selected_history_node_page(&context.vault, append).map_err(|_| denied())?;
            let result = required.seal_outcome(key, rng)?;
            self.outcome = Some(codec::seal_outcome(
                key,
                context,
                &self.entry,
                result,
                &history,
                rng,
            )?);
        }
        Ok(())
    }

    pub(super) fn cover_outcome(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let raw = self.outcome.as_ref().ok_or_else(denied)?;
        let path = fixed(store, false, OUTCOME)?;
        self.require_entry(store)?;
        write_held(store, &path, raw, &mut self.held_outcome)?;
        self.require_entry(store)
    }

    pub(super) fn require_outcome(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_entry(store)?;
        exact_retained(
            &fixed(store, false, OUTCOME)?,
            self.held_outcome.as_ref().ok_or_else(denied)?,
            self.outcome.as_ref().ok_or_else(denied)?,
        )
    }

    pub(super) fn finish_core(&mut self) -> Result<(), AppError> {
        self.selection.as_mut().ok_or_else(denied)?.finish()
    }

    pub(super) fn outcome_data(
        &self,
    ) -> Option<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64)> {
        self.selection.as_deref().and_then(JoinedCore::outcome_data)
    }

    pub(super) fn selected_data(&self) -> Option<SelectedJoinData> {
        self.selection
            .as_deref()
            .and_then(JoinedCore::selected_data)
    }

    pub(super) fn copy_selected_governing_proof(
        &self,
        headers: (OwnerHeaderId, OwnerHeaderId),
        output: &mut Vec<u8>,
    ) -> Result<(), AppError> {
        self.selection
            .as_ref()
            .ok_or_else(denied)?
            .copy_selected_governing_proof(headers, output)
    }

    pub(super) fn attach_node<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore>(
        &self,
        pending: &mut PreparedPinnedPending,
        inputs: &mut Option<JoinedRuntimeInputs<T, R>>,
    ) -> Result<catcoms_sync::ClosedV2JoinedNode<T, R>, AppError> {
        let selection = self
            .selection
            .as_ref()
            .filter(|selected| selected.retired.is_some())
            .ok_or_else(denied)?;
        pending.take_closed_warm_node(&selection.core, inputs)
    }
}

fn write_held(
    store: &ServerStore,
    path: &Path,
    bytes: &[u8],
    receiver: &mut Option<CreatedStage>,
) -> Result<(), AppError> {
    ensure_native_claim(path, receiver)?;
    sync_directory(&store.dir).map_err(io)?;
    ensure(
        path,
        bytes,
        receiver,
        ArchiveOutputMode::Original,
        #[cfg(test)]
        0,
        #[cfg(test)]
        0,
    )?;
    sync_directory(&store.dir).map_err(io)?;
    exact_retained(path, receiver.as_ref().ok_or_else(denied)?, bytes)
}

pub(super) struct RecoveredWelcome {
    entry: OpenedArchiveObject,
    // The paid graph and Core models stay out of catalog and Store frames.
    outcome: Option<Box<ClosedOutcome>>,
    _fee: CatalogReadReservation,
}

struct ClosedOutcome {
    file: OpenedArchiveObject,
    graph: RecoveredV2JoinedOutcome,
    selection: JoinedCore,
}

struct JoinedCore {
    core: PreparedV2JoinedPrefix,
    expected: V2HostSelectionExpectation,
    retired: Option<V2GeneralRetiredData>,
    public_headers: (OwnerHeaderId, OwnerHeaderId),
    public_proof: Zeroizing<Vec<u8>>,
    _view: ProjectionGroupView,
    _group: ProjectionGroup,
    _fee: CatalogReadReservation,
}

struct ProjectionMount(crate::store::chat_admission::CachedPhysicalMountNegative);

impl std::fmt::Debug for ProjectionMount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionMount").finish_non_exhaustive()
    }
}
impl V2OriginNegative for ProjectionMount {
    fn is_live(&self) -> bool {
        self.0.is_live_cached()
    }
}

impl JoinedCore {
    fn prepare(
        store: &ServerStore,
        group: &[u8],
        delivery: &[u8],
        prepare: impl FnOnce(
            &ProjectionGroupView,
            V2ProjectionOrigin,
        )
            -> Result<(PreparedV2JoinedPrefix, V2HostSelectionExpectation), AppError>,
    ) -> Result<Self, AppError> {
        const CORE_FEE: usize = 2 * REPLAY_MATH_PRIVATE_INVOICE
            + V2GeneralPrefixVerifier::metadata_layout_bytes()
            + ProjectionGroup::initial_metadata_layout_bytes()
            + 4 * std::mem::size_of::<JoinedCore>()
            + 2 * MAX_V2_FIRST_ADD_PROOF_BYTES;
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let fee = policy.current.archive.pending.pool.try_reserve(CORE_FEE)?;
        let mut group = ProjectionGroup::new(group).map_err(|_| denied())?;
        let mut view = group.begin_view();
        if !view.try_ready() {
            return Err(denied());
        }
        let origin = V2ProjectionOrigin::new(Arc::new(ProjectionMount(
            store.chat_mount.capture_cached_physical_negative(),
        )));
        let decoded = V2FirstJoinDeliveryData::decode(delivery).map_err(|_| denied())?;
        let (proof, bytes) = decoded.governing_proof_data();
        let public_headers = proof.header_ids_data();
        let public_proof = Zeroizing::new(bytes.to_vec());
        drop(decoded);
        let (core, expected) = prepare(&view, origin)?;
        Ok(Self {
            core,
            expected,
            retired: None,
            public_headers,
            public_proof,
            _view: view,
            _group: group,
            _fee: fee,
        })
    }

    fn finish(&mut self) -> Result<(), AppError> {
        if self.retired.is_none() {
            self.retired = Some(
                self.core
                    .finish_host_selected(&self.expected)
                    .map_err(|_| denied())?,
            );
        }
        Ok(())
    }

    fn outcome_data(
        &self,
    ) -> Option<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64)> {
        let selected = self.selected_data()?;
        Some((selected.governing_add, selected.epoch))
    }

    fn selected_data(&self) -> Option<SelectedJoinData> {
        let replay = self.core.selected_replay_data()?;
        Some(self.selection_data(
            replay.header().id(),
            *replay.header().spec().state.as_bytes(),
            replay.mls_epoch_data(),
        ))
    }

    fn selection_data(
        &self,
        governing_add: OwnerHeaderId,
        state: [u8; 32],
        epoch: u64,
    ) -> SelectedJoinData {
        let profile = self.core.initial_profile_data();
        SelectedJoinData {
            governing_add,
            own_admission: self.core.own_admission_data(),
            state,
            epoch,
            profile: profile.id(),
            initial_tenure: profile.tenure(),
            profile_creation: profile.spec().creation,
        }
    }

    fn copy_selected_governing_proof(
        &self,
        headers: (OwnerHeaderId, OwnerHeaderId),
        output: &mut Vec<u8>,
    ) -> Result<(), AppError> {
        let original = self.retired.as_ref().ok_or_else(denied)?;
        let selected = self.core.selected_replay_data().ok_or_else(denied)?;
        if self.public_headers != headers
            || original.parent_data().header().id() != headers.0
            || selected.header().id() != headers.1
            || !output.is_empty()
            || output.capacity() < self.public_proof.len()
        {
            return Err(denied());
        }
        output.extend_from_slice(&self.public_proof);
        Ok(())
    }
}

impl ClosedOutcome {
    fn prepare(
        store: &ServerStore,
        file: OpenedArchiveObject,
        mut graph: RecoveredV2JoinedOutcome,
        delivery: &[u8],
        history: Option<(&[u8; 32], &[u8])>,
    ) -> Result<Self, AppError> {
        let group = graph.group_data().to_vec();
        let expected = V2HostSelectionExpectation::from_trusted_host(
            graph.governing_head_data(),
            graph.governing_state_data(),
        );
        let selection = JoinedCore::prepare(store, &group, delivery, |view, origin| {
            Ok((
                graph
                    .prepare_initial_prefix(delivery, view, origin)
                    .map_err(|_| denied())?,
                expected,
            ))
        })?;
        if let Some((vault, page)) = history {
            let append = selection.core.pending_history_data().ok_or_else(denied)?;
            SelectedHistoryNodePage::decode(page)
                .and_then(|page| page.verify_exact_append_data(vault, append))
                .map_err(|_| denied())?;
        }
        Ok(Self {
            file,
            graph,
            selection,
        })
    }
}

impl RecoveredWelcome {
    /// Checked by the cold graph restore and the independently derived Core
    /// candidate before startup selects it. This grants no live source.
    pub(super) fn registration_candidate_data(&self) -> Option<SelectedJoinData> {
        let original = self.outcome.as_ref()?;
        Some(original.selection.selection_data(
            original.graph.governing_head_data(),
            *original.graph.governing_state_data().as_bytes(),
            original.graph.epoch_data(),
        ))
    }

    pub(super) fn registration_files(&self) -> Result<RegistrationFiles<'_>, AppError> {
        let outcome = self.outcome.as_ref().ok_or_else(denied)?;
        Ok((
            (&self.entry.0, &self.entry.1),
            (&outcome.file.0, &outcome.file.1),
        ))
    }
    pub(super) fn copy_selected_governing_proof(
        &self,
        headers: (OwnerHeaderId, OwnerHeaderId),
        output: &mut Vec<u8>,
    ) -> Result<(), AppError> {
        self.outcome
            .as_ref()
            .ok_or_else(denied)?
            .selection
            .copy_selected_governing_proof(headers, output)
    }

    pub(super) fn attach_node<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore>(
        &mut self,
        inputs: &mut Option<JoinedRuntimeInputs<T, R>>,
    ) -> Result<catcoms_sync::ClosedV2JoinedNode<T, R>, AppError> {
        let outcome = self
            .outcome
            .as_mut()
            .filter(|outcome| outcome.selection.retired.is_some())
            .ok_or_else(denied)?;
        outcome
            .graph
            .take_closed_recovered_node(&outcome.selection.core, inputs)
            .map_err(|_| denied())
    }

    pub(super) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        let (file, bytes) = &self.entry;
        exact_held(
            &fixed(store, false, ENTRY)?,
            file,
            session_id(file).map_err(io)?,
            bytes,
        )?;
        let path = fixed(store, false, OUTCOME)?;
        if let Some(outcome) = &self.outcome {
            let (file, bytes) = &outcome.file;
            exact_held(&path, file, session_id(file).map_err(io)?, bytes)
        } else {
            absent(&path)
        }
    }

    pub(super) fn finish_core(&mut self) -> Result<(), AppError> {
        if let Some(outcome) = self.outcome.as_mut() {
            outcome.selection.finish()?;
        }
        Ok(())
    }

    pub(super) fn outcome_data(
        &self,
    ) -> Option<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64)> {
        self.outcome
            .as_ref()
            .and_then(|outcome| outcome.selection.outcome_data())
    }

    pub(super) fn selected_data(&self) -> Option<SelectedJoinData> {
        self.outcome
            .as_ref()
            .and_then(|outcome| outcome.selection.selected_data())
    }
}

struct RestorePayment {
    limits: TextCurrentSdkRestoreLimits,
    _fee: CatalogReadReservation,
}

impl TextCurrentSdkGraphReservation for RestorePayment {
    fn limits(&self) -> TextCurrentSdkRestoreLimits {
        self.limits
    }
}

pub(super) fn recover(
    store: &ServerStore,
    key: &[u8; 32],
    pending_frame: &[u8],
    pending: &RecoveredPinnedPending,
) -> Result<Option<RecoveredWelcome>, AppError> {
    let frame_fee = paid_frames(store)?;
    let entry_path = fixed(store, false, ENTRY)?;
    let outcome_path = fixed(store, false, OUTCOME)?;
    let Some(entry) = read(&entry_path, codec::ENTRY_FRAME)? else {
        absent(&outcome_path)?;
        return Ok(None);
    };
    let (request, context) = pending.pending_data();
    let delivery = codec::open_entry(key, context, pending_frame, &entry.1)?;
    request
        .validate_first_join_delivery_data(&delivery)
        .map_err(|_| denied())?;
    let outcome = if let Some(outcome) = read(&outcome_path, codec::OUTCOME_FRAME)? {
        let opened = codec::open_outcome(key, context, &entry.1, &outcome.1)?;
        let restore = TextCurrentAddLimits::governed_text_v2();
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(restore.restore_typed_bytes)?;
        let admission = TextCurrentSdkRestoreAdmission::new(RestorePayment {
            limits: TextCurrentSdkRestoreLimits {
                max_typed_bytes: restore.restore_typed_bytes,
                max_work: restore.restore_work,
            },
            _fee: fee,
        })?;
        let recovered =
            RecoveredV2JoinedOutcome::open(&opened.sealed, key, request, &delivery, admission)
                .map_err(|_| denied())?;
        Some(Box::new(ClosedOutcome::prepare(
            store,
            outcome,
            recovered,
            &delivery,
            opened
                .history
                .as_ref()
                .map(|history| (&context.vault, history.as_slice())),
        )?))
    } else {
        None
    };
    let value = RecoveredWelcome {
        entry,
        outcome,
        _fee: frame_fee,
    };
    value.require(store)?;
    Ok(Some(value))
}
