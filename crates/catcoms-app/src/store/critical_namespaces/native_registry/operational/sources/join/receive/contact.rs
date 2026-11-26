//! Share one original joined listener or connect to a current member with explicit IP approval.
use super::*;
use catcoms_sync::{V2MemberContactData, V2_MEMBER_CONTACT_BYTES};
use std::net::IpAddr;

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    /// Sign the original attached listener under the exact selected joined row.
    pub fn sign_member_contact(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: &RequestCancellation,
    ) -> Result<[u8; V2_MEMBER_CONTACT_BYTES], V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let mut ends = NativeChatKeyEnds::new();
        self.original
            .require_current(caller)
            .map_err(|_| Refusal::Cancelled)?;
        if !selection.is_current_cached() {
            return Err(Refusal::Stale);
        }
        let tcp = self.original.tcp.upgrade().ok_or(Refusal::Unavailable)?;
        let (peer, port) = tcp
            .attached_listener_data()
            .map_err(|_| Refusal::Unavailable)?;
        let original = self
            .original
            .pending
            .upgrade()
            .ok_or(Refusal::Unavailable)?;
        let mounted = self.mounted.upgrade().ok_or(Refusal::Unavailable)?;
        let metadata = self.original.work.as_ref().ok_or(Refusal::Unavailable)?;
        let result = {
            let mut pending = original.state.try_lock().map_err(|_| Refusal::Busy)?;
            let mut slot = mounted.try_lock().map_err(|_| Refusal::Busy)?;
            let runtime = slot
                .as_mut()
                .filter(|value| value.matches_selection(selection))
                .ok_or(Refusal::Stale)?;
            let data = runtime.registration_data().ok_or(Refusal::Unavailable)?;
            if data.transport_data() != (PeerId::new(*blake3::hash(&peer).as_bytes()), port) {
                return Err(Refusal::Stale);
            }
            let recovered = runtime.recovered;
            if recovered.is_some() {
                original
                    .ensure_native_owner(
                        &mut pending,
                        &self.original.command,
                        &self.original._control,
                    )
                    .map_err(|_| Refusal::Unavailable)?;
                if !selection.is_current_cached() {
                    original.close_keys();
                    return Err(Refusal::Stale);
                }
            }
            let owner = pending.owner.as_ref().ok_or(Refusal::Unavailable)?;
            let native = owner.observe();
            let mut cut = owner
                .prepare_cut(
                    &self.original.command.ui.command,
                    NativeChatKeyPurpose::MemberContact,
                )
                .map_err(|_| Refusal::Unavailable)?;
            let node = runtime.node.as_mut().ok_or(Refusal::Unavailable)?;
            metadata.with_registered_joined_member(
                data,
                runtime.registration.as_deref(),
                selection.original.row,
                |_, store| {
                    let recovered = recovered
                        .map(|data| store.capture_recovered_member_prefix(data))
                        .transpose()
                        .map_err(|_| Refusal::Unavailable)?;
                    let prefix = if let Some(recovered) = &recovered {
                        let (prefix, _) = recovered.member_parts();
                        node.bind_recovered_contact_source(prefix, &native, &cut)
                            .map_err(|_| Refusal::Unavailable)?;
                        prefix
                    } else {
                        pending
                            .io
                            .as_ref()
                            .ok_or(Refusal::Unavailable)?
                            .borrow_warm_member_prefix(store)
                            .map_err(|_| Refusal::Unavailable)?
                            .0
                    };
                    self.original
                        .require_current(caller)
                        .map_err(|_| Refusal::Cancelled)?;
                    node.sign_member_contact_native(
                        prefix,
                        (&peer, port),
                        (&native, &mut cut, &mut ends),
                    )
                    .map_err(|_| Refusal::Failed)
                },
            )?
        };
        self.original
            .require_current(caller)
            .map_err(|_| Refusal::Cancelled)?;
        if !selection.is_current_cached() {
            return Err(Refusal::Stale);
        }
        Ok(result)
    }

    /// Require the exact signed current contact and a separately approved IP.
    pub fn prepare_member_contact(
        self,
        selection: &NativeTextJoinedRowSelection,
        raw: &[u8],
        approved_host: IpAddr,
        caller: RequestCancellation,
    ) -> Result<super::super::reconnect::NativeTextJoinedReconnect, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let contact = V2MemberContactData::decode(raw).map_err(|_| Refusal::Stale)?;
        let (peer, port) = contact.address_data();
        let (group, observed, authentication, mount, fee) =
            self.with_member_receiver(selection, &caller, |source| {
                source
                    .node
                    .verify_member_contact(source.prefix, &contact)
                    .map_err(|_| Refusal::Stale)?;
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
        let (peer, address) = catcoms_net::native_chat_direct_tcp_route(peer, approved_host, port)
            .ok_or(Refusal::Stale)?;
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
            super::super::reconnect::NativeTextJoinedReconnect::from_selected_member(
                invocation,
                transport,
                ApprovedContact::from_selected_member(peer, group, address),
                (signal, mount),
                fee,
                self.original.end.ok_or(Refusal::Unavailable)?,
                super::super::reconnect::JoinedContactCandidate {
                    row: selection.clone(),
                    contact: ApprovedMemberPeer {
                        peer,
                        admission: contact.admission_data(),
                    },
                    digest: blake3::hash(raw),
                    host: approved_host,
                    restored: false,
                },
            ),
        )
    }
}
