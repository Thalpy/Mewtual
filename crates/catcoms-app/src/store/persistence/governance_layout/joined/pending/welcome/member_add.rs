//! Covered continuations of one original joined Welcome.
use super::*;
use catcoms_sync::{
    ClosedV2JoinedNode, V2MemberAddRequestData, V2MemberTransitionFrameData,
    MAX_V2_GOVERNING_JOIN_STEPS,
};
use codec::{JoinedMembershipCandidate, RetiredJoinedMembership};
mod header_witness;
use header_witness::HeaderWitness;

const MAX_EPOCH: u64 = MAX_V2_GOVERNING_JOIN_STEPS as u64;
const RECORD_FEE: usize = 8 * (codec::MEMBER_ADD_FRAME + codec::MEMBER_SELECTION_FRAME + 1) + 4096;

#[derive(Default)]
pub(super) struct MemberAdds {
    covered: Vec<Covered>,
    pending: Option<Pending>,
    evicted: bool,
    recovered_removal: Option<catcoms_sync::SelectedRejoinRemoval>,
    first: Option<HeaderWitness>,
    fork: Option<HeaderWitness>,
}

struct Covered {
    path: PathBuf,
    file: OpenedArchiveObject,
    parent: (OwnerHeaderId, u64),
    fetch_verifier: [u8; 32],
    selection: Option<SelectionRecord>,
    witness: Option<HeaderWitness>,
    _fee: CatalogReadReservation,
}

struct Pending {
    path: PathBuf,
    sealed: Zeroizing<Vec<u8>>,
    original: Option<CreatedStage>,
    candidate: JoinedMembershipCandidate,
    selection: SelectionRecord,
    fee: CatalogReadReservation,
}

struct SelectionRecord {
    path: PathBuf,
    frame: Zeroizing<Vec<u8>>,
    original: Option<CreatedStage>,
}

impl SelectionRecord {
    fn require_current(&self) -> Result<(), AppError> {
        exact_retained(
            &self.path,
            self.original.as_ref().ok_or_else(denied)?,
            &self.frame,
        )
    }
}

fn selection_path(
    store: &ServerStore,
    context: V2PendingJoinContextData,
    epoch: u64,
) -> Result<PathBuf, AppError> {
    fixed(
        store,
        false,
        &format!(
            "chat-critical-namespaces.joined-member-selected.{}.{epoch}.bin",
            hex::encode(context.original_pending)
        ),
    )
}

fn path(
    store: &ServerStore,
    context: V2PendingJoinContextData,
    epoch: u64,
) -> Result<PathBuf, AppError> {
    if !(2..=MAX_EPOCH).contains(&epoch) {
        return Err(denied());
    }
    fixed(
        store,
        false,
        &format!(
            "chat-critical-namespaces.joined-member-add.{}.{epoch}.bin",
            hex::encode(context.original_pending)
        ),
    )
}

fn reserve(store: &ServerStore) -> Result<CatalogReadReservation, AppError> {
    let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
    policy.current.archive.pending.pool.try_reserve(RECORD_FEE)
}

impl MemberAdds {
    fn require_original(&self) -> Result<(), AppError> {
        for covered in &self.covered {
            let (file, raw) = &covered.file;
            exact_held(&covered.path, file, session_id(file).map_err(io)?, raw)?;
            if let Some(selection) = &covered.selection {
                selection.require_current()?;
            }
        }
        Ok(())
    }

    /// Reprove the exact selected terminal cut while retaining the old
    /// chain. A new fork or future transition cannot silently coexist
    /// with this handoff to the rejoin writer.
    pub(super) fn require_recovered_removal(
        &self,
        store: &ServerStore,
        context: V2PendingJoinContextData,
        expected: catcoms_sync::SelectedRejoinRemoval,
    ) -> Result<(), AppError> {
        if self.recovered_removal != Some(expected)
            || !self.evicted
            || self.pending.is_some()
            || self.first.is_some()
            || self.fork.is_some()
            || self
                .covered
                .last()
                .is_none_or(|value| value.selection.is_none())
        {
            return Err(denied());
        }
        self.require_original()?;
        for entry in &self.covered {
            let epoch = entry.parent.1.checked_add(1).ok_or_else(denied)?;
            header_witness::require_covered_names(store, context, epoch, entry.witness.as_ref())?;
        }
        for epoch in expected.2.checked_add(1).ok_or_else(denied)?..=MAX_EPOCH {
            absent(&path(store, context, epoch)?)?;
            absent(&selection_path(store, context, epoch)?)?;
            header_witness::require_covered_names(store, context, epoch, None)?;
        }
        Ok(())
    }

    pub(super) fn require_current(&self) -> Result<(), AppError> {
        if self.pending.is_some() || self.evicted || self.fork.is_some() {
            return Err(denied());
        }
        if let Some(first) = &self.first {
            first.require_current()?;
        }
        self.require_original()
    }

    fn predecessor<'a>(&'a self, welcome: &'a [u8]) -> &'a [u8] {
        self.covered
            .last()
            .map_or(welcome, |covered| covered.file.1.as_slice())
    }

    fn reserve_output(&mut self) -> Result<(), AppError> {
        self.covered.try_reserve(1).map_err(|_| denied())
    }

    pub(super) fn encode_selected_successor(
        &self,
        store: &ServerStore,
        context: V2PendingJoinContextData,
        welcome: &[u8],
        request: &V2MemberAddRequestData,
    ) -> Result<Vec<u8>, AppError> {
        self.require_current()?;
        let index = self
            .covered
            .iter()
            .position(|entry| entry.parent == request.parent_data())
            .ok_or_else(denied)?;
        let current = &self.covered[index];
        if !request.matches_verifier_data(current.fetch_verifier) {
            return Err(denied());
        }
        let predecessor = index
            .checked_sub(1)
            .map_or(welcome, |prior| self.covered[prior].file.1.as_slice());
        let key = Zeroizing::new(store.keys.db_key()?);
        let data = codec::open_member_add(&key, context, predecessor, &current.file.1)?;
        self.require_current()?;
        Ok(data.delivery)
    }

    pub(super) fn cover<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore>(
        &mut self,
        (store, context): (&ServerStore, V2PendingJoinContextData),
        welcome: &[u8],
        (core, node): (&mut PreparedV2JoinedPrefix, &mut ClosedV2JoinedNode<T, R>),
        delivery: &[u8],
        caller: &catcoms_rt::RequestCancellation,
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<RetiredJoinedMembership, AppError> {
        let require_live = || {
            if caller.is_cancelled() {
                return Err(denied());
            }
            store.validate_mounted_governance_layout()
        };
        require_live()?;
        if self.evicted || self.fork.is_some() {
            return Err(denied());
        }
        self.require_original()?;
        let (_, _, epoch) = node.selected_join_data()?;
        let epoch = epoch.checked_add(1).ok_or_else(denied)?;
        let path = path(store, context, epoch)?;
        let selection_path = selection_path(store, context, epoch)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let frame = V2MemberTransitionFrameData::decode(delivery)?;
        let predecessor = self
            .covered
            .last()
            .map_or(welcome, |entry| entry.file.1.as_slice());
        header_witness::observe_live(
            header_witness::Scope {
                store,
                key: &key,
                context,
                epoch,
                predecessor,
            },
            (&mut self.first, &mut self.fork),
            (core, node),
            frame.header_data(),
            &require_live,
            rng,
        )?;
        if self.pending.is_none() {
            absent(&path)?;
            absent(&selection_path)?;
            let fee = reserve(store)?;
            self.reserve_output()?;
            let candidate = prepare_transition(store, core, node, delivery)?;
            let sealed = codec::seal_member_add(
                &key,
                context,
                self.predecessor(welcome),
                delivery,
                &candidate,
                rng,
            )?;
            let selection = SelectionRecord {
                path: selection_path.clone(),
                frame: codec::seal_member_selection(
                    &key,
                    context,
                    epoch,
                    self.predecessor(welcome),
                    &sealed,
                    rng,
                )?,
                original: None,
            };
            self.pending = Some(Pending {
                path: path.clone(),
                sealed,
                original: None,
                candidate,
                selection,
                fee,
            });
        }
        let predecessor = self
            .covered
            .last()
            .map_or(welcome, |entry| entry.file.1.as_slice());
        let pending = self.pending.as_mut().ok_or_else(denied)?;
        if pending.path != path
            || pending.selection.path != selection_path
            || !pending.candidate.matches_frame_data(delivery)
        {
            return Err(denied());
        }
        require_live()?;
        write_held(store, &pending.path, &pending.sealed, &mut pending.original)?;
        let opened = codec::open_member_add(&key, context, predecessor, &pending.sealed)?;
        let selected = opened.require_original_candidate(&context.vault, &pending.candidate)?;
        let fetch = node.member_add_request_data(core)?;
        require_live()?;
        self.first.as_ref().ok_or_else(denied)?.require_current()?;
        write_held(
            store,
            &pending.selection.path,
            &pending.selection.frame,
            &mut pending.selection.original,
        )?;
        codec::require_member_selection(
            &key,
            context,
            epoch,
            predecessor,
            &pending.sealed,
            &pending.selection.frame,
        )?;
        exact_retained(
            &pending.path,
            pending.original.as_ref().ok_or_else(denied)?,
            &pending.sealed,
        )?;
        pending.selection.require_current()?;
        require_live()?;
        let retired = install_transition(core, node, &mut pending.candidate, selected)?;
        let witness = self.first.take();
        self.evicted = retired.terminal_data().is_some();
        let pending = self
            .pending
            .take()
            .expect("covered exact membership transition");
        self.covered.push(Covered {
            path: pending.path,
            file: (
                pending.original.expect("held covered transition").file,
                pending.sealed,
            ),
            parent: fetch.parent_data(),
            fetch_verifier: fetch.verifier_data(),
            selection: Some(pending.selection),
            witness,
            _fee: pending.fee,
        });
        Ok(retired)
    }

    pub(super) fn recover<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        context: V2PendingJoinContextData,
        welcome: &[u8],
        core: &mut PreparedV2JoinedPrefix,
        node: &mut ClosedV2JoinedNode<T, R>,
    ) -> Result<Option<catcoms_sync::SelectedRejoinRemoval>, AppError> {
        self.require_current()?;
        store.validate_mounted_governance_layout()?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let mut missing = false;
        let mut removed = None;
        let (_, own, epoch) = node.selected_join_data()?;
        for expected_epoch in epoch.checked_add(1).ok_or_else(denied)?..=MAX_EPOCH {
            let path = path(store, context, expected_epoch)?;
            let selection_path = selection_path(store, context, expected_epoch)?;
            let predecessor = self
                .covered
                .last()
                .map_or(welcome, |entry| entry.file.1.as_slice());
            let first = header_witness::recover(
                store,
                &key,
                context,
                (expected_epoch, predecessor),
                (core, node),
                missing || self.evicted,
            )?;
            if absent(&path).is_ok() {
                absent(&selection_path)?;
                if first.is_some() {
                    self.first = first;
                }
                missing = true;
                continue;
            }
            if missing || self.evicted {
                return Err(denied());
            }
            let fee = reserve(store)?;
            let file = read(&path, codec::MEMBER_ADD_FRAME)?.ok_or_else(denied)?;
            let selection = if let Some((original, frame)) =
                read(&selection_path, codec::MEMBER_SELECTION_FRAME)?
            {
                codec::require_member_selection(
                    &key,
                    context,
                    expected_epoch,
                    predecessor,
                    &file.1,
                    &frame,
                )?;
                let identity = session_id(&original).map_err(io)?;
                Some(SelectionRecord {
                    path: selection_path,
                    frame,
                    original: Some(CreatedStage {
                        file: original,
                        identity: Some(identity),
                    }),
                })
            } else {
                None
            };
            let opened = codec::open_member_add(&key, context, self.predecessor(welcome), &file.1)?;
            self.reserve_output()?;
            let mut candidate = prepare_transition(store, core, node, &opened.delivery)?;
            let selected = opened.require_original_candidate(&context.vault, &candidate)?;
            if selected.0 != expected_epoch {
                return Err(denied());
            }
            exact_held(&path, &file.0, session_id(&file.0).map_err(io)?, &file.1)?;
            if let Some(first) = &first {
                first.require_current()?;
            }
            if let Some(selection) = &selection {
                selection.require_current()?;
            }
            let fetch = node.member_add_request_data(core)?;
            store.validate_mounted_governance_layout()?;
            let retired = install_transition(core, node, &mut candidate, selected)?;
            removed = retired.terminal_data();
            self.evicted = removed.is_some();
            // A merely present outcome can be historical recovery material,
            // but cannot prove a selected terminal Admission for rejoin.
            if self.evicted && selection.is_none() {
                return Err(denied());
            }
            self.covered.push(Covered {
                path,
                file,
                parent: fetch.parent_data(),
                fetch_verifier: fetch.verifier_data(),
                selection,
                witness: first,
                _fee: fee,
            });
            drop(retired);
        }
        if let Some(terminal) = removed {
            if self.pending.is_some() || self.first.is_some() || self.fork.is_some() {
                return Err(denied());
            }
            self.require_original()?;
            store.validate_mounted_governance_layout()?;
            self.require_original()?;
            let original = (terminal.0, own, terminal.1);
            self.recovered_removal = Some(original);
            self.require_recovered_removal(store, context, original)?;
            return Ok(Some(original));
        }
        self.require_current()?;
        Ok(None)
    }
}

fn prepare_transition<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore>(
    store: &ServerStore,
    core: &mut PreparedV2JoinedPrefix,
    node: &ClosedV2JoinedNode<T, R>,
    delivery: &[u8],
) -> Result<JoinedMembershipCandidate, AppError> {
    let frame = V2MemberTransitionFrameData::decode(delivery)?;
    node.observe_member_transition_header(core, &frame)?;
    let payment = crate::store::persistence::v2_text::prepare_sdk_admission(store)?;
    if frame.removal_admission_data()? == Some(core.own_admission_data()) {
        let candidate = node.prepare_member_eviction_frame(core, &frame, payment)?;
        Ok(JoinedMembershipCandidate::Eviction(Some(candidate)))
    } else {
        let candidate = node.prepare_member_transition_frame(core, &frame, payment)?;
        Ok(JoinedMembershipCandidate::Continuing(Some(candidate)))
    }
}

fn install_transition<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore>(
    core: &mut PreparedV2JoinedPrefix,
    node: &mut ClosedV2JoinedNode<T, R>,
    candidate: &mut JoinedMembershipCandidate,
    selected: (u64, [u8; 32]),
) -> Result<RetiredJoinedMembership, AppError> {
    let header = candidate.header_data();
    let head = header.id();
    let expected = V2HostSelectionExpectation::from_trusted_host(head, header.spec().state);
    match candidate {
        JoinedMembershipCandidate::Continuing(candidate) => {
            let retired =
                node.install_host_selected_membership(core, candidate, &expected, selected)?;
            Ok(RetiredJoinedMembership::Continuing { _retired: retired })
        }
        JoinedMembershipCandidate::Eviction(candidate) => {
            let retired =
                node.install_host_selected_eviction(core, candidate, &expected, selected)?;
            Ok(RetiredJoinedMembership::Evicted {
                terminal: (head, selected.0),
                _retired: retired,
            })
        }
    }
}
