//! One explicitly approved connection on an existing joined actor.
catcoms_rt::redacted_debug! {
    NativeTextJoinedReconnect;
    NativeTextJoinedContactApprovalProof;
}

use super::*;
use catcoms_discovery::EndpointDialScheduler;
use std::net::IpAddr;
use std::time::Duration;

/// Establish one authenticated transport connection for an existing row.
pub struct NativeTextJoinedReconnect {
    _invocation: watch::Sender<bool>,
    transport: catcoms_net::MeshConnectionDial,
    contact: ApprovedContact,
    signal: RequestCancellation,
    mount: RequestCancellation,
    payment: Arc<JoinFramePayment>,
    member: Option<JoinedContactCandidate>,
    _end: retirement::PreparationEnd,
}

pub(super) struct JoinedContactCandidate {
    pub(super) row: NativeTextJoinedRowSelection,
    pub(super) contact: ApprovedMemberPeer,
    pub(super) digest: blake3::Hash,
    pub(super) host: IpAddr,
    pub(super) restored: bool,
}

/// Result of the original approved connection. An inbound path can prove
/// identity approval. Only the exact authenticated outbound path can
/// supply a host to reuse during a later native startup.
pub struct NativeTextJoinedContactApprovalProof {
    pub(super) candidate: JoinedContactCandidate,
    pub(super) connection: catcoms_rt::ConnectionLease,
    pub(super) outbound: bool,
    pub(super) consent: NativeTextJoinedContactConsent,
    _payment: Arc<JoinFramePayment>,
}

impl NativeTextJoinedReconnect {
    pub(super) fn as_restored(mut self) -> Self {
        if let Some(candidate) = self.member.as_mut() {
            candidate.restored = true;
        }
        self
    }
    pub(super) fn from_selected_member(
        invocation: watch::Sender<bool>,
        transport: catcoms_net::MeshConnectionDial,
        contact: ApprovedContact,
        (signal, mount): (RequestCancellation, RequestCancellation),
        fee: CatalogReadReservation,
        end: retirement::PreparationEnd,
        member: JoinedContactCandidate,
    ) -> Self {
        Self {
            _invocation: invocation,
            transport,
            contact,
            signal,
            mount,
            payment: Arc::new(JoinFramePayment { _fee: fee }),
            member: Some(member),
            _end: end,
        }
    }

    pub async fn connect(self, scheduler: &EndpointDialScheduler) -> Result<(), AppError> {
        self.establish(scheduler, None).await.map(|_| ())
    }

    pub async fn connect_for_contact_approval(
        mut self,
        scheduler: &EndpointDialScheduler,
        consent: NativeTextJoinedContactConsent,
    ) -> Result<NativeTextJoinedContactApprovalProof, AppError> {
        if self
            .member
            .as_ref()
            .is_none_or(|candidate| candidate.restored || !consent.is_current_for(&candidate.row))
        {
            return Err(refused());
        }
        self.signal = consent.constrain(self.signal);
        self.establish(scheduler, Some(consent))
            .await?
            .ok_or_else(refused)
    }

    async fn establish(
        self,
        scheduler: &EndpointDialScheduler,
        consent: Option<NativeTextJoinedContactConsent>,
    ) -> Result<Option<NativeTextJoinedContactApprovalProof>, AppError> {
        let (peer, address) = self.contact.target_data();
        let route = consent.as_ref().map(|_| address.to_owned());
        connect_approved_host(
            self.contact,
            &self.transport,
            scheduler,
            self.signal.clone(),
            self.mount.clone(),
            self.payment.clone(),
        )
        .await?;
        let Some(candidate) = self.member else {
            return Ok(None);
        };
        if self.signal.is_cancelled() || self.mount.is_cancelled() {
            return Err(refused());
        }
        let Some(route) = route else {
            if candidate.restored {
                candidate
                    .row
                    .install_restored_peer(candidate.contact, &self.signal)?;
            } else {
                candidate.row.remember_governing_peer(candidate.contact)?;
            }
            return Ok(None);
        };
        let original = self.transport.connection_lease(peer).ok_or_else(refused)?;
        let mut signal = self.signal.clone();
        let mut mount = self.mount.clone();
        let checked = tokio::select! {
            biased;
            _ = signal.cancelled() => return Err(refused()),
            _ = mount.cancelled() => return Err(refused()),
            proof = tokio::time::timeout(Duration::from_secs(3), self.transport.authenticated_direct_route(peer, &route)) => {
                proof.map_err(|_| refused())?.map_err(|_| refused())?
            }
        };
        if !original.is_live()
            || self
                .transport
                .connection_lease(peer)
                .is_none_or(|current| !current.is_same(&original))
            || self.signal.is_cancelled()
            || self.mount.is_cancelled()
        {
            return Err(refused());
        }
        Ok(Some(NativeTextJoinedContactApprovalProof {
            candidate,
            consent: consent.ok_or_else(refused)?,
            outbound: checked.is_some_and(|route| route.is_same(&original)),
            connection: original,
            _payment: self.payment,
        }))
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    /// Reprove the stored invitation and selected row before preparing the
    /// dial. The asynchronous attempt holds no registry or metadata slot.
    pub fn prepare_reconnect(
        self,
        selection: &NativeTextJoinedRowSelection,
        invite: &NativeTextInvitePacketData<'_>,
        token: [u8; 32],
        approved_host: IpAddr,
        caller: RequestCancellation,
    ) -> Result<NativeTextJoinedReconnect, AppError> {
        self.original.require_current(&caller)?;
        let invite_id = invite.binding().id_data();
        let (transport, authentication, mount, fee) = {
            let metadata = self.original.work.as_ref().ok_or_else(refused)?;
            let pending = self.original.pending.upgrade().ok_or_else(refused)?;
            let _keys = pending.state.try_lock().map_err(|_| refused())?;
            let mounted = self.mounted.upgrade().ok_or_else(refused)?;
            let slot = mounted.try_lock().map_err(|_| refused())?;
            let runtime = slot
                .as_ref()
                .filter(|runtime| runtime.matches_selection(selection))
                .ok_or_else(refused)?;
            let data = runtime.registration_data().ok_or_else(refused)?;
            if !selection.is_current_cached()
                || data.invocation != token
                || data.invite != invite_id
            {
                return Err(refused());
            }
            let (row, names) = runtime.observe_selected(metadata, data)?;
            if row != selection.original.row
                || names.group != selection.name_data()
                || names.display.as_deref() != selection.display_name_data()
            {
                return Err(refused());
            }
            let (mount, fee) = metadata.joined_transport_resources(16 * 1024)?;
            let authentication = metadata.authentication_negative()?;
            let transport = self
                .original
                .tcp
                .upgrade()
                .ok_or_else(refused)?
                .capture_attached_dial()?;
            let founder = invite.connected_peer_data();
            if *runtime.founder_contact.get_or_init(|| founder) != founder {
                return Err(refused());
            }
            (transport, authentication, mount, fee)
        };
        self.original.require_current(&caller)?;
        let (invocation, live) = watch::channel(false);
        let authority = Arc::new(JoinEffect {
            scope: self.original.scope,
            ui: self.original.command.ui.into_effect_negative(),
            authentication,
            invocation: live,
            caller: caller.clone(),
            row: Some(selection.clone()),
        });
        let signal = caller.with_authority(authority);
        if signal.is_cancelled() || mount.is_cancelled() {
            return Err(refused());
        }
        Ok(NativeTextJoinedReconnect {
            _invocation: invocation,
            transport,
            contact: ApprovedContact::from_invite(invite, approved_host),
            signal,
            mount,
            payment: Arc::new(JoinFramePayment { _fee: fee }),
            member: None,
            _end: self.original.end.ok_or_else(refused)?,
        })
    }
}
