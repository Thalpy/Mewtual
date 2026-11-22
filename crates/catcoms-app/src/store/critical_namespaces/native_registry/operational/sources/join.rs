//! Affine capture for the first accepted request on a mounted native vault.
use super::super::governed::{
    connect_approved_host, receive_connected_join, ConnectedJoinDelivery, JoinFramePayment,
    NativeTextJoinedStorage,
};
use super::super::native_ui::UiEffectNegative;
use super::super::upgrade_source::owning::{
    AuthenticationNegative, JoinedConnectionPreparation, MetadataWork,
};
use super::*;
use crate::store::persistence::governance_layout::joined::{AcceptedFirstJoin, JoinedPendingIo};
use crate::store::NativeTextInvitePacketData;
use catcoms_replication::chat::join_bindings::GenesisAnchorBindingData;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_rt::{PeerId, RequestAuthority, RequestCancellation};
use catcoms_sync::V2PendingJoinContextData;
use std::{sync::Weak, time::Duration};
use tokio::sync::watch;

pub(super) type PendingJoin = Mutex<Option<JoinedPendingIo>>;

pub(super) fn invoice() -> usize {
    2 * std::mem::size_of::<NativeTextJoinedRequestPreparation>()
        + 2 * std::mem::size_of::<NativeTextJoinedDelivery>()
        + std::mem::size_of::<PendingJoin>()
        + 2 * std::mem::size_of::<usize>()
}

/// An original UI command and finite metadata slot. Keep the entire capture
/// outside native guards during IO or destruction, including on refusal.
pub struct NativeTextJoinedRequestPreparation {
    scope: Arc<Scope>,
    pending: Weak<PendingJoin>,
    command: NativeTextSourceCommand,
    work: Option<MetadataWork>,
    end: Option<retirement::PreparationEnd>,
    _control: Arc<retirement::Control>,
}

impl std::fmt::Debug for NativeTextJoinedRequestPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextJoinedRequestPreparation")
            .finish_non_exhaustive()
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextSources<T, R> {
    /// Capture the actual mounted original without locking its book or opening
    /// files. A refused capture still owns the caller's UI command and fee.
    pub fn capture_join_request(
        &self,
        command: NativeTextSourceCommand,
    ) -> NativeTextJoinedRequestPreparation {
        let end = self.retirement.enter_preparation();
        let work = if end.is_some() && self.require_command(&command).is_ok() {
            self.work.metadata()
        } else {
            None
        };
        NativeTextJoinedRequestPreparation {
            scope: self.scope.clone(),
            pending: Arc::downgrade(&self.pending_join),
            command,
            work,
            end,
            _control: self.retirement.clone(),
        }
    }
}

impl NativeTextJoinedRequestPreparation {
    fn require_current(&self, negative: &RequestCancellation) -> Result<(), AppError> {
        if self.end.is_none()
            || !Arc::ptr_eq(&self.scope, &self.command.scope)
            || !self.scope.is_open()
            || !self.command.ui.is_current_cached()
            || self.work.as_ref().is_none_or(|work| !work.is_live_cached())
            || negative.is_cancelled()
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Cover the original request, leaving the affine warm Pending with the
    /// mount for a later exact retry. This receipt carries historical data.
    pub fn cover(
        self,
        invite: &NativeTextInvitePacketData<'_>,
        anchor: &GenesisAnchorBindingData,
        token: [u8; 32],
        now_ms: u64,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<([u8; 32], V2PendingJoinContextData), AppError> {
        let work = self.work.as_ref().ok_or_else(refused)?;
        let require_current = || self.require_current(negative);
        require_current()?;
        let original = self.pending.upgrade().ok_or_else(refused)?;
        let mut pending = original.try_lock().map_err(|_| refused())?;
        let accepted = AcceptedFirstJoin {
            invite,
            anchor,
            token,
            now_ms,
        };
        work.cover_accepted_first_join(&mut pending, accepted, require_current, rng)
    }

    /// Reprove and copy the exact covered request outside native guards. The
    /// metadata slot and all locks leave before the connected call can start.
    pub fn prepare_exchange(
        self,
        invite: &NativeTextInvitePacketData<'_>,
        caller: RequestCancellation,
    ) -> Result<NativeTextJoinedExchange, AppError> {
        self.require_current(&caller)?;
        let prepared = {
            let original = self.pending.upgrade().ok_or_else(refused)?;
            let pending = original.try_lock().map_err(|_| refused())?;
            self.work
                .as_ref()
                .ok_or_else(refused)?
                .prepare_join_connection(pending.as_ref().ok_or_else(refused)?, invite)?
        };
        self.require_current(&caller)?;
        let JoinedConnectionPreparation {
            request,
            mount,
            authentication,
            work,
            fee,
        } = prepared;
        let (invocation, live) = watch::channel(false);
        let authority = Arc::new(JoinEffect {
            scope: self.scope,
            ui: self.command.ui.into_effect_negative(),
            authentication,
            invocation: live,
            caller: caller.clone(),
        });
        Ok(NativeTextJoinedExchange {
            request,
            target: invite.connected_peer_data(),
            signal: caller.with_authority(authority),
            mount,
            work,
            pending: self.pending,
            _invocation: invocation,
            _end: self.end.ok_or_else(refused)?,
            payment: Arc::new(JoinFramePayment { _fee: fee }),
        })
    }
}

struct JoinEffect {
    scope: Arc<Scope>,
    ui: UiEffectNegative,
    authentication: AuthenticationNegative,
    invocation: watch::Receiver<bool>,
    caller: RequestCancellation,
}

impl std::fmt::Debug for JoinEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinEffect").finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl RequestAuthority for JoinEffect {
    fn authorized(&self) -> bool {
        self.invocation.has_changed().is_ok()
            && !self.caller.is_cancelled()
            && self.scope.is_open()
            && self.ui.is_current_cached()
            && self.authentication.is_live_cached()
    }

    async fn revoked(&self) {
        let mut invocation = self.invocation.clone();
        let mut caller = self.caller.clone();
        // The independent original UI/mount negatives can change without a
        // watch publication. At most the two admitted preparations poll them.
        while self.authorized() {
            tokio::select! {
                _ = invocation.changed() => return,
                _ = caller.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_millis(25)) => {},
            }
        }
    }
}

/// One exact mounted request ready to use an already authenticated connection.
pub struct NativeTextJoinedExchange {
    _invocation: watch::Sender<bool>,
    request: Zeroizing<Vec<u8>>,
    target: PeerId,
    signal: RequestCancellation,
    mount: RequestCancellation,
    work: SourceWork,
    pending: Weak<PendingJoin>,
    _end: retirement::PreparationEnd,
    payment: Arc<JoinFramePayment>,
}

impl std::fmt::Debug for NativeTextJoinedExchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextJoinedExchange")
            .finish_non_exhaustive()
    }
}

impl NativeTextJoinedExchange {
    /// Contact one explicitly approved IP using the exact covered invitation.
    pub async fn connect_approved_host(
        &self,
        invite: &NativeTextInvitePacketData<'_>,
        approved_host: std::net::IpAddr,
        transport: &catcoms_net::MeshService,
        scheduler: &catcoms_discovery::EndpointDialScheduler,
    ) -> Result<(), AppError> {
        NativeTextJoinedStorage::require_invite_bytes(&self.request, invite)?;
        connect_approved_host(
            invite,
            approved_host,
            transport,
            scheduler,
            self.signal.clone(),
            self.mount.clone(),
            self.payment.clone(),
        )
        .await
    }

    /// Use only the original invited transport identity. The actual network
    /// actor checks the same UI, mount and connection at request admission.
    pub async fn receive(
        self,
        transport: &impl MeshTransport,
    ) -> Result<NativeTextJoinedDelivery, AppError> {
        let delivery = receive_connected_join(
            &self.request,
            self.target,
            transport,
            self.signal.clone(),
            self.mount.clone(),
            self.payment.clone(),
        )
        .await?;
        Ok(NativeTextJoinedDelivery {
            delivery,
            exchange: self,
        })
    }
}

/// An original connected delivery bound to the mounted warm Pending and
/// invocation. The eventual historical result grants no member lane.
pub struct NativeTextJoinedDelivery {
    delivery: ConnectedJoinDelivery,
    exchange: NativeTextJoinedExchange,
}

impl std::fmt::Debug for NativeTextJoinedDelivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextJoinedDelivery")
            .finish_non_exhaustive()
    }
}

impl NativeTextJoinedDelivery {
    /// Cover Welcome outside native guards only while the original connection,
    /// invocation and mounted files still authorize the original request.
    pub fn cover(self, rng: &mut impl CryptoRngCore) -> Result<(OwnerHeaderId, u64), AppError> {
        let scope = &self.delivery.scope;
        if scope.is_cancelled() {
            return Err(refused());
        }
        let original = self.exchange.pending.upgrade().ok_or_else(refused)?;
        let mut pending = original.try_lock().map_err(|_| refused())?;
        let metadata = self.exchange.work.metadata().ok_or_else(refused)?;
        let outcome = metadata.cover_join_delivery(
            pending.as_mut().ok_or_else(refused)?,
            &self.exchange.request,
            &self.delivery.bytes,
            scope,
            rng,
        )?;
        if scope.is_cancelled() {
            return Err(refused());
        }
        Ok(outcome)
    }
}
