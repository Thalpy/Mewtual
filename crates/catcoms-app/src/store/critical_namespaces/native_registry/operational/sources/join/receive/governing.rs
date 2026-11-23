//! Fetch and apply the exact next governing Add on an existing joined row.
use super::*;
use catcoms_net::{NativeChatMemberDependencyExchange, NativeChatMemberDependencyObservation};
use catcoms_rt::{
    TransportError, CHAT_V2_GOVERNING_REQUEST_MAX_BYTES, CHAT_V2_GOVERNING_RESPONSE_MAX_BYTES,
};

const FRAME_FEE: usize =
    4 * (CHAT_V2_GOVERNING_REQUEST_MAX_BYTES + CHAT_V2_GOVERNING_RESPONSE_MAX_BYTES) + 4096;

pub struct NativeTextJoinedGoverningExchange {
    exchange: NativeChatMemberDependencyExchange,
    proof: catcoms_sync::V2MemberAddRequestData,
    selection: NativeTextJoinedRowSelection,
    signal: RequestCancellation,
    invocation: watch::Sender<bool>,
    end: retirement::PreparationEnd,
}

pub struct NativeTextJoinedGoverningResponse {
    frame: bytes::Bytes,
    parent: (OwnerHeaderId, u64),
    selection: NativeTextJoinedRowSelection,
    original: NativeChatMemberDependencyObservation,
    signal: RequestCancellation,
    _invocation: watch::Sender<bool>,
    _end: retirement::PreparationEnd,
}

impl NativeTextJoinedGoverningExchange {
    pub async fn receive(self) -> Result<NativeTextJoinedGoverningResponse, TransportError> {
        let original = self.exchange.observe_route();
        let bytes = self
            .proof
            .encode()
            .map_err(|_| TransportError::NoResponse)?;
        let frame = self
            .exchange
            .request_governing(&bytes, self.signal.clone())
            .await?;
        if self.signal.is_cancelled() || !original.is_current_cached() {
            return Err(TransportError::Cancelled);
        }
        Ok(NativeTextJoinedGoverningResponse {
            frame,
            parent: self.proof.parent_data(),
            selection: self.selection,
            original,
            signal: self.signal,
            _invocation: self.invocation,
            _end: self.end,
        })
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store::critical_namespaces::native_registry) fn prepare_direct_contact_for_test(
        self,
        selection: &NativeTextJoinedRowSelection,
        peer: PeerId,
        address: String,
        caller: RequestCancellation,
    ) -> Result<super::super::reconnect::NativeTextJoinedReconnect, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let (group, observed, authentication, mount, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                let verifier = source
                    .prefix
                    .selected_member_verifier()
                    .map_err(|_| Refusal::Unavailable)?;
                let group = verifier.replay_data().profile().spec().group.clone();
                let observed = source
                    .node
                    .observe_member_publication(source.prefix)
                    .map_err(|_| Refusal::Unavailable)?;
                let authentication = source
                    .metadata
                    .authentication_negative()
                    .map_err(|_| Refusal::Unavailable)?;
                let (mount, fee) = source
                    .metadata
                    .joined_transport_resources(16 * 1024)
                    .map_err(|_| Refusal::Busy)?;
                Ok((group, observed, authentication, mount, fee))
            })?;
        let (invocation, live) = watch::channel(false);
        let authority = Arc::new(JoinEffect {
            scope: self.original.scope,
            ui: self.original.command.ui.into_effect_negative(),
            authentication,
            invocation: live,
            caller: caller.clone(),
            row: Some(selection.clone()),
        });
        let signal = observed.constrain(caller.with_authority(authority));
        let transport = self
            .original
            .tcp
            .upgrade()
            .ok_or(Refusal::Unavailable)?
            .capture_attached_dial()
            .map_err(|_| Refusal::Unavailable)?;
        Ok(
            super::super::reconnect::NativeTextJoinedReconnect::for_registered_peer_test(
                invocation,
                transport,
                ApprovedContact::for_registered_peer_test(peer, group, address),
                signal,
                mount,
                fee,
                self.original.end.ok_or(Refusal::Unavailable)?,
            ),
        )
    }

    pub fn prepare_governing_exchange(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: RequestCancellation,
    ) -> Result<NativeTextJoinedGoverningExchange, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let tcp = self.original.tcp.upgrade().ok_or(Refusal::Unavailable)?;
        let lane = tcp
            .capture_member_dependencies()
            .map_err(|_| Refusal::Unavailable)?;
        let (exchange, proof, observed, authentication, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                let peer = source.founder.ok_or(Refusal::Unavailable)?;
                let exchange = lane.capture_request(peer).ok_or(Refusal::Unavailable)?;
                let fee = source
                    .metadata
                    .reserve_join_node(FRAME_FEE)
                    .map_err(|_| Refusal::Busy)?;
                let proof = source
                    .node
                    .member_add_request_data(source.prefix)
                    .map_err(|_| Refusal::Unavailable)?;
                let observed = source
                    .node
                    .observe_member_publication(source.prefix)
                    .map_err(|_| Refusal::Unavailable)?;
                let authentication = source
                    .metadata
                    .authentication_negative()
                    .map_err(|_| Refusal::Unavailable)?;
                Ok((exchange, proof, observed, authentication, fee))
            })?;
        let (invocation, live) = watch::channel(false);
        let authority = Arc::new(JoinEffect {
            scope: self.original.scope,
            ui: self.original.command.ui.into_effect_negative(),
            authentication,
            invocation: live,
            caller: caller.clone(),
            row: Some(selection.clone()),
        });
        let signal = observed
            .constrain(caller.with_authority(authority))
            .retain_custody(Arc::new(JoinFramePayment { _fee: fee }));
        if signal.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        Ok(NativeTextJoinedGoverningExchange {
            exchange,
            proof,
            selection: selection.clone(),
            signal,
            invocation,
            end: self.original.end.ok_or(Refusal::Unavailable)?,
        })
    }

    pub fn apply_governing_response(
        &self,
        selection: &NativeTextJoinedRowSelection,
        response: &NativeTextJoinedGoverningResponse,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(OwnerHeaderId, u64), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let require_live = || {
            self.original
                .require_current(caller)
                .map_err(|_| Refusal::Cancelled)?;
            if !selection.same_original(&response.selection) || !selection.is_current_cached() {
                return Err(Refusal::Stale);
            }
            if response.signal.is_cancelled() || !response.original.is_current_cached() {
                return Err(Refusal::Cancelled);
            }
            Ok(())
        };
        require_live()?;
        let mut retired = None;
        let selected = {
            let pending = self
                .original
                .pending
                .upgrade()
                .ok_or(Refusal::Unavailable)?;
            let mut pending = pending.state.try_lock().map_err(|_| Refusal::Busy)?;
            let mounted = self.mounted.upgrade().ok_or(Refusal::Unavailable)?;
            let mut slot = mounted.try_lock().map_err(|_| Refusal::Busy)?;
            let runtime = slot
                .as_mut()
                .filter(|runtime| runtime.matches_selection(selection))
                .ok_or(Refusal::Stale)?;
            let data = runtime.registration_data().ok_or(Refusal::Unavailable)?;
            let node = runtime.node.as_mut().ok_or(Refusal::Unavailable)?;
            let (head, _, epoch) = node
                .selected_join_data()
                .map_err(|_| Refusal::Unavailable)?;
            if node.original_join_data() != data.selected_data() || response.parent != (head, epoch)
            {
                return Err(Refusal::Stale);
            }
            let metadata = self.original.work.as_ref().ok_or(Refusal::Unavailable)?;
            metadata.with_registered_joined_member(
                data,
                runtime.registration.as_deref(),
                selection.original.row,
                |_, store| {
                    let mount = store
                        .member_outbox_mount()
                        .map_err(|_| Refusal::Unavailable)?;
                    let _fee = mount
                        .pay(crate::store::persistence::v2_member_outbox::MEMBER_OUTBOX_LOOKUP_FEE)
                        .map_err(|_| Refusal::Busy)?;
                    mount
                        .require_context_quiescent(
                            data.member_outbox_context_data()
                                .map_err(|_| Refusal::Unavailable)?
                                .id(),
                        )
                        .map_err(|_| Refusal::Busy)?;
                    if let Some(member) = &runtime.member {
                        member
                            .require_quiescent_for_add()
                            .map_err(|_| Refusal::Busy)?;
                    }
                    require_live()?;
                    let signal = caller.clone().with_authority(Arc::new(ApplyScope {
                        response: response.signal.clone(),
                        original: response.original.clone(),
                    }));
                    let result = if runtime.recovered.is_some() {
                        store.cover_recovered_member_add(node, &response.frame, &signal, rng)
                    } else {
                        pending
                            .io
                            .as_mut()
                            .ok_or(Refusal::Unavailable)?
                            .cover_member_add(store, node, &response.frame, &signal, rng)
                    };
                    retired = Some(result.map_err(|error| match error {
                        AppError::Io(_) => Refusal::Busy,
                        _ => Refusal::Failed,
                    })?);
                    mount.require_live(store).map_err(|_| Refusal::Cancelled)?;
                    let (head, _, epoch) = node
                        .selected_join_data()
                        .map_err(|_| Refusal::Unavailable)?;
                    Ok((head, epoch))
                },
            )?
        };
        drop(retired);
        self.original
            .require_current(caller)
            .map_err(|_| Refusal::Cancelled)?;
        Ok(selected)
    }
}

struct ApplyScope {
    response: RequestCancellation,
    original: NativeChatMemberDependencyObservation,
}

impl std::fmt::Debug for ApplyScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTextJoinedAddApplyScope")
    }
}

#[async_trait::async_trait]
impl RequestAuthority for ApplyScope {
    fn authorized(&self) -> bool {
        !self.response.is_cancelled() && self.original.is_current_cached()
    }

    async fn revoked(&self) {
        let mut response = self.response.clone();
        tokio::select! {
            _ = response.cancelled() => {},
            _ = self.original.revoked() => {},
        }
    }
}
