//! Affine capture for the first accepted request on a mounted native vault.
use super::super::governed::{
    connect_approved_host, receive_connected_join, ConnectedJoinDelivery, JoinFramePayment,
    NativeTextJoinedStorage,
};
use super::super::join_tcp;
use super::super::native_ui::UiEffectNegative;
use super::super::upgrade_source::owning::{
    AuthenticationNegative, JoinedConnectionPreparation, MetadataWork,
};
use super::*;
use crate::store::persistence::governance_layout::joined::{
    AcceptedFirstJoin, AcceptedJoinTransport, JoinedPendingIo, JoinedRegistrationDependency,
};
use crate::store::{
    NativeTextInvitePacketData, NativeTextJoinedRegistrationData, NativeTextJoinedRegistryRowData,
};
use catcoms_replication::chat::join_bindings::GenesisAnchorBindingData;
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_rt::{
    Clock, NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatSourceClose, NativeChatUiCommand,
    PeerId, RequestAuthority, RequestCancellation,
};
use catcoms_sync::{
    ClosedV2JoinedNode, NativeV2JoinKeyEntry, V2JoinedRuntimeInputs, V2PendingJoinContextData,
};
use std::{
    sync::{OnceLock, Weak},
    time::Duration,
};
use tokio::sync::watch;

#[derive(Default)]
pub(super) struct PendingJoin {
    state: Mutex<PendingJoinState>,
    close: OnceLock<NativeChatSourceClose>,
}

#[derive(Default)]
struct PendingJoinState {
    // The original graph/provider drains before the source counter is destroyed.
    io: Option<JoinedPendingIo>,
    owner: Option<NativeChatSourceOwner>,
}

impl PendingJoin {
    pub(super) fn close_keys(&self) {
        if let Some(close) = self.close.get() {
            close.close();
        }
    }
}
pub(super) type MountedJoin<R> = Mutex<Option<MountedNode<R>>>;

pub(super) fn invoice<R: CryptoRngCore>() -> usize {
    2 * std::mem::size_of::<NativeTextJoinedRequestPreparation>()
        + 2 * std::mem::size_of::<NativeTextJoinedDelivery>()
        + std::mem::size_of::<PendingJoin>()
        + 2 * std::mem::size_of::<NativeTextJoinedInstallation<R>>()
        + 2 * std::mem::size_of::<NativeTextJoinedSelectionPreparation<R>>()
        + std::mem::size_of::<MountedJoin<R>>()
        + std::mem::size_of::<join_tcp::Transport>()
        + 2 * std::mem::size_of::<NativeChatKeyEnds>()
        + 4096 // original joined Source counter, cuts, and their Arc allocations
        + 6 * std::mem::size_of::<usize>()
}

/// An original UI command and finite metadata slot. Keep the entire capture
/// outside native guards during IO or destruction, including on refusal.
pub struct NativeTextJoinedRequestPreparation {
    scope: Arc<Scope>,
    pending: Weak<PendingJoin>,
    tcp: Weak<join_tcp::Transport>,
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
            tcp: Arc::downgrade(&self.joined_tcp),
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
        let mut ends = NativeChatKeyEnds::new();
        let work = self.work.as_ref().ok_or_else(refused)?;
        let require_current = || self.require_current(negative);
        require_current()?;
        let original = self.pending.upgrade().ok_or_else(refused)?;
        let mut pending = original.state.try_lock().map_err(|_| refused())?;
        if token == [0; 32]
            || anchor != &invite.binding().spec_data().anchor
            || (pending.io.is_none() && invite.binding().expired_at_data(now_ms))
        {
            return Err(refused());
        }
        if pending.owner.is_none() {
            require_current()?;
            let owner = NativeChatSourceOwner::for_trusted_source(
                &self.command.ui.command,
                self._control.clone(),
            )
            .map_err(|_| refused())?;
            original
                .close
                .set(owner.close_handle())
                .map_err(|_| refused())?;
            pending.owner = Some(owner);
            if !self.scope.is_open() {
                original.close_keys();
            }
            require_current()?;
        }
        let owner = pending.owner.as_ref().ok_or_else(refused)?;
        let source = owner.observe();
        let mut cut = owner
            .prepare_cut(&self.command.ui.command, NativeChatKeyPurpose::JoinerDevice)
            .map_err(|_| refused())?;
        let accepted = AcceptedFirstJoin {
            invite,
            anchor,
            token,
            now_ms,
            transport: AcceptedJoinTransport::Native(NativeV2JoinKeyEntry::new(
                &source, &mut cut, &mut ends,
            )),
        };
        work.cover_accepted_first_join(&mut pending.io, accepted, require_current, rng)
    }

    /// Reprove and copy the exact covered request outside native guards. The
    /// metadata slot and all locks leave before the connected call can start.
    pub fn prepare_exchange(
        self,
        invite: &NativeTextInvitePacketData<'_>,
        caller: RequestCancellation,
    ) -> Result<NativeTextJoinedExchange, AppError> {
        self.require_current(&caller)?;
        let (prepared, tcp) = {
            let original = self.pending.upgrade().ok_or_else(refused)?;
            let pending = original.state.try_lock().map_err(|_| refused())?;
            let pending = pending.io.as_ref().ok_or_else(refused)?;
            let metadata = self.work.as_ref().ok_or_else(refused)?;
            let prepared = metadata.prepare_join_connection(pending, invite)?;
            let tcp = self.tcp.upgrade().ok_or_else(refused)?.checkout(
                prepared.local_transport,
                || metadata.prepare_join_tcp(pending),
                || self.require_current(&caller),
            )?;
            (prepared, tcp)
        };
        self.require_current(&caller)?;
        let JoinedConnectionPreparation {
            request,
            mount,
            authentication,
            work,
            local_transport,
            fee,
        } = prepared;
        let (invocation, live) = watch::channel(false);
        let command = self.command.ui.command.clone();
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
            local_transport,
            signal: caller.with_authority(authority),
            command,
            tcp,
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
    local_transport: (PeerId, u16),
    tcp: join_tcp::Loan,
    signal: RequestCancellation,
    command: NativeChatUiCommand,
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
    /// Historical public local transport reservation covered by this Pending.
    pub fn local_transport_data(&self) -> (PeerId, u16) {
        self.local_transport
    }

    /// Contact one explicitly approved IP using the exact covered invitation.
    pub async fn connect_approved_host(
        &self,
        invite: &NativeTextInvitePacketData<'_>,
        approved_host: std::net::IpAddr,
        scheduler: &catcoms_discovery::EndpointDialScheduler,
    ) -> Result<(), AppError> {
        NativeTextJoinedStorage::require_invite_bytes(&self.request, invite)?;
        connect_approved_host(
            invite,
            approved_host,
            self.tcp.borrow()?,
            scheduler,
            self.signal.clone(),
            self.mount.clone(),
            self.payment.clone(),
        )
        .await
    }

    /// Use only the original invited transport identity. The actual network
    /// actor checks the same UI, mount and connection at request admission.
    pub async fn receive(self) -> Result<NativeTextJoinedDelivery, AppError> {
        let delivery = receive_connected_join(
            &self.request,
            self.target,
            self.tcp.borrow()?,
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
        let mut ends = NativeChatKeyEnds::new();
        let scope = &self.delivery.scope;
        if scope.is_cancelled() {
            return Err(refused());
        }
        let metadata = self.exchange.work.metadata().ok_or_else(refused)?;
        let original = self.exchange.pending.upgrade().ok_or_else(refused)?;
        let mut pending = original.state.try_lock().map_err(|_| refused())?;
        let PendingJoinState { io, owner } = &mut *pending;
        let owner = owner.as_ref().ok_or_else(refused)?;
        let source = owner.observe();
        let mut cut = owner
            .prepare_cut(
                &self.exchange.command,
                NativeChatKeyPurpose::JoinerWelcomeSdk,
            )
            .map_err(|_| refused())?;
        let outcome = metadata.cover_join_delivery(
            io.as_mut().ok_or_else(refused)?,
            &self.exchange.request,
            &self.delivery.bytes,
            scope,
            rng,
            NativeV2JoinKeyEntry::new(&source, &mut cut, &mut ends),
        )?;
        if scope.is_cancelled() {
            return Err(refused());
        }
        Ok(outcome)
    }
}

pub(super) struct MountedNode<R: CryptoRngCore> {
    node: Option<ClosedV2JoinedNode<catcoms_net::MeshService, R>>,
    registration: Option<Arc<JoinedRegistrationDependency>>,
    _payment: CatalogReadReservation,
}

/// The exact connected Welcome and the runtime inputs ready for mounted
/// attachment. A refusal before the graph moves keeps the original inputs.
pub struct NativeTextJoinedInstallation<R: CryptoRngCore> {
    // These inputs can contain the final original network owner.
    inputs: Option<V2JoinedRuntimeInputs<catcoms_net::MeshService, R>>,
    mounted: Weak<MountedJoin<R>>,
    delivery: ConnectedJoinDelivery,
    exchange: NativeTextJoinedExchange,
}

impl<R: CryptoRngCore> std::fmt::Debug for NativeTextJoinedInstallation<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextJoinedInstallation")
            .finish_non_exhaustive()
    }
}

impl<R: CryptoRngCore> NativeTextSources<catcoms_net::MeshService, R> {
    /// Capture under a short checked native guard. Inputs stay in the caller's
    /// outer slots on refusal, and no book/physical lock or IO occurs here.
    pub fn capture_join_installation(
        &self,
        delivery: &mut Option<NativeTextJoinedDelivery>,
        inputs: &mut Option<(R, Box<dyn Clock + Send>)>,
    ) -> Result<NativeTextJoinedInstallation<R>, AppError> {
        let original = delivery.as_ref().ok_or_else(refused)?;
        let pending = Arc::downgrade(&self.pending_join);
        if inputs.is_none() || !self.scope.is_open() || !original.exchange.pending.ptr_eq(&pending)
        {
            return Err(refused());
        }
        let transport = delivery.as_mut().ok_or_else(refused)?.exchange.tcp.take()?;
        let (rng, clock) = inputs.take().expect("checked original inputs");
        let NativeTextJoinedDelivery { delivery, exchange } =
            delivery.take().expect("checked original delivery");
        Ok(NativeTextJoinedInstallation {
            inputs: Some((transport, rng, clock)),
            mounted: Arc::downgrade(&self.mounted_join),
            delivery,
            exchange,
        })
    }
}

impl<R: CryptoRngCore> NativeTextJoinedInstallation<R> {
    /// Cover the Welcome, then park the one closed node beside its exact
    /// mounted Pending. Native row publication and signing remain separate.
    pub fn install(&mut self) -> Result<(OwnerHeaderId, OwnerFactId, u64), AppError> {
        let mut ends = NativeChatKeyEnds::new();
        let scope = &self.delivery.scope;
        if scope.is_cancelled() {
            return Err(refused());
        }
        let mut payment;
        let metadata = self.exchange.work.metadata().ok_or_else(refused)?;
        let original = self.exchange.pending.upgrade().ok_or_else(refused)?;
        let mut stored = original.state.try_lock().map_err(|_| refused())?;
        let PendingJoinState { io, owner } = &mut *stored;
        let pending = io.as_mut().ok_or_else(refused)?;
        let mounted = self.mounted.upgrade().ok_or_else(refused)?;
        let mut slot = mounted.try_lock().map_err(|_| refused())?;
        let request = &self.exchange.request;

        if slot.is_none() {
            let (_, rng, _) = self.inputs.as_mut().ok_or_else(refused)?;
            let owner = owner.as_ref().ok_or_else(refused)?;
            let source = owner.observe();
            let mut cut = owner
                .prepare_cut(
                    &self.exchange.command,
                    NativeChatKeyPurpose::JoinerWelcomeSdk,
                )
                .map_err(|_| refused())?;
            metadata.cover_join_delivery(
                pending,
                request,
                &self.delivery.bytes,
                scope,
                rng,
                NativeV2JoinKeyEntry::new(&source, &mut cut, &mut ends),
            )?;
            if scope.is_cancelled() {
                return Err(refused());
            }
            payment = Some(
                metadata.reserve_join_node(
                    ClosedV2JoinedNode::<catcoms_net::MeshService, R>::allocation_bytes(),
                )?,
            );
            let mut node = None;
            let result = metadata.attach_join_node(pending, &mut self.inputs, &mut node);
            // A late storage or session refusal must leave the actual graph
            // with the mounted owner after the one-time move.
            if node.is_some() {
                *slot = Some(MountedNode {
                    node,
                    registration: None,
                    _payment: payment.take().unwrap(),
                });
                self.exchange.tcp.attached();
            }
            result?;
        }

        let node = slot
            .as_ref()
            .and_then(|runtime| runtime.node.as_ref())
            .ok_or_else(refused)?;
        let (head, admission, epoch) = node.selected_join_data().map_err(|_| refused())?;
        if scope.is_cancelled() || metadata.warm_join_outcome(pending, request)? != (head, epoch) {
            return Err(refused());
        }
        if scope.is_cancelled() {
            return Err(refused());
        }
        Ok((head, admission, epoch))
    }
}

impl<R: CryptoRngCore> Drop for NativeTextJoinedInstallation<R> {
    fn drop(&mut self) {
        if let Some((transport, _, _)) = self.inputs.take() {
            self.exchange.tcp.put(transport);
        }
    }
}

/// Fresh native authorization to inspect an already parked joined runtime.
pub struct NativeTextJoinedSelectionPreparation<R: CryptoRngCore> {
    mounted: Weak<MountedJoin<R>>,
    original: NativeTextJoinedRequestPreparation,
}

impl<R: CryptoRngCore> std::fmt::Debug for NativeTextJoinedSelectionPreparation<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextJoinedSelectionPreparation")
            .finish_non_exhaustive()
    }
}

impl<R: CryptoRngCore> NativeTextSources<catcoms_net::MeshService, R> {
    /// Capture current native UI and original physical custody without IO.
    pub fn capture_joined_selection(
        &self,
        command: NativeTextSourceCommand,
    ) -> NativeTextJoinedSelectionPreparation<R> {
        NativeTextJoinedSelectionPreparation {
            mounted: Arc::downgrade(&self.mounted_join),
            original: self.capture_join_request(command),
        }
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    /// Select the native metadata row for the exact parked original. This
    /// commits local registration while joined authoring remains closed.
    pub fn register(
        self,
        invite: &NativeTextInvitePacketData<'_>,
        token: [u8; 32],
        name: String,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedRegistryRowData, AppError> {
        self.original.require_current(negative)?;
        let (receipt, debt) = {
            let original = self.original.pending.upgrade().ok_or_else(refused)?;
            let pending = original.state.try_lock().map_err(|_| refused())?;
            let pending = pending.io.as_ref().ok_or_else(refused)?;
            pending.require_original_join_intent(invite.binding(), token)?;
            let mounted = self.mounted.upgrade().ok_or_else(refused)?;
            let mut slot = mounted.try_lock().map_err(|_| refused())?;
            let runtime = slot.as_mut().ok_or_else(refused)?;
            let node = runtime.node.as_ref().ok_or_else(refused)?;
            let selected = node.selected_join_data().map_err(|_| refused())?;
            let metadata = self.original.work.as_ref().ok_or_else(refused)?;
            if runtime.registration.is_none() {
                runtime.registration = Some(metadata.capture_join_registration(pending)?);
            }
            let proof = runtime.registration.as_ref().ok_or_else(refused)?;
            if proof.data().selected_data() != selected || proof.data().invocation != token {
                return Err(refused());
            }
            self.original.require_current(negative)?;
            metadata.register_joined(name, proof, rng)?
        };
        drop(debt);
        self.original.require_current(negative)?;
        Ok(receipt)
    }

    /// Reprove the original invite/request, mounted files and closed node
    /// after a lost reply. The original founder connection may have retired.
    pub fn inspect(
        self,
        invite: &NativeTextInvitePacketData<'_>,
        token: [u8; 32],
        negative: &RequestCancellation,
    ) -> Result<Option<(OwnerHeaderId, OwnerFactId, u64)>, AppError> {
        Ok(self
            .inspect_registration(invite, token, negative)?
            .map(|value| value.selected_data()))
    }

    /// Check all immutable registration fields for this same parked node
    /// under fresh authorization. The result is authenticated history.
    pub fn inspect_registration(
        self,
        invite: &NativeTextInvitePacketData<'_>,
        token: [u8; 32],
        negative: &RequestCancellation,
    ) -> Result<Option<NativeTextJoinedRegistrationData>, AppError> {
        self.original.require_current(negative)?;
        let original = self.original.pending.upgrade().ok_or_else(refused)?;
        let pending = original.state.try_lock().map_err(|_| refused())?;
        let mounted = self.mounted.upgrade().ok_or_else(refused)?;
        let slot = mounted.try_lock().map_err(|_| refused())?;
        let result = if let Some(runtime) = slot.as_ref() {
            let node = runtime.node.as_ref().ok_or_else(refused)?;
            let (head, admission, epoch) = node.selected_join_data().map_err(|_| refused())?;
            let metadata = self.original.work.as_ref().ok_or_else(refused)?;
            let receipt = metadata.observe_join_installation(
                pending.io.as_ref().ok_or_else(refused)?,
                invite,
                token,
            )?;
            if receipt.selected_data() != (head, admission, epoch) {
                return Err(refused());
            }
            Some(receipt)
        } else {
            None
        };
        self.original.require_current(negative)?;
        Ok(result)
    }
}
