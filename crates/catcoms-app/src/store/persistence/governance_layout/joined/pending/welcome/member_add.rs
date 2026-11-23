//! Covered continuations of one original joined Welcome.
use super::*;
use catcoms_sync::{ClosedV2JoinedNode, V2MemberTransitionFrameData, MAX_V2_GOVERNING_JOIN_STEPS};
use codec::{JoinedMembershipCandidate, RetiredJoinedMembership};

const MAX_EPOCH: u64 = MAX_V2_GOVERNING_JOIN_STEPS as u64;
const RECORD_FEE: usize = 8 * (codec::MEMBER_ADD_FRAME + 1) + 4096;

#[derive(Default)]
pub(super) struct MemberAdds {
    covered: Vec<Covered>,
    pending: Option<Pending>,
    evicted: bool,
}

struct Covered {
    path: PathBuf,
    file: OpenedArchiveObject,
    _fee: CatalogReadReservation,
}

struct Pending {
    path: PathBuf,
    sealed: Zeroizing<Vec<u8>>,
    original: Option<CreatedStage>,
    candidate: JoinedMembershipCandidate,
    fee: CatalogReadReservation,
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
    Ok(policy
        .current
        .archive
        .pending
        .pool
        .try_reserve(RECORD_FEE)?)
}

impl MemberAdds {
    fn require_original(&self) -> Result<(), AppError> {
        for covered in &self.covered {
            let (file, raw) = &covered.file;
            exact_held(&covered.path, file, session_id(file).map_err(io)?, raw)?;
        }
        Ok(())
    }

    pub(super) fn require_current(&self) -> Result<(), AppError> {
        if self.pending.is_some() || self.evicted {
            return Err(denied());
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

    pub(super) fn cover<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        context: V2PendingJoinContextData,
        welcome: &[u8],
        core: &mut PreparedV2JoinedPrefix,
        node: &mut ClosedV2JoinedNode<T, R>,
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
        if self.evicted {
            return Err(denied());
        }
        self.require_original()?;
        let (_, _, epoch) = node.selected_join_data()?;
        let path = path(store, context, epoch.checked_add(1).ok_or_else(denied)?)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        if self.pending.is_none() {
            absent(&path)?;
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
            self.pending = Some(Pending {
                path: path.clone(),
                sealed,
                original: None,
                candidate,
                fee,
            });
        }
        let predecessor = self
            .covered
            .last()
            .map_or(welcome, |entry| entry.file.1.as_slice());
        let pending = self.pending.as_mut().ok_or_else(denied)?;
        if pending.path != path || !pending.candidate.matches_frame_data(delivery) {
            let frame = V2MemberTransitionFrameData::decode(delivery)?;
            node.observe_member_transition_header(core, &frame)?;
            return Err(denied());
        }
        require_live()?;
        write_held(store, &pending.path, &pending.sealed, &mut pending.original)?;
        let opened = codec::open_member_add(&key, context, predecessor, &pending.sealed)?;
        let selected = opened.require_original_candidate(&context.vault, &pending.candidate)?;
        require_live()?;
        let retired = install_transition(core, node, &mut pending.candidate, selected)?;
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
    ) -> Result<(), AppError> {
        self.require_current()?;
        store.validate_mounted_governance_layout()?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let mut missing = false;
        let (_, _, epoch) = node.selected_join_data()?;
        for expected_epoch in epoch.checked_add(1).ok_or_else(denied)?..=MAX_EPOCH {
            let path = path(store, context, expected_epoch)?;
            if absent(&path).is_ok() {
                missing = true;
                continue;
            }
            if missing || self.evicted {
                return Err(denied());
            }
            let fee = reserve(store)?;
            let file = read(&path, codec::MEMBER_ADD_FRAME)?.ok_or_else(denied)?;
            let opened = codec::open_member_add(&key, context, self.predecessor(welcome), &file.1)?;
            self.reserve_output()?;
            let mut candidate = prepare_transition(store, core, node, &opened.delivery)?;
            let selected = opened.require_original_candidate(&context.vault, &candidate)?;
            if selected.0 != expected_epoch {
                return Err(denied());
            }
            exact_held(&path, &file.0, session_id(&file.0).map_err(io)?, &file.1)?;
            store.validate_mounted_governance_layout()?;
            let retired = install_transition(core, node, &mut candidate, selected)?;
            self.evicted = retired.terminal_data().is_some();
            self.covered.push(Covered {
                path,
                file,
                _fee: fee,
            });
            drop(retired);
        }
        self.require_current()
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
            Ok(RetiredJoinedMembership::Continuing(retired))
        }
        JoinedMembershipCandidate::Eviction(candidate) => {
            let retired =
                node.install_host_selected_eviction(core, candidate, &expected, selected)?;
            Ok(RetiredJoinedMembership::Evicted {
                terminal: (head, selected.0),
                retired,
            })
        }
    }
}
