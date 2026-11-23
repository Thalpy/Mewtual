//! Covered continuations of one original joined Welcome.
use super::*;
use catcoms_sync::{
    ClosedV2JoinedNode, PreparedV2JoinedMembership, RetiredV2JoinedAdd,
    V2MemberTransitionFrameData, MAX_V2_GOVERNING_JOIN_STEPS,
};

const MAX_EPOCH: u64 = MAX_V2_GOVERNING_JOIN_STEPS as u64;
const RECORD_FEE: usize = 8 * (codec::MEMBER_ADD_FRAME + 1) + 4096;

#[derive(Default)]
pub(super) struct MemberAdds {
    covered: Vec<Covered>,
    pending: Option<Pending>,
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
    candidate: Option<PreparedV2JoinedMembership>,
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
        if self.pending.is_some() {
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
    ) -> Result<RetiredV2JoinedAdd, AppError> {
        let require_live = || {
            if caller.is_cancelled() {
                return Err(denied());
            }
            store.validate_mounted_governance_layout()
        };
        require_live()?;
        self.require_original()?;
        let (_, _, epoch) = node.selected_join_data()?;
        let path = path(store, context, epoch.checked_add(1).ok_or_else(denied)?)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        if self.pending.is_none() {
            absent(&path)?;
            let fee = reserve(store)?;
            self.reserve_output()?;
            let payment = crate::store::persistence::v2_text::prepare_sdk_admission(store)?;
            let frame = V2MemberTransitionFrameData::decode(delivery)?;
            let candidate = node.prepare_member_transition_frame(core, &frame, payment)?;
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
                candidate: Some(candidate),
                fee,
            });
        }
        let predecessor = self
            .covered
            .last()
            .map_or(welcome, |entry| entry.file.1.as_slice());
        let pending = self.pending.as_mut().ok_or_else(denied)?;
        if pending.path != path
            || !pending
                .candidate
                .as_ref()
                .ok_or_else(denied)?
                .matches_frame_data(delivery)
        {
            let frame = V2MemberTransitionFrameData::decode(delivery)?;
            node.observe_member_transition_header(core, &frame)?;
            return Err(denied());
        }
        require_live()?;
        write_held(store, &pending.path, &pending.sealed, &mut pending.original)?;
        let candidate = pending.candidate.as_ref().ok_or_else(denied)?;
        let opened = codec::open_member_add(&key, context, predecessor, &pending.sealed)?;
        let selected = opened.require_original_candidate(&context.vault, candidate)?;
        require_live()?;
        let expected = V2HostSelectionExpectation::from_trusted_host(
            candidate.header_data().id(),
            candidate.header_data().spec().state,
        );
        let retired = node.install_host_selected_membership(
            core,
            &mut pending.candidate,
            &expected,
            selected,
        )?;
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
            if missing {
                return Err(denied());
            }
            let fee = reserve(store)?;
            let file = read(&path, codec::MEMBER_ADD_FRAME)?.ok_or_else(denied)?;
            let opened = codec::open_member_add(&key, context, self.predecessor(welcome), &file.1)?;
            let frame = V2MemberTransitionFrameData::decode(&opened.delivery)?;
            self.reserve_output()?;
            let payment = crate::store::persistence::v2_text::prepare_sdk_admission(store)?;
            let mut candidate = Some(node.prepare_member_transition_frame(core, &frame, payment)?);
            let facts = candidate.as_ref().ok_or_else(denied)?;
            let selected = opened.require_original_candidate(&context.vault, facts)?;
            if selected.0 != expected_epoch {
                return Err(denied());
            }
            let expected = V2HostSelectionExpectation::from_trusted_host(
                facts.header_data().id(),
                facts.header_data().spec().state,
            );
            exact_held(&path, &file.0, session_id(&file.0).map_err(io)?, &file.1)?;
            store.validate_mounted_governance_layout()?;
            let retired =
                node.install_host_selected_membership(core, &mut candidate, &expected, selected)?;
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
